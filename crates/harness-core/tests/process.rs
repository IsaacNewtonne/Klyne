use harness_core::{Action, PermissionPolicy, ProcessLimits, ToolRegistry};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(std::path::PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "harness-process-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn policy(&self) -> PermissionPolicy {
        PermissionPolicy::milestone_default(&self.0)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn tool_discovery_lists_filesystem_and_supervised_shell() {
    let registry = ToolRegistry::milestone_default();
    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 2);
    let names: Vec<_> = descriptors.iter().map(|d| d.tool.as_str()).collect();
    assert!(names.contains(&"workspace_fs"));
    assert!(names.contains(&"workspace_shell"));
    let shell = descriptors
        .iter()
        .find(|d| d.tool == "workspace_shell")
        .unwrap();
    assert_eq!(shell.actions.len(), 1);
    assert_eq!(shell.actions[0].action, "RunShell");
    assert!(shell.description.contains("argv"));
    let fs = descriptors
        .iter()
        .find(|d| d.tool == "workspace_fs")
        .unwrap();
    assert!(fs.actions.iter().any(|a| a.action == "WriteFile"));
    assert!(fs.actions.iter().any(|a| a.action == "PatchFile"));
}

#[test]
fn shell_defaults_to_denied_and_requires_explicit_grant() {
    let root = Workspace::new();
    let policy = root.policy();
    let tools = ToolRegistry::milestone_default();
    let action = Action::RunShell {
        program: "cargo".into(),
        args: vec!["--version".into()],
    };
    let denied = tools.execute(&action, &policy);
    assert!(!denied.ok);
    assert_eq!(denied.summary, "permission denied");

    let mut granted = root.policy();
    granted.allow_shell_program("cargo");
    let ok = tools.execute(&action, &granted);
    assert!(ok.ok, "{}: {}", ok.summary, ok.data);
    assert!(ok.summary.contains("exited with"));
    assert!(ok.data.contains("cargo"));
}

#[test]
fn denied_commands_include_unknown_and_path_forms() {
    let root = Workspace::new();
    let mut policy = root.policy();
    policy.allow_shell_program("cargo");
    let tools = ToolRegistry::milestone_default();
    for program in ["unknown-xyz", "a/b", "a\\b", "a:b", ""] {
        let obs = tools.execute(
            &Action::RunShell {
                program: program.into(),
                args: vec![],
            },
            &policy,
        );
        assert!(!obs.ok, "{program}");
        assert_eq!(obs.summary, "permission denied");
    }
}

#[test]
fn argument_prefix_grants_are_enforced() {
    let root = Workspace::new();
    let mut policy = root.policy();
    policy.allow_shell_with_arg_prefix("cargo", vec!["--version".into()]);
    let tools = ToolRegistry::milestone_default();
    let allowed = tools.execute(
        &Action::RunShell {
            program: "cargo".into(),
            args: vec!["--version".into()],
        },
        &policy,
    );
    assert!(allowed.ok, "{}: {}", allowed.summary, allowed.data);
    let denied = tools.execute(
        &Action::RunShell {
            program: "cargo".into(),
            args: vec!["--help".into()],
        },
        &policy,
    );
    assert!(!denied.ok);
    assert_eq!(denied.summary, "permission denied");
}

#[test]
fn nonzero_exit_is_reported_with_bounded_evidence() {
    let root = Workspace::new();
    let mut policy = root.policy();
    policy.allow_shell_program("cargo");
    let tools = ToolRegistry::milestone_default();
    let obs = tools.execute(
        &Action::RunShell {
            program: "cargo".into(),
            args: vec!["--nonexistent-flag-xyz".into()],
        },
        &policy,
    );
    assert!(!obs.ok);
    assert!(obs.summary.contains("exited with"));
    assert!(obs.data.len() < 128 * 1024);
}

#[test]
fn excessive_output_is_bounded_and_fails() {
    let root = Workspace::new();
    let tools = ToolRegistry::milestone_with_shell_limits(ProcessLimits::default());
    #[cfg(windows)]
    {
        let mut policy = root.policy();
        policy.allow_shell_program("cmd.exe");
        // ~200 KiB of stdout, well above the 64 KiB per-stream cap.
        // Pure cmd built-in loop: no external executable lookup required.
        let obs = tools.execute(
            &Action::RunShell {
                program: "cmd.exe".into(),
                args: vec![
                    "/d".into(),
                    "/c".into(),
                    "for /L %i in (1,1,20000) do @echo 0123456789".into(),
                ],
            },
            &policy,
        );
        assert!(!obs.ok, "{}: {}", obs.summary, obs.data);
        assert!(
            obs.summary.contains("exceeded") || obs.data.contains("exceeded"),
            "{}: {}",
            obs.summary,
            obs.data
        );
        assert!(obs.data.len() < 256 * 1024);
    }
    #[cfg(not(windows))]
    {
        let mut policy = root.policy();
        policy.allow_shell_program("sh");
        let obs = tools.execute(
            &Action::RunShell {
                program: "sh".into(),
                args: vec!["-c".into(), "yes x | head -c 200000".into()],
            },
            &policy,
        );
        assert!(!obs.ok, "{}: {}", obs.summary, obs.data);
        assert!(
            obs.summary.contains("exceeded") || obs.data.contains("exceeded"),
            "{}: {}",
            obs.summary,
            obs.data
        );
        assert!(obs.data.len() < 256 * 1024);
    }
}

#[test]
fn timeout_terminates_the_process_and_stays_usable() {
    let root = Workspace::new();
    let limits = ProcessLimits {
        timeout: Duration::from_secs(2),
        max_output_bytes: 64 * 1024,
    };
    let tools = ToolRegistry::milestone_with_shell_limits(limits);
    #[cfg(windows)]
    {
        let mut policy = root.policy();
        policy.allow_shell_program("cmd.exe");
        policy.allow_shell_program("cargo");
        let start = std::time::Instant::now();
        let obs = tools.execute(
            &Action::RunShell {
                program: "cmd.exe".into(),
                args: vec![
                    "/d".into(),
                    "/c".into(),
                    "ping -n 30 127.0.0.1 > NUL".into(),
                ],
            },
            &policy,
        );
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "must not wait full ping"
        );
        assert!(!obs.ok, "{}: {}", obs.summary, obs.data);
        assert!(obs.summary.contains("timed out"), "{}", obs.summary);
        // Supervisor must still work after a kill (handle reaped, no lockup).
        let again = tools.execute(
            &Action::RunShell {
                program: "cargo".into(),
                args: vec!["--version".into()],
            },
            &policy,
        );
        assert!(again.ok, "{}: {}", again.summary, again.data);
    }
    #[cfg(not(windows))]
    {
        let mut policy = root.policy();
        policy.allow_shell_program("sleep");
        policy.allow_shell_program("sh");
        let start = std::time::Instant::now();
        let obs = tools.execute(
            &Action::RunShell {
                program: "sleep".into(),
                args: vec!["30".into()],
            },
            &policy,
        );
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "must not wait full sleep"
        );
        assert!(!obs.ok, "{}: {}", obs.summary, obs.data);
        assert!(obs.summary.contains("timed out"), "{}", obs.summary);
        let again = tools.execute(
            &Action::RunShell {
                program: "sh".into(),
                args: vec!["-c".into(), "echo recovered".into()],
            },
            &policy,
        );
        assert!(again.ok, "{}: {}", again.summary, again.data);
        assert!(again.data.contains("recovered"));
    }
}

#[test]
fn windows_tree_kill_reaps_parent_and_child() {
    #[cfg(not(windows))]
    return;
    let root = Workspace::new();
    let limits = ProcessLimits {
        timeout: Duration::from_secs(3),
        max_output_bytes: 64 * 1024,
    };
    let tools = ToolRegistry::milestone_with_shell_limits(limits);
    let mut policy = root.policy();
    policy.allow_shell_program("cmd.exe");
    // cmd spawns ping as a child; taskkill /T must terminate the tree so the
    // supervised call returns on timeout instead of hanging for 30 pings.
    let start = std::time::Instant::now();
    let obs = tools.execute(
        &Action::RunShell {
            program: "cmd.exe".into(),
            args: vec![
                "/d".into(),
                "/c".into(),
                "ping -n 30 127.0.0.1 > NUL".into(),
            ],
        },
        &policy,
    );
    assert!(!obs.ok);
    assert!(obs.summary.contains("timed out"), "{}", obs.summary);
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "tree kill must return promptly, took {:?}",
        start.elapsed()
    );
}

/// Unix-only: a background grandchild must die with the tree. Unverifiable
/// on Windows hosts; run this target on Unix before relying on group kill.
#[cfg(unix)]
#[test]
fn unix_tree_kill_reaps_background_grandchildren() {
    use std::time::{Duration, Instant};
    let root = Workspace::new();
    let limits = ProcessLimits {
        timeout: Duration::from_secs(2),
        max_output_bytes: 64 * 1024,
    };
    let tools = ToolRegistry::milestone_with_shell_limits(limits);
    let mut policy = root.policy();
    policy.allow_shell_program("sh");
    // The shell backgrounds a marker-writing loop, then waits; both share
    // the child's process group, so the timeout must take both down. A
    // surviving grandchild would keep rewriting the marker.
    let start = Instant::now();
    let marker = root.0.join("grandchild-alive");
    let obs = tools.execute(
        &Action::RunShell {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "while true; do date +%s%N > {}; sleep 0.2; done & wait",
                    marker.to_string_lossy()
                ),
            ],
        },
        &policy,
    );
    assert!(!obs.ok, "{}: {}", obs.summary, obs.data);
    assert!(obs.summary.contains("timed out"), "{}", obs.summary);
    assert!(start.elapsed() < Duration::from_secs(20));
    let before = fs::metadata(&marker).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        fs::metadata(&marker).unwrap().modified().unwrap(),
        before,
        "grandchild survived the group kill and kept writing"
    );
    fs::remove_file(&marker).unwrap();
}

#[test]
fn child_environment_is_cleared_except_explicit_grants() {
    let root = Workspace::new();
    unsafe { std::env::set_var("HARNESS_PROC_TEST_SECRET_XYZ", "leak-me") };
    let tools = ToolRegistry::milestone_default();
    #[cfg(windows)]
    {
        let mut policy = root.policy();
        policy.allow_shell_program("cmd.exe");
        let obs = tools.execute(
            &Action::RunShell {
                program: "cmd.exe".into(),
                args: vec![
                    "/d".into(),
                    "/c".into(),
                    "echo %HARNESS_PROC_TEST_SECRET_XYZ%".into(),
                ],
            },
            &policy,
        );
        assert!(obs.ok, "{}: {}", obs.summary, obs.data);
        assert!(
            !obs.data.contains("leak-me"),
            "env must be cleared: {}",
            obs.data
        );
    }
    #[cfg(not(windows))]
    {
        let mut policy = root.policy();
        policy.allow_shell_program("sh");
        let obs = tools.execute(
            &Action::RunShell {
                program: "sh".into(),
                args: vec!["-c".into(), "echo $HARNESS_PROC_TEST_SECRET_XYZ".into()],
            },
            &policy,
        );
        assert!(obs.ok, "{}: {}", obs.summary, obs.data);
        assert!(
            !obs.data.trim().contains("leak-me"),
            "env must be cleared: {}",
            obs.data
        );
        policy.allow_env("HARNESS_PROC_TEST_SECRET_XYZ");
        let forwarded = tools.execute(
            &Action::RunShell {
                program: "sh".into(),
                args: vec!["-c".into(), "echo $HARNESS_PROC_TEST_SECRET_XYZ".into()],
            },
            &policy,
        );
        assert!(forwarded.ok);
        assert!(
            forwarded.data.contains("leak-me"),
            "explicit grant must forward: {}",
            forwarded.data
        );
    }
    unsafe { std::env::remove_var("HARNESS_PROC_TEST_SECRET_XYZ") };
}
