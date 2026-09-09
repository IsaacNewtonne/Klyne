//! Scoped multi-agent delegation.
//!
//! A parent run delegates fenced sub-objectives to child runs. Every
//! delegation is a contract with four enforced properties:
//!
//! 1. **Narrowed scope**: the child workspace is a subdirectory of the
//!    parent's, validated by the parent's own path checks.
//! 2. **Inherited limits**: the child receives a subset of the parent's
//!    capabilities. Shell and env grants never transfer implicitly; each
//!    re-grant is verified against a covering parent grant.
//! 3. **Delegated budget**: the child's tool-call limit must fit inside the
//!    parent's remaining budget, checked before spawning.
//! 4. **Collected evidence**: the child runs in its own database with the
//!    same permissions/budgets/verification boundaries; the parent collects
//!    artifacts through its own reads and records the outcome.

use crate::agent::{AgentRuntime, RunOutcome};
use crate::event_store::EventStore;
use crate::model::Model;
use crate::permissions::Capability;
use crate::permissions::PermissionPolicy;
use crate::tools::ToolRegistry;
use crate::types::Objective;
use crate::verification::SuccessCriterion;
use std::io;
use std::path::{Path, PathBuf};

/// The delegation contract: everything a child may do, stated up front.
#[derive(Clone, Debug)]
pub struct ChildGrant {
    /// Workspace-relative subdirectory forming the child's world.
    pub subdir: String,
    /// Tool-call ceiling, charged against the parent's remaining budget.
    pub tool_call_limit: u64,
    pub max_steps: usize,
    /// Verifier mode: read-only child that checks but cannot change.
    pub read_only: bool,
    /// Explicit shell re-grants as (program, argv prefix) pairs.
    pub shell_grants: Vec<(String, Vec<String>)>,
    /// Explicit environment re-grants (e.g. build variables for compilers).
    pub env_grants: Vec<String>,
}

impl ChildGrant {
    pub fn scoped(subdir: impl Into<String>, tool_call_limit: u64) -> Self {
        Self {
            subdir: subdir.into(),
            tool_call_limit,
            max_steps: 8,
            read_only: false,
            shell_grants: Vec::new(),
            env_grants: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChildReport {
    pub id: String,
    pub outcome: RunOutcome,
    pub tool_calls_used: u64,
    /// Child workspace root, for artifact collection by the parent.
    pub workspace: PathBuf,
}

/// Spawn a fenced child run and drive it to a terminal outcome.
/// Fails closed before spawning when the grant exceeds the parent's
/// remaining budget or escapes its authority.
#[allow(clippy::too_many_arguments)]
pub fn spawn_child<M: Model>(
    parent_workspace: &Path,
    parent_policy: &PermissionPolicy,
    parent_budget_used: u64,
    parent_budget_limit: u64,
    child_id: &str,
    objective: Objective,
    criterion: Option<SuccessCriterion>,
    model: M,
    grant: ChildGrant,
) -> io::Result<ChildReport> {
    let remaining = parent_budget_limit.saturating_sub(parent_budget_used);
    if grant.tool_call_limit > remaining {
        return Err(io::Error::other(format!(
            "child budget {} exceeds parent remaining {remaining}",
            grant.tool_call_limit
        )));
    }
    let mut policy = parent_policy
        .narrow_to_subdir(&grant.subdir)
        .map_err(io::Error::other)?;
    if grant.read_only {
        policy.revoke_capability(Capability::FilesystemWrite);
    }
    for (program, prefix) in &grant.shell_grants {
        policy
            .grant_shell_from(parent_policy, program, prefix.clone())
            .map_err(io::Error::other)?;
    }
    for name in &grant.env_grants {
        policy
            .grant_env_from(parent_policy, name)
            .map_err(io::Error::other)?;
    }
    let workspace = policy.workspace_root().to_path_buf();
    std::fs::create_dir_all(&workspace)?;
    let database = parent_workspace
        .join(".harness")
        .join("children")
        .join(format!("{child_id}.sqlite3"));
    let mut runtime = AgentRuntime::new(
        model,
        ToolRegistry::milestone_default(),
        policy,
        crate::SqliteEventStore::open(&database)?,
    )
    .with_max_steps(grant.max_steps)
    .with_max_tool_calls(grant.tool_call_limit);
    let outcome = match criterion {
        Some(criterion) => runtime.run_with_criterion(objective, Some(criterion))?,
        None => runtime.run(objective)?,
    };
    drop(runtime);
    let mut store = crate::SqliteEventStore::open(&database)?;
    let used = store
        .load()?
        .and_then(|state| state.tool_budget.map(|budget| budget.used))
        .unwrap_or(0);
    Ok(ChildReport {
        id: child_id.into(),
        outcome,
        tool_calls_used: used,
        workspace,
    })
}

/// Collect a child's top-level artifacts through fresh reads bounded by
/// count and bytes. The parent never trusts child-reported bytes; it
/// re-reads them here, which is also what verification should check.
pub fn collect_artifacts(
    child_workspace: &Path,
    max_files: usize,
    max_bytes_per_file: u64,
) -> io::Result<Vec<(String, String)>> {
    let mut artifacts = Vec::new();
    let entries = std::fs::read_dir(child_workspace)?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".harness" || !entry.file_type()?.is_file() {
            continue;
        }
        if artifacts.len() >= max_files {
            return Err(io::Error::other(format!(
                "artifact count exceeds {max_files}"
            )));
        }
        let size = entry.metadata()?.len();
        if size > max_bytes_per_file {
            return Err(io::Error::other(format!(
                "artifact '{name}' exceeds {max_bytes_per_file} bytes"
            )));
        }
        let bytes = std::fs::read(entry.path())?;
        artifacts.push((name, String::from_utf8_lossy(&bytes).into_owned()));
    }
    artifacts.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(artifacts)
}
