//! Stage-1 latency baseline: deterministic fixtures validate orchestration
//! timing attribution (request/model/tool/observation IDs, stage timers,
//! milestones) without a real model. Wall-clock durations are
//! environment-specific; model-call counts and trace structure are exact.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn plan() -> Value {
    json!({"summary":"Make the file.","tasks":[{"agent":"Writer","instruction":"Write proof.txt with the requested contents"}]})
}
fn complete(text: &str) -> Value {
    json!({"decision":"complete","summary":text})
}

fn read_body(stream: &mut TcpStream) -> String {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let length = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|s| s.trim().parse::<usize>().unwrap())
        })
        .unwrap();
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    String::from_utf8(body).unwrap()
}
struct Server {
    child: Child,
    // Keeps the server root alive for the test lifetime; never read directly.
    #[allow(dead_code)]
    root: tempfile::TempDir,
    host: String,
}
fn spawn_verified(root: &std::path::Path, port: u16) -> Option<Child> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_klyne-studio"))
        .args(["--port", &port.to_string(), "--root"])
        .arg(root)
        .stdout(Stdio::null())
        .spawn()
        .ok()?;
    let host = format!("127.0.0.1:{port}");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if child.try_wait().ok().flatten().is_some() {
            return None;
        }
        if TcpStream::connect(&host).is_ok() {
            std::thread::sleep(Duration::from_millis(300));
            if child.try_wait().ok().flatten().is_none() {
                return Some(child);
            }
            return None;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}
impl Server {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for _ in 0..20 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            if let Some(child) = spawn_verified(root.path(), port) {
                return Self {
                    child,
                    root,
                    host: format!("127.0.0.1:{port}"),
                };
            }
        }
        panic!("could not bind a test server port");
    }
    fn api(&self, path: &str, body: Option<Value>) -> Value {
        let mut stream = TcpStream::connect(&self.host).unwrap();
        let method = if body.is_some() { "POST" } else { "GET" };
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        write!(stream,"{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nX-Klyne-Request: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",self.host,body.len()).unwrap();
        serde_json::from_str(&read_body(&mut stream)).unwrap()
    }
    fn wait(&self, id: &str) -> Value {
        let start = Instant::now();
        loop {
            let chat = self.api(&format!("/api/chats/{id}"), None);
            if !matches!(
                chat["status"].as_str(),
                Some("Planning" | "Working" | "Reviewing")
            ) {
                return chat;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "{chat}");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn model(replies: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let thread = std::thread::spawn(move || {
        let mut requests = vec![];
        let start = Instant::now();
        for reply in replies {
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            start.elapsed() < Duration::from_secs(25),
                            "Model was not called"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            requests.push(serde_json::from_str(&read_body(&mut stream)).unwrap());
            let body = json!({"message":{"content":reply.to_string()}}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        requests
    });
    (endpoint, thread)
}
fn request(endpoint: &str) -> Value {
    json!({"message":"Make a short greeting and check it","provider":{"kind":"ollama","endpoint":endpoint,"model":"fixture"},"access":{"web":false,"terminal":false,"apps":false}})
}

#[test]
fn greeting_reports_single_call_trace() {
    let s = Server::new();
    let (endpoint, fixture) = model(vec![complete("Hi! How can I help?")]);
    let mut body = request(&endpoint);
    body["message"] = json!("hi");
    let created = s.api("/api/chats", Some(body));
    let chat = s.wait(created["id"].as_str().unwrap());
    assert_eq!(chat["status"], "Completed", "{chat}");
    // Exactly one model round for a fresh greeting.
    let calls = fixture.join().unwrap();
    assert_eq!(calls.len(), 1);
    let timings = chat["execution"]["model_timings"].as_array().unwrap();
    assert_eq!(timings.len(), 1);
    assert_eq!(timings[0]["role"], "conversation");
    assert_eq!(timings[0]["attempt"], 1);
    assert!(timings[0]["prompt_bytes"].as_u64().unwrap() > 0);
    assert_eq!(timings[0]["transport_ok"], true);
    assert!(timings[0]["call_id"].as_str().unwrap().starts_with("model-"));
    let trace = &chat["execution"]["trace"];
    assert!(trace["request_id"].as_str().unwrap().starts_with("req-"));
    assert_eq!(trace["tool_calls"].as_array().unwrap().len(), 0);
    assert_eq!(trace["observations"].as_array().unwrap().len(), 0);
    assert_eq!(trace["retries"], 0);
    assert_eq!(trace["failures"], 0);
    assert!(trace["milestones"]["first_feedback_ms"].is_number());
    assert!(trace["milestones"]["first_action_ms"].is_null());
}

#[test]
fn verified_file_task_reports_tool_and_verification_stages() {
    let s = Server::new();
    let (endpoint, fixture) = model(vec![
        plan(),
        json!({"decision":"act","action":{"tool":"write_file","path":"proof.txt","contents":"checked"}}),
        complete("Written"),
    ]);
    let mut body = request(&endpoint);
    body["message"] = json!("Create the requested artifact");
    body["contract"] = json!({"goal":"Create the requested artifact","criteria":[{"FileContents":{"path":"proof.txt","expected":"checked"}}]});
    let created = s.api("/api/chats", Some(body));
    let id = created["id"].as_str().unwrap();
    let chat = s.wait(id);
    assert_eq!(chat["status"], "Completed", "{chat}");
    assert_eq!(chat["result"]["outcome"], "verified");
    // Plan + worker act + worker complete: the host contract replaces the
    // model review round.
    assert_eq!(fixture.join().unwrap().len(), 3);
    let timings = chat["execution"]["model_timings"].as_array().unwrap();
    assert_eq!(timings.len(), 3);
    let call_ids: Vec<_> = timings
        .iter()
        .map(|t| t["call_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        call_ids.iter().collect::<std::collections::HashSet<_>>().len(),
        3,
        "model call IDs are unique"
    );
    assert!(timings.iter().all(|t| t["preparation_ms"].as_u64().is_some()));
    let trace = &chat["execution"]["trace"];
    let tools = trace["tool_calls"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["tool"] == "write_file" && t["ok"] == true));
    assert!(tools.iter().any(|t| t["tool"] == "read_file:read-back"));
    let tool_ids: Vec<_> = tools
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        tool_ids.iter().collect::<std::collections::HashSet<_>>().len(),
        tool_ids.len(),
        "tool call IDs are unique"
    );
    let observations = trace["observations"].as_array().unwrap();
    assert!(!observations.is_empty());
    let obs_ids: Vec<_> = observations
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        obs_ids.iter().collect::<std::collections::HashSet<_>>().len(),
        obs_ids.len(),
        "observation IDs are unique"
    );
    // Every persisted evidence entry carries its observation ID.
    assert!(chat["evidence"].as_array().unwrap().iter().all(|e| e["observation_id"].is_string()));
    assert!(trace["milestones"]["first_feedback_ms"].is_number());
    assert!(trace["milestones"]["first_action_ms"].is_number());
    assert!(trace["milestones"]["verified_completion_ms"].is_number());
    assert!(trace["stage"]["verification_ms"].as_u64().is_some());
    assert!(trace["stage"]["db_checkpoint_ms"].as_u64().is_some());
}

#[test]
fn resume_continues_the_same_goal_trace() {
    let s = Server::new();
    let question = json!({"decision":"needs_input","question":"Which title should I use?","blocker":{"kind":"user_preference","missing":"user choice for this fixture","why_user":"The fixture requires a choice not supplied in the original request"}});
    let (endpoint, fixture) = model(vec![
        plan(),
        question,
        complete("Title accepted"),
        complete("Finished"),
    ]);
    let created = s.api("/api/chats", Some(request(&endpoint)));
    let id = created["id"].as_str().unwrap();
    let paused = s.wait(id);
    assert_eq!(paused["status"], "Needs input");
    let request_id = paused["execution"]["trace"]["request_id"].clone();
    assert!(request_id.as_str().unwrap().starts_with("req-"));
    let mut answer = request(&endpoint);
    answer["id"] = json!(id);
    answer["message"] = json!("Use Project notes");
    s.api("/api/chats", Some(answer));
    let done = s.wait(id);
    assert_eq!(done["status"], "Completed", "{done}");
    assert_eq!(done["execution"]["trace"]["request_id"], request_id);
    assert_eq!(done["execution"]["model_timings"].as_array().unwrap().len(), 4);
    assert_eq!(fixture.join().unwrap().len(), 4);
}

#[test]
fn pulse_endpoint_serves_sequenced_events_with_trace() {
    let s = Server::new();
    let (endpoint, fixture) = model(vec![complete("Hi! How can I help?")]);
    let mut body = request(&endpoint);
    body["message"] = json!("hi");
    let created = s.api("/api/chats", Some(body));
    let id = created["id"].as_str().unwrap();
    let chat = s.wait(id);
    assert_eq!(chat["status"], "Completed", "{chat}");
    fixture.join().unwrap();
    let pulse: Value = s.api(&format!("/api/chats/{id}/pulse?since=0"), None);
    assert_eq!(pulse["status"], "Completed");
    let seq = pulse["seq"].as_u64().unwrap();
    assert!(seq > 0, "completed work emits sequenced events");
    assert!(!pulse["events"].as_array().unwrap().is_empty());
    assert!(pulse["serve_ms"].as_u64().is_some());
    assert!(pulse["trace"]["request_id"].as_str().unwrap().starts_with("req-"));
    assert_eq!(pulse["trace"]["model_calls"], 1);
    // Nothing new since the latest sequence: empty events, same sequence.
    let quiet: Value = s.api(&format!("/api/chats/{id}/pulse?since={seq}"), None);
    assert_eq!(quiet["seq"], seq);
    assert!(quiet["events"].as_array().unwrap().is_empty());
    // Unknown conversations error instead of leaking state.
    let missing: Value = s.api("/api/chats/nope-0/pulse?since=0", None);
    assert!(missing["error"].is_string());
}
