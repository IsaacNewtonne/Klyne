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
}
impl Connection {
    pub fn respond(
        &self,
        workspace: &Path,
        system: &str,
        context: &Value,
        prompt_maker: bool,
        screenshot: Option<&Path>,
        stop: &AtomicBool,
    ) -> io::Result<String> {
        let prompt = format!("{system}\n{}", context);
        if prompt.len() > 128 * 1024 {
            return Err(error("Conversation context exceeded its limit"));
        }
        if self.kind == "ollama" {
            let mut body = json!({
                "model":self.model,
                "messages":[{"role":"system","content":system},{"role":"user","content":context.to_string()}],
                "format":"json", "stream":false, "think":false
            });
            if let Some(image) = screenshot_image(screenshot) {
                body["messages"][1]["images"] = json!([image]);
            }
            if prompt_maker {
                body["options"] = json!({"temperature":0.2,"top_p":0.5});
            }
            let response = model_post(self, "/api/chat", None, &body, 120, stop)?;
            return response["message"]["content"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| error("Model returned no message"));
        }
        if self.kind == "demo" {
            return Err(error(
                "Choose an AI connection in Settings to start a conversation.",
            ));
        }
        AgentConnection {
            config: self.clone(),
            workspace: workspace.to_owned(),
            usage: ModelUsage::default(),
        }
        .decide_text(&prompt, system, screenshot, stop)
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
        return Err(error(format!(
            "Model server returned HTTP {}",
            response.status
        )));
    }
    serde_json::from_str(&response.body).map_err(|_| error("Model server returned invalid JSON"))
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
}
