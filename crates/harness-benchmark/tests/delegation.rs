use harness_benchmark::{
    BUILD_ENV_GRANTS, RepairAgent, TaskSpec, task_criterion, task_objective, write_fixture,
};
use harness_core::{
    Action, ChildGrant, Model, Objective, Observation, PermissionPolicy, RunOutcome, StepDecision,
    collect_artifacts, spawn_child,
};
use std::fs;
use std::path::PathBuf;

fn workspace(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "harness-bench-deleg-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn parent_policy(root: &std::path::Path) -> PermissionPolicy {
    let mut policy = PermissionPolicy::milestone_default(root);
    policy.allow_shell_with_arg_prefix("cargo", vec!["test".into()]);
    for name in BUILD_ENV_GRANTS {
        policy.allow_env(*name);
    }
    policy
}

fn worker_spec() -> TaskSpec {
    TaskSpec {
        name: "delegated-a".into(),
        bug: "41".into(),
        fix_first: "42".into(),
        fix_final: "42".into(),
        test_name: "delegated_a_is_fixed".into(),
        shape: "wrong-constant".into(),
    }
}

/// Read-only child agent that recalls the expected fix from shared project
/// memory, reads the file back, and only then claims verification.
struct MemoryCheckingVerifier {
    memory_db: PathBuf,
    path: String,
}

impl MemoryCheckingVerifier {
    fn expected_fix(&self) -> Result<String, String> {
        let store =
            harness_memory::MemoryStore::open(&self.memory_db).map_err(|e| e.to_string())?;
        let hits = store
            .retrieve(&harness_memory::MemoryQuery::new(
                vec![harness_memory::MemoryKind::Project],
                vec!["delegated".into(), "fix".into()],
                5,
            ))
            .map_err(|e| e.to_string())?;
        hits.into_iter()
            .find_map(|hit| {
                serde_json::from_str::<serde_json::Value>(&hit.record.content)
                    .ok()?
                    .get("fix")?
                    .as_str()
                    .map(str::to_string)
            })
            .ok_or_else(|| "no expected fix in shared memory".to_string())
    }
}

impl Model for MemoryCheckingVerifier {
    fn name(&self) -> &str {
        "memory-checking-verifier"
    }

    fn decide(&mut self, _: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        let expected = match self.expected_fix() {
            Ok(fix) => fix,
            Err(reason) => return StepDecision::Fail(reason),
        };
        match history.len() {
            0 => StepDecision::Act(Action::ReadFile {
                path: self.path.clone(),
            }),
            1 => {
                let (_, observation) = &history[0];
                if observation.ok && observation.data.contains(&format!("{{ {expected} }}")) {
                    StepDecision::Complete("verified against shared memory".into())
                } else {
                    StepDecision::Fail("file does not match the remembered fix".into())
                }
            }
            _ => StepDecision::Fail("unexpected history".into()),
        }
    }
}

#[test]
fn delegated_repair_with_read_only_memory_checking_verifier() {
    let root = workspace("deleg");
    let memory_db = root.join(".harness/memory.sqlite3");
    let task = worker_spec();
    // The child world lives at <root>/delegated-a; the fixture nests inside
    // it so the child's relative tool paths resolve there.
    let child_root = root.join("delegated-a");
    fs::create_dir_all(&child_root).unwrap();
    write_fixture(&child_root, &task).unwrap();

    let parent = parent_policy(&root);
    let mut worker_grant = ChildGrant::scoped("delegated-a", 32);
    worker_grant.max_steps = 16;
    worker_grant
        .shell_grants
        .push(("cargo".into(), vec!["test".into()]));
    worker_grant.env_grants = BUILD_ENV_GRANTS
        .iter()
        .map(|name| name.to_string())
        .collect();
    let worker = spawn_child(
        &root,
        &parent,
        0,
        64,
        "worker-a",
        task_objective(&task),
        Some(task_criterion(&child_root, &task).unwrap()),
        RepairAgent::new(task.clone()),
        worker_grant,
    )
    .unwrap();
    assert!(
        matches!(worker.outcome, RunOutcome::Completed(_)),
        "{:?}",
        worker.outcome
    );

    // Parent publishes the verified fix into shared project memory...
    {
        let mut store = harness_memory::MemoryStore::open(&memory_db).unwrap();
        store
            .write(harness_memory::NewMemory::new(
                harness_memory::MemoryKind::Project,
                serde_json::json!({"file": task.file(), "fix": task.fix_final}).to_string(),
                0.9,
                "parent",
                0.8,
            ))
            .unwrap();
    }
    // ...and a read-only verifier child recalls it to check the work.
    // It cannot write even if compromised: the capability is revoked.
    // Runtime verification re-reads the same fixed span independently.
    let mut verifier_grant = ChildGrant::scoped("delegated-a", 16);
    verifier_grant.read_only = true;
    let verifier = spawn_child(
        &root,
        &parent,
        worker.tool_calls_used,
        64,
        "verifier-a",
        Objective::new("verify delegated-a without changing anything"),
        Some(task_criterion(&child_root, &task).unwrap()),
        MemoryCheckingVerifier {
            memory_db: memory_db.clone(),
            path: task.file(),
        },
        verifier_grant,
    )
    .unwrap();
    assert!(
        matches!(verifier.outcome, RunOutcome::Completed(_)),
        "{:?}",
        verifier.outcome
    );

    // The verifier's event log proves it never wrote: no file-mutating
    // tool call, only the read plus the runtime verification read.
    {
        use harness_core::event_store::EventStore;
        let store =
            harness_core::SqliteEventStore::open(root.join(".harness/children/verifier-a.sqlite3"))
                .unwrap();
        let writes = store
            .events()
            .unwrap()
            .into_iter()
            .filter(|event| {
                event.kind == "ToolCalled"
                    && (event.detail.starts_with("write_file:")
                        || event.detail.starts_with("patch_file:"))
            })
            .count();
        assert_eq!(writes, 0);
    }
    // Parent collects artifacts through its own reads. Collection is
    // non-recursive by design (build trees stay out of the mailbox).
    let artifacts =
        collect_artifacts(&child_root.join("tasks/delegated-a/src"), 32, 1024 * 1024).unwrap();
    assert!(
        artifacts
            .iter()
            .any(|(name, data)| name == "main.rs" && data.contains("{ 42 }"))
    );
    fs::remove_dir_all(&root).unwrap();
}
