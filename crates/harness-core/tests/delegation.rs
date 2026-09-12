use harness_core::AgentRuntime;
use harness_core::event_store::EventStore;
use harness_core::{
    Action, ChildGrant, HeuristicModel, Model, Objective, Observation, PermissionPolicy,
    RunOutcome, SqliteEventStore, StepDecision, ToolRegistry, collect_artifacts, spawn_child,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "harness-delegation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

/// Fixed script: replays actions, fails loudly on any tool failure.
struct Script {
    actions: Vec<Action>,
}

impl Model for Script {
    fn name(&self) -> &str {
        "delegation-script"
    }
    fn decide(&mut self, _: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        if let Some((_, observation)) = history.iter().find(|(_, observation)| !observation.ok) {
            return StepDecision::Fail(format!("tool failed: {}", observation.summary));
        }
        match self.actions.get(history.len()) {
            Some(action) => StepDecision::Act(action.clone()),
            None => StepDecision::Fail("script exhausted".into()),
        }
    }
}

fn parent_policy(root: &std::path::Path) -> PermissionPolicy {
    PermissionPolicy::milestone_default(root)
}

#[test]
fn child_works_inside_scope_and_parent_keeps_authority() {
    let w = Workspace::new();
    let parent = parent_policy(&w.0);
    let before = parent.capabilities();
    let report = spawn_child(
        &w.0,
        &parent,
        0,
        32,
        "worker",
        Objective::new("create file ok.txt with content hi"),
        None,
        HeuristicModel,
        ChildGrant::scoped("child", 32),
    )
    .unwrap();
    assert!(
        matches!(report.outcome, RunOutcome::Completed(_)),
        "{:?}",
        report.outcome
    );
    assert_eq!(fs::read_to_string(w.0.join("child/ok.txt")).unwrap(), "hi");
    assert_eq!(report.workspace, w.0.join("child"));
    // The parent policy object is unchanged: delegation never widens it.
    assert_eq!(parent.capabilities(), before);
    assert_eq!(parent.workspace_root(), w.0.as_path());
    // A traversal objective fails inside the child without escaping.
    let denied = spawn_child(
        &w.0,
        &parent,
        0,
        32,
        "jailbreak",
        Objective::new("create file ../escape.txt with content bad"),
        None,
        HeuristicModel,
        ChildGrant::scoped("child", 32),
    )
    .unwrap();
    assert!(matches!(denied.outcome, RunOutcome::Failed(_)));
    assert!(!w.0.join("escape.txt").exists());
}

#[test]
fn scope_escape_is_rejected_before_spawning() {
    let w = Workspace::new();
    let parent = parent_policy(&w.0);
    for scope in ["../outside", "/absolute", "", ".harness/state"] {
        assert!(parent.narrow_to_subdir(scope).is_err(), "{scope}");
        assert!(
            spawn_child(
                &w.0,
                &parent,
                0,
                32,
                "nope",
                Objective::new("create file ok.txt with content hi"),
                None,
                HeuristicModel,
                ChildGrant::scoped(scope, 32),
            )
            .is_err()
        );
    }
    let child = parent.narrow_to_subdir("sub/dir").unwrap();
    assert_eq!(child.workspace_root(), w.0.join("sub/dir").as_path());
    assert!(!child.allows_shell_program("cargo"));
}

#[test]
fn shell_grants_never_transfer_implicitly() {
    let w = Workspace::new();
    let mut parent = parent_policy(&w.0);
    parent.allow_shell_program("cargo");
    // Without an explicit re-grant the child cannot execute.
    let shell_action = Action::RunShell {
        program: "cargo".into(),
        args: vec!["--version".into()],
    };
    let report = spawn_child(
        &w.0,
        &parent,
        0,
        32,
        "quiet",
        Objective::new("unused"),
        None,
        Script {
            actions: vec![shell_action.clone()],
        },
        ChildGrant::scoped("child", 32),
    )
    .unwrap();
    assert!(
        matches!(report.outcome, RunOutcome::Failed(_)),
        "{:?}",
        report.outcome
    );
    let store = SqliteEventStore::open(w.0.join(".harness/children/quiet.sqlite3")).unwrap();
    assert!(
        store
            .events()
            .unwrap()
            .iter()
            .any(|e| e.kind == "PermissionDenied")
    );
    drop(store);
    // A covering re-grant authorizes exactly the granted prefix.
    let mut grant = ChildGrant::scoped("child2", 32);
    grant
        .shell_grants
        .push(("cargo".into(), vec!["--version".into()]));
    let report = spawn_child(
        &w.0,
        &parent,
        0,
        32,
        "loud",
        Objective::new("unused"),
        None,
        Script {
            actions: vec![shell_action],
        },
        grant,
    )
    .unwrap();
    // The action runs (cargo --version succeeds); the script then ends.
    let store = SqliteEventStore::open(w.0.join(".harness/children/loud.sqlite3")).unwrap();
    assert!(
        store
            .events()
            .unwrap()
            .iter()
            .any(|e| e.kind == "ToolCompleted"),
        "re-granted shell action must execute"
    );
    drop(store);
    assert!(matches!(report.outcome, RunOutcome::Failed(_))); // script exhausted, no completion claim
}

#[test]
fn shell_escalation_beyond_parent_is_refused() {
    let w = Workspace::new();
    let mut parent = parent_policy(&w.0);
    parent.allow_shell_with_arg_prefix("cargo", vec!["test".into()]);
    let mut child = parent.narrow_to_subdir("child").unwrap();
    assert!(child.grant_shell_from(&parent, "rm", vec![]).is_err());
    assert!(
        child
            .grant_shell_from(&parent, "cargo", vec!["--version".into()])
            .is_err()
    );
    assert!(
        child
            .grant_shell_from(&parent, "cargo", vec!["test".into(), "--offline".into()])
            .is_ok()
    );
    // A broad parent grant covers any prefix.
    let mut broad = parent_policy(&w.0);
    broad.allow_shell_program("cargo");
    let mut child = broad.narrow_to_subdir("child").unwrap();
    assert!(
        child
            .grant_shell_from(&broad, "cargo", vec!["anything".into()])
            .is_ok()
    );
}

#[test]
fn env_grants_require_matching_parent_grants() {
    let w = Workspace::new();
    let mut parent = parent_policy(&w.0);
    parent.allow_env("BUILD_TMP");
    let mut child = parent.narrow_to_subdir("child").unwrap();
    // Narrowing clears the slate, even for variables the parent holds.
    assert!(child.grant_env_from(&parent, "PATH").is_err());
    assert!(child.grant_env_from(&parent, "BUILD_TMP").is_ok());
}

#[cfg(windows)]
#[test]
fn regranted_env_reaches_child_processes() {
    unsafe { std::env::set_var("HARNESS_DELEG_ENV_XYZ", "present") };
    let w = Workspace::new();
    let mut parent = parent_policy(&w.0);
    parent.allow_shell_program("cmd.exe");
    parent.allow_env("HARNESS_DELEG_ENV_XYZ");
    let echo = Action::RunShell {
        program: "cmd.exe".into(),
        args: vec![
            "/d".into(),
            "/c".into(),
            "echo %HARNESS_DELEG_ENV_XYZ%".into(),
        ],
    };
    let run = |id: &str, regrant: bool| {
        let mut grant = ChildGrant::scoped(format!("child-{id}"), 32);
        grant.shell_grants.push(("cmd.exe".into(), vec![]));
        if regrant {
            grant.env_grants.push("HARNESS_DELEG_ENV_XYZ".into());
        }
        spawn_child(
            &w.0,
            &parent,
            0,
            32,
            id,
            Objective::new("unused"),
            None,
            Script {
                actions: vec![echo.clone()],
            },
            grant,
        )
        .unwrap()
    };
    // With the re-grant the value arrives; without it the slate stays clean.
    let _ = run("with", true);
    let store = SqliteEventStore::open(w.0.join(".harness/children/with.sqlite3")).unwrap();
    let with = store
        .events()
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "ToolCompleted")
        .unwrap();
    assert!(with.detail.contains("present"), "{}", with.detail);
    drop(store);
    let _ = run("without", false);
    let store = SqliteEventStore::open(w.0.join(".harness/children/without.sqlite3")).unwrap();
    let without = store
        .events()
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "ToolCompleted")
        .unwrap();
    assert!(!without.detail.contains("present"), "{}", without.detail);
    drop(store);
    unsafe { std::env::remove_var("HARNESS_DELEG_ENV_XYZ") };
}

#[cfg(not(windows))]
#[test]
fn regranted_env_reaches_child_processes() {
    unsafe { std::env::set_var("HARNESS_DELEG_ENV_XYZ", "present") };
    let w = Workspace::new();
    let mut parent = parent_policy(&w.0);
    parent.allow_shell_program("sh");
    parent.allow_env("HARNESS_DELEG_ENV_XYZ");
    let echo = Action::RunShell {
        program: "sh".into(),
        args: vec!["-c".into(), "echo $HARNESS_DELEG_ENV_XYZ".into()],
    };
    let run = |id: &str, regrant: bool| {
        let mut grant = ChildGrant::scoped(format!("child-{id}"), 32);
        grant.shell_grants.push(("sh".into(), vec![]));
        if regrant {
            grant.env_grants.push("HARNESS_DELEG_ENV_XYZ".into());
        }
        spawn_child(
            &w.0,
            &parent,
            0,
            32,
            id,
            Objective::new("unused"),
            None,
            Script {
                actions: vec![echo.clone()],
            },
            grant,
        )
        .unwrap()
    };
    let _ = run("with", true);
    let store = SqliteEventStore::open(w.0.join(".harness/children/with.sqlite3")).unwrap();
    let with = store
        .events()
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "ToolCompleted")
        .unwrap();
    assert!(with.detail.contains("present"), "{}", with.detail);
    drop(store);
    let _ = run("without", false);
    let store = SqliteEventStore::open(w.0.join(".harness/children/without.sqlite3")).unwrap();
    let without = store
        .events()
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "ToolCompleted")
        .unwrap();
    assert!(!without.detail.contains("present"), "{}", without.detail);
    drop(store);
    unsafe { std::env::remove_var("HARNESS_DELEG_ENV_XYZ") };
}

#[test]
fn verifier_child_is_read_only() {
    let w = Workspace::new();
    fs::create_dir_all(w.0.join("audit")).unwrap();
    fs::write(w.0.join("audit/data.txt"), "evidence").unwrap();
    let parent = parent_policy(&w.0);
    let mut grant = ChildGrant::scoped("audit", 32);
    grant.read_only = true;
    let report = spawn_child(
        &w.0,
        &parent,
        0,
        32,
        "verifier",
        Objective::new("create file overwrite.txt with content malicious"),
        None,
        HeuristicModel,
        grant,
    )
    .unwrap();
    assert!(matches!(report.outcome, RunOutcome::Failed(_)));
    assert!(!w.0.join("audit/overwrite.txt").exists());
    // Reads still work through the narrowed policy.
    let child = parent.narrow_to_subdir("audit").unwrap();
    let observation = ToolRegistry::milestone_default().execute(
        &Action::ReadFile {
            path: "data.txt".into(),
        },
        &child,
    );
    assert!(observation.ok);
    assert_eq!(observation.data, "evidence");
}

#[test]
fn budget_firewall_blocks_over_delegation() {
    let w = Workspace::new();
    let parent = parent_policy(&w.0);
    let denied = spawn_child(
        &w.0,
        &parent,
        30,
        32,
        "greedy",
        Objective::new("create file ok.txt with content hi"),
        None,
        HeuristicModel,
        ChildGrant::scoped("child", 3),
    );
    assert!(denied.is_err());
    assert!(!w.0.join(".harness/children/greedy.sqlite3").exists());
    // Exact fit is allowed and fully accounted to the child.
    let report = spawn_child(
        &w.0,
        &parent,
        29,
        32,
        "exact",
        Objective::new("create file ok.txt with content hi"),
        None,
        HeuristicModel,
        ChildGrant::scoped("child", 3),
    )
    .unwrap();
    assert!(matches!(report.outcome, RunOutcome::Completed(_)));
    assert_eq!(report.tool_calls_used, 3);
}

#[test]
fn parent_collects_child_artifacts_with_bounds() {
    let w = Workspace::new();
    fs::create_dir_all(w.0.join("worker")).unwrap();
    fs::write(w.0.join("worker/b.txt"), "second").unwrap();
    fs::write(w.0.join("worker/a.txt"), "first").unwrap();
    let artifacts = collect_artifacts(&w.0.join("worker"), 16, 1024).unwrap();
    assert_eq!(
        artifacts,
        vec![
            ("a.txt".to_string(), "first".to_string()),
            ("b.txt".to_string(), "second".to_string()),
        ]
    );
    assert!(collect_artifacts(&w.0.join("worker"), 1, 1024).is_err());
    fs::write(w.0.join("worker/big.txt"), "x".repeat(2048)).unwrap();
    assert!(collect_artifacts(&w.0.join("worker"), 16, 1024).is_err());
    fs::remove_file(w.0.join("worker/big.txt")).unwrap();
}

#[test]
fn delegation_composes_with_parent_runtime_checkpoints() {
    // A parent run records the delegation itself as durable plan evidence.
    let w = Workspace::new();
    let db = w.0.join("run.sqlite3");
    let mut parent = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        parent_policy(&w.0),
        SqliteEventStore::open(&db).unwrap(),
    );
    parent
        .create_run(
            Objective::new("create file done.txt with content done"),
            {
                let mut plan = harness_core::plan::PlanState::default();
                plan.add_goal("top", "delegate then finish", 1).unwrap();
                plan.add_task("delegate", "top", "child writes delegated.txt", vec![])
                    .unwrap();
                plan.add_task("finish", "top", "write done.txt", vec!["delegate".into()])
                    .unwrap();
                plan
            },
            None,
        )
        .unwrap();
    let policy = parent_policy(&w.0);
    let report = spawn_child(
        &w.0,
        &policy,
        0,
        32,
        "worker",
        Objective::new("create file delegated.txt with content proof"),
        None,
        HeuristicModel,
        ChildGrant::scoped("child", 32),
    )
    .unwrap();
    assert!(matches!(report.outcome, RunOutcome::Completed(_)));
    parent
        .revise_plan("child evidence", |plan| {
            plan.mark_succeeded("delegate", "child completed: delegated.txt")
        })
        .unwrap();
    assert_eq!(
        parent.plan_snapshot().unwrap().ready_tasks(),
        vec!["finish".to_string()]
    );
    let artifacts = collect_artifacts(&report.workspace, 16, 1024).unwrap();
    assert!(
        artifacts
            .iter()
            .any(|(name, data)| name == "delegated.txt" && data == "proof")
    );
}

#[test]
fn unsafe_child_ids_are_rejected_before_side_effects() {
    let w = Workspace::new();
    let parent = parent_policy(&w.0);
    for id in [
        "",
        "../escape",
        "nested/id",
        "nested\\id",
        "C:\\escape",
        "id:stream",
        ".",
        "..",
        &"x".repeat(129),
    ] {
        let error = spawn_child(
            &w.0,
            &parent,
            0,
            32,
            id,
            Objective::new("create file ok.txt with content hi"),
            None,
            HeuristicModel,
            ChildGrant::scoped("child", 32),
        )
        .unwrap_err();
        assert!(error.to_string().contains("child ID"), "{id}: {error}");
    }
    assert!(!w.0.join("child").exists());
    assert!(!w.0.join(".harness").exists());
}

#[test]
fn read_only_child_refuses_even_authorized_shell_grants() {
    let w = Workspace::new();
    let mut parent = parent_policy(&w.0);
    parent.allow_shell_program("cargo");
    let mut grant = ChildGrant::scoped("child", 32);
    grant.read_only = true;
    grant.shell_grants.push(("cargo".into(), vec![]));
    let error = spawn_child(
        &w.0,
        &parent,
        0,
        32,
        "verifier",
        Objective::new("unused"),
        None,
        HeuristicModel,
        grant,
    )
    .unwrap_err();
    assert!(error.to_string().contains("read-only"));
    assert!(!w.0.join("child").exists());
}

#[test]
fn database_root_must_match_parent_authority() {
    let w = Workspace::new();
    let other = Workspace::new();
    let parent = parent_policy(&w.0);
    let error = spawn_child(
        &other.0,
        &parent,
        0,
        32,
        "worker",
        Objective::new("unused"),
        None,
        HeuristicModel,
        ChildGrant::scoped("child", 32),
    )
    .unwrap_err();
    assert!(error.to_string().contains("parent workspace"));
    assert!(!w.0.join("child").exists());
    assert!(!other.0.join(".harness").exists());
}
