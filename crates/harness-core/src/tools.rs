use crate::permissions::{PermissionDecision, PermissionPolicy};
use crate::types::{Action, Observation};
use std::fs;
use std::process::Command;

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation;
}

#[derive(Default)]
pub struct WorkspaceFsTool;

impl Tool for WorkspaceFsTool {
    fn name(&self) -> &'static str { "workspace_fs" }

    fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation {
        if let PermissionDecision::Deny(reason) | PermissionDecision::Ask(reason) = policy.check(action) {
            return Observation { ok: false, summary: "permission denied".into(), data: reason };
        }
        match action {
            Action::WriteFile { path, contents } => {
                let full = match policy.resolve_workspace_path(path) {
                    Ok(v) => v,
                    Err(e) => return Observation { ok: false, summary: "invalid path".into(), data: e },
                };
                if let Some(parent) = full.parent() {
                    if let Err(e) = fs::create_dir_all(parent) {
                        return Observation { ok: false, summary: "mkdir failed".into(), data: e.to_string() };
                    }
                }
                match fs::write(&full, contents) {
                    Ok(()) => Observation { ok: true, summary: format!("wrote {path}"), data: contents.len().to_string() },
                    Err(e) => Observation { ok: false, summary: format!("write failed for {path}"), data: e.to_string() },
                }
            }
            Action::ReadFile { path } => {
                let full = match policy.resolve_workspace_path(path) {
                    Ok(v) => v,
                    Err(e) => return Observation { ok: false, summary: "invalid path".into(), data: e },
                };
                match fs::read_to_string(&full) {
                    Ok(data) => Observation { ok: true, summary: format!("read {path}"), data },
                    Err(e) => Observation { ok: false, summary: format!("read failed for {path}"), data: e.to_string() },
                }
            }
            _ => Observation { ok: false, summary: "unsupported filesystem action".into(), data: String::new() },
        }
    }
}

#[derive(Default)]
pub struct WorkspaceShellTool;

impl Tool for WorkspaceShellTool {
    fn name(&self) -> &'static str { "workspace_shell" }

    fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation {
        if let PermissionDecision::Deny(reason) | PermissionDecision::Ask(reason) = policy.check(action) {
            return Observation { ok: false, summary: "permission denied".into(), data: reason };
        }
        let Action::RunShell { program, args } = action else {
            return Observation { ok: false, summary: "unsupported shell action".into(), data: String::new() };
        };
        match Command::new(program).args(args).current_dir(policy.workspace_root()).output() {
            Ok(out) => {
                let mut data = String::from_utf8_lossy(&out.stdout).into_owned();
                if !out.stderr.is_empty() {
                    data.push_str("\n[stderr]\n");
                    data.push_str(&String::from_utf8_lossy(&out.stderr));
                }
                Observation { ok: out.status.success(), summary: format!("{program} exited with {}", out.status), data }
            }
            Err(e) => Observation { ok: false, summary: format!("failed to launch {program}"), data: e.to_string() },
        }
    }
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn milestone_default() -> Self {
        Self { tools: vec![Box::new(WorkspaceFsTool), Box::new(WorkspaceShellTool)] }
    }

    pub fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation {
        let wanted = action.tool_name();
        match self.tools.iter().find(|t| t.name() == wanted) {
            Some(tool) => tool.execute(action, policy),
            None if matches!(action, Action::Finish { .. }) => Observation { ok: true, summary: "finished".into(), data: String::new() },
            None => Observation { ok: false, summary: format!("tool not found: {wanted}"), data: String::new() },
        }
    }
}
