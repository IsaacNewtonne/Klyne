//! First autonomous coding benchmark: controlled Rust repair tasks.
//!
//! Each task is a disposable dependency-free Rust crate with one injected
//! bug. A scripted (deterministic, non-intelligent) agent must inspect the
//! repo, patch it, compile it, run its tests, and repair failures, all
//! through the runtime's permissions, budgets, persistence, and independent
//! verification. An independent oracle recompiles and retests outside the
//! agent's tool calls. Metrics mirror the architecture's benchmark schema.

use harness_core::event_store::EventStore;
use harness_core::verification::SuccessCriterion;
use harness_core::{
    Action, AgentRuntime, Model, Objective, Observation, PermissionPolicy, ProcessLimits,
    RunOutcome, SqliteEventStore, StepDecision, ToolRegistry,
};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One controlled bug. The fixture crate asserts `answer() == fixed_value`;
/// the agent sees only the workspace files, never this spec.
#[derive(Clone, Debug)]
pub struct TaskSpec {
    /// Task directory name under `<workspace>/tasks`.
    pub name: String,
    /// Buggy source snippet present in the fixture, e.g. `41`.
    pub bug: String,
    /// First fix the agent attempts. Differs from `fixed` to force a
    /// test-failure/repair round on tasks exercising failure recovery.
    pub fix_first: String,
    /// Value the oracle asserts, e.g. `42`.
    pub fix_final: String,
    /// Expected unit test name in the fixture.
    pub test_name: String,
    /// Bug family tag used to match procedural memories, e.g.
    /// `wrong-constant`. Values, not shapes, still decide each patch.
    pub shape: String,
}

impl TaskSpec {
    pub fn file(&self) -> String {
        format!("tasks/{}/src/main.rs", self.name)
    }

    pub fn manifest_arg(&self) -> String {
        format!("tasks/{}/Cargo.toml", self.name)
    }
}

/// Toolchain all fixtures pin. Matches the harness workspace so task builds
/// never resolve to a broken default toolchain outside the repository.
pub const FIXTURE_TOOLCHAIN: &str = "1.98.1";

/// Write a disposable dependency-free fixture crate into `workspace`.
/// The toolchain pin lives at the workspace root because rustup resolves it
/// from the process working directory (where cargo runs), not from
/// `--manifest-path`.
pub fn write_fixture(workspace: &Path, spec: &TaskSpec) -> io::Result<()> {
    let dir = workspace.join("tasks").join(&spec.name).join("src");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        workspace.join("rust-toolchain.toml"),
        format!("[toolchain]\nchannel = \"{FIXTURE_TOOLCHAIN}\"\nprofile = \"minimal\"\n"),
    )?;
    std::fs::write(
        workspace.join("tasks").join(&spec.name).join("Cargo.toml"),
        format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            spec.name.replace('-', "_")
        ),
    )?;
    std::fs::write(
        dir.join("main.rs"),
        format!(
            "fn answer() -> i32 {{ {} }}\n\nfn main() {{\n    println!(\"{{}}\", answer());\n}}\n\n#[cfg(test)]\nmod tests {{\n    use super::*;\n\n    #[test]\n    fn {}() {{\n        assert_eq!(answer(), {});\n    }}\n}}\n",
            spec.bug, spec.test_name, spec.fix_final
        ),
    )?;
    Ok(())
}

/// Cargo-test argv shared by the agent (through the supervised tool) and the
/// oracle (directly, outside agent control). Offline: fixtures have no deps.
pub fn test_argv(spec: &TaskSpec) -> Vec<String> {
    vec![
        "test".into(),
        "--offline".into(),
        "--manifest-path".into(),
        spec.manifest_arg(),
    ]
}

/// Independent acceptance oracle. Runs outside the agent's tool registry and
/// proves success through compiler/test evidence, not agent claims.
pub fn oracle(workspace: &Path, spec: &TaskSpec) -> Result<String, String> {
    let output = std::process::Command::new("cargo")
        .args(test_argv(spec))
        .current_dir(workspace)
        .output()
        .map_err(|e| format!("oracle failed to launch cargo: {e}"))?;
    let combined = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    if output.status.success() && combined.contains("test result: ok") {
        Ok(combined)
    } else {
        Err(format!(
            "oracle: cargo test failed with {}: {}",
            output.status,
            combined.chars().take(2000).collect::<String>()
        ))
    }
}

fn test_outcome(observation: &Observation) -> TestOutcome {
    if observation.data.contains("test result: ok") {
        TestOutcome::Passed
    } else if observation.data.contains("test result: FAILED")
        || observation.data.contains("FAILED")
        || observation.data.contains("error: could not compile")
        || observation.data.contains("error[")
    {
        TestOutcome::Failed
    } else {
        TestOutcome::Unclear
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TestOutcome {
    Passed,
    Failed,
    Unclear,
}

/// Deterministic scripted repair strategy (no intelligence claimed).
/// History-driven, so it resumes correctly from any persisted prefix:
/// inspect via search, confirm the span, hash for a fresh digest, patch,
/// compile/test through the supervised tool, and repair on failure.
pub struct RepairAgent {
    spec: TaskSpec,
}

impl RepairAgent {
    pub fn new(spec: TaskSpec) -> Self {
        Self { spec }
    }

    fn last_successful_patch(&self, history: &[(Action, Observation)]) -> Option<Action> {
        history.iter().rev().find_map(|(action, observation)| {
            if matches!(action, Action::PatchFile { .. }) && observation.ok {
                Some(action.clone())
            } else {
                None
            }
        })
    }

    /// Text the file currently holds per our own applied history.
    fn current_text(&self, history: &[(Action, Observation)]) -> String {
        match self.last_successful_patch(history) {
            Some(Action::PatchFile { replacement, .. }) => replacement,
            _ => self.spec.bug.clone(),
        }
    }

    /// A digest is fresh only if its hash postdates our latest applied patch.
    fn fresh_digest(&self, history: &[(Action, Observation)]) -> Option<String> {
        let last_patch = history
            .iter()
            .rposition(|(action, _)| matches!(action, Action::PatchFile { .. }));
        history
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, (action, observation))| match action {
                Action::HashFile { .. }
                    if observation.ok && last_patch.is_none_or(|position| index > position) =>
                {
                    serde_json::from_str::<serde_json::Value>(&observation.data)
                        .ok()?
                        .get("sha256")?
                        .as_str()
                        .map(str::to_string)
                }
                _ => None,
            })
    }

    fn match_offset(history: &[(Action, Observation)]) -> Option<u64> {
        history.iter().find_map(|(action, observation)| {
            matches!(action, Action::SearchFile { .. }).then(|| {
                serde_json::from_str::<serde_json::Value>(&observation.data)
                    .ok()?
                    .get("matches")?
                    .get(0)?
                    .get("offset")?
                    .as_u64()
            })?
        })
    }

    fn test_runs(history: &[(Action, Observation)]) -> usize {
        history
            .iter()
            .filter(|(action, _)| matches!(action, Action::RunShell { .. }))
            .count()
    }

    fn patch_attempts(history: &[(Action, Observation)]) -> usize {
        history
            .iter()
            .filter(|(action, _)| matches!(action, Action::PatchFile { .. }))
            .count()
    }
}

impl Model for RepairAgent {
    fn name(&self) -> &str {
        "benchmark-repair-agent"
    }

    fn decide(&mut self, _: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        let path = self.spec.file();
        let last_test = history
            .iter()
            .rposition(|(action, _)| matches!(action, Action::RunShell { .. }));
        let last_patch_ok = history.iter().rposition(|(action, observation)| {
            matches!(action, Action::PatchFile { .. }) && observation.ok
        });
        // Terminal evidence first: an independent passing test run ends the task.
        if let Some(index) = last_test
            && test_outcome(&history[index].1) == TestOutcome::Passed
        {
            return StepDecision::Complete("tests pass".into());
        }
        // A failed non-test action is fatal, except a stale patch which earns
        // one re-hash; anything else means the workspace defeated the script.
        if let Some((action, observation)) = history.last()
            && !observation.ok
            && !matches!(action, Action::RunShell { .. })
        {
            if matches!(action, Action::PatchFile { .. }) && Self::patch_attempts(history) < 4 {
                return StepDecision::Act(Action::HashFile { path });
            }
            return StepDecision::Fail(format!("tool failed: {}", observation.summary));
        }
        let current = self.current_text(history);
        // Failure-driven repair, only when the failure is newer than our
        // latest applied patch; a stale failure means "test again".
        let fresh_failure = match (last_test, last_patch_ok) {
            (Some(test), patch) => {
                test_outcome(&history[test].1) == TestOutcome::Failed
                    && patch.is_none_or(|position| test > position)
            }
            _ => false,
        };
        if fresh_failure {
            if current == self.spec.fix_final || Self::test_runs(history) >= 4 {
                return StepDecision::Fail("tests still fail after the final fix".into());
            }
            if let Some(digest) = self.fresh_digest(history) {
                let offset = match Self::match_offset(history) {
                    Some(offset) => offset,
                    None => return StepDecision::Fail("lost the bug location".into()),
                };
                return StepDecision::Act(Action::PatchFile {
                    path,
                    offset,
                    expected: current,
                    replacement: self.spec.fix_final.clone(),
                    expected_sha256: digest,
                });
            }
            return StepDecision::Act(Action::HashFile { path });
        }
        // Initial inspection and first fix.
        if !history
            .iter()
            .any(|(action, _)| matches!(action, Action::SearchFile { .. }))
        {
            return StepDecision::Act(Action::SearchFile {
                path: path.clone(),
                needle: current,
                max_matches: 10,
            });
        }
        let offset = match Self::match_offset(history) {
            Some(offset) => offset,
            None => return StepDecision::Fail("bug not found by search".into()),
        };
        if !history
            .iter()
            .any(|(action, _)| matches!(action, Action::ReadFileRange { .. }))
        {
            return StepDecision::Act(Action::ReadFileRange {
                path: path.clone(),
                offset,
                length: current.len() as u64,
            });
        }
        if self.last_successful_patch(history).is_none() {
            if let Some(digest) = self.fresh_digest(history) {
                let target = if Self::patch_attempts(history) == 0 {
                    self.spec.fix_first.clone()
                } else {
                    self.spec.fix_final.clone()
                };
                return StepDecision::Act(Action::PatchFile {
                    path,
                    offset,
                    expected: current,
                    replacement: target,
                    expected_sha256: digest,
                });
            }
            return StepDecision::Act(Action::HashFile { path });
        }
        StepDecision::Act(Action::RunShell {
            program: "cargo".into(),
            args: test_argv(&self.spec),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskReport {
    pub name: String,
    pub outcome: TaskOutcome,
    pub verified: bool,
    pub tool_calls: u64,
    pub model_calls: u64,
    pub test_runs: u64,
    pub retries: u64,
    pub denials: u64,
    pub seconds: f64,
    pub tokens: u64,
    pub cost_usd: f64,
    pub detail: String,
}

/// Build the task runtime: file tools plus `cargo test` only, through the
/// supervised process tool with a compile-sized timeout. The shell grant is
/// deliberately narrow: argv must start with `test`.
/// Non-secret host variables a Windows MSVC build needs. Empirically, a
/// cleared environment with only the OS minimum cannot link: `link.exe`
/// discovery fails. The exact load-bearing variable is unresolved (leaving
/// one out at a time still links, so the mechanism is redundant), hence this
/// conservative documented set instead of a minimal claim. Absent entries
/// are skipped; nothing secret-bearing is included.
pub const BUILD_ENV_GRANTS: &[&str] = &[
    "TEMP",
    "TMP",
    "USERPROFILE",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramData",
    "SystemDrive",
    "OS",
    "NUMBER_OF_PROCESSORS",
    "COMPUTERNAME",
];

pub fn agent_runtime<M: Model>(
    workspace: &Path,
    database: &Path,
    model: M,
) -> io::Result<AgentRuntime<M, SqliteEventStore>> {
    let mut policy = PermissionPolicy::milestone_default(workspace);
    policy.allow_shell_with_arg_prefix("cargo", vec!["test".into()]);
    // These host-created disposable fixtures deliberately fail tests before
    // repair. Explicitly accept Cargo's diagnostic exit for each exact argv;
    // arbitrary commands and interrupted processes remain unreconciled.
    for task in std::fs::read_dir(workspace.join("tasks"))? {
        let task = task?;
        if task.file_type()?.is_dir() {
            policy.allow_shell_failure_recovery(
                "cargo",
                vec![
                    "test".into(),
                    "--offline".into(),
                    "--manifest-path".into(),
                    format!("tasks/{}/Cargo.toml", task.file_name().to_string_lossy()),
                ],
                101,
            );
        }
    }
    for name in BUILD_ENV_GRANTS {
        policy.allow_env(*name);
    }
    let registry = ToolRegistry::milestone_with_shell_limits(ProcessLimits {
        timeout: Duration::from_secs(300),
        max_output_bytes: 64 * 1024,
    });
    Ok(
        AgentRuntime::new(model, registry, policy, SqliteEventStore::open(database)?)
            .with_max_steps(16),
    )
}

/// Independent range claim over the fixed span. Same-length fixes keep the
/// bug offset stable from search to final verification, so on a resumed
/// run the already-applied fix marks the same span as the original bug.
pub fn task_criterion(workspace: &Path, spec: &TaskSpec) -> io::Result<SuccessCriterion> {
    assert_eq!(spec.bug.len(), spec.fix_first.len());
    assert_eq!(spec.bug.len(), spec.fix_final.len());
    let source = std::fs::read_to_string(workspace.join(spec.file()))?;
    let offset = source
        .find(&spec.bug)
        .or_else(|| source.find(&spec.fix_final))
        .ok_or_else(|| io::Error::other("fixture lacks the specified bug"))?
        as u64;
    Ok(SuccessCriterion::FileRange {
        path: spec.file(),
        offset,
        expected: spec.fix_final.clone(),
    })
}

pub fn task_objective(spec: &TaskSpec) -> Objective {
    Objective::new(format!("repair {} so its tests pass", spec.name))
}

/// Run one task end to end and prove the result with the oracle.
/// The workspace must already contain the fixture. A non-terminal
/// checkpoint resumes with the same history-driven model instead of
/// erroring, so kills mid-flight continue rather than restart: completed
/// effects are kept, pending file actions reconcile, and only missing work
/// re-executes. Terminal checkpoints report without re-running.
pub fn run_task(workspace: &Path, database: &Path, spec: &TaskSpec) -> io::Result<TaskReport> {
    let started = Instant::now();
    let existing = SqliteEventStore::open(database)?.load()?;
    let outcome = match existing {
        None => {
            let mut runtime = agent_runtime(workspace, database, RepairAgent::new(spec.clone()))?;
            runtime
                .run_with_criterion(task_objective(spec), Some(task_criterion(workspace, spec)?))?
        }
        Some(state) if state.outcome.is_some() => state.outcome.clone().expect("terminal"),
        Some(state) => {
            let mut runtime = agent_runtime(workspace, database, RepairAgent::new(spec.clone()))?;
            if state.success_criterion.is_none() {
                runtime.set_success_criterion(task_criterion(workspace, spec)?)?;
            }
            if state.pending.is_some() {
                runtime.reconcile_and_resume()?
            } else {
                runtime.resume()?
            }
        }
    };
    // The scripted agent's completion claim is advisory; runtime range
    // evidence plus the outside oracle decide verification below.
    report_for(workspace, database, spec, &outcome, started)
}

/// Durable multi-task driver result. `adopted` counts tasks whose prior
/// verified completion was reused instead of re-executed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanSummary {
    pub goal: String,
    pub completed: bool,
    pub adopted: u64,
    pub tasks: Vec<TaskReport>,
}

fn plan_task_db(workspace: &Path, task: &str) -> PathBuf {
    workspace
        .join(".harness")
        .join("tasks")
        .join(format!("{task}.sqlite3"))
}

/// Adopt a previously verified task completion without re-executing it.
/// Returns `None` when no terminal completion exists or the oracle no
/// longer passes, in which case the caller re-runs the task.
fn adopt_task(workspace: &Path, spec: &TaskSpec) -> io::Result<Option<TaskReport>> {
    let db = plan_task_db(workspace, &spec.name);
    let mut store = match SqliteEventStore::open(&db) {
        Ok(store) => store,
        Err(_) => return Ok(None),
    };
    let completed = matches!(
        store.load()?,
        Some(state) if matches!(state.outcome, Some(RunOutcome::Completed(_)))
    );
    drop(store);
    if !completed || oracle(workspace, spec).is_err() {
        return Ok(None);
    }
    let started = Instant::now();
    let outcome = RunOutcome::Completed("adopted verified completion".into());
    let mut report = report_for(workspace, &db, spec, &outcome, started)?;
    report.detail = format!("adopted without re-execution: {}", report.detail);
    Ok(Some(report))
}

/// Run every task in a persisted plan to completion. The plan (goal plus
/// chained tasks) is created once and checkpointed; each call resumes it:
/// succeeded tasks are adopted from their own checkpoints, never
/// re-executed. Set `HARNESS_PLAN_HANG_AFTER_TASK=<task>` to hang after a
/// task commits, which crash tests use to kill the driver mid-plan.
pub fn run_plan(
    workspace: &Path,
    plan_db: &Path,
    goal: &str,
    specs: &[TaskSpec],
) -> io::Result<PlanSummary> {
    use harness_core::plan::{Lifecycle, PlanState, TaskStatus};
    let fresh = SqliteEventStore::open(plan_db)?.load()?.is_none();
    if fresh {
        let mut plan = PlanState::default();
        plan.add_goal("top", goal, 1).map_err(io::Error::other)?;
        let mut previous: Option<String> = None;
        for spec in specs {
            let deps = previous.clone().into_iter().collect();
            plan.add_task(&spec.name, "top", format!("repair {}", spec.name), deps)
                .map_err(io::Error::other)?;
            previous = Some(spec.name.clone());
        }
        let mut seed = AgentRuntime::new(
            RepairAgent::new(specs[0].clone()),
            ToolRegistry::milestone_default(),
            PermissionPolicy::milestone_default(workspace),
            SqliteEventStore::open(plan_db)?,
        );
        seed.create_run(Objective::new(goal), plan, None)?;
    }
    let mut adopted = 0u64;
    let mut tasks = Vec::new();
    // Replays of a resumed plan re-adopt every recorded success first, so
    // the summary always covers the whole plan, not just this call's work.
    // A recorded success the oracle no longer confirms is a hard error:
    // the plan must be repaired, not silently re-run.
    {
        let mut seeder = AgentRuntime::new(
            RepairAgent::new(specs[0].clone()),
            ToolRegistry::milestone_default(),
            PermissionPolicy::milestone_default(workspace),
            SqliteEventStore::open(plan_db)?,
        );
        let snapshot = seeder.plan_snapshot()?;
        for task in snapshot
            .tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Succeeded)
        {
            let spec = specs
                .iter()
                .find(|spec| spec.name == task.id)
                .ok_or_else(|| io::Error::other(format!("plan task '{}' has no spec", task.id)))?;
            match adopt_task(workspace, spec)? {
                Some(report) => {
                    adopted += 1;
                    tasks.push(report);
                }
                None => {
                    return Err(io::Error::other(format!(
                        "recorded success for '{}' no longer verifies; repair the plan",
                        task.id
                    )));
                }
            }
        }
    }
    loop {
        let snapshot = {
            let mut probe = AgentRuntime::new(
                RepairAgent::new(specs[0].clone()),
                ToolRegistry::milestone_default(),
                PermissionPolicy::milestone_default(workspace),
                SqliteEventStore::open(plan_db)?,
            );
            probe.plan_snapshot()?
        };
        if !matches!(snapshot.lifecycle, Lifecycle::Active) {
            return Err(io::Error::other("plan is not active"));
        }
        let ready = snapshot.ready_tasks();
        if ready.is_empty() {
            let mut closer = AgentRuntime::new(
                RepairAgent::new(specs[0].clone()),
                ToolRegistry::milestone_default(),
                PermissionPolicy::milestone_default(workspace),
                SqliteEventStore::open(plan_db)?,
            );
            let outcome = closer.complete_run(format!("plan complete: {goal}"))?;
            return Ok(PlanSummary {
                goal: goal.into(),
                completed: matches!(outcome, RunOutcome::Completed(_)),
                adopted,
                tasks,
            });
        }
        let task_id = ready.into_iter().next().expect("ready is non-empty");
        let spec = specs
            .iter()
            .find(|spec| spec.name == task_id)
            .ok_or_else(|| io::Error::other(format!("plan task '{task_id}' has no spec")))?
            .clone();
        // Kill window: the child run verified but the success was never
        // recorded (killed between the two commits). Adopt the result
        // instead of re-executing the task.
        if let Some(report) = adopt_task(workspace, &spec)? {
            let mut marker = AgentRuntime::new(
                RepairAgent::new(spec.clone()),
                ToolRegistry::milestone_default(),
                PermissionPolicy::milestone_default(workspace),
                SqliteEventStore::open(plan_db)?,
            );
            marker.revise_plan("task succeeded (adopted)", |plan| {
                plan.mark_succeeded(&task_id, report.detail.clone())
            })?;
            adopted += 1;
            tasks.push(report);
            continue;
        }
        {
            let mut marker = AgentRuntime::new(
                RepairAgent::new(spec.clone()),
                ToolRegistry::milestone_default(),
                PermissionPolicy::milestone_default(workspace),
                SqliteEventStore::open(plan_db)?,
            );
            marker.revise_plan("task running", |plan| plan.mark_running(&task_id))?;
        }
        let db = plan_task_db(workspace, &spec.name);
        let report = run_task(workspace, &db, &spec)?;
        if !matches!(report.outcome, TaskOutcome::Completed) {
            let mut marker = AgentRuntime::new(
                RepairAgent::new(spec.clone()),
                ToolRegistry::milestone_default(),
                PermissionPolicy::milestone_default(workspace),
                SqliteEventStore::open(plan_db)?,
            );
            marker.revise_plan("task failed", |plan| {
                plan.mark_failed(&task_id, report.detail.clone())
            })?;
            return Err(io::Error::other(format!(
                "plan task '{}' failed: {}",
                spec.name, report.detail
            )));
        }
        {
            let mut marker = AgentRuntime::new(
                RepairAgent::new(spec.clone()),
                ToolRegistry::milestone_default(),
                PermissionPolicy::milestone_default(workspace),
                SqliteEventStore::open(plan_db)?,
            );
            marker.revise_plan("task succeeded", |plan| {
                plan.mark_succeeded(&task_id, report.detail.clone())
            })?;
        }
        tasks.push(report);
        if std::env::var("HARNESS_PLAN_HANG_AFTER_TASK").as_deref() == Ok(task_id.as_str()) {
            std::fs::write(workspace.join("ready"), b"task checkpointed")
                .map_err(io::Error::other)?;
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

/// A recalled fix proposal with its audit trail. The fix is a suggestion:
/// callers still patch through the runtime and verify through tests.
#[derive(Clone, Debug)]
pub struct FixHint {
    pub fix: String,
    pub confidence: f64,
    pub memory_id: i64,
    pub reasons: Vec<String>,
}

/// Procedural fix encoding. JSON keeps the convention parseable and
/// strict: unparseable rows are ignored, never applied.
fn encode_fix(shape: &str, bug: &str, fix: &str) -> String {
    serde_json::json!({"shape": shape, "bug": bug, "fix": fix}).to_string()
}

/// Record verified repair experience: one episodic outcome plus one
/// procedural fix mapping. Refuses to record unverified reports, so only
/// proven experience becomes memory.
pub fn record_repair(
    memory_db: &Path,
    spec: &TaskSpec,
    report: &TaskReport,
    provenance: &str,
) -> io::Result<()> {
    use harness_memory::{MemoryKind, NewMemory};
    if !(matches!(report.outcome, TaskOutcome::Completed) && report.verified) {
        return Err(io::Error::other("only verified repairs become memory"));
    }
    let mut store = harness_memory::MemoryStore::open(memory_db)?;
    store.write(NewMemory::new(
        MemoryKind::Episodic,
        format!(
            "repaired {}: replaced {} with {}; cargo test passed in {:.1}s",
            spec.name, spec.bug, spec.fix_final, report.seconds
        ),
        0.8,
        provenance,
        0.6,
    ))?;
    store.write(NewMemory::new(
        MemoryKind::Procedural,
        encode_fix(&spec.shape, &spec.bug, &spec.fix_final),
        0.9,
        provenance,
        0.8,
    ))?;
    Ok(())
}

/// Recall the best procedural fix for this shape and bug value. Matches on
/// parsed shape and bug (not on score alone), requires `min_confidence`,
/// and returns the retrieval reasons so the selection stays explainable.
/// Distractors for other shapes or bugs never match, however confident.
pub fn recall_fix(
    memory_db: &Path,
    spec: &TaskSpec,
    min_confidence: f64,
) -> io::Result<Option<FixHint>> {
    use harness_memory::{MemoryKind, MemoryQuery};
    let store = harness_memory::MemoryStore::open(memory_db)?;
    let hits = store.retrieve(&MemoryQuery::new(
        vec![MemoryKind::Procedural],
        vec![spec.shape.clone(), spec.bug.clone()],
        5,
    ))?;
    for hit in hits {
        let value: serde_json::Value = match serde_json::from_str(&hit.record.content) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let shape = value.get("shape").and_then(|field| field.as_str());
        let bug = value.get("bug").and_then(|field| field.as_str());
        let fix = value.get("fix").and_then(|field| field.as_str());
        match (shape, bug, fix) {
            (Some(shape), Some(bug), Some(fix))
                if shape == spec.shape
                    && bug == spec.bug
                    && hit.record.confidence >= min_confidence =>
            {
                return Ok(Some(FixHint {
                    fix: fix.into(),
                    confidence: hit.record.confidence,
                    memory_id: hit.record.id,
                    reasons: hit.reasons,
                }));
            }
            _ => continue,
        }
    }
    Ok(None)
}

/// Score a terminal outcome with oracle evidence and persisted metrics.
pub fn report_for(
    workspace: &Path,
    database: &Path,
    spec: &TaskSpec,
    outcome: &RunOutcome,
    started: Instant,
) -> io::Result<TaskReport> {
    let mut store = SqliteEventStore::open(database)?;
    let events = store.events()?;
    let state = store
        .load()?
        .ok_or_else(|| io::Error::other("missing checkpoint"))?;
    let oracle_result = oracle(workspace, spec);
    // Completion already required runtime range evidence; the oracle adds
    // compiler/test proof from outside agent control.
    let verified = matches!(outcome, RunOutcome::Completed(_)) && oracle_result.is_ok();
    Ok(TaskReport {
        name: spec.name.clone(),
        outcome: match (outcome, verified) {
            (RunOutcome::Completed(_), true) => TaskOutcome::Completed,
            _ => TaskOutcome::Failed,
        },
        verified,
        tool_calls: state.tool_budget.map(|budget| budget.used).unwrap_or(0),
        model_calls: events
            .iter()
            .filter(|event| event.kind == "CognitiveStep")
            .count() as u64,
        test_runs: state
            .history
            .iter()
            .filter(|(action, _)| matches!(action, Action::RunShell { .. }))
            .count() as u64,
        retries: state
            .history
            .iter()
            .filter(|(_, observation)| !observation.ok)
            .count() as u64,
        denials: events
            .iter()
            .filter(|event| event.kind == "PermissionDenied")
            .count() as u64,
        seconds: started.elapsed().as_secs_f64(),
        tokens: 0,
        cost_usd: 0.0,
        detail: match (outcome, &oracle_result) {
            (RunOutcome::Completed(_), Ok(_)) => "repaired and oracle-verified".into(),
            (RunOutcome::Completed(_), Err(reason)) => format!("agent completed but {reason}"),
            (RunOutcome::Failed(reason), _) => format!("agent failed: {reason}"),
        },
    })
}
