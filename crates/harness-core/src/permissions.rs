use crate::types::Action;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    FilesystemRead,
    FilesystemWrite,
    ShellExecute,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny(String),
    Ask(String),
}

#[derive(Clone, Debug)]
pub struct PermissionPolicy {
    workspace_root: PathBuf,
    capabilities: BTreeSet<Capability>,
    shell_allowlist: BTreeSet<String>,
}

impl PermissionPolicy {
    pub fn milestone_default(workspace_root: impl Into<PathBuf>) -> Self {
        let capabilities = [Capability::FilesystemRead, Capability::FilesystemWrite]
            .into_iter()
            .collect();
        let shell_allowlist = ["cat", "echo", "ls", "pwd", "printf", "wc"]
            .into_iter()
            .map(String::from)
            .collect();
        Self {
            workspace_root: workspace_root.into(),
            capabilities,
            shell_allowlist,
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn check(&self, action: &Action) -> PermissionDecision {
        match action {
            Action::WriteFile { path, .. } => self.check_path(path, Capability::FilesystemWrite),
            Action::ReadFile { path }
            | Action::ReadFileRange { path, .. }
            | Action::HashFile { path } => self.check_path(path, Capability::FilesystemRead),
            Action::PatchFile { path, .. } => {
                let read = self.check_path(path, Capability::FilesystemRead);
                if read != PermissionDecision::Allow {
                    return read;
                }
                self.check_path(path, Capability::FilesystemWrite)
            }
            Action::RunShell { program, .. } => {
                if !self.capabilities.contains(&Capability::ShellExecute) {
                    return PermissionDecision::Deny("shell.execute capability is disabled".into());
                }
                if program.contains('/') || !self.shell_allowlist.contains(program) {
                    return PermissionDecision::Deny(format!(
                        "executable '{program}' is outside the milestone allowlist"
                    ));
                }
                PermissionDecision::Allow
            }
            Action::Finish { .. } => PermissionDecision::Allow,
        }
    }

    fn check_path(&self, raw: &str, cap: Capability) -> PermissionDecision {
        if !self.capabilities.contains(&cap) {
            return PermissionDecision::Deny(format!("capability {cap:?} is disabled"));
        }
        let path = Path::new(raw);
        if raw.is_empty()
            || raw.contains(':')
            || raw.contains('\\')
            || raw.split('/').any(|part| {
                let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
                part.eq_ignore_ascii_case(".harness")
                    || part.ends_with('.')
                    || part.ends_with(' ')
                    || matches!(
                        stem.as_str(),
                        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                    )
                    || (stem.len() == 4
                        && (stem.starts_with("COM") || stem.starts_with("LPT"))
                        && stem.as_bytes()[3].is_ascii_digit())
            })
        {
            return PermissionDecision::Deny(
                "empty, reserved, or ambiguous path is not allowed".into(),
            );
        }
        if path.is_absolute() {
            return PermissionDecision::Deny(
                "absolute paths are not allowed in milestone 1".into(),
            );
        }
        if path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return PermissionDecision::Deny("path traversal is not allowed".into());
        }
        let mut current = self.workspace_root.clone();
        for component in path.components() {
            current.push(component);
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    #[cfg(windows)]
                    let linked = {
                        use std::os::windows::fs::MetadataExt;
                        metadata.file_attributes() & 0x400 != 0
                    };
                    #[cfg(not(windows))]
                    let linked = metadata.file_type().is_symlink();
                    if linked {
                        return PermissionDecision::Deny(
                            "symlinks and reparse points are not allowed".into(),
                        );
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return PermissionDecision::Deny(format!("cannot inspect path: {e}")),
            }
        }
        PermissionDecision::Allow
    }

    pub fn resolve_workspace_path(&self, relative: &str) -> Result<PathBuf, String> {
        match self.check_path(relative, Capability::FilesystemRead) {
            PermissionDecision::Allow => Ok(self.workspace_root.join(relative)),
            PermissionDecision::Deny(reason) | PermissionDecision::Ask(reason) => Err(reason),
        }
    }
}
