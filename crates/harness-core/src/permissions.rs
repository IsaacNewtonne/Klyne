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
    /// Optional per-program argument grants. Absent entry means any argv is
    /// allowed for an allowlisted program (broad grant, audited in events).
    /// Present entry requires argv to start with one of the allowed prefixes.
    shell_arg_grants: std::collections::BTreeMap<String, Vec<Vec<String>>>,
    /// Environment variable names forwarded to supervised processes.
    /// Empty by default; process env is otherwise cleared.
    env_allowlist: BTreeSet<String>,
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
            shell_arg_grants: std::collections::BTreeMap::new(),
            env_allowlist: BTreeSet::new(),
        }
    }

    /// Explicitly grant a supervised executable. Enables the shell capability
    /// and allowlists `program` with unrestricted argv (audited per call).
    /// Prefer [`PermissionPolicy::allow_shell_with_arg_prefix`] for tighter grants.
    pub fn allow_shell_program(&mut self, program: impl Into<String>) {
        self.capabilities.insert(Capability::ShellExecute);
        self.shell_allowlist.insert(program.into());
    }

    /// Grant `program` only when argv starts with `prefix` (exact element match).
    /// Multiple prefixes may be registered; one match suffices.
    pub fn allow_shell_with_arg_prefix(&mut self, program: impl Into<String>, prefix: Vec<String>) {
        let program = program.into();
        self.capabilities.insert(Capability::ShellExecute);
        self.shell_allowlist.insert(program.clone());
        self.shell_arg_grants
            .entry(program)
            .or_default()
            .push(prefix);
    }

    /// Grant an environment variable name for forwarding to child processes.
    pub fn allow_env(&mut self, name: impl Into<String>) {
        self.env_allowlist.insert(name.into());
    }

    /// Resolve the child environment: cleared process env plus allowlisted
    /// names present in this process. Executable lookup requires `PATH`
    /// (plus `PATHEXT`/`COMSPEC`/`SYSTEMROOT` on Windows), so those
    /// non-secret OS-minimum entries are always forwarded when present.
    /// Anything else, including secrets, requires an explicit grant.
    pub fn shell_env(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for name in &self.env_allowlist {
            if let Ok(value) = std::env::var(name) {
                out.push((name.clone(), value));
            }
        }
        let mut forward_os_minimum = |name: &str| {
            if !self.env_allowlist.iter().any(|n| n == name)
                && let Ok(value) = std::env::var(name)
            {
                out.push((name.to_string(), value));
            }
        };
        forward_os_minimum("PATH");
        #[cfg(windows)]
        {
            forward_os_minimum("SYSTEMROOT");
            forward_os_minimum("WINDIR");
            forward_os_minimum("COMSPEC");
            forward_os_minimum("PATHEXT");
        }
        out
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Capabilities currently held, for intersection checks by delegating
    /// parents. Children only ever receive a subset of these.
    pub fn capabilities(&self) -> Vec<Capability> {
        self.capabilities.iter().cloned().collect()
    }

    /// Whether this policy authorizes `program` for supervised execution.
    pub fn allows_shell_program(&self, program: &str) -> bool {
        self.capabilities.contains(&Capability::ShellExecute)
            && self.shell_allowlist.contains(program)
    }

    /// Revoke one capability, e.g. write access for verifier children.
    pub fn revoke_capability(&mut self, capability: Capability) {
        self.capabilities.remove(&capability);
        if capability == Capability::ShellExecute {
            self.shell_allowlist.clear();
            self.shell_arg_grants.clear();
        }
    }

    /// Narrow this policy to a workspace subdirectory for a child agent.
    /// The child keeps the file capabilities but loses every shell and env
    /// grant; those need explicit re-granting through
    /// [`PermissionPolicy::grant_shell_from`]. The scope must pass this
    /// policy's own path checks, so a child can never be scoped outside
    /// the parent's workspace, and an existing non-directory blocks reuse.
    pub fn narrow_to_subdir(&self, subdir: &str) -> Result<PermissionPolicy, String> {
        match self.check(&Action::ReadFile {
            path: subdir.into(),
        }) {
            PermissionDecision::Allow => {}
            PermissionDecision::Deny(reason) | PermissionDecision::Ask(reason) => {
                return Err(format!("child scope denied: {reason}"));
            }
        }
        let root = self.workspace_root.join(subdir);
        match std::fs::symlink_metadata(&root) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err("child scope is an existing non-directory".into());
            }
            Ok(_) | Err(_) => {}
        }
        let mut capabilities = self.capabilities.clone();
        capabilities.remove(&Capability::ShellExecute);
        Ok(PermissionPolicy {
            workspace_root: root,
            capabilities,
            shell_allowlist: BTreeSet::new(),
            shell_arg_grants: Default::default(),
            env_allowlist: BTreeSet::new(),
        })
    }

    /// Re-grant one environment variable to a narrowed child policy. Fails
    /// unless the parent holds the same grant: environment visibility also
    /// shrinks down the delegation chain, never grows.
    pub fn grant_env_from(&mut self, parent: &PermissionPolicy, name: &str) -> Result<(), String> {
        if !parent.env_allowlist.contains(name) {
            return Err(format!(
                "parent does not grant env '{name}'; cannot delegate it"
            ));
        }
        self.env_allowlist.insert(name.into());
        Ok(())
    }

    /// Re-grant one supervised executable (with an argv prefix) to a
    /// narrowed child policy. Fails unless the parent holds a covering
    /// grant: privilege shrinks down the delegation chain, never grows.
    /// A broad (prefix-free) parent grant covers any prefix; otherwise the
    /// requested prefix must extend a parent-allowed one.
    pub fn grant_shell_from(
        &mut self,
        parent: &PermissionPolicy,
        program: &str,
        arg_prefix: Vec<String>,
    ) -> Result<(), String> {
        if !parent.allows_shell_program(program) {
            return Err(format!(
                "parent does not grant '{program}'; cannot delegate it"
            ));
        }
        let covered = match parent.shell_arg_grants.get(program) {
            None => true,
            Some(prefixes) => prefixes
                .iter()
                .any(|parent_prefix| arg_prefix.starts_with(&parent_prefix[..])),
        };
        if !covered {
            return Err(format!(
                "argv prefix for '{program}' exceeds the parent grant"
            ));
        }
        self.capabilities.insert(Capability::ShellExecute);
        self.shell_allowlist.insert(program.into());
        self.shell_arg_grants
            .entry(program.into())
            .or_default()
            .push(arg_prefix);
        Ok(())
    }

    pub fn check(&self, action: &Action) -> PermissionDecision {
        match action {
            Action::WriteFile { path, .. } => self.check_path(path, Capability::FilesystemWrite),
            Action::ReadFile { path }
            | Action::ReadFileRange { path, .. }
            | Action::HashFile { path }
            | Action::SearchFile { path, .. } => self.check_path(path, Capability::FilesystemRead),
            Action::PatchFile { path, .. } => {
                let read = self.check_path(path, Capability::FilesystemRead);
                if read != PermissionDecision::Allow {
                    return read;
                }
                self.check_path(path, Capability::FilesystemWrite)
            }
            Action::RunShell { program, args } => {
                if !self.capabilities.contains(&Capability::ShellExecute) {
                    return PermissionDecision::Deny("shell.execute capability is disabled".into());
                }
                if program.is_empty()
                    || program.contains('/')
                    || program.contains('\\')
                    || program.contains(':')
                    || !self.shell_allowlist.contains(program)
                {
                    return PermissionDecision::Deny(format!(
                        "executable '{program}' is outside the milestone allowlist"
                    ));
                }
                if let Some(prefixes) = self.shell_arg_grants.get(program) {
                    let allowed = prefixes.iter().any(|prefix| {
                        args.len() >= prefix.len() && args[..prefix.len()] == prefix[..]
                    });
                    if !allowed {
                        return PermissionDecision::Deny(format!(
                            "argv for '{program}' is outside the granted argument prefixes"
                        ));
                    }
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
