use harness_core::event_store::EventStore;
use harness_core::plan::{Lifecycle, PlanState};
use harness_core::{
    AgentRuntime, HeuristicModel, Objective, PermissionPolicy, RunOutcome, SqliteEventStore,
    ToolRegistry,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "harness-plan-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn db(&self) -> PathBuf {
        self.0.join("run.sqlite3")
    }
    fn runtime(&self) -> AgentRuntime<HeuristicModel, SqliteEventStore> {
        AgentRuntime::new(
            HeuristicModel,
            ToolRegistry::milestone_default(),
            PermissionPolicy::milestone_default(&self.0),
            SqliteEventStore::open(self.db()).unwrap(),
        )
    }
    fn plan(&self) -> PlanState {
        let mut plan = PlanState::default();
        plan.add_goal("top", "ship the repair", 1).unwrap();
        plan.add_task("first", "top", "write a.txt", vec![])
            .unwrap();
        plan.add_task("second", "top", "write b.txt", vec!["first".into()])
            .unwrap();
        plan
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn durable_plan_survives_restart_with_goal_intact() {
    let w = Workspace::new();
    w.runtime()
        .create_run(
            Objective::new("create file done.txt with content done"),
            w.plan(),
            None,
        )
        .unwrap();
    w.runtime()
        .revise_plan("first running", |plan| plan.mark_running("first"))
        .unwrap();
    w.runtime()
        .revise_plan("first done", |plan| {
            plan.mark_succeeded("first", "a.txt written")
        })
        .unwrap();
    // Simulated crash: runtimes (and their store locks) drop here.
    let plan = w.runtime().plan_snapshot().unwrap();
    assert_eq!(plan.goals.len(), 1);
    assert_eq!(plan.goals[0].id, "top");
    assert_eq!(plan.ready_tasks(), vec!["second".to_string()]);
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(events.iter().any(|e| e.kind == "PlanCreated"));
    assert_eq!(events.iter().filter(|e| e.kind == "PlanRevised").count(), 2);
}

#[test]
fn terminal_runs_refuse_revision() {
    let w = Workspace::new();
    w.runtime()
        .create_run(
            Objective::new("create file done.txt with content done"),
            w.plan(),
            None,
        )
        .unwrap();
    w.runtime()
        .revise_plan("finish all", |plan| {
            plan.mark_succeeded("first", "ok")?;
            plan.mark_succeeded("second", "ok")
        })
        .unwrap();
    let outcome = w.runtime().complete_run("plan done").unwrap();
    assert!(matches!(outcome, RunOutcome::Completed(_)));
    assert!(
        w.runtime()
            .revise_plan("too late", |plan| plan.mark_running("first"))
            .is_err()
    );
    assert!(w.runtime().plan_snapshot().is_err());
    assert_eq!(w.runtime().resume().unwrap(), outcome);
}

#[test]
fn replan_after_failure_keeps_goal_and_attempts() {
    let w = Workspace::new();
    w.runtime()
        .create_run(
            Objective::new("create file done.txt with content done"),
            w.plan(),
            None,
        )
        .unwrap();
    w.runtime()
        .revise_plan("first ok", |plan| plan.mark_succeeded("first", "ok"))
        .unwrap();
    w.runtime()
        .revise_plan("second broke", |plan| {
            plan.mark_failed("second", "wrong contents")
        })
        .unwrap();
    w.runtime()
        .revise_plan("repair second", |plan| {
            plan.repair("second", "second-retry", "corrected contents")
        })
        .unwrap();
    let plan = w.runtime().plan_snapshot().unwrap();
    assert_eq!(plan.goals.len(), 1);
    assert_eq!(plan.ready_tasks(), vec!["second-retry".to_string()]);
    w.runtime()
        .revise_plan("retry ok", |plan| plan.mark_succeeded("second-retry", "ok"))
        .unwrap();
    assert!(matches!(
        w.runtime().complete_run("repaired").unwrap(),
        RunOutcome::Completed(_)
    ));
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(events.iter().any(|e| e.kind == "GoalCompleted"));
}

#[test]
fn budget_amendment_is_recorded_not_silent() {
    let w = Workspace::new();
    w.runtime()
        .create_run(
            Objective::new("create file done.txt with content done"),
            w.plan(),
            None,
        )
        .unwrap();
    w.runtime()
        .amend_tool_budget(64, "wider repair loop")
        .unwrap();
    w.runtime()
        .amend_tool_budget(16, "tighten after scope cut")
        .unwrap();
    let plan = w.runtime().plan_snapshot().unwrap();
    assert_eq!(plan.amendments.len(), 2);
    assert_eq!(plan.amendments[0].previous_limit, 32);
    assert_eq!(plan.amendments[0].new_limit, 64);
    assert_eq!(plan.amendments[1].new_limit, 16);
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert_eq!(
        events.iter().filter(|e| e.kind == "BudgetAmended").count(),
        2
    );
}

#[test]
fn lifecycle_pause_blocks_drive_and_resume_continues() {
    let w = Workspace::new();
    w.runtime()
        .create_run(
            Objective::new("create file done.txt with content done"),
            w.plan(),
            None,
        )
        .unwrap();
    w.runtime().set_lifecycle(Lifecycle::Paused).unwrap();
    let paused = w.runtime().resume().unwrap_err().to_string();
    assert!(paused.contains("paused"), "{paused}");
    // Pausing never settles the run: no terminal outcome is persisted.
    let state = SqliteEventStore::open(w.db())
        .unwrap()
        .load()
        .unwrap()
        .unwrap();
    assert!(state.outcome.is_none());
    assert!(!w.0.join("done.txt").exists());
    w.runtime().set_lifecycle(Lifecycle::Active).unwrap();
    let outcome = w.runtime().resume().unwrap();
    assert!(matches!(outcome, RunOutcome::Completed(_)));
    assert_eq!(fs::read_to_string(w.0.join("done.txt")).unwrap(), "done");
}

#[test]
fn cancel_is_terminal_and_survives_restart() {
    let w = Workspace::new();
    w.runtime()
        .create_run(
            Objective::new("create file done.txt with content done"),
            w.plan(),
            None,
        )
        .unwrap();
    let outcome = w.runtime().cancel_run("operator stop").unwrap();
    assert!(matches!(outcome, RunOutcome::Failed(_)));
    assert_eq!(w.runtime().resume().unwrap(), outcome);
    assert!(!w.0.join("done.txt").exists());
}

#[test]
fn complete_run_requires_finished_plan() {
    let w = Workspace::new();
    w.runtime()
        .create_run(
            Objective::new("create file done.txt with content done"),
            w.plan(),
            None,
        )
        .unwrap();
    assert!(w.runtime().complete_run("too early").is_err());
    w.runtime()
        .revise_plan("all done", |plan| {
            plan.mark_succeeded("first", "ok")?;
            plan.mark_succeeded("second", "ok")
        })
        .unwrap();
    assert!(matches!(
        w.runtime().complete_run("done").unwrap(),
        RunOutcome::Completed(_)
    ));
}
