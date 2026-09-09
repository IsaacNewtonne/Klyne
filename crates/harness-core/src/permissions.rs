use crate::types::Action;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    FilesystemRead,
    FilesystemWrite,
    ShellExecute,
    NetworkFetch,
}

/// A strictly parsed subset of URLs: `https://host[:port][/path]`, plus
/// `http://` for loopback hosts only (local mock servers and LAN devices).
/// No userinfo, no whitespace, no other schemes. Anything else is rejected
/// before any grant is consulted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedUrl {
    pub scheme: String,
    pub host: String,
    pub path: String,
}

pub fn parse_fetch_url(raw: &str) -> Result<ParsedUrl, String> {
    if raw.len() > 2048 {
        return Err("URL exceeds 2048 characters".into());
    }
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| "URL must look like scheme://host/path".to_string())?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err(format!("scheme '{scheme}' is not allowed; use https"));
    }
    if rest.is_empty() {
        return Err("URL has no host".into());
    }
    let end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    if authority.contains('@') {
        return Err("URL userinfo is not allowed".into());
    }
    // Split an optional numeric port; bracketed IPv6 keeps its brackets so
    // the allowlist sees one canonical form.
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        let (inner, rest) = bracketed
            .split_once(']')
            .ok_or_else(|| "IPv6 host is missing ']'".to_string())?;
        if !rest.is_empty() {
            let port = rest
                .strip_prefix(':')
                .ok_or_else(|| "malformed IPv6 authority".to_string())?;
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return Err("port must be numeric".into());
            }
        }
        if inner.is_empty() || !inner.bytes().all(|b| b.is_ascii_hexdigit() || b == b':') {
            return Err("host contains illegal characters".into());
        }
        format!("[{inner}]").to_ascii_lowercase()
    } else {
        let host = match authority.rsplit_once(':') {
            Some((host, port))
                if !host.is_empty()
                    && !port.is_empty()
                    && port.bytes().all(|b| b.is_ascii_digit()) =>
            {
                host
            }
            _ => authority,
        };
        let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
        let malformed = host.is_empty()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            || host.starts_with('-')
            || host.starts_with('.')
            || host.contains("..");
        if malformed {
            return Err("host is malformed".into());
        }
        host
    };
    if scheme == "http" && !is_loopback(&host) {
        return Err("plain http is allowed for loopback hosts only".into());
    }
    Ok(ParsedUrl {
        scheme,
        host,
        path: if path.is_empty() {
            "/".into()
        } else {
            path.into()
        },
    })
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host == "::1"
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
    /// Exact hostnames fetchable over the network. Empty by default, and
    /// matching is exact (case-insensitive): subdomains are not implied.
    network_allowlist: BTreeSet<String>,
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
            network_allowlist: BTreeSet::new(),
        }
    }

    /// Grant fetching from one exact hostname (case-insensitive, all ports).
    /// Enables the network capability; loopback hosts additionally require
    /// this grant, so tests and local devices stay explicit too.
    pub fn allow_network_domain(&mut self, host: impl Into<String>) {
        self.capabilities.insert(Capability::NetworkFetch);
        self.network_allowlist
            .insert(host.into().to_ascii_lowercase());
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
            // Narrowed children fetch nothing until explicitly re-granted.
            network_allowlist: BTreeSet::new(),
        })
    }

    /// Re-grant one fetch hostname to a narrowed child policy. Fails unless
    /// the parent holds the exact same grant.
    pub fn grant_network_from(
        &mut self,
        parent: &PermissionPolicy,
        host: &str,
    ) -> Result<(), String> {
        let host = host.to_ascii_lowercase();
        if !parent.network_allowlist.contains(&host) {
            return Err(format!(
                "parent does not grant host '{host}'; cannot delegate it"
            ));
        }
        self.capabilities.insert(Capability::NetworkFetch);
        self.network_allowlist.insert(host);
        Ok(())
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
            Action::FetchUrl { url } => {
                if !self.capabilities.contains(&Capability::NetworkFetch) {
                    return PermissionDecision::Deny("network.fetch capability is disabled".into());
                }
                match parse_fetch_url(url) {
                    Err(reason) => PermissionDecision::Deny(reason),
                    Ok(parsed) if self.network_allowlist.contains(&parsed.host) => {
                        PermissionDecision::Allow
                    }
                    Ok(parsed) => PermissionDecision::Deny(format!(
                        "host '{}' is outside the network allowlist",
                        parsed.host
                    )),
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fetch(url: &str) -> Action {
        Action::FetchUrl { url: url.into() }
    }

    #[test]
    fn url_parser_accepts_canonical_forms() {
        let parsed = parse_fetch_url("https://api.github.com/search?q=x").unwrap();
        assert_eq!(parsed.scheme, "https");
        assert_eq!(parsed.host, "api.github.com");
        assert_eq!(parsed.path, "/search?q=x");
        assert_eq!(
            parse_fetch_url("HTTPS://Example.COM:8443/a").unwrap().host,
            "example.com"
        );
        assert_eq!(
            parse_fetch_url("https://example.com./a").unwrap().host,
            "example.com"
        );
        assert_eq!(parse_fetch_url("https://example.com").unwrap().path, "/");
        assert_eq!(
            parse_fetch_url("http://127.0.0.1:8080/a").unwrap().scheme,
            "http"
        );
        assert_eq!(
            parse_fetch_url("http://localhost/a").unwrap().host,
            "localhost"
        );
        assert_eq!(parse_fetch_url("http://[::1]/a").unwrap().host, "[::1]");
    }

    #[test]
    fn url_parser_rejects_hostile_shapes() {
        for raw in [
            "ftp://example.com/a",
            "file:///etc/passwd",
            "data:text/plain,hi",
            "http://example.com/a",
            "http://[::1]evil/a",
            "https://user:pass@example.com/a",
            "https://exa mple.com/a",
            "https://-bad.com/a",
            "https://.bad.com/a",
            "https://bad..com/a",
            "https://example.com:abc/a",
            "https://",
            "https://[::1/a",
            "not a url",
            "",
        ] {
            assert!(parse_fetch_url(raw).is_err(), "{raw}");
        }
        assert!(parse_fetch_url(&format!("https://example.com/{}", "x".repeat(3000))).is_err());
    }

    #[test]
    fn network_gating_defaults_deny_and_matches_exactly() {
        let root = std::env::temp_dir();
        let bare = PermissionPolicy::milestone_default(&root);
        assert!(matches!(
            bare.check(&fetch("https://api.github.com/x")),
            PermissionDecision::Deny(_)
        ));
        let mut policy = PermissionPolicy::milestone_default(&root);
        policy.allow_network_domain("API.GitHub.COM");
        assert_eq!(
            policy.check(&fetch("https://api.github.com/x")),
            PermissionDecision::Allow
        );
        // Subdomains are not implied; lookalikes do not match.
        for url in [
            "https://evil-api.github.com/x",
            "https://api.github.com.evil.com/x",
            "http://api.github.com/x",
        ] {
            assert!(
                matches!(policy.check(&fetch(url)), PermissionDecision::Deny(_)),
                "{url}"
            );
        }
        // Loopback still needs its own explicit grant.
        assert!(matches!(
            policy.check(&fetch("http://127.0.0.1:9/x")),
            PermissionDecision::Deny(_)
        ));
        policy.allow_network_domain("127.0.0.1");
        assert_eq!(
            policy.check(&fetch("http://127.0.0.1:9/x")),
            PermissionDecision::Allow
        );
        // Delegation parity: children re-grant only what parents hold.
        let mut child = policy.narrow_to_subdir("sub").unwrap();
        assert!(child.grant_network_from(&policy, "other.com").is_err());
        assert!(child.grant_network_from(&policy, "api.github.com").is_ok());
        assert_eq!(
            child.check(&fetch("https://api.github.com/x")),
            PermissionDecision::Allow
        );
    }
}
