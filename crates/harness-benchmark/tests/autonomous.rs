use harness_benchmark::{
    TaskOutcome, TaskReport, TaskSpec, agent_runtime, oracle, report_for, run_task, task_criterion,
    task_objective, write_fixture,
};
use harness_core::event_store::EventStore;
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

fn flight_spec() -> TaskSpec {
    TaskSpec {
        name: "flight-task".into(),
        bug: "41".into(),
        fix_first: "42".into(),
        fix_final: "42".into(),
        test_name: "flight_task_is_fixed".into(),
        shape: "wrong-constant".into(),
    }
}

/// Worker entry: runs a task, then hangs after the patch checkpoint so the
/// parent kill deterministically lands mid-flight (patch committed, cargo
/// test never started), unlike racing the compiler.
#[test]
fn bench_flight_worker() {
    let Some(root) = std::env::var_os("HARNESS_FLIGHT_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    struct PauseBeforeTest<M: Model> {
        inner: M,
        root: PathBuf,
    }
    impl<M: Model> Model for PauseBeforeTest<M> {
        fn name(&self) -> &str {
            "pause-before-test"
        }
        fn decide(
            &mut self,
            objective: &Objective,
            history: &[(Action, Observation)],
        ) -> StepDecision {
            let patched = history.iter().any(|(action, observation)| {
                matches!(action, Action::PatchFile { .. }) && observation.ok
            });
            let tested = history
                .iter()
                .any(|(action, _)| matches!(action, Action::RunShell { .. }));
            if patched && !tested {
                fs::write(self.root.join("ready"), b"patch checkpointed").unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
            self.inner.decide(objective, history)
        }
    }
    let task = flight_spec();
    let db = root.join(".harness/flight.sqlite3");
    let mut runtime = harness_benchmark::agent_runtime(
        &root,
        &db,
        PauseBeforeTest {
            inner: harness_benchmark::RepairAgent::new(task.clone()),
            root: root.clone(),
        },
    )
    .unwrap();
    let criterion = harness_benchmark::task_criterion(&root, &task).unwrap();
    runtime
        .run_with_criterion(harness_benchmark::task_objective(&task), Some(criterion))
        .unwrap();
}

#[test]
fn crafted_partial_run_resumes_without_restarting() {
    use harness_core::agent::RunState;
    use harness_core::event_store::EventStore;
    let root = workspace("crafted");
    let task = flight_spec();
    write_fixture(&root, &task).unwrap();
    // Pre-apply the patch on disk, exactly as a killed run would leave it.
    let path = root.join(task.file());
    let source = fs::read_to_string(&path).unwrap().replace("41", "42");
    fs::write(&path, source).unwrap();
    let offset = fs::read_to_string(&path).unwrap().find("42").unwrap() as u64;
    let ok = |summary: &str, data: &str| Observation {
        ok: true,
        summary: summary.into(),
        data: data.into(),
    };
    let db = root.join(".harness/crafted.sqlite3");
    let mut store = harness_core::SqliteEventStore::open(&db).unwrap();
    store
        .checkpoint(&RunState {
            version: 1,
            objective: task_objective(&task),
            workspace: fs::canonicalize(&root).unwrap(),
            history: vec![
                (
                    Action::SearchFile {
                        path: task.file(),
                        needle: "41".into(),
                        max_matches: 10,
                    },
                    ok(
                        "search",
                        &format!("{{\"matches\":[{{\"offset\":{offset}}}]}}"),
                    ),
                ),
                (
                    Action::ReadFileRange {
                        path: task.file(),
                        offset,
                        length: 2,
                    },
                    ok("range", r#"{"text":"41"}"#),
                ),
                (
                    Action::HashFile { path: task.file() },
                    ok("hash", r#"{"sha256":"abc"}"#),
                ),
                (
                    Action::PatchFile {
                        path: task.file(),
                        offset,
                        expected: "41".into(),
                        replacement: "42".into(),
                        expected_sha256: "abc".into(),
                    },
                    ok("patch", r#"{"after_sha256":"abc"}"#),
                ),
            ],
            steps: 4,
            max_steps: 16,
            pending: None,
            outcome: None,
            tool_budget: Some(Default::default()),
            success_criterion: None,
            plan: Default::default(),
            started_at_ms: None,
            used_tokens: 0,
            used_cost_usd: 0.0,
            resource_limits: Default::default(),
        })
        .unwrap();
    drop(store);
    // No success criterion was ever persisted: run_task repairs it, then
    // resumes into test and verification instead of erroring.
    let report = run_task(&root, &db, &task).unwrap();
    assert_repaired(&report);
    assert_eq!(report.test_runs, 1);
    let mut store = harness_core::SqliteEventStore::open(&db).unwrap();
    let state = store.load().unwrap().unwrap();
    assert_eq!(
        state
            .history
            .iter()
            .filter(|(action, _)| matches!(action, Action::PatchFile { .. }))
            .count(),
        1
    );
    assert!(
        store
            .events()
            .unwrap()
            .iter()
            .any(|event| event.kind == "SuccessCriterionRevised")
    );
    drop(store);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn kill_mid_flight_resumes_without_duplicating_patch() {
    let root = workspace("flight");
    let task = flight_spec();
    write_fixture(&root, &task).unwrap();
    let db = root.join(".harness/flight.sqlite3");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bench_flight_worker", "--nocapture"])
        .env("HARNESS_FLIGHT_ROOT", &root)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Deterministic interruption point: the worker hangs after the patch
    // checkpoint, before ever invoking cargo.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !root.join("ready").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        root.join("ready").exists(),
        "worker never reached the patch"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    let report = run_task(&root, &db, &task).unwrap();
    assert_repaired(&report);
    let mut store = harness_core::SqliteEventStore::open(&db).unwrap();
    let state = store.load().unwrap().unwrap();
    assert_eq!(
        state
            .history
            .iter()
            .filter(|(action, _)| matches!(action, Action::PatchFile { .. }))
            .count(),
        1,
        "resume must test, not re-patch"
    );
    assert!(
        store
            .events()
            .unwrap()
            .iter()
            .any(|event| event.kind == "AgentResumed"),
        "expected the resume path, not a fresh run"
    );
    drop(store);
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
