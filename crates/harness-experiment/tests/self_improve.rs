use harness_experiment::{CheckSuite, Decision, ExperimentRunner, ExperimentSpec};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn git(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn fixture_repo(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "harness-exp-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.98.1\"\nprofile = \"minimal\"\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "/target/\nCargo.lock\n").unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn answer() -> i32 { 42 }\n\nfn main() {\n    println!(\"{}\", answer());\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn value_is_fixed() {\n        assert_eq!(answer(), 42);\n    }\n\n    #[test]\n    fn arithmetic_holds() {\n        assert_eq!(answer() + 0, 42);\n    }\n}\n",
    )
    .unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["add", "-A"]);
    git(
        &root,
        &[
            "-c",
            "user.name=harness-test",
            "-c",
            "user.email=test@local",
            "commit",
            "-m",
            "fixture",
        ],
    );
    root
}

fn spec(id: &str) -> ExperimentSpec {
    ExperimentSpec {
        id: id.into(),
        branch: format!("exp-{id}"),
        description: format!("experiment {id}"),
    }
}

fn runner(repo: &Path) -> ExperimentRunner {
    ExperimentRunner::new(repo, CheckSuite::default())
}

fn worktrees(repo: &Path) -> String {
    git(repo, &["worktree", "list", "--porcelain"])
}

#[test]
fn improving_change_is_promoted_with_record() {
    let root = fixture_repo("promote");
    let records = root.join("records");
    let report = runner(&root)
        .run_experiment(spec("add-test"), |worktree| {
            fs::write(
                worktree.join("src/main.rs"),
                fs::read_to_string(worktree.join("src/main.rs")).unwrap()
                    + "\n#[test]\nfn experiment_added() {\n    assert_eq!(2 + 2, 4);\n}\n",
            )
            .map_err(io_error)
        })
        .unwrap();
    assert_eq!(report.baseline.passed, 2);
    assert_eq!(report.baseline.failed, 0);
    assert_eq!(report.candidate.passed, 3);
    assert!(
        matches!(report.decision, Decision::Promote { .. }),
        "{:?}",
        report.decision
    );
    // Promoted: branch kept with the change committed, worktree removed.
    assert!(git(&root, &["branch", "--list", "exp-add-test"]).contains("exp-add-test"));
    assert!(!worktrees(&root).contains("add-test"));
    assert!(git(&root, &["show", "exp-add-test:src/main.rs"]).contains("experiment_added"));
    // Repo itself untouched and clean.
    let status = git(&root, &["status", "--porcelain"]);
    assert!(status.trim().is_empty(), "dirty: {status}");
    let record = runner(&root).write_record(&records, &report).unwrap();
    let stored: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(record).unwrap()).unwrap();
    assert_eq!(stored["spec"]["id"], serde_json::json!("add-test"));
    assert!(stored.get("decision").unwrap().get("promote").is_some());
    assert!(stored["finished_at_ms"].as_u64().unwrap() > 0);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn regressing_change_is_rolled_back() {
    let root = fixture_repo("rollback");
    let report = runner(&root)
        .run_experiment(spec("break-value"), |worktree| {
            let path = worktree.join("src/main.rs");
            fs::write(
                path,
                fs::read_to_string(worktree.join("src/main.rs"))
                    .unwrap()
                    .replace("fn answer() -> i32 { 42 }", "fn answer() -> i32 { 43 }"),
            )
            .map_err(io_error)
        })
        .unwrap();
    assert_eq!(report.baseline.failed, 0);
    assert!(report.candidate.failed > 0);
    assert!(
        matches!(report.decision, Decision::Rollback { .. }),
        "{:?}",
        report.decision
    );
    // Rolled back: branch deleted, worktree gone, repo pristine.
    assert!(!git(&root, &["branch", "--list", "exp-break-value"]).contains("exp-break-value"));
    assert!(!worktrees(&root).contains("break-value"));
    assert!(
        fs::read_to_string(root.join("src/main.rs"))
            .unwrap()
            .contains("42")
    );
    assert!(git(&root, &["status", "--porcelain"]).trim().is_empty());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn dirty_repo_refuses_experiment() {
    let root = fixture_repo("dirty");
    fs::write(root.join("src/main.rs"), "dirty").unwrap();
    let before = worktrees(&root);
    let error = runner(&root)
        .run_experiment(spec("nope"), |_| Ok(()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("uncommitted"), "{error}");
    assert_eq!(worktrees(&root), before);
    assert!(!git(&root, &["branch", "--list", "exp-nope"]).contains("exp-nope"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn duplicate_branch_refuses_experiment() {
    let root = fixture_repo("dup");
    git(&root, &["branch", "exp-clash"]);
    let error = runner(&root)
        .run_experiment(
            ExperimentSpec {
                id: "clash".into(),
                branch: "exp-clash".into(),
                description: "clash".into(),
            },
            |_| Ok(()),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("worktree add failed"), "{error}");
    fs::remove_dir_all(&root).unwrap();
}

fn io_error(error: std::io::Error) -> std::io::Error {
    error
}

#[test]
fn cancellation_stops_running_baseline_and_skips_mutation() {
    use std::{
        sync::{Arc, atomic::AtomicBool},
        time::{Duration, Instant},
    };
    let root = fixture_repo("cancel-running");
    fs::write(root.join("src/main.rs"),r#"fn main() {} #[test] fn slow() { std::fs::write("target/ready", "ready").unwrap(); std::thread::sleep(std::time::Duration::from_secs(60)); }"#).unwrap();
    git(&root, &["add", "src/main.rs"]);
    git(
        &root,
        &[
            "-c",
            "user.name=harness-test",
            "-c",
            "user.email=test@local",
            "commit",
            "-m",
            "slow test",
        ],
    );
    let stop = Arc::new(AtomicBool::new(false));
    let child_stop = stop.clone();
    let child_root = root.clone();
    let worker = std::thread::spawn(move || {
        runner(&child_root)
            .run_experiment_cancellable(spec("cancel-running"), &child_stop, |_| {
                panic!("Mutation must not run after cancellation")
            })
            .unwrap()
    });
    let wait = Instant::now();
    while !root.join("target/ready").exists() {
        assert!(
            wait.elapsed() < Duration::from_secs(20),
            "Suite did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let stopped = Instant::now();
    stop.store(true, Ordering::SeqCst);
    let report = worker.join().unwrap();
    assert!(stopped.elapsed() < Duration::from_secs(10));
    assert!(matches!(report.decision, Decision::Rollback { .. }));
    assert!(report.baseline.output_tail.contains("cancelled"));
    assert!(report.candidate.output_tail.contains("skipped"));
    assert_eq!(
        worktrees(&root)
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count(),
        1
    );
    assert!(
        git(&root, &["branch", "--list", "exp-cancel-running"])
            .trim()
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_before_candidate_prevents_promotion() {
    use std::sync::atomic::AtomicBool;
    let root = fixture_repo("cancel-candidate");
    let stop = AtomicBool::new(false);
    let report = runner(&root)
        .run_experiment_cancellable(spec("cancel-candidate"), &stop, |tree| {
            fs::write(tree.join("note.txt"), "candidate")?;
            stop.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    assert!(matches!(report.decision, Decision::Rollback { .. }));
    assert_eq!(report.candidate.passed, 0);
    assert!(!root.join("note.txt").exists());
    assert!(
        git(&root, &["branch", "--list", "exp-cancel-candidate"])
            .trim()
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn removing_tests_cannot_promote() {
    let root = fixture_repo("lost-tests");
    let report = runner(&root)
        .run_experiment(spec("lost-tests"), |tree| {
            fs::write(tree.join("src/main.rs"), "fn main() {}\n")
        })
        .unwrap();
    assert!(matches!(report.decision, Decision::Rollback { .. }));
    assert_eq!(report.candidate.passed, 0);
    assert!(git(&root, &["status", "--porcelain"]).trim().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn equally_failed_suites_cannot_promote() {
    let root = fixture_repo("bad-suites");
    let runner = ExperimentRunner::new(
        &root,
        CheckSuite {
            argv: vec!["--version".into()],
            ..CheckSuite::default()
        },
    );
    let report = runner
        .run_experiment(spec("invalid-suite"), |tree| {
            fs::write(tree.join("note.txt"), "candidate")
        })
        .unwrap();
    assert!(matches!(report.decision, Decision::Rollback { .. }));
    assert!(report.baseline.failed > 0);
    fs::remove_dir_all(root).unwrap();
}
