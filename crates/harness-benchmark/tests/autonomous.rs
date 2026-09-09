use harness_benchmark::{
    TaskOutcome, TaskReport, TaskSpec, agent_runtime, oracle, report_for, run_task, task_criterion,
    task_objective, write_fixture,
};
use harness_core::{Action, Model, Objective, Observation, StepDecision};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

fn workspace(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "harness-bench-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn spec(name: &str, bug: &str, fix_first: &str, fix_final: &str) -> TaskSpec {
    TaskSpec {
        name: name.into(),
        bug: bug.into(),
        fix_first: fix_first.into(),
        fix_final: fix_final.into(),
        test_name: format!("{}_is_fixed", name.replace('-', "_")),
        shape: "wrong-constant".into(),
    }
}

fn assert_repaired(report: &TaskReport) {
    println!("{}", serde_json::to_string_pretty(report).unwrap());
    assert_eq!(report.outcome, TaskOutcome::Completed, "{}", report.detail);
    assert!(report.verified, "{}", report.detail);
    assert_eq!(report.denials, 0, "no permission bypass permitted");
    assert!(report.tool_calls > 0);
    assert!(report.model_calls > 0);
    assert!(report.test_runs >= 1);
    assert_eq!(report.tokens, 0);
}

#[test]
fn repairs_wrong_constant() {
    let root = workspace("constant");
    let task = spec("wrong-constant", "41", "42", "42");
    write_fixture(&root, &task).unwrap();
    let report = run_task(&root, &root.join(".harness/run.sqlite3"), &task).unwrap();
    assert_repaired(&report);
    assert_eq!(report.test_runs, 1);
    assert_eq!(report.retries, 0);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn first_patch_fails_then_failure_driven_repair() {
    let root = workspace("first-fails");
    let task = spec("plausible-but-wrong", "41", "43", "42");
    write_fixture(&root, &task).unwrap();
    let report = run_task(&root, &root.join(".harness/run.sqlite3"), &task).unwrap();
    assert_repaired(&report);
    // One passing run after at least one failing cargo test observation.
    assert!(report.test_runs >= 2, "{}", report.test_runs);
    assert!(report.retries >= 1, "{}", report.retries);
    fs::remove_dir_all(&root).unwrap();
}

/// Worker entry: runs the repair agent, then hangs after the first applied
/// patch is checkpointed so the parent can kill -9 the process mid-task.
#[test]
fn bench_crash_worker() {
    let Some(root) = std::env::var_os("HARNESS_BENCH_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let task = TaskSpec {
        name: std::env::var("HARNESS_BENCH_TASK").unwrap(),
        bug: std::env::var("HARNESS_BENCH_BUG").unwrap(),
        fix_first: std::env::var("HARNESS_BENCH_FIRST").unwrap(),
        fix_final: std::env::var("HARNESS_BENCH_FINAL").unwrap(),
        test_name: std::env::var("HARNESS_BENCH_TEST").unwrap(),
        shape: std::env::var("HARNESS_BENCH_SHAPE").unwrap_or_else(|_| "wrong-constant".into()),
    };
    struct PauseAfterPatch<M: Model> {
        inner: M,
        root: PathBuf,
    }
    impl<M: Model> Model for PauseAfterPatch<M> {
        fn name(&self) -> &str {
            "crash-simulation-model"
        }
        fn decide(
            &mut self,
            objective: &Objective,
            history: &[(Action, Observation)],
        ) -> StepDecision {
            let patched = history.iter().any(|(action, observation)| {
                matches!(action, Action::PatchFile { .. }) && observation.ok
            });
            if patched {
                fs::write(self.root.join("ready"), b"patch checkpointed").unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
            self.inner.decide(objective, history)
        }
    }
    let db = root.join(".harness/run.sqlite3");
    let mut runtime = agent_runtime(
        &root,
        &db,
        PauseAfterPatch {
            inner: harness_benchmark::RepairAgent::new(task.clone()),
            root: root.clone(),
        },
    )
    .unwrap();
    let criterion = task_criterion(&root, &task).unwrap();
    runtime
        .run_with_criterion(task_objective(&task), Some(criterion))
        .unwrap();
}

#[test]
fn crash_restart_completes_task_without_duplicating_effects() {
    let started = Instant::now();
    let root = workspace("crash");
    let task = spec("interrupted-repair", "41", "42", "42");
    write_fixture(&root, &task).unwrap();
    let db = root.join(".harness/run.sqlite3");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bench_crash_worker", "--nocapture"])
        .env("HARNESS_BENCH_ROOT", &root)
        .env("HARNESS_BENCH_TASK", &task.name)
        .env("HARNESS_BENCH_BUG", &task.bug)
        .env("HARNESS_BENCH_FIRST", &task.fix_first)
        .env("HARNESS_BENCH_FINAL", &task.fix_final)
        .env("HARNESS_BENCH_TEST", &task.test_name)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while !root.join("ready").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        root.join("ready").exists(),
        "worker never reached the patch"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    // The kill landed between persisted actions: resume must test, verify,
    // and complete without applying a second patch.
    let mut runtime = agent_runtime(
        &root,
        &db,
        harness_benchmark::RepairAgent::new(task.clone()),
    )
    .unwrap();
    let outcome = runtime.resume().unwrap();
    assert!(
        matches!(outcome, harness_core::RunOutcome::Completed(_)),
        "{outcome:?}"
    );
    drop(runtime);
    let report = report_for(&root, &db, &task, &outcome, started).unwrap();
    assert_repaired(&report);
    assert!(
        oracle(&root, &task).is_ok(),
        "independent oracle must pass after restart"
    );
    let state = harness_core::SqliteEventStore::open(&db)
        .map(|mut store| {
            use harness_core::event_store::EventStore;
            store.load().unwrap().unwrap()
        })
        .unwrap();
    let patches = state
        .history
        .iter()
        .filter(|(action, _)| matches!(action, Action::PatchFile { .. }))
        .count();
    assert_eq!(patches, 1, "restart must not duplicate the patch");
    fs::remove_dir_all(&root).unwrap();
}
