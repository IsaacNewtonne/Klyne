use harness_core::{
    Action, HeuristicModel, Model, ModelUsage, Objective, Observation, StepDecision,
};
use harness_provider::{
    OpenAiCompat, ProviderConfig, ProviderLimits,
    openai::{SYSTEM_PROMPT, map_decision},
};
use reqwest::blocking::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn error(message: impl ToString) -> io::Error {
    io::Error::other(message.to_string())
}
const CAP: usize = 512 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Connection {
    pub kind: String,
    pub endpoint: String,
    pub model: String,
    /// Tool exposure for local models (2026-10-24): "" or "auto" picks
    /// compact mode for Ollama models of 14B parameters or fewer, "compact"
    /// always exposes core tools plus enable_tools, "full" exposes all.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tool_mode: String,
}
/// Streaming progress reported while a model request runs.
pub enum Progress<'a> {
    /// The model is probably not resident (first call or idle past keep-alive).
    LoadingModel,
    /// Request sent; waiting for the first token.
    Waiting,
    /// Tokens are arriving; the text so far (may be partial JSON).
    Generating(&'a str),
}
/// Ollama keep-alive and context requested by Klyne (2026-10-24 fix plan).
pub const OLLAMA_KEEP_ALIVE: &str = "30m";
pub const OLLAMA_NUM_CTX: u64 = 16384;
/// Whether this Ollama request should think. Workers never think (it costs
/// time per tool step); planner and reviewer think only when the setting
/// allows it and the model reports the "thinking" capability.
pub fn ollama_think(role: &str, setting: &str, supports_thinking: bool) -> bool {
    setting == "planning" && supports_thinking && matches!(role, "planner" | "independent reviewer")
}
fn thinking_support() -> &'static std::sync::Mutex<std::collections::HashMap<String, bool>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, bool>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}
/// Reads /api/show capabilities once per endpoint+model; false on any error.
fn ollama_supports_thinking(endpoint: &str, model: &str) -> bool {
    let key = format!("{endpoint}|{model}");
    if let Some(known) = thinking_support().lock().unwrap().get(&key) {
        return *known;
    }
    let supports = client(10)
        .and_then(|http| {
            request_json(
                http.post(format!("{endpoint}/api/show")),
                Some(&json!({"model":model})),
            )
        })
        .map(|show| {
            show["capabilities"]
                .as_array()
                .is_some_and(|caps| caps.iter().any(|c| c == "thinking"))
        });
    match supports {
        Ok(value) => {
            thinking_support().lock().unwrap().insert(key, value);
            value
        }
        Err(_) => false,
    }
}
fn warm_models() -> &'static std::sync::Mutex<std::collections::HashMap<String, Instant>> {
    static WARM: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Instant>>> =
        std::sync::OnceLock::new();
    WARM.get_or_init(Default::default)
}
impl Connection {
    /// One model decision plus the metered usage behind it. Callers accrue
    /// the usage into durable turn totals; Studio budgets enforce them in
    /// `guard` (audit Phase 7: track tokens/cost, not just op counts).
    pub fn respond(
        &self,
        workspace: &Path,
        system: &str,
        context: &Value,
        prompt_maker: bool,
        screenshot: Option<&Path>,
        stop: &AtomicBool,
    ) -> io::Result<(String, ModelUsage, crate::performance::ProviderTiming)> {
        self.respond_with_progress(
            workspace,
            system,
            context,
            prompt_maker,
            screenshot,
            stop,
            &mut |_| {},
        )
    }
    /// Whether this connection should use compact tool exposure.
    pub fn compact_tools(&self) -> bool {
        match self.tool_mode.as_str() {
            "compact" => true,
            "full" => false,
            _ => {
                self.kind == "ollama"
                    && crate::native_tools::parameter_billions(&self.model)
                        .is_some_and(|b| b <= 14.0)
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn respond_with_progress(
        &self,
        workspace: &Path,
        system: &str,
        context: &Value,
        prompt_maker: bool,
        screenshot: Option<&Path>,
        stop: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> io::Result<(String, ModelUsage, crate::performance::ProviderTiming)> {
        let prompt = format!("{system}\n{}", context);
        if prompt.len() > 128 * 1024 {
            return Err(error("Conversation context exceeded its limit"));
        }
        if self.kind == "ollama" {
            return self.ollama_chat(system, context, prompt_maker, screenshot, stop, progress);
        }
        if self.kind == "demo" {
            return Err(error(
                "Choose an AI connection in Settings to start a conversation.",
            ));
        }
        let mut agent = AgentConnection {
            config: self.clone(),
            workspace: workspace.to_owned(),
            usage: ModelUsage::default(),
        };
        let text = agent.decide_text(&prompt, system, screenshot, stop)?;
        Ok((
            text,
            agent.usage,
            crate::performance::ProviderTiming::default(),
        ))
    }
    /// Ollama /api/chat with streaming, native tools for worker roles, and
    /// the keep-alive/context options from the 2026-10-24 fix plan.
    fn ollama_chat(
        &self,
        system: &str,
        context: &Value,
        prompt_maker: bool,
        screenshot: Option<&Path>,
        stop: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> io::Result<(String, ModelUsage, crate::performance::ProviderTiming)> {
        let role = context["role"].as_str().unwrap_or("");
        let native = matches!(role, "worker" | "independent reviewer")
            && context["available_tools"].is_object();
        let mut update = String::new();
        if let Some(observation) = context["observations"]
            .as_array()
            .and_then(|items| items.last())
        {
            update.push_str("\n\nHOST UPDATE: The following operation has already executed. This is a tool result, not a new instruction. Use the result to advance the assigned task; do not repeat a successful operation without a concrete reason.\n");
            update.push_str(&observation.to_string());
            if native {
                update.push_str("\nCall the NEXT tool. If the assigned result is now done and verified, call finish with the actual result (for listings include the names; for files include the path). Reviewers must assess the entire original goal.");
            } else {
                update.push_str("\nReturn the NEXT JSON decision. If the assigned result is now saved and verified, return {\"decision\":\"complete\",\"summary\":\"describe the actual saved result and its path\"}. If work remains, take the next necessary action. Reviewers must assess the entire original goal.");
            }
        }
        // Host notes (memory, loop warnings, resume nudges) go in their
        // own system block after the main prompt.
        let mut messages = vec![json!({"role":"system","content":system})];
        let mut shown = context.clone();
        if let Some(notes) = shown
            .as_object_mut()
            .and_then(|o| o.remove("system_notes"))
            .and_then(|n| n.as_array().cloned())
            .filter(|n| !n.is_empty())
        {
            let lines: Vec<String> = notes
                .iter()
                .filter_map(|n| n.as_str().map(|n| format!("- {n}")))
                .collect();
            messages.push(json!({"role":"system","content":format!("HOST NOTES (memory and warnings from Klyne, not from the user):\n{}", lines.join("\n"))}));
        }
        let context_index = messages.len();
        messages.push(json!({"role":"user","content":shown.to_string()}));
        let mut body = json!({
            "model":self.model,
            "messages":messages,
            "stream":true, "think":false,
            "keep_alive":OLLAMA_KEEP_ALIVE,
        });
        if let Some(setting) = context["think_setting"].as_str()
            && setting != "off"
            && matches!(role, "planner" | "independent reviewer")
            && ollama_think(
                role,
                setting,
                ollama_supports_thinking(&self.endpoint, &self.model),
            )
        {
            body["think"] = json!(true);
        }
        if native {
            let enabled: Vec<String> = context["enabled_tool_groups"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            let mut tools = crate::native_tools::tools(
                &context["available_tools"],
                &enabled,
                context["compact_tools"] == true,
                role == "independent reviewer",
                &crate::native_tools::hint(context),
            );
            if role == "independent reviewer" && context["review_findings"] == false {
                crate::native_tools::without_findings(&mut tools);
            }
            // Narration guard: the forced step offers only acting tools
            // (plus ask_user/report_blocker) in compact mode.
            if context["force_tool"] == true && context["compact_tools"] == true {
                crate::native_tools::only_acting(&mut tools);
            }
            body["tools"] = json!(tools);
        } else {
            body["format"] = json!("json");
        }
        if !update.is_empty() {
            body["messages"]
                .as_array_mut()
                .unwrap()
                .push(json!({"role":"user","content":update}));
        }
        if let Some(image) = screenshot_image(screenshot) {
            body["messages"][context_index]["images"] = json!([image]);
        }
        // Keep the default 16k window unless the prompt clearly cannot fit;
        // Ollama silently drops the start of an over-long prompt (the system
        // instructions), which is worse than a slower, larger window.
        let estimated_tokens =
            (body["messages"].to_string().len() + body["tools"].to_string().len()) / 3 + 2048;
        let num_ctx = if estimated_tokens > OLLAMA_NUM_CTX as usize {
            32768
        } else {
            OLLAMA_NUM_CTX
        };
        body["options"] = json!({"num_ctx":num_ctx});
        if prompt_maker {
            body["options"]["temperature"] = json!(0.2);
            body["options"]["top_p"] = json!(0.5);
        }
        let key = format!("{}|{}", self.endpoint, self.model);
        let cold = warm_models()
            .lock()
            .unwrap()
            .get(&key)
            .is_none_or(|at| at.elapsed() > Duration::from_secs(29 * 60));
        progress(if cold {
            Progress::LoadingModel
        } else {
            Progress::Waiting
        });
        let mut content = String::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        let mut last: Value = Value::Null;
        let mut last_report = Instant::now() - Duration::from_secs(1);
        let mut chunk_error = None;
        let response = crate::network::send_lines(
            |http| http.post(format!("{}/api/chat", self.endpoint)).json(&body),
            120,
            8 * 1024 * 1024,
            stop,
            &mut |line| {
                let Ok(chunk) = serde_json::from_str::<Value>(line) else {
                    return;
                };
                if let Some(message) = chunk["error"].as_str() {
                    chunk_error = Some(message.chars().take(200).collect::<String>());
                }
                if let Some(text) = chunk["message"]["content"].as_str() {
                    content.push_str(text);
                }
                if let Some(calls) = chunk["message"]["tool_calls"].as_array() {
                    tool_calls.extend(calls.iter().cloned());
                }
                if last_report.elapsed() >= Duration::from_millis(400) {
                    last_report = Instant::now();
                    progress(Progress::Generating(&content));
                }
                last = chunk;
            },
        )?;
        if !(200..300).contains(&response.status) {
            return Err(model_http_error(self, response.status));
        }
        if let Some(message) = chunk_error {
            return Err(error(format!(
                "Ollama reported an error while generating: {message}"
            )));
        }
        if last.is_null() {
            return Err(error("Model server returned invalid JSON"));
        }
        if last["message"].is_null() && content.is_empty() && tool_calls.is_empty() {
            return Err(error("Model returned no message"));
        }
        if last["done_reason"] == "length" && tool_calls.is_empty() {
            return Err(error(
                "Model output was cut off at the length limit; retry with a shorter reply",
            ));
        }
        warm_models().lock().unwrap().insert(key, Instant::now());
        let text = if native {
            crate::native_tools::normalize_reply(&content, &tool_calls)
        } else {
            crate::native_tools::strip_think(&content)
        };
        // Ollama reports eval counts on the final chunk.
        let usage = ModelUsage {
            prompt_tokens: last["prompt_eval_count"].as_u64().unwrap_or(0),
            completion_tokens: last["eval_count"].as_u64().unwrap_or(0),
            cost_usd: 0.0,
        };
        Ok((
            text,
            usage,
            crate::performance::ProviderTiming::from_ollama(&last),
        ))
    }
    pub fn parse(value: &Value) -> io::Result<Self> {
        let mut config: Self = if value.is_null() {
            Self::default()
        } else {
            serde_json::from_value(value.clone()).map_err(error)?
        };
        if config.kind.is_empty() {
            config.kind = "demo".into();
        }
        if !["demo", "ollama", "opencode", "codex"].contains(&config.kind.as_str()) {
            return Err(error("Unknown provider"));
        }
        if config.model.len() > 240 || config.model.chars().any(char::is_control) {
            return Err(error("Invalid model name"));
        }
        if !["", "auto", "compact", "full"].contains(&config.tool_mode.as_str()) {
            return Err(error("Unknown tool mode"));
        }
        if config.kind == "ollama" || config.kind == "opencode" {
            if config.endpoint.is_empty() {
                config.endpoint = if config.kind == "ollama" {
                    "http://127.0.0.1:11434"
                } else {
                    "http://127.0.0.1:4096"
                }
                .into();
            }
            let url = reqwest::Url::parse(&config.endpoint)
                .map_err(|_| error("Enter a local HTTP server address"))?;
            if url.scheme() != "http"
                || !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                return Err(error(
                    "Use a loopback HTTP address such as http://127.0.0.1:11434, without credentials or a path",
                ));
            }
            config.endpoint = config.endpoint.trim_end_matches('/').to_owned();
        } else {
            config.endpoint.clear();
        }
        Ok(config)
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= 4096 => {
                Self::parse(&serde_json::from_slice::<Value>(&bytes).map_err(error)?)
            }
            Ok(_) => Err(error("Provider settings are too large")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self::parse(&Value::Null),
            Err(e) => Err(e),
        }
    }
    pub fn check(&self) -> io::Result<Value> {
        match self.kind.as_str() {
            "demo" => Ok(json!({"message":"Built-in file agent is ready.","models":[]})),
            "codex" => {
                let mut command = codex_command()?;
                command.args(["login", "status"]);
                let (_, output) = execute(command, "", Duration::from_secs(8))?;
                if !output.contains("Logged in") {
                    return Err(error("Sign in once with codex login, then check again."));
                }
                Ok(
                    json!({"message":"Codex is signed in. Uses your CLI account; inference may be cloud-hosted.","models":[]}),
                )
            }
            "ollama" => {
                let data =
                    request_json(client(4)?.get(format!("{}/api/tags", self.endpoint)), None)?;
                let models = data["models"]
                    .as_array()
                    .ok_or_else(|| error("Unexpected Ollama model list"))?
                    .iter()
                    .filter_map(|m| m["name"].as_str())
                    .collect::<Vec<_>>();
                Ok(
                    json!({"message":if models.is_empty() {"Ollama is online. Pull a model with ollama pull <model>."} else {"Ollama is online. Choose an installed model."},"models":models}),
                )
            }
            "opencode" => {
                let http = client(4)?;
                let health = request_json(
                    self.auth(http.get(format!("{}/global/health", self.endpoint))),
                    None,
                )?;
                if health["healthy"] != true {
                    return Err(error("OpenCode did not report healthy"));
                }
                let data = request_json_capped(
                    self.auth(http.get(format!("{}/provider", self.endpoint))),
                    None,
                    8 * 1024 * 1024,
                )?;
                let connected = data["connected"]
                    .as_array()
                    .ok_or_else(|| error("Unexpected OpenCode provider list"))?;
                let mut models = Vec::new();
                for provider in data["all"]
                    .as_array()
                    .ok_or_else(|| error("Missing OpenCode providers"))?
                {
                    if !connected.contains(&provider["id"]) {
                        continue;
                    }
                    if let (Some(id), Some(entries)) =
                        (provider["id"].as_str(), provider["models"].as_object())
                    {
                        for model in entries.keys() {
                            models.push(format!("{id}/{model}"));
                        }
                    }
                }
                Ok(
                    json!({"message":"OpenCode server is connected. Model hosting depends on its provider.","models":models}),
                )
            }
            _ => Err(error("Unknown provider")),
        }
    }
    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        if let Ok(password) = std::env::var("OPENCODE_SERVER_PASSWORD") {
            request.basic_auth(
                std::env::var("OPENCODE_SERVER_USERNAME").unwrap_or_else(|_| "opencode".into()),
                Some(password),
            )
        } else {
            request
        }
    }
    pub fn build(&self, workspace: &Path) -> io::Result<Box<dyn Model>> {
        match self.kind.as_str() {
            "demo" => Ok(Box::new(HeuristicModel)),
            "ollama" => {
                if self.model.is_empty() {
                    return Err(error("Choose an installed Ollama model"));
                }
                let config = ProviderConfig::new(
                    format!("{}/v1/chat/completions", self.endpoint),
                    &self.model,
                )
                .with_limits(ProviderLimits {
                    max_retries: 0,
                    timeout_per_attempt: Duration::from_secs(120),
                    ..Default::default()
                });
                Ok(Box::new(OpenAiCompat::new(config).map_err(error)?))
            }
            "codex" => {
                codex_command()?;
                Ok(Box::new(AgentConnection {
                    config: self.clone(),
                    workspace: workspace.to_owned(),
                    usage: ModelUsage::default(),
                }))
            }
            "opencode" => {
                if !self
                    .model
                    .split_once('/')
                    .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
                {
                    return Err(error("Choose an OpenCode model in provider/model format"));
                }
                Ok(Box::new(AgentConnection {
                    config: self.clone(),
                    workspace: workspace.to_owned(),
                    usage: ModelUsage::default(),
                }))
            }
            _ => Err(error("Unknown provider")),
        }
    }
}
fn model_post(
    config: &Connection,
    path: &str,
    workspace: Option<&Path>,
    body: &Value,
    seconds: u64,
    stop: &AtomicBool,
) -> io::Result<Value> {
    let response = crate::network::send(
        |http| {
            let mut request = http.post(format!("{}{path}", config.endpoint)).json(body);
            if let Some(workspace) = workspace {
                request = request.query(&[("directory", workspace.to_string_lossy().as_ref())]);
            }
            if config.kind == "opencode"
                && let Ok(password) = std::env::var("OPENCODE_SERVER_PASSWORD")
            {
                request = request.basic_auth(
                    std::env::var("OPENCODE_SERVER_USERNAME").unwrap_or_else(|_| "opencode".into()),
                    Some(password),
                );
            }
            request
        },
        seconds,
        CAP,
        stop,
    )?;
    if !(200..300).contains(&response.status) {
        return Err(model_http_error(config, response.status));
    }
    serde_json::from_str(&response.body).map_err(|_| error("Model server returned invalid JSON"))
}
fn model_http_error(config: &Connection, status: u16) -> io::Error {
    let guidance = match (config.kind.as_str(), status) {
        ("ollama", 404) => format!(
            "Ollama could not find the selected model or API route. Open Settings, check the connection, and select an installed model. Selected model: {}.",
            config.model
        ),
        (_, 401 | 403) => "The model server refused access. Check the provider's sign-in or credentials in Settings.".into(),
        (_, 429) => "The model server is rate limited. Wait for capacity or select a configured alternative.".into(),
        (_, 500..=599) => "The model server failed. Check that the selected model can load and that the server has enough memory.".into(),
        _ => "Check the selected provider, endpoint and model in Settings.".into(),
    };
    // Do not forward raw server bodies: they may contain credentials or HTML.
    error(format!("Model server returned HTTP {status}. {guidance}"))
}
fn client(seconds: u64) -> io::Result<Client> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(seconds))
        .build()
        .map_err(error)
}
fn request_json(request: RequestBuilder, body: Option<&Value>) -> io::Result<Value> {
    request_json_capped(request, body, CAP)
}
fn request_json_capped(
    request: RequestBuilder,
    body: Option<&Value>,
    cap: usize,
) -> io::Result<Value> {
    let request = if let Some(body) = body {
        request.json(body)
    } else {
        request
    };
    let response = request
        .send()
        .map_err(|_| error("Cannot reach local server. Start it and check the address."))?;
    if !response.status().is_success() {
        return Err(error(format!(
            "Local server returned HTTP {}",
            response.status()
        )));
    }
    let mut bytes = Vec::new();
    response.take(cap as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > cap {
        return Err(error("Local server response exceeded its size limit"));
    }
    serde_json::from_slice(&bytes).map_err(|_| error("Local server returned invalid JSON"))
}
fn codex_command() -> io::Result<Command> {
    // Execute a native binary or the npm JS entrypoint directly. Never interpolate
    // prompts/model names into cmd.exe or PowerShell command text.
    if let Some(path) = std::env::var_os("KLYNE_CODEX_BIN") {
        return Ok(Command::new(path));
    }
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let native = directory.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        if native.is_file() {
            return Ok(Command::new(native));
        }
        let script = directory.join("node_modules/@openai/codex/bin/codex.js");
        if script.is_file() {
            let mut command = Command::new("node");
            command.arg(script);
            return Ok(command);
        }
    }
    Err(error(
        "Codex CLI was not found. Install it or set KLYNE_CODEX_BIN, then restart Studio.",
    ))
}
fn terminate(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
fn execute(command: Command, input: &str, timeout: Duration) -> io::Result<(String, String)> {
    execute_cancellable(command, input, timeout, &AtomicBool::new(false))
}
fn execute_cancellable(
    mut command: Command,
    input: &str,
    timeout: Duration,
    stop: &AtomicBool,
) -> io::Result<(String, String)> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| error("Could not start Codex CLI"))?;
    let _job = match harness_core::process_job::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(e) => {
            terminate(&mut child);
            return Err(e);
        }
    };
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read = |mut stream: Box<dyn Read + Send>| -> io::Result<Vec<u8>> {
        let mut captured = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let n = stream.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            if captured.len() + n <= CAP {
                captured.extend_from_slice(&buffer[..n]);
            } else {
                return Err(error("Codex output exceeded 512 KiB"));
            }
        }
        Ok(captured)
    };
    let out = std::thread::spawn(move || read(Box::new(stdout)));
    let err = std::thread::spawn(move || read(Box::new(stderr)));
    // Write on a separate thread so a child which never reads stdin is bounded too.
    let mut stdin = child.stdin.take().unwrap();
    let input = input.as_bytes().to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) if out.is_finished() && err.is_finished() && writer.is_finished() => {
                break status;
            }
            Ok(_) if Instant::now() < deadline && !stop.load(Ordering::SeqCst) => {
                std::thread::sleep(Duration::from_millis(25))
            }
            _ => {
                terminate(&mut child);
                return Err(error("Codex timed out; its process was stopped."));
            }
        }
    };
    let _ = writer.join();
    let stdout = out
        .join()
        .map_err(|_| error("Codex output reader failed"))??;
    let stderr = err
        .join()
        .map_err(|_| error("Codex output reader failed"))??;
    if !status.success() {
        return Err(error(
            "Codex exited unsuccessfully. Check codex login status and model access.",
        ));
    }
    Ok((
        String::from_utf8(stdout).map_err(error)?,
        String::from_utf8_lossy(&stderr).into_owned(),
    ))
}
struct AgentConnection {
    config: Connection,
    workspace: PathBuf,
    usage: ModelUsage,
}
impl AgentConnection {
    fn decide_text(
        &mut self,
        prompt: &str,
        system: &str,
        screenshot: Option<&Path>,
        stop: &AtomicBool,
    ) -> io::Result<String> {
        if self.config.kind == "codex" {
            let mut command = codex_command()?;
            command.current_dir(&self.workspace).args([
                "exec",
                "--json",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--color",
                "never",
            ]);
            if !self.config.model.is_empty() {
                command.arg("--model").arg(&self.config.model);
            }
            // Codex sees the current screenshot too; OpenCode below stays text-only.
            if screenshot.is_some_and(|path| path.is_file()) {
                command.arg("-i").arg(screenshot.unwrap());
            }
            command.arg("-");
            let (stdout, _) = execute_cancellable(command, prompt, Duration::from_secs(120), stop)?;
            let mut message = None;
            for line in stdout.lines() {
                let event: Value =
                    serde_json::from_str(line).map_err(|_| error("Invalid Codex event"))?;
                if event["type"] == "item.completed" && event["item"]["type"] == "agent_message" {
                    message = event["item"]["text"].as_str().map(str::to_owned);
                }
                if event["type"] == "turn.failed" || event["type"] == "error" {
                    return Err(error("Codex reported a failed turn"));
                }
                if event["type"] == "turn.completed" {
                    self.usage.prompt_tokens +=
                        event["usage"]["input_tokens"].as_u64().unwrap_or(0);
                    self.usage.completion_tokens +=
                        event["usage"]["output_tokens"].as_u64().unwrap_or(0);
                }
            }
            return message.ok_or_else(|| error("Codex returned no final message"));
        }
        let session = model_post(
            &self.config,
            "/session",
            Some(&self.workspace),
            &json!({"title":"Klyne decision","permission":[{"permission":"*","pattern":"*","action":"deny"}]}),
            120,
            stop,
        )?;
        let id = session["id"]
            .as_str()
            .filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .ok_or_else(|| error("Invalid OpenCode session ID"))?;
        let (provider, model) = self
            .config
            .model
            .split_once('/')
            .ok_or_else(|| error("Missing OpenCode model"))?;
        let result = model_post(
            &self.config,
            &format!("/session/{id}/message"),
            Some(&self.workspace),
            &json!({"model":{"providerID":provider,"modelID":model},"system":system,"parts":[{"type":"text","text":prompt}]}),
            120,
            stop,
        );
        if result.is_err() {
            let _ = model_post(
                &self.config,
                &format!("/session/{id}/abort"),
                Some(&self.workspace),
                &json!({}),
                3,
                &AtomicBool::new(false),
            );
        }
        let response = result?;
        if !response["info"]["error"].is_null() {
            return Err(error(
                "OpenCode reported a provider error. Check its model configuration.",
            ));
        }
        self.usage.prompt_tokens += response["info"]["tokens"]["input"].as_u64().unwrap_or(0);
        self.usage.completion_tokens += response["info"]["tokens"]["output"].as_u64().unwrap_or(0);
        self.usage.cost_usd += response["info"]["cost"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(0.0);
        let parts = response["parts"]
            .as_array()
            .ok_or_else(|| error("OpenCode returned no message parts"))?;
        Ok(parts
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"))
    }
}
impl Model for AgentConnection {
    fn name(&self) -> &str {
        &self.config.kind
    }
    fn usage(&self) -> ModelUsage {
        self.usage.clone()
    }
    fn decide(&mut self, objective: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        let context = json!({"objective":objective.text,"history":history});
        let prompt = format!(
            "{SYSTEM_PROMPT}\nDo not call any tools yourself. Return only a JSON decision; Klyne executes it. Write the exact requested content, then verify with read_file, then complete.\n{context}"
        );
        if prompt.len() > 128 * 1024 {
            return StepDecision::Fail("Provider context exceeds 128 KiB".into());
        }
        match self.decide_text(&prompt, SYSTEM_PROMPT, None, &AtomicBool::new(false)) {
            Ok(text) => {
                let text = text
                    .trim()
                    .strip_prefix("```json")
                    .unwrap_or(text.trim())
                    .trim();
                let text = text.strip_suffix("```").unwrap_or(text).trim();
                match serde_json::from_str(text) {
                    Ok(value) => map_decision(&value),
                    Err(_) => StepDecision::Fail(
                        "Provider returned an invalid decision; no action was executed".into(),
                    ),
                }
            }
            Err(e) => StepDecision::Fail(e.to_string()),
        }
    }
}
/// Current desktop screenshot as base64, if desktop observation left one.
/// Missing, oversized or unreadable images yield None so the model still
/// receives the accessibility text. OpenCode callers ignore this by design.
fn screenshot_image(path: Option<&Path>) -> Option<String> {
    let path = path?;
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    Some(base64_encode(&bytes))
}
fn base64_encode(bytes: &[u8]) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut block = [0u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let triple = (block[0] as u32) << 16 | (block[1] as u32) << 8 | block[2] as u32;
        out.push(B64[((triple >> 18) & 63) as usize] as char);
        out.push(B64[((triple >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((triple >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(triple & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn thinking_is_only_for_planner_and_reviewer_when_supported() {
        assert!(super::ollama_think("planner", "planning", true));
        assert!(super::ollama_think(
            "independent reviewer",
            "planning",
            true
        ));
        assert!(!super::ollama_think("worker", "planning", true));
        assert!(!super::ollama_think("planner", "planning", false));
        assert!(!super::ollama_think("planner", "off", true));
    }

    use super::*;
    #[test]
    fn local_connections_reject_remote_hosts_credentials_and_unknown_fields() {
        for endpoint in [
            "https://127.0.0.1:11434",
            "http://example.com",
            "http://127.0.0.1@evil.test",
            "http://user:secret@localhost",
            "http://localhost/api",
            "http://localhost?secret=x",
        ] {
            assert!(Connection::parse(&json!({"kind":"ollama","endpoint":endpoint})).is_err());
        }
        assert!(Connection::parse(&json!({"kind":"unknown"})).is_err());
        assert!(Connection::parse(&json!({"kind":"codex","command":"arbitrary"})).is_err());
        assert_eq!(Connection::parse(&Value::Null).unwrap().kind, "demo");
        assert_eq!(
            Connection::parse(&json!({"kind":"ollama"}))
                .unwrap()
                .endpoint,
            "http://127.0.0.1:11434"
        );
    }
    #[test]
    fn provider_settings_survive_restart_and_do_not_silently_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provider.json");
        std::fs::write(&path, br#"{"kind":"ollama","model":"local-model"}"#).unwrap();
        let config = Connection::read(&path).unwrap();
        assert_eq!(config.kind, "ollama");
        assert_eq!(config.model, "local-model");
        std::fs::write(&path, b"broken").unwrap();
        assert!(Connection::read(&path).is_err());
    }

    #[test]
    fn missing_model_errors_identify_the_setting_to_fix() {
        let config = Connection::parse(&json!({"kind":"ollama","model":"missing-model"})).unwrap();
        let message = model_http_error(&config, 404).to_string();
        assert!(message.contains("HTTP 404"));
        assert!(message.contains("missing-model"));
        assert!(message.contains("select an installed model"));
        assert!(
            model_http_error(&config, 503)
                .to_string()
                .contains("enough memory")
        );
    }
}

// ---- 2026-10-24: installed-model check and plain helper calls ----

/// Installed Ollama model names at `endpoint`, cached for 60 seconds.
/// None when Ollama is unreachable (callers then skip the check).
pub fn ollama_models(endpoint: &str) -> Option<Vec<String>> {
    use std::sync::{Mutex, OnceLock};
    type Cache = Mutex<Vec<(String, std::time::Instant, Vec<String>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(entries) = cache.lock()
        && let Some((_, at, models)) = entries.iter().find(|(e, _, _)| e == endpoint)
        && at.elapsed() < Duration::from_secs(60)
    {
        return Some(models.clone());
    }
    let data = request_json(client(3).ok()?.get(format!("{endpoint}/api/tags")), None).ok()?;
    let models: Vec<String> = data["models"]
        .as_array()?
        .iter()
        .filter_map(|m| m["name"].as_str().map(str::to_owned))
        .collect();
    if let Ok(mut entries) = cache.lock() {
        entries.retain(|(e, _, _)| e != endpoint);
        entries.push((
            endpoint.to_owned(),
            std::time::Instant::now(),
            models.clone(),
        ));
    }
    Some(models)
}

/// Whether `model` is in an installed list; a tag without `:` means `:latest`.
pub fn model_listed(models: &[String], model: &str) -> bool {
    let wanted = if model.contains(':') {
        model.to_owned()
    } else {
        format!("{model}:latest")
    };
    models.iter().any(|m| m == model || *m == wanted)
}

/// Some(false) only when Ollama answered and the model is absent.
pub fn ollama_model_installed(endpoint: &str, model: &str) -> Option<bool> {
    ollama_models(endpoint).map(|models| model_listed(&models, model))
}

/// One plain (no tools, no JSON mode) non-streaming Ollama chat call.
/// Returns the reply text and a usage summary.
pub fn ollama_plain(
    endpoint: &str,
    model: &str,
    system: &str,
    user: &str,
    num_ctx: u64,
    timeout_seconds: u64,
) -> io::Result<(String, Value)> {
    let body = json!({"model":model,"stream":false,"keep_alive":"30m",
        "options":{"num_ctx":num_ctx,"temperature":0.2},
        "messages":[{"role":"system","content":system},{"role":"user","content":user}]});
    let data = request_json(
        client(timeout_seconds.clamp(5, 600))?.post(format!("{endpoint}/api/chat")),
        Some(&body),
    )?;
    if let Some(message) = data["error"].as_str() {
        return Err(error(format!("Helper model error: {message}")));
    }
    let text = crate::native_tools::strip_think(data["message"]["content"].as_str().unwrap_or(""))
        .trim()
        .to_owned();
    let seconds = |key: &str| data[key].as_f64().unwrap_or(0.0) / 1e9;
    Ok((
        text,
        json!({"prompt_tokens":data["prompt_eval_count"],"completion_tokens":data["eval_count"],
            "load_seconds":seconds("load_duration"),"total_seconds":seconds("total_duration")}),
    ))
}

#[cfg(test)]
mod model_check_tests {
    #[test]
    fn model_listed_matches_exact_and_latest() {
        let models = vec!["qwen3.5:9b".to_owned(), "llama3:latest".to_owned()];
        assert!(super::model_listed(&models, "qwen3.5:9b"));
        assert!(super::model_listed(&models, "llama3"));
        assert!(!super::model_listed(
            &models,
            "hf.co/empero-ai/Qwythos-9B:Q4_K_M"
        ));
    }
}
