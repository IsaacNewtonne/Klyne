mod mcp;
mod activation;
mod app_schema;
mod app_adapter;
mod execution_graph;
mod failure_policy;
mod route_recovery;
mod restart_reconciliation;
mod document_save;
mod browser_tools;
mod capabilities;
mod chat;
mod chat_store;
mod connections;
mod desktop;
mod improvement;
mod local_apps;
mod network;
mod recovery;
mod telemetry;

use harness_core::{
    AgentRuntime, Objective, PermissionPolicy, ShutdownHandle, SqliteEventStore, ToolRegistry,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Studio {
    admission: Mutex<()>,
    quiescing: std::sync::atomic::AtomicBool,
    chats: Arc<chat::Chats>,
    root: PathBuf,
    active: Mutex<HashMap<String, ShutdownHandle>>,
}
fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 80 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
fn safe_dir(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(err("run directory is not a regular directory"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(err("reparse directory refused"));
        }
    }
    Ok(())
}
/// Scratch directory for server-level desktop helpers (app list/launch).
/// Separate from conversation workspaces so helper scripts never appear as
/// user artifacts.
fn desktop_scratch(root: &Path) -> io::Result<PathBuf> {
    safe_dir(root)?;
    let dir = root.join("desktop");
    fs::create_dir_all(&dir)?;
    safe_dir(&dir)?;
    Ok(dir)
}
/// Read-only installed Start-menu app list. No desktop lease needed:
/// listing never sends input.
fn desktop_apps(root: &Path) -> io::Result<Value> {
    if !cfg!(windows) {
        return Err(err("Desktop control currently requires Windows."));
    }
    let dir = desktop_scratch(root)?;
    let idle = AtomicBool::new(false);
    desktop::execute(&dir, &desktop::DesktopAction::Apps, None, &idle)
}
/// Explicit user-initiated launch from the Apps picker. The AppID allowlist
/// is enforced twice: format here and installed-app membership inside the
/// desktop helper (plus notepad.exe/calc.exe). Holds the desktop lease so a
/// launch never races a conversation's verified control turn.
fn desktop_launch(root: &Path, app_id: &str) -> io::Result<Value> {
    if !cfg!(windows) {
        return Err(err("Desktop control currently requires Windows."));
    }
    let action = desktop::DesktopAction::parse(
        &json!({"tool": "desktop_launch", "app_id": app_id}),
        true,
        false,
        None,
    )?;
    let stop = Arc::new(AtomicBool::new(false));
    let _lease = desktop::Lease::acquire(stop.clone())
        .map_err(|_| err("Another conversation controls the desktop. Stop it or wait."))?;
    let dir = desktop_scratch(root)?;
    let result = desktop::execute(&dir, &action, None, &stop)?;
    // Keep the picker response small: the full observation (windows and
    // controls) already refreshed the scratch screenshot for the models.
    let windows = result["observation"]["windows"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    Ok(json!({"ok": true, "windows": windows}))
}
impl Studio {
    fn directory(&self, id: &str) -> io::Result<PathBuf> {
        if !valid_id(id) {
            return Err(err("invalid run ID"));
        }
        let path = self.root.join(id);
        safe_dir(&path)?;
        Ok(path)
    }
    fn snapshot(&self, id: &str) -> io::Result<Value> {
        let dir = self.directory(id)?;
        let db = dir.join("run.sqlite3");
        let meta = fs::symlink_metadata(&db)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(err("invalid database"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(err("reparse database refused"));
            }
        }
        let mut conn =
            Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
        conn.busy_timeout(Duration::from_millis(300)).map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        let payload: Option<String> = tx
            .query_row(
                "SELECT payload FROM checkpoints WHERE id=1 AND length(payload) <= 4194304",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let state: Value = payload
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(err)?
            .unwrap_or(Value::Null);
        let mut query = tx
            .prepare(
                "SELECT seq, kind, substr(detail,1,8192) FROM events ORDER BY seq DESC LIMIT 200",
            )
            .map_err(err)?;
        let events = query.query_map([], |r| Ok(json!({"seq":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"detail":r.get::<_,String>(2)?}))).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .map_err(err)?;
        let error = fs::read_to_string(dir.join("error.txt")).ok();
        Ok(
            json!({"id":id,"state":state,"events":events,"event_count":count,"active":self.active.lock().unwrap().contains_key(id),"error":error,"provider":connections::Connection::read(&dir.join("provider.json"))?}),
        )
    }
    fn list(&self) -> io::Result<Value> {
        let mut ids = fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|id| valid_id(id))
            .collect::<Vec<_>>();
        ids.sort();
        ids.reverse();
        ids.truncate(100);
        let runs = ids.iter().map(|id| match self.snapshot(id) {
            Ok(mut s) => { s.as_object_mut().unwrap().remove("events"); s },
            Err(e) => json!({"id":id,"state":null,"error":e.to_string(),"active":self.active.lock().unwrap().contains_key(id)})
        }).collect::<Vec<_>>();
        Ok(json!({"runs":runs,"root":self.root,"mode":"Deterministic file agent"}))
    }
    fn launch(
        self: &Arc<Self>,
        id: String,
        objective: Option<String>,
        budget: u64,
    ) -> io::Result<()> {
        let dir = self.directory(&id)?;
        let mut active = self.active.lock().unwrap();
        if active.contains_key(&id) {
            return Err(err("run is already active"));
        }
        let store = SqliteEventStore::open(dir.join("run.sqlite3"))?;
        let workspace = dir.join("files");
        fs::create_dir_all(&workspace)?;
        safe_dir(&workspace)?;
        let connection = connections::Connection::read(&dir.join("provider.json"))?;
        let model = connection.build(&workspace)?;
        let mut runtime = AgentRuntime::new(
            model,
            ToolRegistry::milestone_default(),
            PermissionPolicy::milestone_default(&workspace),
            store,
        )
        .with_max_tool_calls(budget);
        active.insert(id.clone(), runtime.shutdown_handle());
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match objective {
                    Some(text) => runtime.run(Objective::new(text)),
                    None => runtime.resume(),
                }));
            let message = match outcome {
                Ok(Ok(_)) => String::new(),
                Ok(Err(e)) => e.to_string(),
                Err(_) => "worker panicked; inspect pending action before resuming".into(),
            };
            let _ = fs::write(dir.join("error.txt"), message);
            drop(runtime);
            this.active.lock().unwrap().remove(&id);
        });
        Ok(())
    }
}

fn respond(stream: &mut TcpStream, status: &str, mime: &str, body: &[u8]) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}
fn serve(mut stream: TcpStream, studio: &Arc<Studio>, host: &str) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut raw = Vec::new();
    let mut byte = [0];
    while !raw.ends_with(b"\r\n\r\n") {
        if raw.len() >= 8192 {
            return respond(
                &mut stream,
                "431 Request Header Fields Too Large",
                "text/plain",
                b"headers too large",
            );
        }
        stream.read_exact(&mut byte)?;
        raw.push(byte[0]);
    }
    let head = String::from_utf8(raw).map_err(err)?;
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let parts = first.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 {
        return respond(&mut stream, "400 Bad Request", "text/plain", b"bad request");
    }
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        .collect::<Vec<_>>();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    if header("host") != Some(host)
        || header("origin").is_some_and(|v| v != format!("http://{host}"))
        || header("sec-fetch-site") == Some("cross-site")
    {
        return respond(
            &mut stream,
            "403 Forbidden",
            "text/plain",
            b"local same-origin requests only",
        );
    }
    let (method, path) = (parts[0], parts[1]);
    if method == "GET" {
        let asset = match path {
            "/" => Some((
                "text/html; charset=utf-8",
                include_bytes!("../web/chat.html").as_slice(),
            )),
            "/style.css" => Some(("text/css", include_bytes!("../web/style.css").as_slice())),
            "/files" => Some((
                "text/html; charset=utf-8",
                include_bytes!("../web/index.html").as_slice(),
            )),
            "/chat.css" => Some(("text/css", include_bytes!("../web/chat.css").as_slice())),
            "/production.css" => Some((
                "text/css",
                include_bytes!("../web/production.css").as_slice(),
            )),
            "/production.js" => Some((
                "text/javascript",
                include_bytes!("../web/production.js").as_slice(),
            )),
            "/chat.js" => Some((
                "text/javascript",
                include_bytes!("../web/chat.js").as_slice(),
            )),
            "/app.js" => Some((
                "text/javascript",
                include_bytes!("../web/app.js").as_slice(),
            )),
            _ => None,
        };
        if let Some((mime, bytes)) = asset {
            return respond(&mut stream, "200 OK", mime, bytes);
        }
    }
    let result: io::Result<Value> = (|| {
        if method == "GET" && path == "/api/runtime/ready" {
            return Ok(
                json!({"pid":std::process::id(),"ready":studio.chats.active_count()==0 && studio.active.lock().unwrap().is_empty()}),
            );
        }
        if method == "GET" && path == "/api/runtime/load" {
            return Ok(telemetry::read());
        }
        if method == "GET" && path == "/api/recovery" {
            let settings = recovery::settings(&studio.root);
            let state = fs::read(studio.root.join("recovery/status.json")).ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            return Ok(json!({"enabled":settings.is_some(),"state":state}));
        }
        if method == "GET" && path == "/api/runs" {
            return studio.list();
        }
        if method == "GET" && path == "/api/chats" {
            return studio.chats.list();
        }
        if method == "GET"
            && let Some(id) = path.strip_prefix("/api/chats/")
        {
            return serde_json::to_value(studio.chats.get(id)?).map_err(err);
        }
        if method == "GET"
            && let Some(id) = path.strip_prefix("/api/runs/")
        {
            return studio.snapshot(id);
        }
        if method == "GET" && path == "/api/desktop/apps" {
            return desktop_apps(&studio.root);
        }
        if method == "GET" && path == "/api/apps" {
            let root = studio.root.join("conversations");
            fs::create_dir_all(&root)?;
            return local_apps::execute(&root, &json!({"tool":"app_list"}), true, true);
        }
        if header("transfer-encoding").is_some()
            || headers
                .iter()
                .filter(|(k, _)| k == "content-length")
                .count()
                != 1
        {
            return Err(err("invalid request framing"));
        }
        let length: usize = header("content-length")
            .ok_or_else(|| err("length required"))?
            .parse()
            .map_err(err)?;
        if length > 16384 {
            return Err(err("request exceeds 16 KiB"));
        }
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        if method != "POST"
            || header("x-klyne-request") != Some("1")
            || header("content-type") != Some("application/json")
        {
            return Err(err("unsupported request"));
        }
        let body: Value = serde_json::from_slice(&body).map_err(err)?;
        let starting = matches!(
            path,
            "/api/chats" | "/api/runs" | "/api/desktop/launch" | "/api/apps/select" | "/api/runtime/quiesce"
        );
        let _admission = starting.then(|| studio.admission.lock().unwrap());
        if path == "/api/runtime/quiesce" {
            studio.quiescing.store(true, Ordering::SeqCst);
            return Ok(json!({"quiescing":true}));
        }
        if starting && studio.quiescing.load(Ordering::SeqCst) {
            return Err(err("Runtime upgrade in progress; retry after reconnection"));
        }

        if path == "/api/apps" {
            // The management UI can edit definitions or inspect a schema. It
            // cannot dispatch arbitrary model tools or invoke app operations.
            if !matches!(
                body["tool"].as_str(),
                Some("app_connect" | "app_inspect" | "app_operations" | "app_forget")
            ) {
                return Err(err("Unsupported connection management action"));
            }
            let root = studio.root.join("conversations");
            fs::create_dir_all(&root)?;
            return local_apps::execute(&root, &body, true, false);
        }
        if let Some(id) = path
            .strip_prefix("/api/chats/")
            .and_then(|p| p.strip_suffix("/resolve"))
        {
            return studio.chats.resolve(id, &body);
        }
        if path == "/api/chats" {
            return studio.chats.send(&body);
        }
        if let Some(id) = path
            .strip_prefix("/api/chats/")
            .and_then(|p| p.strip_suffix("/manage"))
        {
            return studio.chats.manage(id, &body);
        }
        if let Some(id) = path
            .strip_prefix("/api/chats/")
            .and_then(|p| p.strip_suffix("/stop"))
        {
            return studio.chats.stop(id);
        }
        if path == "/api/connections/check" {
            return connections::Connection::parse(&body)?.check();
        }
        if path == "/api/apps/select" {
            let id=body["app_id"].as_str().ok_or_else(||err("Choose an installed app"))?;
            let apps=desktop_apps(&studio.root)?;
            let app=apps["apps"].as_array().and_then(|a|a.iter().find(|a|a["AppID"]==id)).ok_or_else(||err("App is not installed"))?;
            let name=app["Name"].as_str().unwrap_or("");
            let api_connection = body.get("api_connection").map(|v| v.as_str().ok_or_else(||err("Invalid API connection"))).transpose()?;
            let route=app_adapter::select(&studio.root.join("conversations"),name,id,api_connection)?;
            if route["route"]=="desktop" {desktop_launch(&studio.root,id)?;}
            return Ok(route);
        }
        if path == "/api/desktop/launch" {
            let app_id = body["app_id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 500)
                .ok_or_else(|| err("Choose an installed app first"))?;
            return desktop_launch(&studio.root, app_id);
        }
        if path == "/api/runs" {
            let text = body["objective"]
                .as_str()
                .filter(|s| !s.trim().is_empty() && s.len() <= 8192)
                .ok_or_else(|| err("objective must contain 1–8192 bytes"))?;
            let budget = body["budget"]
                .as_u64()
                .filter(|n| *n <= 128)
                .ok_or_else(|| err("budget must be an integer between 0 and 128"))?;
            if studio.active.lock().unwrap().len() >= 4 {
                return Err(err("four runs are already active"));
            }
            let connection = connections::Connection::parse(&body["provider"])?;
            // Validate model selection before allocating a run directory.
            if connection.kind == "ollama" && connection.model.is_empty() {
                return Err(err("Choose an installed Ollama model"));
            }
            if connection.kind == "opencode"
                && !connection
                    .model
                    .split_once('/')
                    .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
            {
                return Err(err("Choose an OpenCode model in provider/model format"));
            }
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let id = format!(
                "{}-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            fs::create_dir(studio.root.join(&id))?;
            fs::write(
                studio.root.join(&id).join("provider.json"),
                serde_json::to_vec(&connection).map_err(err)?,
            )?;
            studio.launch(id.clone(), Some(text.into()), budget)?;
            return Ok(json!({"id":id}));
        }
        if let Some(rest) = path.strip_prefix("/api/runs/")
            && let Some((id, action)) = rest.split_once('/')
        {
            studio.directory(id)?;
            match action {
                "stop" => {
                    let active = studio.active.lock().unwrap();
                    active
                        .get(id)
                        .ok_or_else(|| err("run is not active"))?
                        .request();
                }
                "resume" => {
                    let s = studio.snapshot(id)?;
                    if !s["state"]["outcome"].is_null() || !s["state"]["pending"].is_null() {
                        return Err(err("terminal or uncertain pending run cannot resume here"));
                    }
                    studio.launch(id.into(), None, 32)?;
                }
                _ => return Err(err("unknown action")),
            }
            return Ok(json!({"id":id}));
        }
        Err(err("route not found"))
    })();
    let (status, body) = match result {
        Ok(v) => ("200 OK", v),
        Err(e) => ("400 Bad Request", json!({"error":e.to_string()})),
    };
    respond(
        &mut stream,
        status,
        "application/json",
        body.to_string().as_bytes(),
    )
}
fn main() -> io::Result<()> {
    if std::env::args().any(|a| a == "--runtime-check") {
        println!("{{\"klyne_runtime_protocol\":1}}");
        return Ok(());
    }
    let mut args = std::env::args().skip(1);
    let mut root = PathBuf::from("workspace/studio");
    let mut port = 4317u16;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                root = args
                    .next()
                    .ok_or_else(|| err("--root needs a directory"))?
                    .into()
            }
            "--port" => {
                port = args
                    .next()
                    .ok_or_else(|| err("--port needs a port"))?
                    .parse()
                    .map_err(err)?
            }
            _ => return Err(err("usage: klyne-studio [--root directory] [--port 4317]")),
        }
    }
    fs::create_dir_all(&root)?;
    let root = fs::canonicalize(root)?;
    safe_dir(&root)?;
    let owner = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("studio.lock"))?;
    owner
        .try_lock()
        .map_err(|_| err("Another Studio process owns this root"))?;
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let host = listener.local_addr()?.to_string();
    println!("Klyne Studio → http://{host}");
    let studio = Arc::new(Studio {
        admission: Mutex::new(()),
        quiescing: std::sync::atomic::AtomicBool::new(false),
        chats: Arc::new(chat::Chats::new(&root)),
        root,
        active: Mutex::new(HashMap::new()),
    });
    studio.chats.recover()?;
    // Serial bounded HTTP handling; agent execution occurs on separate workers.
    let http_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if http_active.fetch_add(1, Ordering::SeqCst) >= 32 {
                    http_active.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let studio = studio.clone();
                let host = host.clone();
                let active = http_active.clone();
                std::thread::spawn(move || {
                    struct Slot(Arc<std::sync::atomic::AtomicUsize>);
                    impl Drop for Slot {
                        fn drop(&mut self) {
                            self.0.fetch_sub(1, Ordering::SeqCst);
                        }
                    }
                    let _slot = Slot(active);
                    let _ = serve(stream, &studio, &host);
                });
            }
            Err(e) => eprintln!("connection: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::HeuristicModel;
    use std::time::Instant;

    fn post(studio: &Arc<Studio>, path: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = listener.local_addr().unwrap().to_string();
        let app = Arc::clone(studio);
        let server_host = host.clone();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, &app, &server_host).unwrap();
        });
        let mut client = TcpStream::connect(&host).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(client,"POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nX-Klyne-Request: 1\r\nContent-Length: 2\r\n\r\n{{}}").unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();
        response
    }

    #[test]
    fn stop_endpoint_signals_runtime_then_resume_preserves_budget_and_finishes() {
        let root = tempfile::tempdir().unwrap();
        let id = "123456-0";
        let directory = root.path().join(id);
        let files = directory.join("files");
        fs::create_dir_all(&files).unwrap();
        let studio = Arc::new(Studio {
            admission: Mutex::new(()),
            quiescing: std::sync::atomic::AtomicBool::new(false),
            chats: Arc::new(chat::Chats::new(root.path())),
            root: root.path().into(),
            active: Mutex::new(HashMap::new()),
        });
        let mut runtime = AgentRuntime::new(
            HeuristicModel,
            ToolRegistry::milestone_default(),
            PermissionPolicy::milestone_default(&files),
            SqliteEventStore::open(directory.join("run.sqlite3")).unwrap(),
        )
        .with_max_tool_calls(7);
        let stop = runtime.shutdown_handle();
        studio
            .active
            .lock()
            .unwrap()
            .insert(id.into(), stop.clone());
        assert!(post(&studio, &format!("/api/runs/{id}/stop")).starts_with("HTTP/1.1 200"));
        assert!(stop.is_requested());
        let result = runtime.run(Objective::new(
            "create file resumed.txt with content preserved",
        ));
        assert!(result.unwrap_err().to_string().contains("shutdown"));
        drop(runtime);
        studio.active.lock().unwrap().remove(id);
        let checkpoint = studio.snapshot(id).unwrap();
        assert_eq!(checkpoint["state"]["tool_budget"]["limit"], 7);
        assert!(checkpoint["state"]["outcome"].is_null());
        assert!(!files.join("resumed.txt").exists());
        assert!(post(&studio, &format!("/api/runs/{id}/resume")).starts_with("HTTP/1.1 200"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while studio.active.lock().unwrap().contains_key(id) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        let checkpoint = studio.snapshot(id).unwrap();
        assert!(checkpoint["state"]["outcome"]["Completed"].is_string());
        assert_eq!(checkpoint["state"]["tool_budget"]["limit"], 7);
        assert_eq!(checkpoint["state"]["tool_budget"]["used"], 3);
        assert_eq!(
            fs::read_to_string(files.join("resumed.txt")).unwrap(),
            "preserved"
        );
        assert!(post(&studio, &format!("/api/runs/{id}/stop")).starts_with("HTTP/1.1 400"));
    }
}
