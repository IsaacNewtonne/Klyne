use crate::err;
use harness_core::{Action, PermissionPolicy, ToolRegistry};
use harness_experiment::{CheckSuite, ExperimentRunner, ExperimentSpec};
use serde_json::{Value, json};
use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

fn snapshot(repo: &Path, records: &Path, id: &str, stop: &AtomicBool) -> io::Result<PathBuf> {
    use std::{fs, process::Command};
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(err("Invalid snapshot id"));
    }
    let listing = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .current_dir(repo)
        .output()?;
    if !listing.status.success() {
        return Err(err("Cannot enumerate repository snapshot"));
    }
    let policy = PermissionPolicy::milestone_default(repo);
    let files = String::from_utf8(listing.stdout).map_err(err)?;
    let target = records.join("snapshots").join(id);
    fs::create_dir_all(target.parent().unwrap())?;
    fs::create_dir(&target)?;
    for relative in files.split('\0').filter(|s| !s.is_empty()) {
        if stop.load(Ordering::SeqCst) {
            return Err(err("Snapshot cancelled"));
        }
        let action = Action::ReadFile {
            path: relative.into(),
        };
        if policy.check(&action) != harness_core::PermissionDecision::Allow {
            return Err(err(format!("Snapshot path refused: {relative}")));
        }
        let source = repo.join(relative);
        if !source.exists() {
            continue;
        } // tracked deletion is preserved
        let destination = target.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    }
    for args in [
        vec!["init", "-b", "main"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=Klyne snapshot",
            "-c",
            "user.email=klyne@local",
            "commit",
            "-m",
            "Isolated working-tree snapshot",
        ],
    ] {
        let result = Command::new("git")
            .args(args)
            .current_dir(&target)
            .output()?;
        if !result.status.success() {
            return Err(err("Snapshot Git initialization failed"));
        }
    }
    Ok(target)
}

pub const INSTRUCTIONS: &str = r#"With terminal access, self_improve {repo,id,changes:[{path,contents}],description} tests a candidate of 1-16 files in an isolated Git worktree of an explicitly selected Rust repository. Dirty repositories are first snapshotted into an isolated Git repository, preserving the original edits; the report identifies that repository. The legacy single-file path/contents form is also accepted, but cannot be combined with changes. Use this for harness or client capability improvements needed by the user's goal. Inspect source and existing tests first; include meaningful tests alongside implementation and preserve existing tests. The fixed cargo test --offline suite runs against baseline and candidate. Stop interrupts the test process and rolls back. Passing changes remain on branch klyne/<id> for review, never merged by this experiment. To activate a built runtime candidate, use runtime_stage under klyne-supervisor after testing and hashing it. Failed gates roll back. This tool is unavailable to reviewers. Report branch and test evidence; do not claim the running harness has changed merely because a candidate passed."#;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    path: String,
    contents: String,
}

fn changes(action: &Value) -> io::Result<Vec<Change>> {
    let changes: Vec<Change> = if let Some(changes) = action.get("changes") {
        if action.get("path").is_some() || action.get("contents").is_some() {
            return Err(err("Use changes or path/contents, not both"));
        }
        serde_json::from_value(changes.clone()).map_err(err)?
    } else {
        vec![
            serde_json::from_value(json!({"path":action["path"],"contents":action["contents"]}))
                .map_err(err)?,
        ]
    };
    if changes.is_empty() || changes.len() > 16 {
        return Err(err("Candidate requires 1-16 files"));
    }
    let mut names = std::collections::HashSet::new();
    let mut bytes = 0usize;
    for change in &changes {
        // The runtime additionally validates each path against the worktree.
        if change.path.is_empty()
            || change.path.len() > 1024
            || !names.insert(change.path.replace('\\', "/").to_lowercase())
        {
            return Err(err("Invalid or duplicate candidate path"));
        }
        bytes = bytes.saturating_add(change.contents.len());
    }
    if bytes > 1024 * 1024 {
        return Err(err("Candidate exceeds 1 MiB"));
    }
    Ok(changes)
}

pub fn execute(
    action: &Value,
    records: &Path,
    terminal: bool,
    review: bool,
    stop: &AtomicBool,
) -> io::Result<Value> {
    if !terminal || review {
        return Err(err(
            "Self-improvement requires terminal access and a worker role",
        ));
    }
    let field = |name: &str| -> io::Result<&str> {
        action[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(format!("Missing {name}")))
    };
    let repo = Path::new(field("repo")?);
    if !repo.is_absolute() {
        return Err(err("Repository must be an absolute path"));
    }
    let id = field("id")?;
    let changes = changes(action)?;
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo)
        .output()?;
    if !dirty.status.success() {
        return Err(err("Cannot inspect repository status"));
    }
    let isolated = if dirty.stdout.is_empty() {
        None
    } else {
        Some(snapshot(repo, records, id, stop)?)
    };
    let source = isolated.as_deref().unwrap_or(repo);
    let runner = ExperimentRunner::new(source, CheckSuite::default());
    let report = runner.run_experiment_cancellable(
        ExperimentSpec {
            id: id.into(),
            branch: format!("klyne/{id}"),
            description: field("description")?.into(),
        },
        stop,
        |worktree| {
            let policy = PermissionPolicy::milestone_default(worktree);
            let tools = ToolRegistry::milestone_default();
            for change in changes {
                let result = tools.execute(
                    &Action::WriteFile {
                        path: change.path.clone(),
                        contents: change.contents.clone(),
                    },
                    &policy,
                );
                if !result.ok {
                    return Err(err(result.summary));
                }
                let read = tools.execute(&Action::ReadFile { path: change.path }, &policy);
                if !read.ok || read.data != change.contents {
                    return Err(err("Candidate read-back failed"));
                }
            }
            Ok(())
        },
    )?;
    let record = runner.write_record(records, &report)?;
    Ok(
        json!({"ok":matches!(report.decision, harness_experiment::Decision::Promote { .. }),"report":report,"record":record,"repository":source,"original_repository":repo,"isolated_snapshot":isolated.is_some()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_and_tests_are_verified_together_without_changing_main_tree() {
        use std::{fs, process::Command};
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(repo.join("tests")).unwrap();
        fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname=\"improvement_fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        fs::write(
            repo.join("rust-toolchain.toml"),
            "[toolchain]\nchannel=\"1.98.1\"\nprofile=\"minimal\"\n",
        )
        .unwrap();
        fs::write(repo.join(".gitignore"), "/target/\nCargo.lock\n").unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn answer()->u32{42}\n").unwrap();
        fs::write(
            repo.join("tests/original.rs"),
            "#[test] fn original(){assert_eq!(improvement_fixture::answer(),42);}\n",
        )
        .unwrap();
        for args in [
            vec!["init", "-b", "main"],
            vec!["add", "-A"],
            vec![
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@local",
                "commit",
                "-m",
                "baseline",
            ],
        ] {
            let out = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(out.status.success(), "{:?}", out);
        }
        let result=execute(&json!({"repo":repo,"id":"with-tests","description":"Add doubled with regression coverage","changes":[{"path":"src/lib.rs","contents":"pub fn answer()->u32{42}\npub fn doubled()->u32{answer()*2}\n"},{"path":"tests/doubled.rs","contents":"#[test] fn doubled(){assert_eq!(improvement_fixture::doubled(),84);}\n"}]}),&root.path().join("records"),true,false,&AtomicBool::new(false)).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["report"]["baseline"]["passed"], 1);
        assert_eq!(result["report"]["candidate"]["passed"], 2);
        assert!(!repo.join("tests/doubled.rs").exists());
        assert!(
            !fs::read_to_string(repo.join("src/lib.rs"))
                .unwrap()
                .contains("doubled")
        );
        assert!(root.path().join("records/with-tests.json").is_file());
        let dirty_source = "pub fn answer()->u32{42}\npub fn tripled()->u32{answer()*3}\n";
        fs::write(repo.join("src/lib.rs"), dirty_source).unwrap();
        fs::write(
            repo.join("tests/in_progress.rs"),
            "#[test] fn in_progress(){assert_eq!(improvement_fixture::tripled(),126);}",
        )
        .unwrap();
        let dirty=execute(&json!({"repo":repo,"id":"dirty-project","description":"Preserve in-progress work while adding a function","changes":[{"path":"src/lib.rs","contents":format!("{dirty_source}pub fn doubled()->u32{{answer()*2}}\n")},{"path":"tests/doubled.rs","contents":"#[test] fn doubled(){assert_eq!(improvement_fixture::doubled(),84);}"}]}),&root.path().join("records"),true,false,&AtomicBool::new(false)).unwrap();
        assert_eq!(dirty["ok"], true, "{dirty}");
        assert_eq!(dirty["isolated_snapshot"], true);
        assert_eq!(dirty["report"]["baseline"]["passed"], 2);
        assert_eq!(dirty["report"]["candidate"]["passed"], 3);
        assert_eq!(
            fs::read_to_string(repo.join("src/lib.rs")).unwrap(),
            dirty_source
        );
        assert!(!repo.join("tests/doubled.rs").exists());
    }
    #[test]
    fn candidates_accept_implementation_and_tests_and_reject_ambiguous_paths() {
        assert_eq!(changes(&json!({"changes":[{"path":"src/lib.rs","contents":"implementation"},{"path":"tests/feature.rs","contents":"tests"}]})).unwrap().len(),2);
        assert!(changes(&json!({"changes":[]})).is_err());
        assert!(
            changes(
                &json!({"changes":[{"path":"file","contents":"a"},{"path":"FILE","contents":"b"}]})
            )
            .is_err()
        );
        assert!(changes(&json!({"path":"file","contents":"a","changes":[]})).is_err());
        assert_eq!(
            changes(&json!({"path":"file","contents":""})).unwrap()[0].contents,
            ""
        );
    }
    #[test]
    fn permission_denials_precede_repository_access() {
        let root = tempfile::tempdir().unwrap();
        for (terminal, review) in [(false, false), (false, true), (true, true)] {
            let error = execute(
                &json!({}),
                root.path(),
                terminal,
                review,
                &AtomicBool::new(false),
            )
            .unwrap_err();
            assert!(error.to_string().contains("requires terminal access"));
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
}
