use harness_core::{
    Action, AgentRuntime, FileEventStore, Model, Objective, Observation, PermissionDecision,
    PermissionPolicy, RunOutcome, StepDecision, ToolRegistry,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "harness-security-{}-{}",
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

struct Liar;
impl Model for Liar {
    fn name(&self) -> &str {
        "untrusted-model"
    }
    fn decide(&mut self, _: &Objective, _: &[(Action, Observation)]) -> StepDecision {
        StepDecision::Complete("trust me".into())
    }
}

#[test]
fn model_cannot_claim_completion_without_evidence() {
    let root = Workspace::new();
    let events = FileEventStore::open(root.0.join("events.log")).unwrap();
    let mut runtime = AgentRuntime::new(
        Liar,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&root.0),
        events,
    );
    let outcome = runtime
        .run(Objective::new(
            "create file result.txt with content correct",
        ))
        .unwrap();
    assert!(matches!(outcome, RunOutcome::Failed(_)));
    let log = fs::read_to_string(root.0.join("events.log")).unwrap();
    assert!(log.contains("VerificationFailed"));
    assert!(!log.contains("GoalCompleted"));
}

#[test]
fn denies_escapes_reserved_paths_and_ambient_shell() {
    let root = Workspace::new();
    let policy = PermissionPolicy::milestone_default(&root.0);
    for path in [
        "",
        "../outside",
        "/absolute",
        "C:/outside",
        "file:stream",
        "dir\\file",
        ".harness/events.log",
        ".HARNESS/db",
        "dir./file",
        "dir /file",
    ] {
        assert!(
            matches!(
                policy.check(&Action::WriteFile {
                    path: path.into(),
                    contents: "bad".into()
                }),
                PermissionDecision::Deny(_)
            ),
            "{path}"
        );
    }
    assert!(matches!(
        policy.check(&Action::RunShell {
            program: "cat".into(),
            args: vec!["/etc/passwd".into()]
        }),
        PermissionDecision::Deny(_)
    ));
}

#[cfg(unix)]
#[test]
fn denies_symlink_escape() {
    let root = Workspace::new();
    let outside = Workspace::new();
    std::os::unix::fs::symlink(&outside.0, root.0.join("link")).unwrap();
    let policy = PermissionPolicy::milestone_default(&root.0);
    let result = ToolRegistry::milestone_default().execute(
        &Action::WriteFile {
            path: "link/escaped".into(),
            contents: "bad".into(),
        },
        &policy,
    );
    assert!(!result.ok);
    assert!(!outside.0.join("escaped").exists());
}

#[cfg(windows)]
#[test]
fn denies_directory_junction_escape() {
    let root = Workspace::new();
    let outside = Workspace::new();
    let link = root.0.join("junction");
    let result = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(&outside.0)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let policy = PermissionPolicy::milestone_default(&root.0);
    let observation = ToolRegistry::milestone_default().execute(
        &Action::WriteFile {
            path: "junction/escaped".into(),
            contents: "bad".into(),
        },
        &policy,
    );
    assert!(!observation.ok);
    assert!(!outside.0.join("escaped").exists());
    fs::remove_dir(link).unwrap();
}
