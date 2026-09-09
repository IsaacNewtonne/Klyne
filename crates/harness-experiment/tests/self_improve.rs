use harness_experiment::{CheckSuite, Decision, ExperimentRunner, ExperimentSpec};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn git(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn fixture_repo(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "harness-exp-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.98.1\"\nprofile = \"minimal\"\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "/target/\nCargo.lock\n").unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn answer() -> i32 { 42 }\n\nfn main() {\n    println!(\"{}\", answer());\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn value_is_fixed() {\n        assert_eq!(answer(), 42);\n    }\n\n    #[test]\n    fn arithmetic_holds() {\n        assert_eq!(answer() + 0, 42);\n    }\n}\n",
    )
    .unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["add", "-A"]);
    git(
        &root,
        &[
            "-c",
            "user.name=harness-test",
            "-c",
            "user.email=test@local",
            "commit",
            "-m",
            "fixture",
        ],
    );
    root
}

fn spec(id: &str) -> ExperimentSpec {
    ExperimentSpec {
        id: id.into(),
        branch: format!("exp-{id}"),
        description: format!("experiment {id}"),
    }
}

fn runner(repo: &Path) -> ExperimentRunner {
    ExperimentRunner::new(repo, CheckSuite::default())
}

fn worktrees(repo: &Path) -> String {
    git(repo, &["worktree", "list", "--porcelain"])
}

#[test]
fn improving_change_is_promoted_with_record() {
    let root = fixture_repo("promote");
    let records = root.join("records");
    let report = runner(&root)
        .run_experiment(spec("add-test"), |worktree| {
            fs::write(
                worktree.join("src/main.rs"),
                fs::read_to_string(worktree.join("src/main.rs")).unwrap()
                    + "\n#[test]\nfn experiment_added() {\n    assert_eq!(2 + 2, 4);\n}\n",
            )
            .map_err(io_error)
        })
        .unwrap();
    assert_eq!(report.baseline.passed, 2);
    assert_eq!(report.baseline.failed, 0);
    assert_eq!(report.candidate.passed, 3);
    assert!(
        matches!(report.decision, Decision::Promote { .. }),
        "{:?}",
        report.decision
    );
    // Promoted: branch kept with the change committed, worktree removed.
    assert!(git(&root, &["branch", "--list", "exp-add-test"]).contains("exp-add-test"));
    assert!(!worktrees(&root).contains("add-test"));
    assert!(git(&root, &["show", "exp-add-test:src/main.rs"]).contains("experiment_added"));
    // Repo itself untouched and clean.
    let status = git(&root, &["status", "--porcelain"]);
    assert!(status.trim().is_empty(), "dirty: {status}");
    let record = runner(&root).write_record(&records, &report).unwrap();
    let stored: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(record).unwrap()).unwrap();
    assert_eq!(stored["spec"]["id"], serde_json::json!("add-test"));
    assert!(stored.get("decision").unwrap().get("promote").is_some());
    assert!(stored["finished_at_ms"].as_u64().unwrap() > 0);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn regressing_change_is_rolled_back() {
    let root = fixture_repo("rollback");
    let report = runner(&root)
        .run_experiment(spec("break-value"), |worktree| {
            let path = worktree.join("src/main.rs");
            fs::write(
                path,
                fs::read_to_string(worktree.join("src/main.rs"))
                    .unwrap()
                    .replace("fn answer() -> i32 { 42 }", "fn answer() -> i32 { 43 }"),
            )
            .map_err(io_error)
        })
        .unwrap();
    assert_eq!(report.baseline.failed, 0);
    assert!(report.candidate.failed > 0);
    assert!(
        matches!(report.decision, Decision::Rollback { .. }),
        "{:?}",
        report.decision
    );
    // Rolled back: branch deleted, worktree gone, repo pristine.
    assert!(!git(&root, &["branch", "--list", "exp-break-value"]).contains("exp-break-value"));
    assert!(!worktrees(&root).contains("break-value"));
    assert!(
        fs::read_to_string(root.join("src/main.rs"))
            .unwrap()
            .contains("42")
    );
    assert!(git(&root, &["status", "--porcelain"]).trim().is_empty());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn dirty_repo_refuses_experiment() {
    let root = fixture_repo("dirty");
    fs::write(root.join("src/main.rs"), "dirty").unwrap();
    let before = worktrees(&root);
    let error = runner(&root)
        .run_experiment(spec("nope"), |_| Ok(()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("uncommitted"), "{error}");
    assert_eq!(worktrees(&root), before);
    assert!(!git(&root, &["branch", "--list", "exp-nope"]).contains("exp-nope"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn duplicate_branch_refuses_experiment() {
    let root = fixture_repo("dup");
    git(&root, &["branch", "exp-clash"]);
    let error = runner(&root)
        .run_experiment(
            ExperimentSpec {
                id: "clash".into(),
                branch: "exp-clash".into(),
                description: "clash".into(),
            },
            |_| Ok(()),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("worktree add failed"), "{error}");
    fs::remove_dir_all(&root).unwrap();
}

fn io_error(error: std::io::Error) -> std::io::Error {
    error
}
