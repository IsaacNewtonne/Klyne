use harness_core::agent::RunState;
use harness_core::event_store::EventStore;
use harness_core::{
    AgentRuntime, HeuristicModel, Objective, PermissionPolicy, RunOutcome, SqliteEventStore,
    ToolRegistry,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "harness-recovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn db(&self) -> PathBuf {
        self.0.join("run.sqlite3")
    }
    fn state(&self) -> RunState {
        RunState {
            version: 1,
            objective: Objective::new("create file result.txt with content durable"),
            workspace: fs::canonicalize(&self.0).unwrap(),
            history: vec![],
            steps: 0,
            max_steps: 8,
            pending: None,
            outcome: None,
        }
    }
    fn runtime(&self) -> AgentRuntime<HeuristicModel, SqliteEventStore> {
        AgentRuntime::new(
            HeuristicModel,
            ToolRegistry::milestone_default(),
            PermissionPolicy::milestone_default(&self.0),
            SqliteEventStore::open(self.db()).unwrap(),
        )
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn completed_run_reopens_without_repeating_actions() {
    let w = Workspace::new();
    let result = w.runtime().run(w.state().objective).unwrap();
    assert!(matches!(result, RunOutcome::Completed(_)));
    let before = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert_eq!(w.runtime().resume().unwrap(), result);
    let after = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert_eq!(before, after);
    assert_eq!(
        fs::read_to_string(w.0.join("result.txt")).unwrap(),
        "durable"
    );
}

#[test]
fn interrupted_side_effect_is_not_replayed() {
    let w = Workspace::new();
    let mut state = w.state();
    state.pending = Some(harness_core::Action::WriteFile {
        path: "result.txt".into(),
        contents: "durable".into(),
    });
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    assert!(
        w.runtime()
            .resume()
            .unwrap_err()
            .to_string()
            .contains("reconciliation")
    );
    assert!(!w.0.join("result.txt").exists());
}

#[test]
fn exhausted_budget_survives_restart() {
    let w = Workspace::new();
    let mut state = w.state();
    state.steps = state.max_steps;
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    assert!(matches!(
        w.runtime().resume().unwrap(),
        RunOutcome::Failed(_)
    ));
    assert!(!w.0.join("result.txt").exists());
}

#[test]
fn excludes_second_executor_and_rejects_newer_schema() {
    let w = Workspace::new();
    let first = SqliteEventStore::open(w.db()).unwrap();
    assert!(SqliteEventStore::open(w.db()).is_err());
    drop(first);
    let connection = rusqlite::Connection::open(w.db()).unwrap();
    connection.execute_batch("PRAGMA user_version=99;").unwrap();
    drop(connection);
    assert!(SqliteEventStore::open(w.db()).is_err());
}

#[test]
fn events_are_append_only_and_payload_round_trips() {
    let w = Workspace::new();
    let mut store = SqliteEventStore::open(w.db()).unwrap();
    let event = store
        .append("ToolCompleted", "tabs\tnewlines\nUnicode: 日本語")
        .unwrap();
    assert_eq!(store.events().unwrap(), vec![event]);
    drop(store);
    let connection = rusqlite::Connection::open(w.db()).unwrap();
    assert!(connection.execute("DELETE FROM events", []).is_err());
    assert!(
        connection
            .execute("UPDATE events SET detail='changed'", [])
            .is_err()
    );
}

#[test]
fn crash_worker() {
    let Some(root) = std::env::var_os("HARNESS_CRASH_TEST_ROOT") else {
        return;
    };
    struct PauseModel(PathBuf);
    impl harness_core::Model for PauseModel {
        fn name(&self) -> &str {
            "crash-test-model"
        }
        fn decide(
            &mut self,
            objective: &Objective,
            history: &[(harness_core::Action, harness_core::Observation)],
        ) -> harness_core::StepDecision {
            if !history.is_empty() {
                fs::write(self.0.join("ready"), b"checkpoint saved").unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
            harness_core::Model::decide(&mut HeuristicModel, objective, history)
        }
    }
    let root = PathBuf::from(root);
    let mut runtime = AgentRuntime::new(
        PauseModel(root.clone()),
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&root),
        SqliteEventStore::open(root.join("run.sqlite3")).unwrap(),
    );
    runtime
        .run(Objective::new(
            "create file result.txt with content durable",
        ))
        .unwrap();
}

#[test]
fn resumes_every_committed_non_pending_checkpoint() {
    use std::sync::{Arc, Mutex};
    struct Capture {
        store: SqliteEventStore,
        snapshots: Arc<Mutex<Vec<RunState>>>,
    }
    impl EventStore for Capture {
        fn append(&mut self, kind: &str, detail: &str) -> std::io::Result<harness_core::Event> {
            self.store.append(kind, detail)
        }
        fn checkpoint(&mut self, state: &RunState) -> std::io::Result<()> {
            self.store.checkpoint(state)?;
            self.snapshots.lock().unwrap().push(state.clone());
            Ok(())
        }
    }
    let w = Workspace::new();
    let snapshots = Arc::new(Mutex::new(Vec::new()));
    let store = Capture {
        store: SqliteEventStore::open(w.db()).unwrap(),
        snapshots: snapshots.clone(),
    };
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        store,
    );
    runtime.run(w.state().objective).unwrap();
    drop(runtime);
    for (index, state) in snapshots.lock().unwrap().iter().enumerate() {
        let path = w.0.join(format!("replay-{index}.sqlite3"));
        let mut store = SqliteEventStore::open(path).unwrap();
        store.checkpoint(state).unwrap();
        let mut runtime = AgentRuntime::new(
            HeuristicModel,
            ToolRegistry::milestone_default(),
            PermissionPolicy::milestone_default(&w.0),
            store,
        );
        let result = runtime.resume();
        if state.pending.is_some() {
            assert!(result.is_err());
        } else {
            assert!(
                matches!(result.unwrap(), RunOutcome::Completed(_)),
                "checkpoint {index}"
            );
        }
    }
}

#[test]
fn persistence_failure_prevents_side_effect() {
    struct FailingStore;
    impl EventStore for FailingStore {
        fn append(&mut self, _: &str, _: &str) -> std::io::Result<harness_core::Event> {
            Err(std::io::Error::other("injected disk failure"))
        }
    }
    let w = Workspace::new();
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        FailingStore,
    );
    assert!(runtime.run(w.state().objective).is_err());
    assert!(!w.0.join("result.txt").exists());
}

#[test]
fn process_kill_then_resume_preserves_completed_action() {
    let w = Workspace::new();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_worker", "--nocapture"])
        .env("HARNESS_CRASH_TEST_ROOT", &w.0)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !w.0.join("ready").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let ready = w.0.join("ready").exists();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(ready, "worker did not reach checkpoint before deadline");
    assert!(matches!(
        w.runtime().resume().unwrap(),
        RunOutcome::Completed(_)
    ));
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "ToolCalled" && e.detail == "write_file:result.txt")
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(w.0.join("result.txt")).unwrap(),
        "durable"
    );
}
