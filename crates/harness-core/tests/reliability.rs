use harness_core::event_store::EventStore;
use harness_core::tools::MAX_FILE_BYTES;
use harness_core::{
    Action, AgentRuntime, HeuristicModel, Model, ModelUsage, Objective, Observation,
    PermissionPolicy, ResourceLimits, RunOutcome, SqliteEventStore, StepDecision, ToolRegistry,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "harness-reliability-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn db(&self) -> PathBuf {
        self.0.join("run.sqlite3")
    }
    fn file_objective(&self) -> Objective {
        Objective::new("create file done.txt with content done")
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

/// Scripted model with metered spend: every decision accrues tokens/cost,
/// then follows the fixed script.
struct Metered {
    per_call_tokens: u64,
    cost_per_call: f64,
    spent: ModelUsage,
    script: Vec<StepDecision>,
}

impl Model for Metered {
    fn name(&self) -> &str {
        "metered-script"
    }

    fn usage(&self) -> ModelUsage {
        self.spent.clone()
    }

    fn decide(&mut self, _: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        self.spent.prompt_tokens += self.per_call_tokens;
        self.spent.cost_usd += self.cost_per_call;
        self.script
            .get(history.len())
            .cloned()
            .unwrap_or(StepDecision::Fail("script exhausted".into()))
    }
}

fn write_action(path: &str, contents: &str) -> StepDecision {
    StepDecision::Act(Action::WriteFile {
        path: path.into(),
        contents: contents.into(),
    })
}

#[test]
fn wall_clock_budget_fails_closed_without_actions() {
    let w = Workspace::new();
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    runtime
        .create_run(w.file_objective(), Default::default(), None)
        .unwrap();
    runtime
        .amend_resource_limits(
            ResourceLimits {
                wall_clock_secs: Some(0),
                ..Default::default()
            },
            "test expiry",
        )
        .unwrap();
    // Wall budgets tick in whole seconds; sleep past the boundary.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let outcome = runtime.resume().unwrap();
    drop(runtime);
    assert!(matches!(outcome, RunOutcome::Failed(_)), "{outcome:?}");
    assert!(!w.0.join("done.txt").exists());
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(events.iter().any(|e| e.kind == "WallClockExhausted"));
    assert!(events.iter().any(|e| e.kind == "ResourceLimitsAmended"));
}

#[test]
fn wall_clock_origin_survives_resume() {
    let w = Workspace::new();
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    runtime
        .create_run(w.file_objective(), Default::default(), None)
        .unwrap();
    drop(runtime);
    let before = SqliteEventStore::open(w.db())
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
        .started_at_ms
        .unwrap();
    let outcome = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    )
    .resume()
    .unwrap();
    assert!(matches!(outcome, RunOutcome::Completed(_)));
    let after = SqliteEventStore::open(w.db())
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
        .started_at_ms
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn token_budget_accrues_and_trips() {
    let w = Workspace::new();
    let model = Metered {
        per_call_tokens: 100,
        cost_per_call: 0.0,
        spent: ModelUsage::default(),
        script: vec![write_action("a.txt", "a"), write_action("b.txt", "b")],
    };
    let mut runtime = AgentRuntime::new(
        model,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    runtime
        .create_run(w.file_objective(), Default::default(), None)
        .unwrap();
    runtime
        .amend_resource_limits(
            ResourceLimits {
                token_limit: Some(150),
                ..Default::default()
            },
            "test tokens",
        )
        .unwrap();
    let outcome = runtime.resume().unwrap();
    drop(runtime);
    assert!(
        matches!(&outcome, RunOutcome::Failed(reason) if reason.contains("token")),
        "{outcome:?}"
    );
    let state = SqliteEventStore::open(w.db())
        .unwrap()
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(state.used_tokens, 200);
    assert_eq!(state.history.len(), 1);
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(events.iter().any(|e| e.kind == "TokenExhausted"));
    let recorded: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "UsageRecorded")
        .collect();
    assert_eq!(recorded.len(), 2);
    assert!(recorded[0].detail.contains("\"total_tokens\":100"));
}

#[test]
fn cost_budget_trips() {
    let w = Workspace::new();
    let model = Metered {
        per_call_tokens: 0,
        cost_per_call: 0.05,
        spent: ModelUsage::default(),
        script: vec![write_action("a.txt", "a")],
    };
    let mut runtime = AgentRuntime::new(
        model,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    runtime
        .create_run(w.file_objective(), Default::default(), None)
        .unwrap();
    runtime
        .amend_resource_limits(
            ResourceLimits {
                cost_limit_usd: Some(0.04),
                ..Default::default()
            },
            "test cost",
        )
        .unwrap();
    let outcome = runtime.resume().unwrap();
    drop(runtime);
    assert!(
        matches!(&outcome, RunOutcome::Failed(reason) if reason.contains("cost")),
        "{outcome:?}"
    );
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(events.iter().any(|e| e.kind == "CostExhausted"));
}

#[test]
fn repeated_consecutive_failures_trip_closed() {
    let w = Workspace::new();
    // Every write is oversized: the tool fails without side effects.
    let big = "x".repeat(MAX_FILE_BYTES + 1);
    let model = Metered {
        per_call_tokens: 0,
        cost_per_call: 0.0,
        spent: ModelUsage::default(),
        script: vec![
            write_action("a.txt", &big),
            write_action("b.txt", &big),
            write_action("c.txt", &big),
            write_action("d.txt", &big),
        ],
    };
    let mut runtime = AgentRuntime::new(
        model,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    let outcome = runtime.run(w.file_objective()).unwrap();
    drop(runtime);
    assert!(
        matches!(&outcome, RunOutcome::Failed(reason) if reason.contains("repeated")),
        "{outcome:?}"
    );
    assert!(!w.0.join("a.txt").exists());
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(events.iter().any(|e| e.kind == "RepeatedFailures"));
}

#[test]
fn isolated_failures_do_not_trip() {
    let w = Workspace::new();
    let big = "x".repeat(MAX_FILE_BYTES + 1);
    let model = Metered {
        per_call_tokens: 0,
        cost_per_call: 0.0,
        spent: ModelUsage::default(),
        script: vec![
            write_action("junk.txt", &big),
            write_action("ok.txt", "ok"),
            StepDecision::Complete("done".into()),
        ],
    };
    // Objective matches the verified file, not the junk.
    let mut runtime = AgentRuntime::new(
        model,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    let outcome = runtime
        .run(Objective::new("create file ok.txt with content ok"))
        .unwrap();
    assert!(matches!(outcome, RunOutcome::Completed(_)), "{outcome:?}");
}

#[test]
fn shutdown_stops_without_settling_then_resume_completes() {
    let w = Workspace::new();
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    let handle = runtime.shutdown_handle();
    handle.request();
    let error = runtime.run(w.file_objective()).unwrap_err().to_string();
    assert!(error.contains("shutdown"), "{error}");
    drop(runtime);
    let state = SqliteEventStore::open(w.db())
        .unwrap()
        .load()
        .unwrap()
        .unwrap();
    assert!(state.outcome.is_none());
    let outcome = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    )
    .resume()
    .unwrap();
    assert!(matches!(outcome, RunOutcome::Completed(_)));
}

#[test]
fn heartbeats_carry_budget_snapshots() {
    let w = Workspace::new();
    fs::write(w.0.join("data.txt"), "data").unwrap();
    struct Chattie;
    impl Model for Chattie {
        fn name(&self) -> &str {
            "chattie"
        }
        fn decide(&mut self, _: &Objective, history: &[(Action, Observation)]) -> StepDecision {
            match history.len() {
                0..=10 => StepDecision::Act(Action::ReadFile {
                    path: "data.txt".into(),
                }),
                11 => write_action("done.txt", "done"),
                _ => StepDecision::Complete("done".into()),
            }
        }
    }
    let mut runtime = AgentRuntime::new(
        Chattie,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    )
    .with_max_steps(16);
    let outcome = runtime.run(w.file_objective()).unwrap();
    drop(runtime);
    assert!(matches!(outcome, RunOutcome::Completed(_)), "{outcome:?}");
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    let heartbeats: Vec<_> = events.iter().filter(|e| e.kind == "Heartbeat").collect();
    assert_eq!(heartbeats.len(), 1);
    assert!(heartbeats[0].detail.contains("tools="));
}

#[test]
fn two_hundred_actions_complete_with_steady_heartbeats() {
    let w = Workspace::new();
    struct BulkWriter;
    impl Model for BulkWriter {
        fn name(&self) -> &str {
            "bulk-writer"
        }
        fn decide(&mut self, _: &Objective, history: &[(Action, Observation)]) -> StepDecision {
            match history.len() {
                0..200 => StepDecision::Act(Action::WriteFile {
                    path: format!("file-{:03}.txt", history.len()),
                    contents: "bulk".into(),
                }),
                200 => write_action("done.txt", "done"),
                _ => StepDecision::Complete("done".into()),
            }
        }
    }
    // Objective matches the final verified file, not the bulk files.
    let mut runtime = AgentRuntime::new(
        BulkWriter,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    )
    .with_max_steps(250)
    .with_max_tool_calls(300);
    let outcome = runtime.run(w.file_objective()).unwrap();
    drop(runtime);
    assert!(matches!(outcome, RunOutcome::Completed(_)), "{outcome:?}");
    let state = SqliteEventStore::open(w.db())
        .unwrap()
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(state.tool_budget.unwrap().used, 202);
    assert_eq!(
        fs::read_to_string(w.0.join("file-199.txt")).unwrap(),
        "bulk"
    );
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "Heartbeat").count(), 20);
}

#[test]
fn repeated_resumes_repeat_nothing() {
    let w = Workspace::new();
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    let outcome = runtime.run(w.file_objective()).unwrap();
    drop(runtime);
    let before = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    for _ in 0..5 {
        let again = AgentRuntime::new(
            HeuristicModel,
            ToolRegistry::milestone_default(),
            PermissionPolicy::milestone_default(&w.0),
            SqliteEventStore::open(w.db()).unwrap(),
        )
        .resume()
        .unwrap();
        assert_eq!(again, outcome);
    }
    let after = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert_eq!(before, after);
}

#[test]
fn inspect_reports_machine_readable_state() {
    let w = Workspace::new();
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    runtime.run(w.file_objective()).unwrap();
    drop(runtime);
    let mut store = SqliteEventStore::open(w.db()).unwrap();
    let report = harness_core::inspect::inspect_run(&mut store).unwrap();
    assert!(report["outcome"].get("Completed").is_some(), "{report}");
    assert_eq!(report["tools"]["used"], serde_json::json!(3));
    assert_eq!(report["history"], serde_json::json!(3));
    assert_eq!(
        report["objective"]["text"],
        serde_json::json!("create file done.txt with content done")
    );
    assert!(report["kinds"]["CognitiveStep"].as_u64().unwrap() >= 3);
}
