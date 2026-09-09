use harness_benchmark::{TaskSpec, run_plan, write_fixture};
use std::fs;
use std::path::PathBuf;

fn workspace(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "harness-bench-plan-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn specs() -> Vec<TaskSpec> {
    vec![
        TaskSpec {
            name: "plan-first".into(),
            bug: "41".into(),
            fix_first: "42".into(),
            fix_final: "42".into(),
            test_name: "plan_first_is_fixed".into(),
            shape: "wrong-constant".into(),
        },
        TaskSpec {
            name: "plan-second".into(),
            bug: "43".into(),
            fix_first: "44".into(),
            fix_final: "44".into(),
            test_name: "plan_second_is_fixed".into(),
            shape: "wrong-constant".into(),
        },
    ]
}

/// Worker entry: drives the shared plan, then hangs after the first task
/// commits so the parent can kill the process mid-plan.
#[test]
fn bench_plan_crash_worker() {
    let Some(root) = std::env::var_os("HARNESS_PLAN_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let tasks = specs();
    for task in &tasks {
        write_fixture(&root, task).unwrap();
    }
    run_plan(
        &root,
        &root.join(".harness/plan.sqlite3"),
        "repair both services",
        &tasks,
    )
    .unwrap();
}

#[test]
fn multi_step_plan_survives_kill_without_duplicating_tasks() {
    let root = workspace("multi");
    let tasks = specs();
    for task in &tasks {
        write_fixture(&root, task).unwrap();
    }
    let plan_db = root.join(".harness/plan.sqlite3");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bench_plan_crash_worker", "--nocapture"])
        .env("HARNESS_PLAN_ROOT", &root)
        .env("HARNESS_PLAN_HANG_AFTER_TASK", "plan-first")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
    while !root.join("ready").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        root.join("ready").exists(),
        "worker never finished task one"
    );
    let first_db = root.join(".harness/tasks/plan-first.sqlite3");
    let before = task_events(&first_db);
    child.kill().unwrap();
    child.wait().unwrap();
    // Resume the same plan in this process: the finished task is adopted
    // from its checkpoint while the top-level goal stays intact.
    let summary = run_plan(&root, &plan_db, "repair both services", &tasks).unwrap();
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    assert!(summary.completed);
    assert_eq!(summary.goal, "repair both services");
    assert_eq!(summary.tasks.len(), 2);
    assert_eq!(summary.adopted, 1);
    assert!(summary.tasks.iter().all(|task| task.verified));
    assert_eq!(task_events(&first_db), before);
    fs::remove_dir_all(&root).unwrap();
}

fn task_events(db: &std::path::Path) -> Vec<(String, String)> {
    let store = harness_core::SqliteEventStore::open(db).unwrap();
    store
        .events()
        .unwrap()
        .into_iter()
        .map(|event| (event.kind, event.detail))
        .collect()
}
