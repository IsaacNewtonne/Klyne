//! Phase 4 filesystem tests: directory operations, atomic writes, and
//! permission gates for the six new workspace actions.
use harness_core::{Action, PermissionPolicy, ToolRegistry};
use std::fs;

struct Workspace {
    root: std::path::PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "harness-fileops-{}-{}-{}",
            name,
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn policy(&self) -> PermissionPolicy {
        PermissionPolicy::milestone_default(&self.root)
    }

    fn tools() -> ToolRegistry {
        ToolRegistry::milestone_default()
    }

    fn run(&self, action: &Action) -> harness_core::Observation {
        Self::tools().execute(action, &self.policy())
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn writes_publish_atomically_and_overwrite_fully() {
    let ws = Workspace::new("atomic");
    let write = |contents: &str| {
        ws.run(&Action::WriteFile {
            path: "note.txt".into(),
            contents: contents.into(),
        })
    };
    assert!(write("first").ok);
    assert_eq!(
        fs::read_to_string(ws.root.join("note.txt")).unwrap(),
        "first"
    );
    // Overwrite replaces the whole file; no torn mix of old and new bytes.
    assert!(write("second, longer contents").ok);
    assert_eq!(
        fs::read_to_string(ws.root.join("note.txt")).unwrap(),
        "second, longer contents"
    );
    // No stray staging files leak into the workspace.
    let names: Vec<_> = fs::read_dir(&ws.root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["note.txt".to_string()]);
}

#[test]
fn list_and_stat_describe_the_workspace() {
    let ws = Workspace::new("list");
    assert!(ws.run(&Action::MakeDir { path: "sub".into() }).ok);
    assert!(
        ws.run(&Action::WriteFile {
            path: "sub/a.txt".into(),
            contents: "a".into(),
        })
        .ok
    );
    assert!(
        ws.run(&Action::WriteFile {
            path: "b.txt".into(),
            contents: "bb".into(),
        })
        .ok
    );
    let listed = ws.run(&Action::ListDir { path: ".".into() });
    assert!(listed.ok, "{}: {}", listed.summary, listed.data);
    let body: serde_json::Value = serde_json::from_str(&listed.data).unwrap();
    let names: Vec<_> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["b.txt", "sub"]);
    assert_eq!(body["truncated"], false);

    let stat = ws.run(&Action::StatPath {
        path: "b.txt".into(),
    });
    let body: serde_json::Value = serde_json::from_str(&stat.data).unwrap();
    assert_eq!(body["kind"], "file");
    assert_eq!(body["size"], 2);
    let stat = ws.run(&Action::StatPath { path: "sub".into() });
    let body: serde_json::Value = serde_json::from_str(&stat.data).unwrap();
    assert_eq!(body["kind"], "dir");
    assert!(
        !ws.run(&Action::StatPath {
            path: "missing.txt".into()
        })
        .ok
    );
    assert!(
        !ws.run(&Action::ListDir {
            path: "b.txt".into()
        })
        .ok
    );
}

#[test]
fn copy_move_delete_follow_no_overwrite_no_recurse_rules() {
    let ws = Workspace::new("copy");
    assert!(
        ws.run(&Action::WriteFile {
            path: "src.txt".into(),
            contents: "payload".into(),
        })
        .ok
    );
    // Copy stages atomically and refuses existing destinations.
    let copied = ws.run(&Action::CopyFile {
        from: "src.txt".into(),
        to: "dst.txt".into(),
    });
    assert!(copied.ok, "{}: {}", copied.summary, copied.data);
    assert_eq!(
        fs::read_to_string(ws.root.join("dst.txt")).unwrap(),
        "payload"
    );
    let clash = ws.run(&Action::CopyFile {
        from: "src.txt".into(),
        to: "dst.txt".into(),
    });
    assert!(!clash.ok);
    assert!(clash.data.contains("exists"), "{}", clash.data);
    assert_eq!(
        fs::read_to_string(ws.root.join("dst.txt")).unwrap(),
        "payload"
    );
    // Directories are not copied.
    assert!(
        !ws.run(&Action::CopyFile {
            from: ".".into(),
            to: "dir-copy".into(),
        })
        .ok
    );
    // Move is a rename: source vanishes, content intact.
    assert!(
        ws.run(&Action::MoveFile {
            from: "dst.txt".into(),
            to: "sub/moved.txt".into(),
        })
        .ok
    );
    assert!(!ws.root.join("dst.txt").exists());
    assert_eq!(
        fs::read_to_string(ws.root.join("sub/moved.txt")).unwrap(),
        "payload"
    );
    assert!(
        !ws.run(&Action::MoveFile {
            from: "src.txt".into(),
            to: "sub/moved.txt".into(),
        })
        .ok
    );
    // Delete removes files and empty dirs, never recurses.
    assert!(
        ws.run(&Action::DeletePath {
            path: "src.txt".into()
        })
        .ok
    );
    assert!(!ws.root.join("src.txt").exists());
    let non_empty = ws.run(&Action::DeletePath { path: "sub".into() });
    assert!(!non_empty.ok, "non-empty dir must be refused");
    assert!(non_empty.data.contains("not empty"), "{}", non_empty.data);
    assert!(
        ws.run(&Action::DeletePath {
            path: "sub/moved.txt".into()
        })
        .ok
    );
    assert!(ws.run(&Action::DeletePath { path: "sub".into() }).ok);
    assert!(
        !ws.run(&Action::DeletePath {
            path: "gone.txt".into()
        })
        .ok
    );
}

#[test]
fn directory_ops_reject_traversal_and_honor_capabilities() {
    let ws = Workspace::new("gates");
    for action in [
        Action::ListDir {
            path: "../evil".into(),
        },
        Action::StatPath { path: "..".into() },
        Action::MakeDir {
            path: "../evil".into(),
        },
        Action::CopyFile {
            from: "../a".into(),
            to: "b".into(),
        },
        Action::CopyFile {
            from: "a".into(),
            to: "../b".into(),
        },
        Action::MoveFile {
            from: "../a".into(),
            to: "b".into(),
        },
        Action::DeletePath {
            path: "../a".into(),
        },
    ] {
        let obs = ws.run(&action);
        assert!(!obs.ok, "{action}: {}", obs.data);
    }
    // Read-only reviewers keep list/stat and lose every mutating op.
    let mut review = ws.policy();
    review.revoke_capability(harness_core::Capability::FilesystemWrite);
    let tools = ToolRegistry::milestone_default();
    assert!(
        tools
            .execute(&Action::ListDir { path: ".".into() }, &review)
            .ok
    );
    assert!(
        tools
            .execute(&Action::StatPath { path: ".".into() }, &review)
            .ok
    );
    for action in [
        Action::MakeDir { path: "x".into() },
        Action::CopyFile {
            from: "a".into(),
            to: "b".into(),
        },
        Action::MoveFile {
            from: "a".into(),
            to: "b".into(),
        },
        Action::DeletePath { path: "a".into() },
    ] {
        assert!(!tools.execute(&action, &review).ok, "{action}");
    }
    // Read capability revoked: even list/stat close.
    let mut blind = ws.policy();
    blind.revoke_capability(harness_core::Capability::FilesystemRead);
    assert!(
        !ToolRegistry::milestone_default()
            .execute(&Action::ListDir { path: ".".into() }, &blind)
            .ok
    );
}
