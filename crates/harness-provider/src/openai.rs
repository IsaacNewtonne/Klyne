//! OpenAI-compatible chat-completions adapter with strict decisions.
//!
//! Works with any endpoint serving the chat-completions dialect, including
//! local servers where compatible. The adapter proposes only the twelve
//! workspace file actions; shell execution is never proposed. Every response
//! is validated before it can become a [`harness_core::StepDecision`]:
//! unknown tools, missing fields, and wrong types become `Fail`, never an
//! executed action.

use crate::{ProviderLimits, Usage};
use harness_core::{Action, Model, ModelUsage, Objective, Observation, StepDecision};

/// Optional per-1k-token rates used to price metered spend. Absent rates
/// mean spend is counted in tokens only and cost stays zero.
#[derive(Clone, Debug, PartialEq)]
pub struct Pricing {
    pub usd_per_1k_prompt_tokens: f64,
    pub usd_per_1k_completion_tokens: f64,
}

/// Endpoint configuration. Holds no secrets: `api_key_env` names an
/// environment variable whose value is read per HTTP call and sent only as
/// an `Authorization` header. It never enters prompts, decisions, or errors.
#[derive(Clone, Debug)]
pub struct ProviderConfig {
    /// Full chat-completions URL, e.g.
    /// `http://127.0.0.1:11434/v1/chat/completions`.
    pub endpoint: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub limits: ProviderLimits,
    pub pricing: Option<Pricing>,
}

impl ProviderConfig {
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key_env: None,
            limits: ProviderLimits::default(),
            pricing: None,
        }
    }

    pub fn with_api_key_env(mut self, env_var: impl Into<String>) -> Self {
        self.api_key_env = Some(env_var.into());
        self
    }

    pub fn with_pricing(mut self, pricing: Pricing) -> Self {
        self.pricing = Some(pricing);
        self
    }

    pub fn with_limits(mut self, limits: ProviderLimits) -> Self {
        self.limits = limits;
        self
    }
}

pub const SYSTEM_PROMPT: &str = r#"You are the cognition module of a file-scoped agent runtime. You propose exactly one decision per turn as a JSON object with one of these shapes:
{"decision":"act","action":{...}}
{"decision":"verify","action":{...}}
{"decision":"complete","summary":"..."}
{"decision":"fail","reason":"..."}
The action object has {"tool":<name>, ...fields} where tool is one of:
write_file {path, contents}, read_file {path}, read_range {path, offset, length}, hash_file {path}, patch_file {path, offset, expected, replacement, expected_sha256}, search_file {path, needle, max_matches}, list_dir {path}, stat_path {path}, make_dir {path}, copy_file {from, to}, move_file {from, to}, delete_path {path}.
Paths are workspace-relative: use the exact relative path from the objective (for example hello.txt), never /workspace/hello.txt or any absolute path. list_dir lists one directory (<=500 entries); stat_path reports kind/size/readonly. make_dir creates parents. copy_file and move_file refuse existing destinations; delete_path removes one file or empty directory only and never recurses. After writing, request verify with read_file and the same relative path, then complete after its observation matches. Offsets and lengths are byte counts. patch_file requires the current whole-file SHA-256 hex digest. Never propose anything else; the runtime validates, authorizes, and executes."#;

/// Adapter state. Usage accumulates across `decide` calls; read transport
/// telemetry with [`OpenAiCompat::telemetry`] and metered spend through the
/// [`Model`] trait.
pub struct OpenAiCompat {
    config: ProviderConfig,
    client: reqwest::blocking::Client,
    usage: Usage,
}

impl OpenAiCompat {
    pub fn new(config: ProviderConfig) -> Result<Self, String> {
        let url = config
            .endpoint
            .parse::<reqwest::Url>()
            .map_err(|e| format!("invalid endpoint: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("endpoint must use http or https".into());
        }
        if config.model.trim().is_empty() {
            return Err("model name must not be empty".into());
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(config.limits.timeout_per_attempt)
            .tls_built_in_webpki_certs(true)
            .build()
            .map_err(|e| format!("client build: {e}"))?;
        Ok(Self {
            config,
            client,
            usage: Usage::default(),
        })
    }

    /// Transport telemetry: HTTP calls, retries, raw token counts.
    /// Metered spend for budget enforcement lives behind the [`Model`]
    /// trait as [`harness_core::ModelUsage`].
    pub fn telemetry(&self) -> Usage {
        self.usage.clone()
    }

    fn api_key(&self) -> Option<String> {
        self.config
            .api_key_env
            .as_deref()
            .and_then(|name| std::env::var(name).ok())
            .filter(|value| !value.is_empty())
    }

    fn render_history(&self, history: &[(Action, Observation)]) -> String {
        let limits = &self.config.limits;
        let mut out = String::from("[no prior actions]");
        if history.is_empty() {
            return out;
        }
        out.clear();
        let start = history.len().saturating_sub(limits.max_history_entries);
        for (index, (action, observation)) in history.iter().enumerate().skip(start) {
            let mut data = observation.data.clone();
            if crate::truncate_to_char_boundary(&mut data, limits.max_observation_chars) {
                data.push_str("[truncated]");
            }
            out.push_str(&format!(
                "{index}: {action} -> ok={} {} | {}\n",
                observation.ok, observation.summary, data
            ));
        }
        out
    }

    fn request_body(
        &self,
        objective: &Objective,
        history: &[(Action, Observation)],
    ) -> serde_json::Value {
        serde_json::json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": format!("Objective: {}\nHistory:\n{}", objective.text, self.render_history(history))},
            ],
            "response_format": {"type": "json_object"},
        })
    }

    fn post_capped(&mut self, body: &serde_json::Value) -> Result<serde_json::Value, String> {
        let limits = &self.config.limits;
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            self.usage.http_calls += 1;
            let mut request = self.client.post(self.config.endpoint.clone()).json(body);
            if let Some(key) = self.api_key() {
                request = request.bearer_auth(key);
            }
            match request.send() {
                Err(e) => {
                    if attempts <= limits.max_retries {
                        self.usage.retries += 1;
                        backoff(attempts);
                        continue;
                    }
                    return Err(format!("transport error after {attempts} attempts: {e}"));
                }
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return read_capped(response, limits.max_response_bytes).and_then(
                            |bytes| {
                                serde_json::from_slice(&bytes)
                                    .map_err(|e| format!("invalid response JSON: {e}"))
                            },
                        );
                    }
                    if (status == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || status.is_server_error())
                        && attempts <= limits.max_retries
                    {
                        self.usage.retries += 1;
                        backoff(attempts);
                        continue;
                    }
                    return Err(format!("provider refused: HTTP {status}"));
                }
            }
        }
    }

    fn decide_from_envelope(&mut self, envelope: serde_json::Value) -> StepDecision {
        let content = envelope
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_str())
            .unwrap_or("");
        if let Some(usage) = envelope.get("usage") {
            self.usage.prompt_tokens += usage
                .get("prompt_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            self.usage.completion_tokens += usage
                .get("completion_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
        }
        let stripped = strip_fences(content.trim());
        let decision: serde_json::Value = match serde_json::from_str(stripped) {
            Ok(value) => value,
            Err(e) => return StepDecision::Fail(format!("model returned invalid JSON: {e}")),
        };
        map_decision(&decision)
    }
}

fn backoff(failed_attempts: u32) {
    let millis = 200u64.saturating_mul(1u64 << failed_attempts.min(4));
    std::thread::sleep(std::time::Duration::from_millis(millis.min(2000)));
}

fn read_capped(response: reqwest::blocking::Response, max: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut body = Vec::new();
    response
        .take(max as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("response read error: {e}"))?;
    if body.len() > max {
        return Err(format!("response exceeded {max} bytes and was rejected"));
    }
    Ok(body)
}

fn strip_fences(content: &str) -> &str {
    let mut text = content;
    if let Some(rest) = text.strip_prefix("```json") {
        text = rest;
    } else if let Some(rest) = text.strip_prefix("```") {
        text = rest;
    }
    if let Some(rest) = text.strip_suffix("```") {
        text = rest;
    }
    text.trim()
}

fn required_str(value: &serde_json::Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("action field '{field}' must be a string"))
}

fn required_u64(value: &serde_json::Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("action field '{field}' must be an unsigned integer"))
}

fn bounded_text(value: String, field: &str) -> Result<String, String> {
    if value.len() > 2 * 1024 * 1024 {
        return Err(format!("action field '{field}' exceeds 2 MiB"));
    }
    Ok(value)
}

/// Map a validated decision object to a runtime step. Anything outside the
/// closed schema is a failure, never an executed action.
pub fn map_decision(decision: &serde_json::Value) -> StepDecision {
    let kind = decision
        .get("decision")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    match kind {
        "act" | "verify" => {
            let action = decision.get("action").unwrap_or(&serde_json::Value::Null);
            match map_action(action) {
                Ok(action) => {
                    if kind == "act" {
                        StepDecision::Act(action)
                    } else {
                        StepDecision::Verify(action)
                    }
                }
                Err(reason) => StepDecision::Fail(format!("invalid model action: {reason}")),
            }
        }
        "complete" => match decision.get("summary").and_then(serde_json::Value::as_str) {
            Some(summary) => StepDecision::Complete(summary.to_string()),
            None => StepDecision::Fail("complete requires a string summary".into()),
        },
        "fail" => StepDecision::Fail(
            decision
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("model reported failure")
                .to_string(),
        ),
        _ => StepDecision::Fail(format!(
            "unknown decision '{kind}'; expected act, verify, complete, or fail"
        )),
    }
}

fn map_action(action: &serde_json::Value) -> Result<Action, String> {
    let tool = action
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .ok_or("action is missing a string 'tool'")?;
    match tool {
        "write_file" => Ok(Action::WriteFile {
            path: required_str(action, "path")?,
            contents: bounded_text(required_str(action, "contents")?, "contents")?,
        }),
        "read_file" => Ok(Action::ReadFile {
            path: required_str(action, "path")?,
        }),
        "read_range" => Ok(Action::ReadFileRange {
            path: required_str(action, "path")?,
            offset: required_u64(action, "offset")?,
            length: required_u64(action, "length")?,
        }),
        "hash_file" => Ok(Action::HashFile {
            path: required_str(action, "path")?,
        }),
        "patch_file" => Ok(Action::PatchFile {
            path: required_str(action, "path")?,
            offset: required_u64(action, "offset")?,
            expected: bounded_text(required_str(action, "expected")?, "expected")?,
            replacement: bounded_text(required_str(action, "replacement")?, "replacement")?,
            expected_sha256: required_str(action, "expected_sha256")?,
        }),
        "search_file" => Ok(Action::SearchFile {
            path: required_str(action, "path")?,
            needle: bounded_text(required_str(action, "needle")?, "needle")?,
            max_matches: required_u64(action, "max_matches")?,
        }),
        "list_dir" => Ok(Action::ListDir {
            path: required_str(action, "path")?,
        }),
        "stat_path" => Ok(Action::StatPath {
            path: required_str(action, "path")?,
        }),
        "make_dir" => Ok(Action::MakeDir {
            path: required_str(action, "path")?,
        }),
        "copy_file" => Ok(Action::CopyFile {
            from: required_str(action, "from")?,
            to: required_str(action, "to")?,
        }),
        "move_file" => Ok(Action::MoveFile {
            from: required_str(action, "from")?,
            to: required_str(action, "to")?,
        }),
        "delete_path" => Ok(Action::DeletePath {
            path: required_str(action, "path")?,
        }),
        _ => Err(format!("unknown tool '{tool}'")),
    }
}

impl Model for OpenAiCompat {
    fn name(&self) -> &str {
        "openai-compatible-provider"
    }

    /// Cumulative metered spend. Cost needs configured pricing; without it
    /// only tokens accrue and cost stays zero.
    fn usage(&self) -> ModelUsage {
        let cost_usd = self.config.pricing.as_ref().map_or(0.0, |pricing| {
            self.usage.prompt_tokens as f64 / 1000.0 * pricing.usd_per_1k_prompt_tokens
                + self.usage.completion_tokens as f64 / 1000.0
                    * pricing.usd_per_1k_completion_tokens
        });
        ModelUsage {
            prompt_tokens: self.usage.prompt_tokens,
            completion_tokens: self.usage.completion_tokens,
            cost_usd,
        }
    }

    fn decide(&mut self, objective: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        let body = self.request_body(objective, history);
        match self.post_capped(&body) {
            Ok(envelope) => self.decide_from_envelope(envelope),
            Err(reason) => StepDecision::Fail(reason),
        }
    }
}

#[cfg(test)]
mod directory_tool_tests {
    use super::map_decision;
    use harness_core::{Action, StepDecision};

    fn act(tool: &str, fields: serde_json::Value) -> serde_json::Value {
        let mut action = serde_json::json!({"tool": tool});
        for (key, value) in fields.as_object().unwrap() {
            action[key] = value.clone();
        }
        serde_json::json!({"decision": "act", "action": action})
    }

    #[test]
    fn directory_tools_parse_to_typed_actions() {
        for (tool, fields, expected) in [
            (
                "list_dir",
                serde_json::json!({"path": "sub"}),
                Action::ListDir { path: "sub".into() },
            ),
            (
                "stat_path",
                serde_json::json!({"path": "a.txt"}),
                Action::StatPath {
                    path: "a.txt".into(),
                },
            ),
            (
                "make_dir",
                serde_json::json!({"path": "new/nested"}),
                Action::MakeDir {
                    path: "new/nested".into(),
                },
            ),
            (
                "copy_file",
                serde_json::json!({"from": "a.txt", "to": "b.txt"}),
                Action::CopyFile {
                    from: "a.txt".into(),
                    to: "b.txt".into(),
                },
            ),
            (
                "move_file",
                serde_json::json!({"from": "a.txt", "to": "sub/b.txt"}),
                Action::MoveFile {
                    from: "a.txt".into(),
                    to: "sub/b.txt".into(),
                },
            ),
            (
                "delete_path",
                serde_json::json!({"path": "old.txt"}),
                Action::DeletePath {
                    path: "old.txt".into(),
                },
            ),
        ] {
            assert_eq!(
                map_decision(&act(tool, fields)),
                StepDecision::Act(expected)
            );
        }
        // Missing fields and unknown tools stay failures, never actions.
        assert!(matches!(
            map_decision(&act("copy_file", serde_json::json!({"from": "a"}))),
            StepDecision::Fail(_)
        ));
        assert!(matches!(
            map_decision(&act("format_disk", serde_json::json!({}))),
            StepDecision::Fail(_)
        ));
    }
}
