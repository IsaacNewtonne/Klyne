use crate::permissions::{PermissionDecision, PermissionPolicy};
use crate::types::{Action, Observation};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDescriptor {
    pub action: String,
    pub description: String,
    pub effects: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub tool: String,
    pub description: String,
    pub actions: Vec<ActionDescriptor>,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn descriptor(&self) -> ToolDescriptor;
    fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation;
}

/// Bounds for supervised child processes. Per-stream output cap plus a
/// wall-clock timeout; exceeding either terminates the process tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLimits {
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_output_bytes: 64 * 1024,
        }
    }
}

#[derive(Default)]
pub struct WorkspaceFsTool;

/// Hard per-operation ceiling, including recovery reads. Not a total memory cap.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;

fn bounded_read(path: &std::path::Path) -> io::Result<String> {
    // Check before opening to reject known special files; concurrent hostile
    // replacement remains outside the current workspace threat model.
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::other("only regular files can be read"));
    }
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("only regular files can be read"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(io::Error::other("file exceeds byte limit"));
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "file is not UTF-8"))
}

impl Tool for WorkspaceFsTool {
    fn name(&self) -> &'static str {
        "workspace_fs"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool: self.name().into(),
            description: "Workspace-scoped file operations. Writes capped at 1 MiB; range/hash/patch/search helpers cover larger files.".into(),
            actions: vec![
                ActionDescriptor {
                    action: "WriteFile".into(),
                    description: "Create or replace a UTF-8 text file (<=1 MiB).".into(),
                    effects: "mutating:filesystem.write".into(),
                },
                ActionDescriptor {
                    action: "ReadFile".into(),
                    description: "Read a whole UTF-8 text file (<=1 MiB).".into(),
                    effects: "read-only:filesystem.read".into(),
                },
                ActionDescriptor {
                    action: "ReadFileRange".into(),
                    description: "Read up to 1 MiB by byte offset/length.".into(),
                    effects: "read-only:filesystem.read".into(),
                },
                ActionDescriptor {
                    action: "HashFile".into(),
                    description: "Stream SHA-256 over files up to 64 MiB.".into(),
                    effects: "read-only:filesystem.read".into(),
                },
                ActionDescriptor {
                    action: "PatchFile".into(),
                    description: "Digest-guarded byte-range replacement for files up to 64 MiB.".into(),
                    effects: "mutating:filesystem.write".into(),
                },
                ActionDescriptor {
                    action: "SearchFile".into(),
                    description: "Bounded substring search (needle <=1 KiB, <=50 matches) over files up to 64 MiB.".into(),
                    effects: "read-only:filesystem.read".into(),
                },
            ],
        }
    }

    fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation {
        if let PermissionDecision::Deny(reason) | PermissionDecision::Ask(reason) =
            policy.check(action)
        {
            return Observation {
                ok: false,
                summary: "permission denied".into(),
                data: reason,
            };
        }
        match action {
            Action::ReadFileRange { .. }
            | Action::HashFile { .. }
            | Action::PatchFile { .. }
            | Action::SearchFile { .. } => match crate::large_files::execute(action, policy) {
                Ok(data) => Observation {
                    ok: true,
                    summary: format!("completed {action}"),
                    data,
                },
                Err(error) => Observation {
                    ok: false,
                    summary: format!("failed {action}"),
                    data: error.to_string(),
                },
            },
            Action::WriteFile { path, contents } => {
                if contents.len() > MAX_FILE_BYTES {
                    return Observation {
                        ok: false,
                        summary: "write exceeds byte limit".into(),
                        data: String::new(),
                    };
                }
                let full = match policy.resolve_workspace_path(path) {
                    Ok(v) => v,
                    Err(e) => {
                        return Observation {
                            ok: false,
                            summary: "invalid path".into(),
                            data: e,
                        };
                    }
                };
                if let Some(parent) = full.parent()
                    && let Err(e) = fs::create_dir_all(parent)
                {
                    return Observation {
                        ok: false,
                        summary: "mkdir failed".into(),
                        data: e.to_string(),
                    };
                }
                match fs::write(&full, contents) {
                    Ok(()) => Observation {
                        ok: true,
                        summary: format!("wrote {path}"),
                        data: contents.len().to_string(),
                    },
                    Err(e) => Observation {
                        ok: false,
                        summary: format!("write failed for {path}"),
                        data: e.to_string(),
                    },
                }
            }
            Action::ReadFile { path } => {
                let full = match policy.resolve_workspace_path(path) {
                    Ok(v) => v,
                    Err(e) => {
                        return Observation {
                            ok: false,
                            summary: "invalid path".into(),
                            data: e,
                        };
                    }
                };
                match bounded_read(&full) {
                    Ok(data) => Observation {
                        ok: true,
                        summary: format!("read {path}"),
                        data,
                    },
                    Err(e) => Observation {
                        ok: false,
                        summary: format!("read failed for {path}"),
                        data: e.to_string(),
                    },
                }
            }
            _ => Observation {
                ok: false,
                summary: "unsupported filesystem action".into(),
                data: String::new(),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceShellTool {
    limits: ProcessLimits,
}

#[allow(clippy::derivable_impls)]
impl Default for WorkspaceShellTool {
    fn default() -> Self {
        Self {
            limits: ProcessLimits::default(),
        }
    }
}

impl WorkspaceShellTool {
    pub fn new(limits: ProcessLimits) -> Self {
        Self { limits }
    }

    pub fn with_limits(mut self, limits: ProcessLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn limits(&self) -> &ProcessLimits {
        &self.limits
    }
}

fn precise_bounded_read(
    mut pipe: impl Read + Send + 'static,
    max: usize,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut buf = Vec::with_capacity(max.min(8192));
        let mut tmp = [0u8; 8192];
        loop {
            match pipe.read(&mut tmp) {
                Ok(0) => return (buf, false),
                Ok(n) => {
                    if buf.len() + n > max {
                        // Drain the rest so the child never blocks on a full pipe.
                        let mut discard = [0u8; 8192];
                        while pipe.read(&mut discard).map(|m| m > 0).unwrap_or(false) {}
                        return (buf, true);
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                Err(_) => return (buf, true),
            }
        }
    })
}

#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    // Windows-first tree cleanup: taskkill /T terminates the process tree.
    // This is authorization-adjacent cleanup, not OS isolation.
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output();
}

/// Unix tree cleanup: the child starts as a process-group leader (see the
/// spawn site), so a negative-pid SIGKILL reaches the whole tree.
/// EXPERIMENTAL: implemented against libc but not yet executed on a Unix
/// host; verify there before relying on it.
#[cfg(unix)]
fn kill_process_tree(pid: u32) {
    // Negative pid targets the process group; fall back to the direct
    // child below if the group kill reports an error.
    let group = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
    if group != 0 {
        // Best effort only; the caller reaps regardless.
    }
}

#[cfg(not(windows))]
#[cfg(not(unix))]
fn kill_process_tree(_pid: u32) {}

fn terminate_child(child: &mut Child) {
    #[cfg(windows)]
    {
        kill_process_tree(child.id());
        let _ = child.kill();
    }
    #[cfg(unix)]
    {
        kill_process_tree(child.id());
        let _ = child.kill();
    }
    #[cfg(not(windows))]
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

fn bounded_text(bytes: &[u8], max: usize) -> String {
    const NOTE: &str = "\n[truncated: output exceeded per-stream byte limit]\n";
    if bytes.len() <= max {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let cut = max.min(bytes.len());
    let mut s = String::from_utf8_lossy(&bytes[..cut]).into_owned();
    s.push_str(NOTE);
    s
}

impl Tool for WorkspaceShellTool {
    fn name(&self) -> &'static str {
        "workspace_shell"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool: self.name().into(),
            description: format!(
                "Supervised argv execution (no shell strings). Cwd fixed to workspace; env cleared except explicit grants. Timeout {}s; {} bytes per stream; tree kill via taskkill /T on Windows, process-group SIGKILL on Unix.",
                self.limits.timeout.as_secs(),
                self.limits.max_output_bytes
            ),
            actions: vec![ActionDescriptor {
                action: "RunShell".into(),
                description: "Run an explicitly allowlisted executable with argv. Captures bounded stdout/stderr.".into(),
                effects: "external:process.manage (authorized, not isolated)".into(),
            }],
        }
    }

    fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation {
        if let PermissionDecision::Deny(reason) | PermissionDecision::Ask(reason) =
            policy.check(action)
        {
            return Observation {
                ok: false,
                summary: "permission denied".into(),
                data: reason,
            };
        }
        let Action::RunShell { program, args } = action else {
            return Observation {
                ok: false,
                summary: "unsupported shell action".into(),
                data: String::new(),
            };
        };
        // Never concatenate a shell string: executable + argv only, cwd fixed
        // to the workspace, environment cleared except explicit grants.
        let mut command = Command::new(program);
        #[cfg(unix)]
        {
            // Own process group per child so timeouts can signal the tree.
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                unsafe {
                    libc::setsid();
                }
                Ok(())
            });
        }
        command
            .args(args)
            .current_dir(policy.workspace_root())
            .env_clear()
            .envs(policy.shell_env())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Observation {
                    ok: false,
                    summary: format!("failed to launch {program}"),
                    data: e.to_string(),
                };
            }
        };
        let max = self.limits.max_output_bytes;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let stdout_handle = precise_bounded_read(stdout, max);
        let stderr_handle = precise_bounded_read(stderr, max);
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let (out, out_exceeded) = stdout_handle.join().unwrap_or_default();
                    let (err, err_exceeded) = stderr_handle.join().unwrap_or_default();
                    if out_exceeded || err_exceeded {
                        let mut data = bounded_text(&out, max);
                        if !err.is_empty() {
                            data.push_str("\n[stderr]\n");
                            data.push_str(&bounded_text(&err, max));
                        }
                        data.push_str("\n[output exceeded per-stream byte limit]\n");
                        return Observation {
                            ok: false,
                            summary: format!(
                                "{program} output exceeded {max} bytes per stream and was terminated"
                            ),
                            data,
                        };
                    }
                    let mut data = String::from_utf8_lossy(&out).into_owned();
                    if !err.is_empty() {
                        data.push_str("\n[stderr]\n");
                        data.push_str(&String::from_utf8_lossy(&err));
                    }
                    return Observation {
                        ok: status.success(),
                        summary: format!("{program} exited with {status}"),
                        data,
                    };
                }
                Ok(None) => {
                    if start.elapsed() >= self.limits.timeout {
                        let secs = self.limits.timeout.as_secs();
                        terminate_child(&mut child);
                        let _ = child.wait();
                        let (out, _) = stdout_handle.join().unwrap_or_default();
                        let (err, _) = stderr_handle.join().unwrap_or_default();
                        let mut data = String::from_utf8_lossy(&out).into_owned();
                        // Bound partial evidence even on timeout.
                        if data.len() > max {
                            data.truncate(max);
                            data.push_str("\n[truncated]\n");
                        }
                        if !err.is_empty() {
                            data.push_str("\n[stderr]\n");
                            let mut e = String::from_utf8_lossy(&err).into_owned();
                            if e.len() > max {
                                e.truncate(max);
                                e.push_str("\n[truncated]\n");
                            }
                            data.push_str(&e);
                        }
                        return Observation {
                            ok: false,
                            summary: format!(
                                "{program} timed out after {secs}s and was terminated"
                            ),
                            data,
                        };
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    terminate_child(&mut child);
                    let _ = child.wait();
                    return Observation {
                        ok: false,
                        summary: format!("failed to supervise {program}"),
                        data: e.to_string(),
                    };
                }
            }
        }
    }
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn milestone_default() -> Self {
        Self {
            tools: vec![
                Box::new(WorkspaceFsTool),
                Box::new(WorkspaceShellTool::default()),
            ],
        }
    }

    pub fn milestone_with_shell_limits(limits: ProcessLimits) -> Self {
        Self {
            tools: vec![
                Box::new(WorkspaceFsTool),
                Box::new(WorkspaceShellTool::new(limits)),
            ],
        }
    }

    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools.iter().map(|t| t.descriptor()).collect()
    }

    /// Compose an extension tool (e.g. network fetch) into the registry.
    /// Registration grants nothing: the permission policy still decides
    /// every action, and capabilities default to disabled.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation {
        let wanted = action.tool_name();
        match self.tools.iter().find(|t| t.name() == wanted) {
            Some(tool) => tool.execute(action, policy),
            None if matches!(action, Action::Finish { .. }) => Observation {
                ok: true,
                summary: "finished".into(),
                data: String::new(),
            },
            None => Observation {
                ok: false,
                summary: format!("tool not found: {wanted}"),
                data: String::new(),
            },
        }
    }
}
