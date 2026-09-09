use harness_benchmark::{
    TaskOutcome, TaskSpec, recall_fix, record_repair, run_task, write_fixture,
};
use std::fs;
use std::path::PathBuf;

fn workspace(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "harness-bench-recall-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn memory_db(root: &std::path::Path) -> PathBuf {
    root.join(".harness/memory.sqlite3")
}

fn patch_replacements(db: &std::path::Path) -> Vec<String> {
    use harness_core::event_store::EventStore;
    let store = harness_core::SqliteEventStore::open(db).unwrap();
    store
        .events()
        .unwrap()
        .into_iter()
        .filter_map(|event| {
            if event.kind != "ActionPrepared" {
                return None;
            }
            let detail: serde_json::Value = serde_json::from_str(&event.detail).ok()?;
            let inner = detail.get("action")?.as_object()?.values().next()?;
            inner
                .get("replacement")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .collect()
}

#[test]
fn recalled_experience_improves_repeated_benchmark() {
    let root = workspace("primed");
    let memory = memory_db(&root);
    // Phase one: full failure-driven loop learns the lesson the hard way.
    let teacher = TaskSpec {
        name: "lesson-task".into(),
        bug: "41".into(),
        fix_first: "43".into(),
        fix_final: "42".into(),
        test_name: "lesson_task_is_fixed".into(),
        shape: "wrong-constant".into(),
    };
    write_fixture(&root, &teacher).unwrap();
    let taught = run_task(&root, &root.join(".harness/lesson.sqlite3"), &teacher).unwrap();
    assert_eq!(taught.outcome, TaskOutcome::Completed);
    record_repair(&memory, &teacher, &taught, "lesson-task").unwrap();
    // A confident distractor for another shape must never leak across.
    {
        let mut store = harness_memory::MemoryStore::open(&memory).unwrap();
        store
            .write(harness_memory::NewMemory::new(
                harness_memory::MemoryKind::Procedural,
                serde_json::json!({"shape": "off-by-one", "bug": "41", "fix": "99"}).to_string(),
                0.95,
                "other-project",
                0.8,
            ))
            .unwrap();
    }
    // Phase two: same bug value, new task. Baseline stumbles; primed recalls.
    let task = TaskSpec {
        name: "repeat-task".into(),
        bug: "41".into(),
        fix_first: "43".into(),
        fix_final: "42".into(),
        test_name: "repeat_task_is_fixed".into(),
        shape: "wrong-constant".into(),
    };
    write_fixture(&root, &task).unwrap();
    let baseline = run_task(&root, &root.join(".harness/baseline.sqlite3"), &task).unwrap();
    assert_eq!(baseline.outcome, TaskOutcome::Completed);
    let hint = recall_fix(&memory, &task, 0.7)
        .unwrap()
        .expect("lesson recalled");
    assert_eq!(hint.fix, "42");
    assert!(!hint.reasons.is_empty());
    println!("hint reasons: {:?}", hint.reasons);
    let mut primed_spec = task.clone();
    primed_spec.fix_first = hint.fix.clone();
    // Reset the fixture: the baseline run already repaired this directory.
    write_fixture(&root, &task).unwrap();
    let primed = run_task(&root, &root.join(".harness/primed.sqlite3"), &primed_spec).unwrap();
    assert_eq!(primed.outcome, TaskOutcome::Completed);
    assert!(primed.verified);
    assert!(
        primed.tool_calls < baseline.tool_calls,
        "primed {} vs baseline {}",
        primed.tool_calls,
        baseline.tool_calls
    );
    // The primed run never attempted the wrong fix or the distractor fix:
    // memory informed, verification still decided.
    let replacements = patch_replacements(&root.join(".harness/primed.sqlite3"));
    assert!(
        replacements.iter().all(|replacement| replacement == "42"),
        "{replacements:?}"
    );
    assert!(!replacements.iter().any(|replacement| replacement == "99"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn stale_memory_fails_safe_through_test_evidence() {
    let root = workspace("stale");
    let memory = memory_db(&root);
    // Plant a confident but wrong lesson: the file asserts 42, memory says 99.
    {
        let mut store = harness_memory::MemoryStore::open(&memory).unwrap();
        store
            .write(harness_memory::NewMemory::new(
                harness_memory::MemoryKind::Procedural,
                serde_json::json!({"shape": "wrong-constant", "bug": "41", "fix": "99"})
                    .to_string(),
                0.9,
                "outdated-run",
                0.8,
            ))
            .unwrap();
    }
    let task = TaskSpec {
        name: "stale-task".into(),
        bug: "41".into(),
        fix_first: "99".into(),
        fix_final: "42".into(),
        test_name: "stale_task_is_fixed".into(),
        shape: "wrong-constant".into(),
    };
    write_fixture(&root, &task).unwrap();
    let hint = recall_fix(&memory, &task, 0.7)
        .unwrap()
        .expect("stale hint recalled");
    assert_eq!(hint.fix, "99");
    // The agent tries the remembered fix, the tests reject it, and the
    // normal repair loop still completes: memory advises, evidence decides.
    let mut primed = task.clone();
    primed.fix_first = hint.fix;
    let report = run_task(&root, &root.join(".harness/stale.sqlite3"), &primed).unwrap();
    assert_eq!(report.outcome, TaskOutcome::Completed);
    assert!(report.verified);
    assert!(report.retries >= 1);
    let replacements = patch_replacements(&root.join(".harness/stale.sqlite3"));
    assert!(replacements.contains(&"99".to_string()), "{replacements:?}");
    assert!(replacements.contains(&"42".to_string()), "{replacements:?}");
    // Retire the stale lesson: it must stop retrieving afterwards.
    {
        let mut store = harness_memory::MemoryStore::open(&memory).unwrap();
        let correction = store
            .write(harness_memory::NewMemory::new(
                harness_memory::MemoryKind::Procedural,
                serde_json::json!({"shape": "wrong-constant", "bug": "41", "fix": "42"})
                    .to_string(),
                0.95,
                "stale-task",
                0.8,
            ))
            .unwrap();
        store.supersede(hint.memory_id, correction).unwrap();
    }
    let corrected = recall_fix(&memory, &task, 0.7)
        .unwrap()
        .expect("correction recalled");
    assert_eq!(corrected.fix, "42");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn only_verified_experience_becomes_memory() {
    let root = workspace("guard");
    let memory = memory_db(&root);
    let task = TaskSpec {
        name: "guard-task".into(),
        bug: "41".into(),
        fix_first: "42".into(),
        fix_final: "42".into(),
        test_name: "guard_task_is_fixed".into(),
        shape: "wrong-constant".into(),
    };
    write_fixture(&root, &task).unwrap();
    let report = run_task(&root, &root.join(".harness/guard.sqlite3"), &task).unwrap();
    assert_eq!(report.outcome, TaskOutcome::Completed);
    let mut unverified = report.clone();
    unverified.verified = false;
    assert!(record_repair(&memory, &task, &unverified, "guard").is_err());
    record_repair(&memory, &task, &report, "guard").unwrap();
    assert!(recall_fix(&memory, &task, 0.7).unwrap().is_some());
    // Unknown shapes and low-confidence recalls stay silent.
    let mut other = task.clone();
    other.shape = "never-seen".into();
    assert!(recall_fix(&memory, &other, 0.7).unwrap().is_none());
    assert!(recall_fix(&memory, &task, 0.99).unwrap().is_none());
    fs::remove_dir_all(&root).unwrap();
}
