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
            tool_budget: Some(Default::default()),
            success_criterion: None,
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
fn reconcile_matching_write_without_rewriting_and_preserve_id() {
    let w = Workspace::new();
    let mut state = w.state();
    state.pending = Some(harness_core::Action::WriteFile {
        path: "result.txt".into(),
        contents: "durable".into(),
    });
    let action_id = state.next_action_id();
    fs::write(w.0.join("result.txt"), "durable").unwrap();
    let modified = fs::metadata(w.0.join("result.txt"))
        .unwrap()
        .modified()
        .unwrap();
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    assert!(matches!(
        w.runtime().reconcile_and_resume().unwrap(),
        RunOutcome::Completed(_)
    ));
    assert_eq!(
        fs::metadata(w.0.join("result.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(
        !events
            .iter()
            .any(|e| e.kind == "ToolCalled" && e.detail == "write_file:result.txt")
    );
    let reconciled = events
        .iter()
        .find(|e| e.kind == "ActionReconciled")
        .unwrap();
    let detail: serde_json::Value = serde_json::from_str(&reconciled.detail).unwrap();
    assert_eq!(detail["action_id"], action_id);
}

#[test]
fn reconciliation_refuses_missing_mismatching_and_forbidden_paths() {
    for (path, existing) in [
        ("result.txt", None),
        ("result.txt", Some("other")),
        ("../escape.txt", None),
        (".harness/private", None),
    ] {
        let w = Workspace::new();
        let mut state = w.state();
        let action = harness_core::Action::WriteFile {
            path: path.into(),
            contents: "durable".into(),
        };
        state.pending = Some(action.clone());
        if let Some(contents) = existing {
            fs::write(w.0.join(path), contents).unwrap();
        }
        SqliteEventStore::open(w.db())
            .unwrap()
            .checkpoint(&state)
            .unwrap();
        assert!(w.runtime().reconcile_and_resume().is_err());
        assert_eq!(
            SqliteEventStore::open(w.db())
                .unwrap()
                .load()
                .unwrap()
                .unwrap()
                .pending,
            Some(action)
        );
        if let Some(contents) = existing {
            assert_eq!(fs::read_to_string(w.0.join(path)).unwrap(), contents);
        }
    }
}

#[test]
fn interrupted_shell_is_never_replayed_by_reconciler() {
    let w = Workspace::new();
    let mut state = w.state();
    state.pending = Some(harness_core::Action::RunShell {
        program: "echo".into(),
        args: vec!["unsafe".into()],
    });
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    assert!(
        w.runtime()
            .reconcile_and_resume()
            .unwrap_err()
            .to_string()
            .contains("unsupported")
    );
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(!events.iter().any(|e| e.kind == "ToolCalled"));
}

#[test]
fn pending_read_is_refreshed_then_independently_verified() {
    let w = Workspace::new();
    let mut state = w.state();
    state.history.push((
        harness_core::Action::WriteFile {
            path: "result.txt".into(),
            contents: "durable".into(),
        },
        harness_core::Observation {
            ok: true,
            summary: "written".into(),
            data: "7".into(),
        },
    ));
    state.pending = Some(harness_core::Action::ReadFile {
        path: "result.txt".into(),
    });
    fs::write(w.0.join("result.txt"), "durable").unwrap();
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    assert!(matches!(
        w.runtime().reconcile_and_resume().unwrap(),
        RunOutcome::Completed(_)
    ));
    let store = SqliteEventStore::open(w.db()).unwrap();
    let events = store.events().unwrap();
    assert!(events.iter().any(|e| e.kind == "VerificationPassed"));
    let prepared: Vec<serde_json::Value> = events
        .iter()
        .filter(|e| e.kind == "ActionPrepared")
        .map(|e| serde_json::from_str(&e.detail).unwrap())
        .collect();
    assert_eq!(prepared.len(), 1); // Final runtime-owned verification remains mandatory.
    assert_ne!(prepared[0]["action_id"], state.next_action_id());
}

#[test]
fn failed_reconciliation_checkpoint_can_be_retried_without_write() {
    struct FailCheckpoint(SqliteEventStore);
    impl EventStore for FailCheckpoint {
        fn append(&mut self, kind: &str, detail: &str) -> std::io::Result<harness_core::Event> {
            self.0.append(kind, detail)
        }
        fn load(&mut self) -> std::io::Result<Option<RunState>> {
            self.0.load()
        }
        fn checkpoint(&mut self, state: &RunState) -> std::io::Result<()> {
            if state.pending.is_some() {
                self.0.checkpoint(state)
            } else {
                Err(std::io::Error::other("injected checkpoint failure"))
            }
        }
    }
    let w = Workspace::new();
    let mut state = w.state();
    state.pending = Some(harness_core::Action::WriteFile {
        path: "result.txt".into(),
        contents: "durable".into(),
    });
    fs::write(w.0.join("result.txt"), "durable").unwrap();
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        FailCheckpoint(SqliteEventStore::open(w.db()).unwrap()),
    );
    assert!(runtime.reconcile_and_resume().is_err());
    drop(runtime);
    assert_eq!(
        SqliteEventStore::open(w.db())
            .unwrap()
            .load()
            .unwrap()
            .unwrap()
            .pending,
        state.pending
    );
    assert!(matches!(
        w.runtime().reconcile_and_resume().unwrap(),
        RunOutcome::Completed(_)
    ));
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(
        !events
            .iter()
            .any(|e| e.kind == "ToolCalled" && e.detail == "write_file:result.txt")
    );
}

#[test]
fn failed_reconciliation_reads_consume_persisted_budget() {
    let w = Workspace::new();
    let mut state = w.state();
    state.tool_budget = Some(harness_core::agent::ToolBudget { used: 0, limit: 2 });
    state.pending = Some(harness_core::Action::WriteFile {
        path: "result.txt".into(),
        contents: "durable".into(),
    });
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    for used in 1..=2 {
        assert!(w.runtime().reconcile_and_resume().is_err());
        let saved = SqliteEventStore::open(w.db())
            .unwrap()
            .load()
            .unwrap()
            .unwrap();
        assert_eq!(saved.tool_budget.unwrap().used, used);
    }
    fs::write(w.0.join("result.txt"), "durable").unwrap();
    assert!(
        w.runtime()
            .reconcile_and_resume()
            .unwrap_err()
            .to_string()
            .contains("budget exhausted")
    );
    let mut store = SqliteEventStore::open(w.db()).unwrap();
    assert!(store.load().unwrap().unwrap().pending.is_some());
    assert_eq!(
        store
            .events()
            .unwrap()
            .iter()
            .filter(|e| e.kind == "ReconciliationObserved")
            .count(),
        2
    );
}

#[test]
fn zero_budget_prevents_write_and_restart_does_not_reset_it() {
    let w = Workspace::new();
    assert!(
        w.runtime()
            .with_max_tool_calls(0)
            .run(w.state().objective)
            .is_err()
    );
    assert!(!w.0.join("result.txt").exists());
    assert!(w.runtime().with_max_tool_calls(99).resume().is_err());
    let mut store = SqliteEventStore::open(w.db()).unwrap();
    let budget = store.load().unwrap().unwrap().tool_budget.unwrap();
    assert_eq!(budget.limit, 0);
    assert_eq!(budget.used, 0);
    assert!(
        !store
            .events()
            .unwrap()
            .iter()
            .any(|e| e.kind == "ToolCalled")
    );
}

#[test]
fn verification_consumes_tool_budget() {
    let w = Workspace::new();
    assert!(
        w.runtime()
            .with_max_tool_calls(2)
            .run(w.state().objective)
            .is_err()
    );
    let mut store = SqliteEventStore::open(w.db()).unwrap();
    assert_eq!(store.load().unwrap().unwrap().tool_budget.unwrap().used, 2);
    assert!(
        !store
            .events()
            .unwrap()
            .iter()
            .any(|e| e.kind == "GoalCompleted")
    );
}

#[test]
fn legacy_active_accounting_is_not_invented() {
    let w = Workspace::new();
    let mut state = w.state();
    let mut legacy = serde_json::to_value(&state).unwrap();
    legacy.as_object_mut().unwrap().remove("tool_budget");
    state = serde_json::from_value(legacy).unwrap();
    assert!(state.tool_budget.is_none());
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    assert!(
        w.runtime()
            .resume()
            .unwrap_err()
            .to_string()
            .contains("legacy checkpoint")
    );
    state.outcome = Some(RunOutcome::Completed("historical".into()));
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    assert_eq!(
        w.runtime().resume().unwrap(),
        RunOutcome::Completed("historical".into())
    );
}

#[test]
fn reservation_commit_failure_prevents_invocation() {
    struct RejectReservation(SqliteEventStore);
    impl EventStore for RejectReservation {
        fn append(&mut self, kind: &str, detail: &str) -> std::io::Result<harness_core::Event> {
            self.0.append(kind, detail)
        }
        fn load(&mut self) -> std::io::Result<Option<RunState>> {
            self.0.load()
        }
        fn checkpoint(&mut self, state: &RunState) -> std::io::Result<()> {
            if state.pending.is_some() {
                Err(std::io::Error::other("reservation commit failed"))
            } else {
                self.0.checkpoint(state)
            }
        }
    }
    let w = Workspace::new();
    let store = RejectReservation(SqliteEventStore::open(w.db()).unwrap());
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        store,
    );
    assert!(runtime.run(w.state().objective).is_err());
    drop(runtime);
    assert!(!w.0.join("result.txt").exists());
    let mut store = SqliteEventStore::open(w.db()).unwrap();
    assert_eq!(store.load().unwrap().unwrap().tool_budget.unwrap().used, 0);
    assert!(
        !store
            .events()
            .unwrap()
            .iter()
            .any(|e| e.kind == "ToolCalled")
    );
}

#[test]
fn oversized_reconciliation_read_is_charged_and_remains_pending() {
    let w = Workspace::new();
    let mut state = w.state();
    state.pending = Some(harness_core::Action::WriteFile {
        path: "result.txt".into(),
        contents: "durable".into(),
    });
    fs::write(
        w.0.join("result.txt"),
        vec![b'x'; harness_core::tools::MAX_FILE_BYTES + 1],
    )
    .unwrap();
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(&state)
        .unwrap();
    assert!(w.runtime().reconcile_and_resume().is_err());
    let mut store = SqliteEventStore::open(w.db()).unwrap();
    let saved = store.load().unwrap().unwrap();
    assert_eq!(saved.pending, state.pending);
    assert_eq!(saved.tool_budget.unwrap().used, 1);
    let events = store.events().unwrap();
    let evidence = events
        .iter()
        .find(|e| e.kind == "ReconciliationObserved")
        .unwrap();
    assert!(evidence.detail.len() < 512);
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
    struct CrashStore {
        inner: SqliteEventStore,
        root: PathBuf,
    }
    impl EventStore for CrashStore {
        fn append(&mut self, kind: &str, detail: &str) -> std::io::Result<harness_core::Event> {
            let result = self.inner.append(kind, detail)?;
            if kind == "ActionObserved" && std::env::var_os("HARNESS_CRASH_AFTER_TOOL").is_some() {
                fs::write(
                    self.root.join("ready"),
                    b"tool executed, checkpoint still pending",
                )?;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
            Ok(result)
        }
        fn checkpoint(&mut self, state: &RunState) -> std::io::Result<()> {
            self.inner.checkpoint(state)
        }
        fn load(&mut self) -> std::io::Result<Option<RunState>> {
            self.inner.load()
        }
    }
    let mut runtime = AgentRuntime::new(
        PauseModel(root.clone()),
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&root),
        CrashStore {
            inner: SqliteEventStore::open(root.join("run.sqlite3")).unwrap(),
            root: root.clone(),
        },
    );
    runtime
        .run(Objective::new(
            "create file result.txt with content durable",
        ))
        .unwrap();
}

#[test]
fn kill_after_write_before_checkpoint_then_reconcile() {
    let w = Workspace::new();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_worker", "--nocapture"])
        .env("HARNESS_CRASH_TEST_ROOT", &w.0)
        .env("HARNESS_CRASH_AFTER_TOOL", "1")
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
    assert!(ready);
    let state = SqliteEventStore::open(w.db())
        .unwrap()
        .load()
        .unwrap()
        .unwrap();
    assert!(state.pending.is_some());
    assert!(state.history.is_empty());
    assert_eq!(state.tool_budget.as_ref().unwrap().used, 1);
    let action_id = state.next_action_id();
    assert!(w.runtime().resume().is_err());
    assert!(matches!(
        w.runtime().reconcile_and_resume().unwrap(),
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
    for kind in ["ActionPrepared", "ActionObserved", "ActionReconciled"] {
        let event = events.iter().find(|e| e.kind == kind).unwrap();
        let detail: serde_json::Value = serde_json::from_str(&event.detail).unwrap();
        assert_eq!(detail["action_id"], action_id);
    }
    assert_eq!(
        fs::read_to_string(w.0.join("result.txt")).unwrap(),
        "durable"
    );
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
