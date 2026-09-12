use harness_browser::{BrowserLimits, ControlledBrowser};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
// Chrome startup is resource intensive; avoid competing isolated launches.
static BROWSER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Server {
    child: Child,
    root: tempfile::TempDir,
    addr: String,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let root = tempfile::tempdir().unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_klyne-studio"))
            .args(["--port", &port.to_string(), "--root"])
            .arg(root.path())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let server = Self {
            child,
            root,
            addr: format!("127.0.0.1:{port}"),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(&server.addr).is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        }
        server
    }
    fn request(
        &self,
        method: &str,
        path: &str,
        host: Option<&str>,
        body: Option<&str>,
        custom: bool,
    ) -> String {
        let mut stream = TcpStream::connect(&self.addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let body = body.unwrap_or("");
        write!(stream,"{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{body}",host.unwrap_or(&self.addr),body.len(),if custom{"X-Klyne-Request: 1\r\n"}else{""}).unwrap();
        let mut result = String::new();
        // A refused request may close with unread request bytes on Windows.
        // Read the response by its declared length, rather than requiring EOF.
        while !result.ends_with("\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            result.push(byte[0] as char);
        }
        let length: usize = result
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        result.push_str(&String::from_utf8(body).unwrap());
        result
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn wait(browser: &mut ControlledBrowser, expression: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if browser.eval(expression).unwrap() == Value::Bool(true) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out: {expression}");
        std::thread::sleep(Duration::from_millis(100));
    }
}
#[test]
fn api_connection_management_requires_intent_and_cannot_invoke_tools() {
    let s = Server::new();
    let connect = r#"{"tool":"app_connect","name":"fixture","base_url":"http://127.0.0.1:1234"}"#;
    assert!(
        s.request("POST", "/api/apps", None, Some(connect), false)
            .starts_with("HTTP/1.1 400")
    );
    assert!(
        s.request("GET", "/api/apps", Some("attacker.invalid"), None, true)
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        s.request("POST", "/api/apps", None, Some(connect), true)
            .contains("\"saved\":\"fixture\"")
    );
    assert!(
        s.request("GET", "/api/apps", None, None, false)
            .contains("\"name\":\"fixture\"")
    );
    for tool in ["app_call", "app_invoke", "self_improve", "run_shell"] {
        let body = serde_json::json!({"tool":tool,"name":"fixture","path":"/danger"}).to_string();
        assert!(
            s.request("POST", "/api/apps", None, Some(&body), true)
                .starts_with("HTTP/1.1 400")
        );
    }
    let forget = r#"{"tool":"app_forget","name":"fixture"}"#;
    assert!(
        s.request("POST", "/api/apps", None, Some(forget), true)
            .contains("\"forgotten\":\"fixture\"")
    );
    assert!(
        s.request("GET", "/api/apps", None, None, false)
            .contains("\"connections\":[]")
    );
}

#[test]
fn local_api_refuses_cross_host_missing_intent_and_invalid_runs() {
    let s = Server::new();
    assert!(
        s.request("GET", "/api/runs", Some("attacker.invalid"), None, false)
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        s.request(
            "POST",
            "/api/runs",
            None,
            Some(r#"{"objective":"test","budget":32}"#),
            false
        )
        .starts_with("HTTP/1.1 400")
    );
    assert!(
        s.request(
            "POST",
            "/api/runs",
            None,
            Some(r#"{"objective":"test","budget":129}"#),
            true
        )
        .starts_with("HTTP/1.1 400")
    );
    assert!(
        s.request("GET", "/api/runs/../outside", None, None, false)
            .starts_with("HTTP/1.1 400")
    );
    assert_eq!(
        fs::read_dir(s.root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec!["studio.lock".to_string()]
    );
}
#[test]
fn desktop_launch_validates_app_ids_before_touching_the_desktop() {
    let s = Server::new();
    // Missing intent header is refused before any validation.
    assert!(
        s.request(
            "POST",
            "/api/desktop/launch",
            None,
            Some(r#"{"app_id":"notepad.exe"}"#),
            false
        )
        .starts_with("HTTP/1.1 400")
    );
    // Empty, oversized and control-character IDs are rejected pre-execution:
    // no helper scripts may appear under the studio root.
    for body in [
        r#"{"app_id":""}"#.to_string(),
        r#"{"app_id":"badid"}"#.to_string(),
        format!(r#"{{"app_id":"{}"}}"#, "x".repeat(501)),
    ] {
        assert!(
            s.request("POST", "/api/desktop/launch", None, Some(&body), true)
                .starts_with("HTTP/1.1 400"),
            "{body}"
        );
    }
    assert!(!s.root.path().join("desktop").exists());
}
#[test]
fn studio_simple_flow_creates_completes_and_exports() {
    let _browser_guard = BROWSER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Server::new();
    let downloads = tempfile::tempdir().unwrap();
    let mut b = ControlledBrowser::launch_isolated(BrowserLimits::default()).unwrap();
    b.set_download_directory(downloads.path()).unwrap();
    b.navigate(&format!("http://{}/files", s.addr)).unwrap();
    wait(
        &mut b,
        "document.querySelector('#connection').textContent.includes('Connected')",
    );
    assert!(b.text().unwrap().contains("Ready when you are."));
    // Provider selection, discovery, persistence, and stale response protection.
    b.eval("window.__fetch=window.fetch;window.fetch=(url,options)=>String(url)==='/api/connections/check'?Promise.resolve(new Response(JSON.stringify({message:'Fixture connected',models:['fixture-model']}),{status:200})):__fetch(url,options);document.querySelector('#provider-kind').value='ollama';document.querySelector('#provider-kind').dispatchEvent(new Event('change'))").unwrap();
    b.click("#check-provider").unwrap();
    wait(
        &mut b,
        "document.querySelector('#provider-model').value==='fixture-model'",
    );
    assert_eq!(
        b.eval("JSON.parse(localStorage.getItem('klyne-provider')).kind")
            .unwrap(),
        "ollama"
    );
    assert_eq!(
        b.eval("document.querySelector('#provider-settings').open")
            .unwrap(),
        true
    );
    b.eval("document.querySelector('#provider-kind').value='codex';document.querySelector('#provider-kind').dispatchEvent(new Event('change'))").unwrap();
    assert_eq!(b.eval("document.querySelector('#endpoint-field').hidden && !document.querySelector('#provider-model').required").unwrap(),true);
    b.eval("document.querySelector('#provider-kind').value='demo';document.querySelector('#provider-kind').dispatchEvent(new Event('change'));window.fetch=__fetch").unwrap();
    b.eval("window.__errors=[];addEventListener('error',e=>__errors.push(e.message));addEventListener('unhandledrejection',e=>__errors.push(String(e.reason)))").unwrap();
    // Invalid paths are rejected without creating runs.
    b.fill("#file-path", "../escape.txt").unwrap();
    b.click("#launch-button").unwrap();
    wait(
        &mut b,
        "document.querySelector('#form-error').textContent.length>0",
    );
    assert_eq!(
        fs::read_dir(s.root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec!["studio.lock".to_string()]
    );
    // A real run completes and its file is read back from disk.
    b.fill("#file-path", "proof.txt").unwrap();
    b.fill(
        "#file-contents",
        "<img src=x onerror=window.injected=true> verified by Klyne",
    )
    .unwrap();
    b.click("#launch-button").unwrap();
    wait(
        &mut b,
        "document.querySelector('#status-badge').textContent==='Completed'",
    );
    let id = b
        .eval("document.querySelector('#run-id').textContent")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        fs::read_to_string(s.root.path().join(&id).join("files/proof.txt")).unwrap(),
        "<img src=x onerror=window.injected=true> verified by Klyne"
    );
    // Markup in file contents is shown as text, never executed.
    assert_eq!(b.eval("window.injected===undefined").unwrap(), true);
    assert_eq!(
        b.eval("document.querySelectorAll('.evidence-item').length")
            .unwrap(),
        3
    );
    // Event filtering and run filtering both work.
    b.click(".events-panel > summary").unwrap();
    b.fill("#event-search", "VerificationPassed").unwrap();
    assert!(
        b.eval("document.querySelectorAll('.event-row').length>0")
            .unwrap()
            .as_bool()
            .unwrap()
    );
    b.fill("#event-search", "no-such-event").unwrap();
    assert!(
        b.eval("document.querySelector('#event-ledger').textContent.includes('No matching')")
            .unwrap()
            .as_bool()
            .unwrap()
    );
    b.fill("#event-search", "").unwrap();
    b.fill("#run-search", "not-a-run").unwrap();
    assert!(
        b.eval("document.querySelector('#run-list').textContent.includes('No matching')")
            .unwrap()
            .as_bool()
            .unwrap()
    );
    b.fill("#run-search", "").unwrap();
    // Export is a real Chrome download, parsed from disk.
    b.click("#export").unwrap();
    let file = downloads.path().join(format!("klyne-{id}-evidence.json"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !file.exists() {
        assert!(Instant::now() < deadline, "download did not arrive");
        std::thread::sleep(Duration::from_millis(50));
    }
    let exported: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    assert_eq!(exported["id"], id);
    assert!(exported["state"]["outcome"]["Completed"].is_string());
    // Screenshots stay available for a quick visual check.
    b.click(".events-panel > summary").unwrap();
    b.eval("window.scrollTo(0,0)").unwrap();
    let artifacts =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workspace/studio-qa");
    fs::create_dir_all(&artifacts).unwrap();
    b.set_viewport(1280, 900).unwrap();
    fs::write(
        artifacts.join("studio-desktop.png"),
        b.screenshot().unwrap(),
    )
    .unwrap();
    b.set_viewport(390, 844).unwrap();
    b.eval("window.scrollTo(0,0)").unwrap();
    assert_eq!(
        b.eval("document.documentElement.scrollWidth <= innerWidth")
            .unwrap(),
        true
    );
    fs::write(artifacts.join("studio-mobile.png"), b.screenshot().unwrap()).unwrap();
    assert_eq!(b.eval("__errors").unwrap(), serde_json::json!([]));
    b.close();
}

#[test]
fn run_survives_server_restart_and_terminal_resume_is_refused() {
    let mut s = Server::new();
    let reply = s.request(
        "POST",
        "/api/runs",
        None,
        Some(r#"{"objective":"create file durable.txt with content persisted","budget":32}"#),
        true,
    );
    let body: Value = serde_json::from_str(reply.split_once("\r\n\r\n").unwrap().1).unwrap();
    let id = body["id"].as_str().unwrap();
    let route = format!("/api/runs/{id}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let reply = s.request("GET", &route, None, None, false);
        let body: Value = serde_json::from_str(reply.split_once("\r\n\r\n").unwrap().1).unwrap();
        if body["state"]["outcome"]["Completed"].is_string() && body["active"] == false {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(30));
    }
    s.child.kill().unwrap();
    s.child.wait().unwrap();
    let port = s.addr.split(':').next_back().unwrap();
    s.child = Command::new(env!("CARGO_BIN_EXE_klyne-studio"))
        .args(["--port", port, "--root"])
        .arg(s.root.path())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(&s.addr).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(30));
    }
    let reply = s.request("GET", &route, None, None, false);
    let body: Value = serde_json::from_str(reply.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert!(body["state"]["outcome"]["Completed"].is_string());
    let before = body["event_count"].clone();
    assert!(
        s.request("POST", &format!("{route}/resume"), None, Some("{}"), true)
            .starts_with("HTTP/1.1 400")
    );
    let reply = s.request("GET", &route, None, None, false);
    let after: Value = serde_json::from_str(reply.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(before, after["event_count"]);
    assert_eq!(
        fs::read_to_string(s.root.path().join(id).join("files/durable.txt")).unwrap(),
        "persisted"
    );
}

#[test]
fn studio_stop_control_and_real_checkpoint_resume() {
    let _browser_guard = BROWSER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use harness_core::{
        AgentRuntime, HeuristicModel, Objective, PermissionPolicy, SqliteEventStore, ToolRegistry,
    };
    let s = Server::new();
    let id = "1788999999999-0";
    let directory = s.root.path().join(id);
    let files = directory.join("files");
    fs::create_dir_all(&files).unwrap();
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&files),
        SqliteEventStore::open(directory.join("run.sqlite3")).unwrap(),
    )
    .with_max_tool_calls(9);
    runtime
        .create_run(
            Objective::new("create file resumed.txt with content real checkpoint"),
            Default::default(),
            None,
        )
        .unwrap();
    drop(runtime);
    let mut b = ControlledBrowser::launch_isolated(BrowserLimits::default()).unwrap();
    b.navigate(&format!("http://{}/files", s.addr)).unwrap();
    wait(&mut b, "!document.querySelector('#resume').hidden");
    // Only the too-short-to-click active state and stop response use a browser
    // fixture. The native stop endpoint has an independent runtime unit test.
    b.eval("window.__originalFetch=window.fetch;window.__activeFixture=true;window.__stopFailure=true;window.fetch=async(url,options)=>{if(String(url).endsWith('/stop')){if(__stopFailure)return new Response(JSON.stringify({error:'Controlled stop failure'}),{status:400});window.__activeFixture=false;return new Response('{}',{status:200});}const r=await __originalFetch(url,options);if(String(url).startsWith('/api/runs')){const data=await r.json();if(data.state&&!data.state.outcome)data.active=__activeFixture;if(data.runs)data.runs.forEach(run=>{if(!run.state?.outcome)run.active=__activeFixture});return new Response(JSON.stringify(data),{status:r.status});}return r;}").unwrap();
    b.click("#refresh").unwrap();
    wait(&mut b, "!document.querySelector('#stop').hidden");
    b.click("#stop").unwrap();
    wait(
        &mut b,
        "document.querySelector('#toast').textContent==='Controlled stop failure'",
    );
    assert_eq!(
        b.eval("document.querySelector('#stop').disabled").unwrap(),
        false
    );
    b.eval("window.__stopFailure=false").unwrap();
    b.click("#stop").unwrap();
    wait(&mut b, "!document.querySelector('#resume').hidden");
    b.eval("window.fetch=__originalFetch").unwrap();
    b.click("#resume").unwrap();
    wait(
        &mut b,
        "document.querySelector('#status-badge').textContent==='Completed'",
    );
    assert_eq!(
        fs::read_to_string(files.join("resumed.txt")).unwrap(),
        "real checkpoint"
    );
    b.close();
}

#[test]
fn studio_ollama_connection_executes_and_persists_real_provider_choice() {
    let s = Server::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let mock = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        for turn in 0..4 {
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(Instant::now() < deadline, "provider fixture timed out");
                std::thread::sleep(Duration::from_millis(20));
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|v| v.parse::<usize>().ok())
                })
                .unwrap_or(0);
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).unwrap();
            let response = if turn == 0 {
                assert!(head.starts_with("GET /api/tags "));
                serde_json::json!({"models":[{"name":"fixture-model"}]})
            } else {
                assert!(head.starts_with("POST /v1/chat/completions "));
                let request:Value=serde_json::from_slice(&bytes).unwrap();
                assert_eq!(request["model"],"fixture-model");
                let decision = match turn {
                    1 => serde_json::json!({"decision":"act","action":{"tool":"write_file","path":"provider.txt","contents":"provider proof"}}),
                    2 => serde_json::json!({"decision":"verify","action":{"tool":"read_file","path":"provider.txt"}}),
                    _ => serde_json::json!({"decision":"complete","summary":"verified"}),
                };
                serde_json::json!({"choices":[{"message":{"content":decision.to_string()}}],"usage":{"prompt_tokens":10,"completion_tokens":5}})
            }.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        }
    });
    let provider = serde_json::json!({"kind":"ollama","endpoint":endpoint,"model":"fixture-model"});
    let check = s.request(
        "POST",
        "/api/connections/check",
        None,
        Some(&provider.to_string()),
        true,
    );
    assert!(check.starts_with("HTTP/1.1 200"));
    assert!(check.contains("fixture-model"));
    let created=s.request("POST","/api/runs",None,Some(&serde_json::json!({"objective":"create file provider.txt with content provider proof","budget":8,"provider":provider}).to_string()),true);
    let created: Value = serde_json::from_str(created.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let id = created["id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = s.request("GET", &format!("/api/runs/{id}"), None, None, false);
        let snapshot: Value =
            serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        if snapshot["active"] == false {
            assert!(
                snapshot["state"]["outcome"]["Completed"].is_string(),
                "{snapshot}"
            );
            assert_eq!(snapshot["provider"], provider);
            assert_eq!(snapshot["state"]["used_tokens"], 45);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(30));
    }
    mock.join().unwrap();
    assert_eq!(
        fs::read_to_string(s.root.path().join(id).join("files/provider.txt")).unwrap(),
        "provider proof"
    );
    let persisted: Value =
        serde_json::from_slice(&fs::read(s.root.path().join(id).join("provider.json")).unwrap())
            .unwrap();
    assert_eq!(persisted, provider);
    for invalid in [
        serde_json::json!({"kind":"ollama","endpoint":"http://external.test"}),
        serde_json::json!({"kind":"codex","command":"arbitrary"}),
    ] {
        assert!(
            s.request(
                "POST",
                "/api/connections/check",
                None,
                Some(&invalid.to_string()),
                true
            )
            .starts_with("HTTP/1.1 400")
        );
    }
    assert!(
        s.request("POST", "/api/connections/check", None, Some("{}"), false)
            .starts_with("HTTP/1.1 400")
    );
}
