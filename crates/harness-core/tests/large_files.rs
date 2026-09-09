use harness_core::{
    Action, AgentRuntime, Model, Objective, Observation, PermissionPolicy, RunOutcome,
    SqliteEventStore, StepDecision, ToolRegistry,
};
use harness_core::{event_store::EventStore, tools::MAX_FILE_BYTES};
use serde_json::Value;
use std::fs;

fn execute(root: &std::path::Path, action: Action) -> Observation {
    ToolRegistry::milestone_default().execute(&action, &PermissionPolicy::milestone_default(root))
}
fn hash(root: &std::path::Path) -> String {
    let obs = execute(
        root,
        Action::HashFile {
            path: "data".into(),
        },
    );
    assert!(obs.ok, "{}", obs.data);
    serde_json::from_str::<Value>(&obs.data).unwrap()["sha256"]
        .as_str()
        .unwrap()
        .into()
}
fn patch(digest: String, offset: u64, expected: &str, replacement: &str) -> Action {
    Action::PatchFile {
        path: "data".into(),
        offset,
        expected: expected.into(),
        replacement: replacement.into(),
        expected_sha256: digest,
    }
}

#[test]
fn hashes_known_sha256_vector() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("data"), "abc").unwrap();
    assert_eq!(
        hash(root.path()),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn range_reads_large_file_without_relaxing_whole_file_limit() {
    let root = tempfile::tempdir().unwrap();
    let contents = format!("{}tail", "x".repeat(MAX_FILE_BYTES + 16));
    fs::write(root.path().join("data"), &contents).unwrap();
    assert!(
        !execute(
            root.path(),
            Action::ReadFile {
                path: "data".into()
            }
        )
        .ok
    );
    let obs = execute(
        root.path(),
        Action::ReadFileRange {
            path: "data".into(),
            offset: (MAX_FILE_BYTES + 16) as u64,
            length: 4,
        },
    );
    assert!(obs.ok, "{}", obs.data);
    let result: Value = serde_json::from_str(&obs.data).unwrap();
    assert_eq!(result["text"], "tail");
    assert_eq!(result["eof"], true);
    assert_eq!(result["file_size"], contents.len());
    for (offset, length) in [
        (0, MAX_FILE_BYTES as u64 + 1),
        (u64::MAX, 1),
        (contents.len() as u64, 1),
    ] {
        assert!(
            !execute(
                root.path(),
                Action::ReadFileRange {
                    path: "data".into(),
                    offset,
                    length
                }
            )
            .ok
        );
    }
    fs::write(root.path().join("data"), "é").unwrap();
    assert!(
        !execute(
            root.path(),
            Action::ReadFileRange {
                path: "data".into(),
                offset: 0,
                length: 1
            }
        )
        .ok
    );
}

#[test]
fn patches_large_file_and_rejects_stale_retry_without_changes() {
    let root = tempfile::tempdir().unwrap();
    let prefix = "x".repeat(MAX_FILE_BYTES + 7);
    let before = format!("{prefix}old tail");
    fs::write(root.path().join("data"), &before).unwrap();
    let action = patch(
        hash(root.path()),
        prefix.len() as u64,
        "old",
        "new and longer",
    );
    let observation = execute(root.path(), action.clone());
    assert!(observation.ok, "{}", observation.data);
    let after = format!("{prefix}new and longer tail");
    assert_eq!(fs::read_to_string(root.path().join("data")).unwrap(), after);
    let output: Value = serde_json::from_str(&observation.data).unwrap();
    assert_eq!(output["after_sha256"], hash(root.path()));
    assert!(!execute(root.path(), action).ok);
    assert_eq!(fs::read_to_string(root.path().join("data")).unwrap(), after);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn patches_support_insertion_deletion_and_reject_bad_preconditions() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("data"), "abc").unwrap();
    for action in [
        patch(hash(root.path()), 0, "wrong", "x"),
        patch(hash(root.path()), u64::MAX, "a", "x"),
        patch("bad digest".into(), 0, "a", "x"),
        patch(hash(root.path()), 0, "a", &"x".repeat(MAX_FILE_BYTES + 1)),
    ] {
        assert!(!execute(root.path(), action).ok);
        assert_eq!(fs::read_to_string(root.path().join("data")).unwrap(), "abc");
    }
    assert!(execute(root.path(), patch(hash(root.path()), 3, "", "def")).ok);
    assert!(execute(root.path(), patch(hash(root.path()), 1, "bcde", "")).ok);
    assert_eq!(fs::read_to_string(root.path().join("data")).unwrap(), "af");
}

#[test]
fn stream_ceiling_and_permissions_apply() {
    let root = tempfile::tempdir().unwrap();
    let file = fs::File::create(root.path().join("data")).unwrap();
    file.set_len(harness_core::large_files::MAX_STREAM_BYTES + 1)
        .unwrap();
    drop(file);
    assert!(
        !execute(
            root.path(),
            Action::HashFile {
                path: "data".into()
            }
        )
        .ok
    );
    for path in ["../escape", ".harness/state", "/absolute"] {
        for action in [
            Action::HashFile { path: path.into() },
            Action::ReadFileRange {
                path: path.into(),
                offset: 0,
                length: 1,
            },
            Action::PatchFile {
                path: path.into(),
                offset: 0,
                expected: "".into(),
                replacement: "x".into(),
                expected_sha256: "0".repeat(64),
            },
        ] {
            assert!(!execute(root.path(), action).ok);
        }
    }
}

#[test]
fn large_file_actions_use_runtime_budget_and_persistence() {
    struct Script(Vec<Action>);
    impl Model for Script {
        fn name(&self) -> &str {
            "large-file-integration"
        }
        fn decide(&mut self, _: &Objective, history: &[(Action, Observation)]) -> StepDecision {
            if history.iter().any(|(_, obs)| !obs.ok) {
                return StepDecision::Fail("action failed".into());
            }
            match self.0.get(history.len()) {
                Some(action) => StepDecision::Act(action.clone()),
                None => StepDecision::Complete("candidate completion".into()),
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("data"),
        format!("old{}", "x".repeat(MAX_FILE_BYTES)),
    )
    .unwrap();
    let script = Script(vec![
        Action::HashFile {
            path: "data".into(),
        },
        patch(hash(root.path()), 0, "old", "new"),
        Action::ReadFileRange {
            path: "data".into(),
            offset: 0,
            length: 3,
        },
        Action::WriteFile {
            path: "receipt".into(),
            contents: "done".into(),
        },
    ]);
    let db = root.path().join(".harness/run.sqlite3");
    let mut runtime = AgentRuntime::new(
        script,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(root.path()),
        SqliteEventStore::open(&db).unwrap(),
    )
    .with_max_tool_calls(5);
    assert!(matches!(
        runtime
            .run(Objective::new("create file receipt with content done"))
            .unwrap(),
        RunOutcome::Completed(_)
    ));
    drop(runtime);
    let mut store = SqliteEventStore::open(db).unwrap();
    let state = store.load().unwrap().unwrap();
    assert_eq!(state.tool_budget.unwrap().used, 5);
    assert!(matches!(state.history[1].0, Action::PatchFile { .. }));
    assert!(
        fs::read(root.path().join("data"))
            .unwrap()
            .starts_with(b"new")
    );
}

#[test]
fn pending_patch_is_not_replayed_after_restart() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("data"), "abc").unwrap();
    let action = patch(hash(root.path()), 0, "a", "z");
    let state = harness_core::agent::RunState {
        version: 1,
        objective: Objective::new("create file receipt with content done"),
        workspace: fs::canonicalize(root.path()).unwrap(),
        history: vec![],
        steps: 1,
        max_steps: 8,
        pending: Some(action.clone()),
        outcome: None,
        tool_budget: Some(Default::default()),
        success_criterion: None,
        plan: Default::default(),
        started_at_ms: None,
        used_tokens: 0,
        used_cost_usd: 0.0,
        resource_limits: Default::default(),
    };
    let db = root.path().join(".harness/run.sqlite3");
    let mut store = SqliteEventStore::open(&db).unwrap();
    store.checkpoint(&state).unwrap();
    drop(store);
    let mut runtime = AgentRuntime::new(
        harness_core::HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(root.path()),
        SqliteEventStore::open(&db).unwrap(),
    );
    assert!(
        runtime
            .reconcile_and_resume()
            .unwrap_err()
            .to_string()
            .contains("unsupported")
    );
    drop(runtime);
    assert_eq!(fs::read_to_string(root.path().join("data")).unwrap(), "abc");
    assert_eq!(
        SqliteEventStore::open(db)
            .unwrap()
            .load()
            .unwrap()
            .unwrap()
            .pending,
        Some(action)
    );
}
