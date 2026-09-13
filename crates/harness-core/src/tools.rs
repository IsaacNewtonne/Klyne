use crate::permissions::{PermissionDecision, PermissionPolicy};
use crate::types::{Action, Observation};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
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

/// Maximum directory entries returned by one `list_dir`; the remainder sets
/// `truncated` instead of growing the observation.
pub const MAX_LIST_ENTRIES: usize = 500;

/// Maximum bytes moved by one `copy_file`; larger transfers need an
/// approved job or chunked protocol (Phase 4 remainder).
pub const MAX_COPY_BYTES: u64 = 64 * 1024 * 1024;

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
                ActionDescriptor {
                    action: "ListDir".into(),
                    description: "List a workspace directory (names and kinds, <=500 entries, sorted).".into(),
                    effects: "read-only:filesystem.read".into(),
                },
                ActionDescriptor {
                    action: "StatPath".into(),
                    description: "Report kind (file/dir), size, and readonly flag for one workspace path.".into(),
                    effects: "read-only:filesystem.read".into(),
                },
                ActionDescriptor {
                    action: "MakeDir".into(),
                    description: "Create a workspace directory and missing parents.".into(),
                    effects: "mutating:filesystem.write".into(),
                },
                ActionDescriptor {
                    action: "CopyFile".into(),
                    description: "Copy one regular file (<=64 MiB); the destination must not exist.".into(),
                    effects: "mutating:filesystem.write".into(),
                },
                ActionDescriptor {
                    action: "MoveFile".into(),
                    description: "Atomically rename a file or directory; the destination must not exist.".into(),
                    effects: "mutating:filesystem.write".into(),
                },
                ActionDescriptor {
                    action: "DeletePath".into(),
                    description: "Delete one file or empty directory; non-empty directories are refused.".into(),
                    effects: "mutating:filesystem.write".into(),
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
                // Atomic publication: the previous content (or absence) stays
                // intact until the staged replacement renames over it, so a
                // crash or power loss can never leave a truncated file.
                // (Audit HIGH: destructive write behavior.)
                let parent = full
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| policy.workspace_root().to_path_buf());
                match (|| -> io::Result<usize> {
                    let mut staged = tempfile::NamedTempFile::new_in(&parent)?;
                    staged.write_all(contents.as_bytes())?;
                    staged.as_file().sync_all()?;
                    staged.persist(&full).map_err(|e| e.error)?;
                    Ok(contents.len())
                })() {
                    Ok(size) => Observation {
                        ok: true,
                        summary: format!("wrote {path}"),
                        data: size.to_string(),
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
            Action::ListDir { path } => {
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
                match (|| -> io::Result<String> {
                    let mut entries = Vec::new();
                    let mut truncated = false;
                    let mut count = 0;
                    for entry in fs::read_dir(&full)? {
                        let entry = entry?;
                        count += 1;
                        if entries.len() >= MAX_LIST_ENTRIES {
                            truncated = true;
                            continue;
                        }
                        let kind = entry.file_type()?;
                        entries.push(serde_json::json!({
                            "name": entry.file_name().to_string_lossy(),
                            "kind": if kind.is_dir() { "dir" } else if kind.is_file() { "file" } else { "other" },
                        }));
                    }
                    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                    Ok(serde_json::json!({
                        "path": path,
                        "entries": entries,
                        "scanned": count,
                        "truncated": truncated,
                    })
                    .to_string())
                })() {
                    Ok(data) => Observation {
                        ok: true,
                        summary: format!("listed {path}"),
                        data,
                    },
                    Err(e) => Observation {
                        ok: false,
                        summary: format!("list failed for {path}"),
                        data: e.to_string(),
                    },
                }
            }
            Action::StatPath { path } => {
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
                match (|| -> io::Result<String> {
                    let metadata = fs::symlink_metadata(&full)?;
                    if metadata.file_type().is_symlink() {
                        return Err(io::Error::other("symlinks are not allowed"));
                    }
                    Ok(serde_json::json!({
                        "path": path,
                        "kind": if metadata.is_dir() { "dir" } else if metadata.is_file() { "file" } else { "other" },
                        "size": metadata.len(),
                        "readonly": metadata.permissions().readonly(),
                    })
                    .to_string())
                })() {
                    Ok(data) => Observation {
                        ok: true,
                        summary: format!("stated {path}"),
                        data,
                    },
                    Err(e) => Observation {
                        ok: false,
                        summary: format!("stat failed for {path}"),
                        data: e.to_string(),
                    },
                }
            }
            Action::MakeDir { path } => {
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
                match fs::create_dir_all(&full) {
                    Ok(()) => Observation {
                        ok: true,
                        summary: format!("created directory {path}"),
                        data: full.is_dir().to_string(),
                    },
                    Err(e) => Observation {
                        ok: false,
                        summary: format!("mkdir failed for {path}"),
                        data: e.to_string(),
                    },
                }
            }
            Action::CopyFile { from, to } => {
                let source = match policy.resolve_workspace_path(from) {
                    Ok(v) => v,
                    Err(e) => {
                        return Observation {
                            ok: false,
                            summary: "invalid source path".into(),
                            data: e,
                        };
                    }
                };
                // Destination needs the write grant; resolve checks read, and
                // the top-level policy check already enforced write on `to`.
                let dest = match policy.resolve_workspace_path(to) {
                    Ok(v) => v,
                    Err(e) => {
                        return Observation {
                            ok: false,
                            summary: "invalid destination path".into(),
                            data: e,
                        };
                    }
                };
                match (|| -> io::Result<u64> {
                    let metadata = fs::symlink_metadata(&source)?;
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        return Err(io::Error::other("copy supports regular files only"));
                    }
                    if metadata.len() > MAX_COPY_BYTES {
                        return Err(io::Error::other("file exceeds 64 MiB copy limit"));
                    }
                    // Checked before staging: Unix rename would otherwise
                    // silently overwrite an existing destination.
                    if fs::symlink_metadata(&dest).is_ok() {
                        return Err(io::Error::other("destination exists; delete it first"));
                    }
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let mut staged = tempfile::NamedTempFile::new_in(
                        dest.parent()
                            .ok_or_else(|| io::Error::other("missing parent"))?,
                    )?;
                    let mut input = fs::File::open(&source)?;
                    let bytes = io::copy(&mut (&mut input).take(MAX_COPY_BYTES + 1), &mut staged)?;
                    if bytes > MAX_COPY_BYTES {
                        return Err(io::Error::other("file exceeds 64 MiB copy limit"));
                    }
                    staged.as_file().sync_all()?;
                    staged.persist(&dest).map_err(|e| e.error)?;
                    Ok(bytes)
                })() {
                    Ok(bytes) => Observation {
                        ok: true,
                        summary: format!("copied {from} to {to}"),
                        data: bytes.to_string(),
                    },
                    Err(e) => Observation {
                        ok: false,
                        summary: format!("copy failed for {from}"),
                        data: e.to_string(),
                    },
                }
            }
            Action::MoveFile { from, to } => {
                let source = match policy.resolve_workspace_path(from) {
                    Ok(v) => v,
                    Err(e) => {
                        return Observation {
                            ok: false,
                            summary: "invalid source path".into(),
                            data: e,
                        };
                    }
                };
                let dest = match policy.resolve_workspace_path(to) {
                    Ok(v) => v,
                    Err(e) => {
                        return Observation {
                            ok: false,
                            summary: "invalid destination path".into(),
                            data: e,
                        };
                    }
                };
                match (|| -> io::Result<()> {
                    let metadata = fs::symlink_metadata(&source)?;
                    if metadata.file_type().is_symlink() {
                        return Err(io::Error::other("symlinks are not allowed"));
                    }
                    // Best-effort guard: Windows rename fails on existing
                    // destinations; Unix rename would replace one, so check
                    // first (a concurrent creator racing this call is outside
                    // the workspace threat model, as elsewhere here).
                    if dest.exists() {
                        return Err(io::Error::other("destination exists; delete it first"));
                    }
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    // Same-volume rename: atomic, no copy window.
                    fs::rename(&source, &dest)
                })() {
                    Ok(()) => Observation {
                        ok: true,
                        summary: format!("moved {from} to {to}"),
                        data: to.clone(),
                    },
                    Err(e) => Observation {
                        ok: false,
                        summary: format!("move failed for {from}"),
                        data: e.to_string(),
                    },
                }
            }
            Action::DeletePath { path } => {
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
                match (|| -> io::Result<String> {
                    let metadata = fs::symlink_metadata(&full)?;
                    if metadata.file_type().is_symlink() {
                        return Err(io::Error::other("symlinks are not allowed"));
                    }
                    if metadata.is_dir() {
                        // No recursive deletion without a scoped approval
                        // broker (deferred Phase 2); empty dirs go directly.
                        fs::remove_dir(&full).map_err(|_| {
                            io::Error::other("directory is not empty; refusing recursive delete")
                        })?;
                        return Ok("dir".into());
                    }
                    if !metadata.is_file() {
                        return Err(io::Error::other("only regular files and empty directories"));
                    }
                    fs::remove_file(&full)?;
                    Ok("file".into())
                })() {
                    Ok(kind) => Observation {
                        ok: true,
                        summary: format!("deleted {path}"),
                        data: kind,
                    },
                    Err(e) => Observation {
                        ok: false,
                        summary: format!("delete failed for {path}"),
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
    overflow: Arc<AtomicBool>,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut buf = Vec::with_capacity(max.min(8192));
        let mut tmp = [0u8; 8192];
        loop {
            match pipe.read(&mut tmp) {
                Ok(0) => return (buf, false),
                Ok(n) => {
                    if buf.len() + n > max {
                        overflow.store(true, Ordering::SeqCst);
                        // Drain until the supervisor terminates the owned process.
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
        self.execute_cancellable(action, policy, &AtomicBool::new(false))
    }
}
impl WorkspaceShellTool {
    pub fn execute_cancellable(
        &self,
        action: &Action,
        policy: &PermissionPolicy,
        stop: &AtomicBool,
    ) -> Observation {
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
        let job = match crate::process_job::ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(e) => {
                terminate_child(&mut child);
                return Observation {
                    ok: false,
                    summary: "Could not own process descendants; outcome may be uncertain".into(),
                    data: e.to_string(),
                };
            }
        };
        let max = self.limits.max_output_bytes;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let overflow = Arc::new(AtomicBool::new(false));
        let stdout_handle = precise_bounded_read(stdout, max, overflow.clone());
        let stderr_handle = precise_bounded_read(stderr, max, overflow.clone());
        let start = Instant::now();
        loop {
            if stop.load(Ordering::SeqCst)
                || overflow.load(Ordering::SeqCst)
                || start.elapsed() >= self.limits.timeout
            {
                job.terminate();
                terminate_child(&mut child);
                let _ = child.wait();
                // A descendant can retain a pipe after its parent exits. Never join
                // an unfinished reader: the deadline includes output collection.
                let out = if stdout_handle.is_finished() {
                    stdout_handle.join().unwrap_or_default().0
                } else {
                    Vec::new()
                };
                let err = if stderr_handle.is_finished() {
                    stderr_handle.join().unwrap_or_default().0
                } else {
                    Vec::new()
                };
                let reason = if stop.load(Ordering::SeqCst) {
                    "cancelled"
                } else if overflow.load(Ordering::SeqCst) {
                    "output exceeded per-stream byte limit"
                } else {
                    "timed out"
                };
                return Observation {
                    ok: false,
                    summary: format!(
                        "{program} {reason}; process stopped; external outcome may be uncertain"
                    ),
                    data: format!("{}{}", bounded_text(&out, max), bounded_text(&err, max)),
                };
            }
            match child.try_wait() {
                Ok(Some(status)) if stdout_handle.is_finished() && stderr_handle.is_finished() => {
                    let (out, out_exceeded) = stdout_handle.join().unwrap_or_default();
                    let (err, err_exceeded) = stderr_handle.join().unwrap_or_default();
                    let mut data = String::from_utf8_lossy(&out).into_owned();
                    if !err.is_empty() {
                        data.push_str("\n[stderr]\n");
                        data.push_str(&String::from_utf8_lossy(&err));
                    }
                    return Observation {
                        ok: status.success() && !out_exceeded && !err_exceeded,
                        summary: format!("{program} exited with {status}"),
                        data,
                    };
                }
                Ok(_) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => {
                    job.terminate();
                    terminate_child(&mut child);
                    return Observation {
                        ok: false,
                        summary: format!(
                            "failed to supervise {program}; external outcome may be uncertain"
                        ),
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
