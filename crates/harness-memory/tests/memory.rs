use harness_memory::{MemoryKind, MemoryQuery, MemoryStore, NewMemory};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn store() -> (MemoryStore, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "harness-memory-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let db = path.join("memory.sqlite3");
    (MemoryStore::open(&db).unwrap(), path)
}

fn remember(
    store: &mut MemoryStore,
    kind: MemoryKind,
    content: &str,
    confidence: f64,
    provenance: &str,
    salience: f64,
) -> i64 {
    store
        .write(NewMemory::new(
            kind, content, confidence, provenance, salience,
        ))
        .unwrap()
}

#[test]
fn relevant_procedure_outranks_distractors() {
    let (mut store, root) = store();
    remember(
        &mut store,
        MemoryKind::Procedural,
        "repair wrong constants with a same-length digest-guarded patch at the searched offset",
        0.9,
        "run-1",
        0.8,
    );
    remember(
        &mut store,
        MemoryKind::Procedural,
        "browser automation prefers accessibility selectors over screenshots",
        0.9,
        "run-2",
        0.8,
    );
    remember(
        &mut store,
        MemoryKind::Episodic,
        "repaired plan-first by patching 41 to 42 and cargo test passed",
        0.7,
        "run-1",
        0.6,
    );
    let hits = store
        .retrieve(&MemoryQuery::new(
            vec![MemoryKind::Procedural, MemoryKind::Episodic],
            vec!["patch".into(), "constant".into()],
            5,
        ))
        .unwrap();
    assert!(!hits.is_empty());
    assert!(hits[0].record.content.contains("digest-guarded patch"));
    assert!(hits.iter().all(|hit| !hit.reasons.is_empty()));
    assert!(
        !hits
            .iter()
            .any(|hit| hit.record.content.contains("browser")),
        "distractor shares no terms and must not retrieve"
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn superseded_memories_never_retrieve() {
    let (mut store, root) = store();
    let stale = remember(
        &mut store,
        MemoryKind::Procedural,
        "fix plausible-but-wrong by patching 41 to 43",
        0.6,
        "run-1",
        0.7,
    );
    let fresh = remember(
        &mut store,
        MemoryKind::Procedural,
        "fix plausible-but-wrong by patching 41 to 42",
        0.9,
        "run-2",
        0.7,
    );
    store.supersede(stale, fresh).unwrap();
    let hits = store
        .retrieve(&MemoryQuery::new(
            vec![MemoryKind::Procedural],
            vec!["plausible".into(), "wrong".into()],
            5,
        ))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.id, fresh);
    assert!(store.get(stale).unwrap().unwrap().superseded_by == Some(fresh));
    assert!(store.supersede(stale, fresh).is_err());
    assert!(store.supersede(fresh, fresh).is_err());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn consolidation_merges_duplicates_and_keeps_evidence() {
    let (mut store, root) = store();
    let first = remember(
        &mut store,
        MemoryKind::Failure,
        "cargo test failed: assertion left 43 right 42",
        0.5,
        "run-1",
        0.6,
    );
    remember(
        &mut store,
        MemoryKind::Failure,
        "cargo test failed: assertion left 43 right 42",
        0.8,
        "run-2",
        0.6,
    );
    remember(
        &mut store,
        MemoryKind::Failure,
        "linker link.exe missing without build env grants",
        0.9,
        "run-3",
        0.9,
    );
    assert_eq!(store.consolidate().unwrap(), 1);
    let keeper = store.get(first).unwrap().unwrap();
    assert_eq!(keeper.confidence, 0.8);
    assert_eq!(keeper.reinforced, 2);
    let hits = store
        .retrieve(&MemoryQuery::new(vec![MemoryKind::Failure], vec![], 10))
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(store.consolidate().unwrap(), 0);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn retention_forgets_old_trivia_but_keeps_salient_rows() {
    let (mut store, root) = store();
    remember(
        &mut store,
        MemoryKind::Episodic,
        "old trivia nobody needs",
        0.4,
        "run-1",
        0.2,
    );
    remember(
        &mut store,
        MemoryKind::Project,
        "workspace pins rust-toolchain 1.98.1",
        0.9,
        "run-1",
        0.95,
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    // Pretend a year passed; keep only salience >= 0.5.
    let deleted = store
        .forget_as_of(now + 365 * 86_400_000, 30 * 86_400_000, 0.5)
        .unwrap();
    assert_eq!(deleted, 1);
    let remaining = store
        .retrieve(&MemoryQuery::new(
            vec![MemoryKind::Episodic, MemoryKind::Project],
            vec![],
            10,
        ))
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert!(remaining[0].record.content.contains("rust-toolchain"));
    // Nothing old enough yet: no deletions.
    assert_eq!(store.forget_as_of(now, 30 * 86_400_000, 0.5).unwrap(), 0);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn kind_filter_and_limit_behave() {
    let (mut store, root) = store();
    for index in 0..5 {
        remember(
            &mut store,
            MemoryKind::Episodic,
            &format!("episode {index} about patching"),
            0.5,
            "run-1",
            0.5,
        );
    }
    remember(
        &mut store,
        MemoryKind::Project,
        "project fact about patching",
        0.5,
        "run-1",
        0.5,
    );
    let hits = store
        .retrieve(&MemoryQuery::new(
            vec![MemoryKind::Project],
            vec!["patching".into()],
            10,
        ))
        .unwrap();
    assert_eq!(hits.len(), 1);
    let hits = store
        .retrieve(&MemoryQuery::new(
            vec![MemoryKind::Episodic],
            vec!["patching".into()],
            3,
        ))
        .unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(store.count().unwrap(), 6);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
