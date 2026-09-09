use harness_core::agent::RunState;
use harness_core::event_store::EventStore;
use harness_core::verification::SuccessCriterion;
use harness_core::{
    Action, AgentRuntime, HeuristicModel, Model, Objective, Observation, PermissionPolicy,
    RunOutcome, SqliteEventStore, StepDecision, ToolRegistry,
};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(std::path::PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "harness-coding-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn db(&self) -> std::path::PathBuf {
        self.0.join("run.sqlite3")
    }
    fn seeded_state(&self, objective: Objective, pending: Option<Action>) -> RunState {
        RunState {
            version: 1,
            objective,
            workspace: fs::canonicalize(&self.0).unwrap(),
            history: vec![],
            steps: 0,
            max_steps: 8,
            pending,
            outcome: None,
            tool_budget: Some(Default::default()),
            success_criterion: None,
            plan: Default::default(),
        }
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

const REPO: &str = "// service configuration\nfn answer() -> i32 {\n    return 41;\n}\n";

fn json_field(observation: &Observation, field: &str) -> String {
    serde_json::from_str::<serde_json::Value>(&observation.data).unwrap()[field]
        .as_str()
        .unwrap()
        .to_string()
}

/// Deterministic test agent: search for the bug, read the exact span, hash
/// for a fresh digest, then patch. No model intelligence is claimed; this
/// exercises the runtime boundary with typed actions.
struct RepairAgent {
    path: String,
    bug: String,
    fix: String,
}

impl RepairAgent {
    fn offset_of_bug(&self, history: &[(Action, Observation)]) -> Result<u64, String> {
        let value: serde_json::Value =
            serde_json::from_str(&history[0].1.data).map_err(|e| e.to_string())?;
        value["matches"][0]["offset"]
            .as_u64()
            .ok_or_else(|| "search returned no match offset".to_string())
    }
}

impl Model for RepairAgent {
    fn name(&self) -> &str {
        "deterministic-repair-agent"
    }

    fn decide(&mut self, _: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        if let Some((_, obs)) = history.iter().find(|(_, obs)| !obs.ok) {
            return StepDecision::Fail(format!("action failed: {}", obs.summary));
        }
        match history.len() {
            0 => StepDecision::Act(Action::SearchFile {
                path: self.path.clone(),
                needle: self.bug.clone(),
                max_matches: 10,
            }),
            1 => match self.offset_of_bug(history) {
                Ok(offset) => StepDecision::Act(Action::ReadFileRange {
                    path: self.path.clone(),
                    offset,
                    length: self.bug.len() as u64,
                }),
                Err(reason) => StepDecision::Fail(reason),
            },
            2 => {
                let text: serde_json::Value =
                    serde_json::from_str(&history[1].1.data).unwrap_or_default();
                if text["text"].as_str() != Some(self.bug.as_str()) {
                    return StepDecision::Fail("range observation went stale".into());
                }
                StepDecision::Act(Action::HashFile {
                    path: self.path.clone(),
                })
            }
            3 => {
                let digest = json_field(&history[2].1, "sha256");
                match self.offset_of_bug(history) {
                    Ok(offset) => StepDecision::Act(Action::PatchFile {
                        path: self.path.clone(),
                        offset,
                        expected: self.bug.clone(),
                        replacement: self.fix.clone(),
                        expected_sha256: digest,
                    }),
                    Err(reason) => StepDecision::Fail(reason),
                }
            }
            4 => StepDecision::Complete("patched".into()),
            _ => StepDecision::Fail("unexpected runtime history shape".into()),
        }
    }
}

#[test]
fn deterministic_agent_inspects_patches_and_verifies_controlled_repo() {
    let w = Workspace::new();
    fs::write(w.0.join("service.rs"), REPO).unwrap();
    let offset = REPO.find("41").unwrap() as u64;
    let agent = RepairAgent {
        path: "service.rs".into(),
        bug: "41".into(),
        fix: "42".into(),
    };
    let mut runtime = AgentRuntime::new(
        agent,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    );
    let criterion = SuccessCriterion::FileRange {
        path: "service.rs".into(),
        offset,
        expected: "42".into(),
    };
    let outcome = runtime
        .run_with_criterion(
            Objective::new("repair service.rs: replace 41 with 42"),
            Some(criterion),
        )
        .unwrap();
    drop(runtime);
    assert!(matches!(outcome, RunOutcome::Completed(_)), "{outcome:?}");
    assert!(
        fs::read_to_string(w.0.join("service.rs"))
            .unwrap()
            .contains("return 42;")
    );
    // Four agent actions plus one runtime-owned independent verification.
    let state = SqliteEventStore::open(w.db())
        .unwrap()
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(state.tool_budget.unwrap().used, 5);
    assert!(matches!(state.history[3].0, Action::PatchFile { .. }));
    assert!(matches!(state.history[4].0, Action::ReadFileRange { .. }));
    let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
    assert!(events.iter().any(|e| e.kind == "VerificationPassed"));
    assert!(events.iter().any(|e| e.kind == "GoalCompleted"));
    // Discovery exposes the full inspect/patch surface used above.
    let actions: Vec<String> = ToolRegistry::milestone_default()
        .descriptors()
        .into_iter()
        .flat_map(|d| d.actions.into_iter().map(|a| a.action))
        .collect();
    for expected in ["SearchFile", "ReadFileRange", "HashFile", "PatchFile"] {
        assert!(actions.contains(&expected.to_string()), "{actions:?}");
    }
    // Duplicate prevention: reopening a terminal run repeats nothing.
    let before = events;
    {
        let again = AgentRuntime::new(
            RepairAgent {
                path: "service.rs".into(),
                bug: "41".into(),
                fix: "42".into(),
            },
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
fn stale_patch_is_rejected_without_touching_the_file() {
    let w = Workspace::new();
    fs::write(w.0.join("service.rs"), REPO).unwrap();
    let policy = PermissionPolicy::milestone_default(&w.0);
    let tools = ToolRegistry::milestone_default();
    let digest = json_field(
        &tools.execute(
            &Action::HashFile {
                path: "service.rs".into(),
            },
            &policy,
        ),
        "sha256",
    );
    let offset = REPO.find("41").unwrap() as u64;
    let stale = Action::PatchFile {
        path: "service.rs".into(),
        offset,
        expected: "41".into(),
        replacement: "43".into(),
        expected_sha256: "0".repeat(64),
    };
    assert!(!tools.execute(&stale, &policy).ok);
    let wrong_span = Action::PatchFile {
        path: "service.rs".into(),
        offset,
        expected: "99".into(),
        replacement: "43".into(),
        expected_sha256: digest.clone(),
    };
    assert!(!tools.execute(&wrong_span, &policy).ok);
    let repair = Action::PatchFile {
        path: "service.rs".into(),
        offset,
        expected: "41".into(),
        replacement: "42".into(),
        expected_sha256: digest,
    };
    let observation = tools.execute(&repair, &policy);
    assert!(observation.ok, "{}", observation.data);
    assert!(!tools.execute(&repair, &policy).ok);
    let contents = fs::read_to_string(w.0.join("service.rs")).unwrap();
    assert!(contents.contains("return 42;"));
    assert!(!contents.contains("return 41;"));
}

#[test]
fn search_reports_offsets_lines_and_truncation() {
    let w = Workspace::new();
    fs::write(w.0.join("service.rs"), REPO).unwrap();
    let policy = PermissionPolicy::milestone_default(&w.0);
    let tools = ToolRegistry::milestone_default();
    let observation = tools.execute(
        &Action::SearchFile {
            path: "service.rs".into(),
            needle: "return".into(),
            max_matches: 10,
        },
        &policy,
    );
    assert!(observation.ok, "{}", observation.data);
    let result: serde_json::Value = serde_json::from_str(&observation.data).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["matches"][0]["line"], 3);
    assert_eq!(result["matches"][0]["offset"], REPO.find("return").unwrap());
    assert!(
        result["matches"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("return 41;")
    );
    assert_eq!(result["truncated"], false);
    fs::write(w.0.join("many"), "a\n".repeat(100)).unwrap();
    let observation = tools.execute(
        &Action::SearchFile {
            path: "many".into(),
            needle: "a".into(),
            max_matches: 10,
        },
        &policy,
    );
    assert!(observation.ok, "{}", observation.data);
    let result: serde_json::Value = serde_json::from_str(&observation.data).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 10);
    assert_eq!(result["truncated"], true);
    for action in [
        Action::SearchFile {
            path: "service.rs".into(),
            needle: "".into(),
            max_matches: 10,
        },
        Action::SearchFile {
            path: "service.rs".into(),
            needle: "x".repeat(1025),
            max_matches: 10,
        },
        Action::SearchFile {
            path: "service.rs".into(),
            needle: "return".into(),
            max_matches: 0,
        },
        Action::SearchFile {
            path: "service.rs".into(),
            needle: "return".into(),
            max_matches: 51,
        },
        Action::SearchFile {
            path: "../escape".into(),
            needle: "return".into(),
            max_matches: 10,
        },
        Action::SearchFile {
            path: ".harness/state".into(),
            needle: "return".into(),
            max_matches: 10,
        },
        Action::SearchFile {
            path: "missing".into(),
            needle: "return".into(),
            max_matches: 10,
        },
    ] {
        assert!(!tools.execute(&action, &policy).ok, "{action}");
    }
}

fn seeded_history_state(w: &Workspace, pending: Action) -> RunState {
    let mut state = w.seeded_state(
        Objective::new("create file result.txt with content durable"),
        Some(pending),
    );
    state.history.push((
        Action::WriteFile {
            path: "result.txt".into(),
            contents: "durable".into(),
        },
        Observation {
            ok: true,
            summary: "written".into(),
            data: "7".into(),
        },
    ));
    state
}

fn checkpoint(w: &Workspace, state: &RunState) {
    SqliteEventStore::open(w.db())
        .unwrap()
        .checkpoint(state)
        .unwrap();
}

/// Resume model that accepts the reconciled read-only observation and asks
/// for independent completion verification through the normal runtime path.
struct AcceptRecovered;
impl Model for AcceptRecovered {
    fn name(&self) -> &str {
        "accept-recovered"
    }
    fn decide(&mut self, _: &Objective, _: &[(Action, Observation)]) -> StepDecision {
        StepDecision::Complete("recovered".into())
    }
}

#[test]
fn interrupted_range_hash_and_search_reconcile_with_stable_ids() {
    for pending in [
        Action::HashFile {
            path: "result.txt".into(),
        },
        Action::ReadFileRange {
            path: "result.txt".into(),
            offset: 0,
            length: 7,
        },
        Action::SearchFile {
            path: "result.txt".into(),
            needle: "dur".into(),
            max_matches: 10,
        },
    ] {
        let w = Workspace::new();
        fs::write(w.0.join("result.txt"), "durable").unwrap();
        let state = seeded_history_state(&w, pending.clone());
        let action_id = state.next_action_id();
        checkpoint(&w, &state);
        let outcome = AgentRuntime::new(
            AcceptRecovered,
            ToolRegistry::milestone_default(),
            PermissionPolicy::milestone_default(&w.0),
            SqliteEventStore::open(w.db()).unwrap(),
        )
        .reconcile_and_resume()
        .unwrap();
        assert!(matches!(outcome, RunOutcome::Completed(_)), "{pending}");
        let events = SqliteEventStore::open(w.db()).unwrap().events().unwrap();
        let reconciled = events
            .iter()
            .find(|e| e.kind == "ActionReconciled")
            .unwrap();
        let detail: serde_json::Value = serde_json::from_str(&reconciled.detail).unwrap();
        assert_eq!(detail["action_id"], action_id);
        assert!(
            !events
                .iter()
                .any(|e| e.kind == "ToolCalled" && e.detail == pending.to_string()),
            "recovery re-observes without a duplicate tool invocation record"
        );
    }
}

#[test]
fn interrupted_patch_stays_blocked_without_side_effects() {
    let w = Workspace::new();
    fs::write(w.0.join("service.rs"), REPO).unwrap();
    let policy = PermissionPolicy::milestone_default(&w.0);
    let digest = json_field(
        &ToolRegistry::milestone_default().execute(
            &Action::HashFile {
                path: "service.rs".into(),
            },
            &policy,
        ),
        "sha256",
    );
    let pending = Action::PatchFile {
        path: "service.rs".into(),
        offset: REPO.find("41").unwrap() as u64,
        expected: "41".into(),
        replacement: "42".into(),
        expected_sha256: digest,
    };
    let state = w.seeded_state(Objective::new("repair service.rs"), Some(pending.clone()));
    checkpoint(&w, &state);
    let error = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    )
    .reconcile_and_resume()
    .unwrap_err()
    .to_string();
    assert!(error.contains("unsupported"), "{error}");
    assert_eq!(fs::read_to_string(w.0.join("service.rs")).unwrap(), REPO);
    assert_eq!(
        SqliteEventStore::open(w.db())
            .unwrap()
            .load()
            .unwrap()
            .unwrap()
            .pending,
        Some(pending)
    );
}

#[cfg(unix)]
#[test]
fn permission_change_to_symlink_denies_search_reconciliation() {
    let w = Workspace::new();
    let outside = Workspace::new();
    fs::write(outside.0.join("data"), "durable").unwrap();
    let pending = Action::SearchFile {
        path: "link/data".into(),
        needle: "dur".into(),
        max_matches: 10,
    };
    let state = w.seeded_state(
        Objective::new("create file result.txt with content durable"),
        Some(pending),
    );
    checkpoint(&w, &state);
    std::os::unix::fs::symlink(&outside.0, w.0.join("link")).unwrap();
    let error = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    )
    .reconcile_and_resume()
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("symlink") || error.contains("reparse"),
        "{error}"
    );
    assert!(
        SqliteEventStore::open(w.db())
            .unwrap()
            .load()
            .unwrap()
            .unwrap()
            .pending
            .is_some()
    );
}

#[cfg(windows)]
#[test]
fn permission_change_to_junction_denies_search_reconciliation() {
    let w = Workspace::new();
    let outside = Workspace::new();
    fs::write(outside.0.join("data"), "durable").unwrap();
    let pending = Action::SearchFile {
        path: "junction/data".into(),
        needle: "dur".into(),
        max_matches: 10,
    };
    let state = w.seeded_state(
        Objective::new("create file result.txt with content durable"),
        Some(pending),
    );
    checkpoint(&w, &state);
    let link = w.0.join("junction");
    let created = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(&outside.0)
        .output()
        .unwrap();
    assert!(created.status.success());
    let error = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&w.0),
        SqliteEventStore::open(w.db()).unwrap(),
    )
    .reconcile_and_resume()
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("symlink") || error.contains("reparse"),
        "{error}"
    );
    assert!(
        SqliteEventStore::open(w.db())
            .unwrap()
            .load()
            .unwrap()
            .unwrap()
            .pending
            .is_some()
    );
    fs::remove_dir(link).unwrap();
}
