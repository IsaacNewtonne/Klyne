use crate::permissions::{PermissionDecision, PermissionPolicy};
use crate::types::{Action, Observation};
use std::fs;
use std::io::{self, Read};
use std::process::Command;

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation;
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

#[derive(Default)]
pub struct WorkspaceShellTool;

impl Tool for WorkspaceShellTool {
    fn name(&self) -> &'static str {
        "workspace_shell"
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
        match Command::new(program)
            .args(args)
            .current_dir(policy.workspace_root())
            .output()
        {
            Ok(out) => {
                let mut data = String::from_utf8_lossy(&out.stdout).into_owned();
                if !out.stderr.is_empty() {
                    data.push_str("\n[stderr]\n");
                    data.push_str(&String::from_utf8_lossy(&out.stderr));
                }
                Observation {
                    ok: out.status.success(),
                    summary: format!("{program} exited with {}", out.status),
                    data,
                }
            }
            Err(e) => Observation {
                ok: false,
                summary: format!("failed to launch {program}"),
                data: e.to_string(),
            },
        }
    }
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn milestone_default() -> Self {
        Self {
            tools: vec![Box::new(WorkspaceFsTool), Box::new(WorkspaceShellTool)],
        }
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
