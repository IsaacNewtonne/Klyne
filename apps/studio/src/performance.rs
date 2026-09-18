//! Content-free model timing, execution trace IDs, and stage budgets.
//!
//! Stage-1 latency baseline: every goal gets a request ID; every model
//! attempt, tool execution and persisted observation gets a stable ID.
//! Monotonic stage timers attribute wall time to preparation, model
//! requests, tool execution, verification and database checkpoints.
//! Milestones record time to first feedback, first action and verified
//! completion. No prompt text is recorded here.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;

pub const PROMPT_BYTES: usize = 90_000;

/// Wall-clock milliseconds for interval attribution. Differences only;
///
/// never a trusted timestamp.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderTiming {
    pub total_duration_ns: Option<u64>,
    pub load_duration_ns: Option<u64>,
    pub prompt_eval_duration_ns: Option<u64>,
    pub eval_duration_ns: Option<u64>,
    pub cached_prompt_tokens: Option<u64>,
}
impl ProviderTiming {
    pub fn from_ollama(value: &Value) -> Self {
        Self {
            total_duration_ns: value["total_duration"].as_u64(),
            load_duration_ns: value["load_duration"].as_u64(),
            prompt_eval_duration_ns: value["prompt_eval_duration"].as_u64(),
            eval_duration_ns: value["eval_duration"].as_u64(),
            cached_prompt_tokens: value["prompt_eval_cached_count"].as_u64(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModelSample {
    pub role: String,
    pub attempt: usize,
    pub elapsed_ms: u64,
    pub prompt_bytes: usize,
    pub transport_ok: bool,
    pub provider: ProviderTiming,
    /// Stable per-goal model-call ID (`model-<n>`), correlating retries of
    /// one role with the attempt number.
    #[serde(default)]
    pub call_id: String,
    /// Context assembly, policy refresh, scrubbing and budget fitting that
    /// preceded this transport, in milliseconds.
    #[serde(default)]
    pub preparation_ms: u64,
    /// Worker step index from the assignment, when this call served one.
    #[serde(default)]
    pub task_step: Option<u64>,
    /// Metered tokens reported for this call (0 when the provider omits them).
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// Accumulated wall time per execution stage, in milliseconds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StageMs {
    pub preparation_ms: u64,
    pub model_request_ms: u64,
    pub tool_execution_ms: u64,
    pub verification_ms: u64,
    pub db_checkpoint_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolSample {
    pub id: String,
    pub tool: String,
    pub elapsed_ms: u64,
    pub ok: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ObservationSample {
    pub id: String,
    pub action: String,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Milestones {
    pub first_feedback_ms: Option<u64>,
    pub first_action_ms: Option<u64>,
    pub verified_completion_ms: Option<u64>,
}

/// Per-goal latency trace. Content-free: IDs, durations and counts only.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Trace {
    pub request_id: String,
    pub goal_started_wall_ms: u64,
    pub next_id: u64,
    pub stage: StageMs,
    pub tool_calls: Vec<ToolSample>,
    pub observations: Vec<ObservationSample>,
    pub milestones: Milestones,
    pub retries: u64,
    pub failures: u64,
}

impl Trace {
    pub fn alloc(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}-{}", self.next_id)
    }
    /// Milliseconds since goal start for a wall-clock reading.
    pub fn at_ms(&self, wall_ms: u64) -> u64 {
        wall_ms.saturating_sub(self.goal_started_wall_ms)
    }
    /// Latency breakdown for benchmarks and the experiment ledger. Token
    /// totals come from the caller; durations and counts come from the trace.
    /// Retries, failures and tool calls are included; failures are reported,
    /// never excluded from the record.
    pub fn report(
        &self,
        model_calls: usize,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> Value {
        serde_json::json!({
            "request_id": self.request_id,
            "model_calls": model_calls,
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "tool_calls": self.tool_calls.len(),
            "observations": self.observations.len(),
            "retries": self.retries,
            "failures": self.failures,
            "stage_ms": {
                "preparation": self.stage.preparation_ms,
                "model_request": self.stage.model_request_ms,
                "tool_execution": self.stage.tool_execution_ms,
                "verification": self.stage.verification_ms,
                "db_checkpoint": self.stage.db_checkpoint_ms,
            },
            "milestones_ms": {
                "first_feedback": self.milestones.first_feedback_ms,
                "first_action": self.milestones.first_action_ms,
                "verified_completion": self.milestones.verified_completion_ms,
            },
        })
    }
}

/// Host/meta evidence actions that record orchestration rather than a tool
/// dispatch. Everything else carrying an `action` field counts as action
/// evidence for the first-action milestone.
pub fn is_action_evidence(action: &str) -> bool {
    !matches!(
        action,
        "unsupported_completion"
            | "route_switch"
            | "completion_check"
            | "acceptance_check"
            | "operation_check"
            | "desktop_recover"
            | "desktop_reconcile"
            | "document_save_reconcile"
            | "restart_reconcile"
            | "experience_index"
            | "evidence_read"
            | "result_review"
    )
}

/// Preserve protected fields and newest evidence. Never silently truncate a goal.
/// This is a byte guard, not a provider token counter.
pub fn fit_context(system: &str, context: &mut Value) -> io::Result<usize> {
    loop {
        let size = system.len().saturating_add(1).saturating_add(context.to_string().len());
        if size <= PROMPT_BYTES { return Ok(size); }
        let removable = ["observations", "messages"].into_iter().find(|key| {
            context[*key].as_array().is_some_and(|items| items.len() > if *key == "messages" { 2 } else { 1 })
        });
        match removable {
            Some(key) => { context[key].as_array_mut().unwrap().remove(0); }
            None => return Err(io::Error::other("Protected model context exceeds the request budget; no instructions or current evidence were truncated")),
        }
    }
}

pub fn greeting(text: &str) -> bool {
    matches!(text.trim().trim_end_matches(['!', '.', '?']).to_ascii_lowercase().as_str(),
        "hi" | "hello" | "hey" | "good morning" | "good afternoon" | "good evening")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn final_budget_preserves_goal_and_latest_evidence() {
        let mut context = json!({"original_request":"keep me", "observations":["x".repeat(PROMPT_BYTES), "fresh"], "messages":["old","user","new"]});
        assert!(fit_context(&"s".repeat(1000), &mut context).unwrap() <= PROMPT_BYTES);
        assert_eq!(context["original_request"], "keep me");
        assert_eq!(context["observations"], json!(["fresh"]));
        context["retry_feedback"] = json!("x".repeat(PROMPT_BYTES));
        assert!(fit_context("system", &mut context).is_err());
        assert_eq!(context["observations"], json!(["fresh"]));
    }
    #[test]
    fn greetings_do_not_match_action_requests() {
        assert!(greeting("Hello!"));
        for text in ["hello, send a message", "go on", "hi\nopen Chrome", "say hi to Bob"] {
            assert!(!greeting(text));
        }
    }
    #[test]
    fn missing_provider_metrics_remain_unknown() {
        assert!(ProviderTiming::from_ollama(&json!({})).load_duration_ns.is_none());
        assert_eq!(ProviderTiming::from_ollama(&json!({"load_duration":42})).load_duration_ns, Some(42));
    }
    #[test]
    fn trace_ids_are_stable_and_monotonic() {
        let mut trace = Trace::default();
        trace.goal_started_wall_ms = 1000;
        assert_eq!(trace.alloc("model"), "model-1");
        assert_eq!(trace.alloc("tool"), "tool-2");
        assert_eq!(trace.at_ms(1500), 500);
        assert_eq!(trace.at_ms(500), 0);
        let report = trace.report(2, 10, 5);
        assert_eq!(report["request_id"], "");
        assert_eq!(report["model_calls"], 2);
        assert_eq!(report["stage_ms"]["model_request"], 0);
        assert!(report["milestones_ms"]["first_feedback"].is_null());
    }
    #[test]
    fn host_checks_are_not_action_evidence() {
        for meta in ["completion_check", "acceptance_check", "route_switch", "evidence_read", "document_save_reconcile"] {
            assert!(!is_action_evidence(meta), "{meta}");
        }
        for tool in ["browser_read", "desktop_observe", "read_file:.", "write_file:greeting.txt", "mcp_call", "runtime_attest"] {
            assert!(is_action_evidence(tool), "{tool}");
        }
    }
}
