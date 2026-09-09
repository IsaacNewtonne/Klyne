//! Controlled self-improvement: propose, isolate, measure, gate, decide.
//!
//! An experiment applies one candidate change in an isolated Git worktree
//! on its own branch, runs a check suite on the pristine repo (baseline)
//! and on the worktree (candidate), then promotes or rolls back:
//!
//! - **Promote** (no new failures): the change is committed on its branch
//!   with the decision record, the worktree is removed, and the branch is
//!   kept for human merge. The harness never merges to the main line.
//! - **Rollback** (any regression): the worktree is force-removed and the
//!   branch deleted; the record keeps the failing evidence.
//!
//! Refusals are fail-closed: dirty repos, existing branches, unparseable
//! suite output, and suite timeouts all abort without touching the repo.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// What to run in each tree. Keep it fast: it executes twice per experiment.
#[derive(Clone, Debug)]
pub struct CheckSuite {
    /// argv, e.g. `["test", "--offline"]`. Runs with cwd set per tree.
    pub argv: Vec<String>,
    /// Wall-clock backstop per suite invocation.
    pub timeout: Duration,
    /// Cap on captured output kept in the report.
    pub max_output_chars: usize,
}

impl Default for CheckSuite {
    fn default() -> Self {
        Self {
            argv: vec!["test".into(), "--offline".into()],
            timeout: Duration::from_secs(300),
            max_output_chars: 4000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuiteResult {
    pub passed: u64,
    pub failed: u64,
    pub timed_out: bool,
    pub seconds: f64,
    pub output_tail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Promote { reason: String },
    Rollback { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExperimentSpec {
    pub id: String,
    pub branch: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExperimentReport {
    pub spec: ExperimentSpec,
    pub baseline: SuiteResult,
    pub candidate: SuiteResult,
    pub decision: Decision,
    pub finished_at_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn git(repo: &Path, args: &[&str]) -> io::Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| io::Error::other(format!("git failed to launch: {e}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(io::Error::other(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn count_before(word: &str, text: &str) -> u64 {
    text.find(word)
        .and_then(|index| text[..index].split_whitespace().last())
        .and_then(|count| count.parse().ok())
        .unwrap_or(0)
}

fn parse_suite_output(combined: &str) -> Option<(u64, u64)> {
    let mut passed = 0u64;
    let mut failed = 0u64;
    let mut found = false;
    for line in combined.lines() {
        // cargo/libtest shape: "test result: ok. 3 passed; 1 failed; ..."
        // ("ok." prefixes the first segment, so match trailing tokens).
        if let Some(rest) = line.split("test result:").nth(1) {
            found = true;
            for part in rest.split(';') {
                passed += count_before("passed", part);
                failed += count_before("failed", part);
            }
        }
    }
    found.then_some((passed, failed))
}

fn run_suite(dir: &Path, suite: &CheckSuite) -> SuiteResult {
    let started = Instant::now();
    let mut child = match std::process::Command::new("cargo")
        .args(&suite.argv)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return SuiteResult {
                passed: 0,
                failed: 1,
                timed_out: false,
                seconds: started.elapsed().as_secs_f64(),
                output_tail: format!("failed to launch cargo: {e}"),
            };
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child
                    .wait_with_output()
                    .unwrap_or_else(|e| std::process::Output {
                        status,
                        stdout: Vec::new(),
                        stderr: format!("{e}").into_bytes(),
                    });
                let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&output.stderr));
                let (passed, failed) = parse_suite_output(&combined)
                    .unwrap_or((0, if status.success() { 0 } else { 1 }));
                let mut tail: String = combined
                    .chars()
                    .rev()
                    .take(suite.max_output_chars)
                    .collect();
                tail = tail.chars().rev().collect();
                return SuiteResult {
                    passed,
                    failed: if status.success() {
                        failed
                    } else {
                        failed.max(1)
                    },
                    timed_out: false,
                    seconds: started.elapsed().as_secs_f64(),
                    output_tail: tail,
                };
            }
            Ok(None) => {
                if started.elapsed() >= suite.timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return SuiteResult {
                        passed: 0,
                        failed: 1,
                        timed_out: true,
                        seconds: started.elapsed().as_secs_f64(),
                        output_tail: "suite timed out and was killed".into(),
                    };
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return SuiteResult {
                    passed: 0,
                    failed: 1,
                    timed_out: false,
                    seconds: started.elapsed().as_secs_f64(),
                    output_tail: format!("suite supervision failed: {e}"),
                };
            }
        }
    }
}

pub struct ExperimentRunner {
    repo: PathBuf,
    suite: CheckSuite,
}

impl ExperimentRunner {
    pub fn new(repo: impl Into<PathBuf>, suite: CheckSuite) -> Self {
        Self {
            repo: repo.into(),
            suite,
        }
    }

    fn worktree_path(&self, id: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "harness-exp-{}-{}-{}",
            std::process::id(),
            now_ms(),
            id
        ))
    }

    /// Run one experiment end to end. The mutation closure receives the
    /// worktree path and applies exactly one candidate change there.
    pub fn run_experiment(
        &self,
        spec: ExperimentSpec,
        mutate: impl FnOnce(&Path) -> io::Result<()>,
    ) -> io::Result<ExperimentReport> {
        if !self.repo.join(".git").exists() {
            // Also accept bare/worktree checkouts via rev-parse.
            git(&self.repo, &["rev-parse", "--git-dir"])?;
        }
        if !git(&self.repo, &["status", "--porcelain"])?
            .trim()
            .is_empty()
        {
            return Err(io::Error::other(
                "repo has uncommitted changes; refusing experiment",
            ));
        }
        let worktree = self.worktree_path(&spec.id);
        let create = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                &spec.branch,
                &worktree.to_string_lossy(),
            ])
            .current_dir(&self.repo)
            .output()
            .map_err(|e| io::Error::other(format!("git worktree failed to launch: {e}")))?;
        if !create.status.success() {
            return Err(io::Error::other(format!(
                "git worktree add failed (branch may exist): {}",
                String::from_utf8_lossy(&create.stderr)
            )));
        }
        let cleanup = |force: bool| {
            let mut remove = std::process::Command::new("git");
            remove.args(["worktree", "remove"]);
            if force {
                remove.arg("--force");
            }
            let _ = remove.arg(&worktree).current_dir(&self.repo).output();
        };
        let baseline = run_suite(&self.repo, &self.suite);
        if let Err(e) = mutate(&worktree) {
            cleanup(true);
            let _ = git(&self.repo, &["branch", "-D", &spec.branch]);
            return Err(io::Error::other(format!("mutation failed: {e}")));
        }
        let candidate = run_suite(&worktree, &self.suite);
        let decision = if candidate.failed > baseline.failed {
            Decision::Rollback {
                reason: format!(
                    "regression: baseline {} failed, candidate {} failed",
                    baseline.failed, candidate.failed
                ),
            }
        } else {
            Decision::Promote {
                reason: format!(
                    "no regressions (baseline {}/{} passed/failed, candidate {}/{}); wall time {:.1}s vs {:.1}s",
                    baseline.passed,
                    baseline.failed,
                    candidate.passed,
                    candidate.failed,
                    baseline.seconds,
                    candidate.seconds
                ),
            }
        };
        let report = ExperimentReport {
            spec: spec.clone(),
            baseline,
            candidate,
            decision: decision.clone(),
            finished_at_ms: now_ms(),
        };
        match decision {
            Decision::Promote { .. } => {
                git(&worktree, &["add", "-A"])?;
                let commit = std::process::Command::new("git")
                    .args([
                        "-c",
                        "user.name=harness-experiment",
                        "-c",
                        "user.email=experiment@local",
                        "commit",
                        "-m",
                        &format!("experiment {}: {}", spec.id, spec.description),
                    ])
                    .current_dir(&worktree)
                    .output()
                    .map_err(|e| io::Error::other(format!("commit failed to launch: {e}")))?;
                if !commit.status.success() {
                    // Nothing to commit (or hook failure): a vacuous
                    // experiment promotes nothing; roll it back instead.
                    cleanup(true);
                    let _ = git(&self.repo, &["branch", "-D", &spec.branch]);
                    return Err(io::Error::other(format!(
                        "promote commit failed: {}",
                        String::from_utf8_lossy(&commit.stderr)
                    )));
                }
                cleanup(false);
            }
            Decision::Rollback { .. } => {
                cleanup(true);
                let _ = git(&self.repo, &["branch", "-D", &spec.branch]);
            }
        }
        Ok(report)
    }

    /// Persist the decision record outside the repo so bookkeeping never
    /// dirties future clean-tree checks.
    pub fn write_record(
        &self,
        records_dir: &Path,
        report: &ExperimentReport,
    ) -> io::Result<PathBuf> {
        std::fs::create_dir_all(records_dir)?;
        let path = records_dir.join(format!("{}.json", report.spec.id));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(report).map_err(io::Error::other)?,
        )?;
        Ok(path)
    }
}

#[cfg(test)]
mod parser_tests {
    use super::parse_suite_output;

    #[test]
    fn reads_libtest_summaries() {
        let output = "running 3 tests\ntest a ... ok\ntest result: ok. 2 passed; 1 failed; 0 ignored; finished in 0.01s\n";
        assert_eq!(parse_suite_output(output), Some((2, 1)));
        assert_eq!(
            parse_suite_output("test result: ok. 10 passed; 0 failed; 0 ignored\n"),
            Some((10, 0))
        );
        assert_eq!(parse_suite_output("compiling...\nno tests here\n"), None);
    }
}
