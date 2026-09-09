use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objective {
    pub id: String,
    pub text: String,
}

impl Objective {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        Self {
            id: format!("goal-{now}"),
            text,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    WriteFile {
        path: String,
        contents: String,
    },
    ReadFile {
        path: String,
    },
    ReadFileRange {
        path: String,
        offset: u64,
        length: u64,
    },
    HashFile {
        path: String,
    },
    PatchFile {
        path: String,
        offset: u64,
        expected: String,
        replacement: String,
        expected_sha256: String,
    },
    SearchFile {
        path: String,
        needle: String,
        max_matches: u64,
    },
    RunShell {
        program: String,
        args: Vec<String>,
    },
    Finish {
        summary: String,
    },
}

impl Action {
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::WriteFile { .. }
            | Self::ReadFile { .. }
            | Self::ReadFileRange { .. }
            | Self::HashFile { .. }
            | Self::PatchFile { .. }
            | Self::SearchFile { .. } => "workspace_fs",
            Self::RunShell { .. } => "workspace_shell",
            Self::Finish { .. } => "runtime",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepDecision {
    Act(Action),
    Verify(Action),
    Complete(String),
    Fail(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub ok: bool,
    pub summary: String,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verification {
    pub passed: bool,
    pub evidence: String,
}

/// Cumulative provider-side spend for one model instance. The runtime
/// accrues deltas into the checkpoint and enforces token/cost limits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: f64,
}

impl ModelUsage {
    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub kind: String,
    pub detail: String,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::WriteFile { path, .. } => write!(f, "write_file:{path}"),
            Action::ReadFile { path } => write!(f, "read_file:{path}"),
            Action::ReadFileRange {
                path,
                offset,
                length,
            } => write!(f, "read_range:{path}:{offset}:{length}"),
            Action::HashFile { path } => write!(f, "hash_file:{path}"),
            Action::PatchFile { path, offset, .. } => write!(f, "patch_file:{path}:{offset}"),
            Action::SearchFile { path, needle, .. } => write!(f, "search_file:{path}:{needle}"),
            Action::RunShell { program, args } => write!(f, "shell:{} {}", program, args.join(" ")),
            Action::Finish { summary } => write!(f, "finish:{summary}"),
        }
    }
}
