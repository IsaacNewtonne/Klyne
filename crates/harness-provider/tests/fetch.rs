use harness_core::permissions::PermissionPolicy;
use harness_core::tools::ToolRegistry;
use harness_core::types::{Action, Observation};
use harness_provider::radar::{fetch_text, github_search_url, render_digest};
use harness_provider::{FetchLimits, FetchTool};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

fn serve(
    status: u16,
    body: Vec<u8>,
    delay_ms: u64,
) -> (
    String,
    mpsc::Receiver<FetchSeen>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (send, receive) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut raw = Vec::new();
        let mut chunk = [0u8; 4096];
        let end = loop {
            match stream.read(&mut chunk) {
                // Client went away (e.g. timeout kill): nothing to serve.
                Ok(0) | Err(_) => return,
                Ok(count) => {
                    raw.extend_from_slice(&chunk[..count]);
                    if let Some(position) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break position + 4;
                    }
                }
            }
        };
        let head = String::from_utf8_lossy(&raw[..end]).into_owned();
        let user_agent = head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("user-agent")
                .then(|| value.trim().to_string())
        });
        send.send(FetchSeen { user_agent }).unwrap();
        if delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
        let reason = if status == 200 { "OK" } else { "Error" };
        let head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        // A timed-out client may already be gone; never fail the mock.
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&body);
    });
    (format!("http://127.0.0.1:{port}/items"), receive, handle)
}

struct FetchSeen {
    user_agent: Option<String>,
}

fn registry() -> ToolRegistry {
    let mut registry = ToolRegistry::milestone_default();
    registry.register(Box::new(FetchTool::default()));
    registry
}

fn granted(url: &str) -> (ToolRegistry, PermissionPolicy) {
    // Grants cover hosts; ports ride along (the policy parser strips them).
    let authority = url.split("://").nth(1).unwrap().split('/').next().unwrap();
    let host = authority.split(':').next().unwrap().to_string();
    let mut policy = PermissionPolicy::milestone_default(std::env::temp_dir());
    policy.allow_network_domain(host);
    (registry(), policy)
}

fn fetch(registry: &ToolRegistry, policy: &PermissionPolicy, url: &str) -> Observation {
    registry.execute(&Action::FetchUrl { url: url.into() }, policy)
}

#[test]
fn allowed_fetch_returns_body_with_fixed_user_agent() {
    let (url, seen, handle) = serve(200, b"{\"items\":[]}".to_vec(), 0);
    let (registry, policy) = granted(&url);
    let observation = fetch(&registry, &policy, &url);
    assert!(
        observation.ok,
        "{}: {}",
        observation.summary, observation.data
    );
    assert_eq!(observation.data, "{\"items\":[]}");
    assert!(
        seen.recv_timeout(Duration::from_secs(5))
            .unwrap()
            .user_agent
            .as_deref()
            .unwrap_or("")
            .contains("klyne-harness"),
        "fixed User-Agent identifies the harness"
    );
    handle.join().unwrap();
}

#[test]
fn denial_happens_before_any_byte_moves() {
    let (registry, policy) = granted("http://127.0.0.1:9/nothing");
    // Unlisted host: denied with no server even running.
    let denied = fetch(&registry, &policy, "https://unlisted.example.com/items");
    assert!(!denied.ok);
    assert_eq!(denied.summary, "permission denied");
    // Default policy denies everything without touching the network.
    let bare = PermissionPolicy::milestone_default(std::env::temp_dir());
    let denied = ToolRegistry::milestone_default().execute(
        &Action::FetchUrl {
            url: "https://api.github.com/x".into(),
        },
        &bare,
    );
    assert!(!denied.ok);
    // Plain http outside loopback is rejected even when granted by name.
    let mut policy = PermissionPolicy::milestone_default(std::env::temp_dir());
    policy.allow_network_domain("example.com");
    let denied = registry.execute(
        &Action::FetchUrl {
            url: "http://example.com/items".into(),
        },
        &policy,
    );
    assert!(!denied.ok);
}

#[test]
fn oversized_and_non_utf8_bodies_rejected() {
    let (url, _seen, handle) = serve(200, vec![b'x'; 300 * 1024], 0);
    let (registry, policy) = granted(&url);
    let observation = fetch(&registry, &policy, &url);
    assert!(!observation.ok);
    assert!(
        observation.data.contains("exceeded"),
        "{}",
        observation.data
    );
    handle.join().unwrap();

    let (url, _seen, handle) = serve(200, vec![0xff, 0xfe, 0x41], 0);
    let (registry, policy) = granted(&url);
    let observation = fetch(&registry, &policy, &url);
    assert!(!observation.ok);
    assert!(observation.data.contains("UTF-8"), "{}", observation.data);
    handle.join().unwrap();
}

#[test]
fn error_status_and_timeout_fail_closed() {
    let (url, _seen, handle) = serve(500, b"boom".to_vec(), 0);
    let (registry, policy) = granted(&url);
    let observation = fetch(&registry, &policy, &url);
    assert!(!observation.ok);
    assert!(observation.data.contains("500"), "{}", observation.data);
    handle.join().unwrap();

    let (url, _seen, handle) = serve(200, b"slow".to_vec(), 10_000);
    let mut limited = ToolRegistry::milestone_default();
    limited.register(Box::new(
        FetchTool::new(FetchLimits {
            timeout: Duration::from_secs(1),
            max_bytes: 64 * 1024,
        })
        .unwrap(),
    ));
    let (_, policy) = granted(&url);
    let start = std::time::Instant::now();
    let observation = fetch(&limited, &policy, &url);
    assert!(start.elapsed() < Duration::from_secs(8));
    assert!(
        !observation.ok,
        "{}: {}",
        observation.summary, observation.data
    );
    handle.join().unwrap();
}

#[test]
fn radar_url_digest_helpers() {
    assert_eq!(
        github_search_url("created:>2026-08-10", 5),
        "https://api.github.com/search/repositories?q=created:>2026-08-10&sort=stars&order=desc&per_page=5"
    );
    assert!(github_search_url("q", 0).ends_with("per_page=1"));
    assert!(github_search_url("q", 200).ends_with("per_page=100"));
    let payload = serde_json::json!({
        "total_count": 2,
        "items": [
            {"full_name": "a/one", "stargazers_count": 10, "language": "Rust",
             "description": "first", "html_url": "https://github.com/a/one"},
            {"full_name": "b/two", "stargazers_count": 5, "language": None::<String>,
             "description": None::<String>, "html_url": "https://github.com/b/two"},
        ],
    })
    .to_string();
    let digest = render_digest(&payload, 10).unwrap();
    assert!(digest.contains("untrusted network data"));
    assert!(digest.contains("1. a/one ★10 (Rust) — first"));
    assert!(digest.contains("https://github.com/b/two"));
    assert!(render_digest("not json", 10).is_err());
    assert!(render_digest(r#"{"total_count":0}"#, 10).is_err());
}

#[test]
fn radar_end_to_end_through_mock_api() {
    let payload = serde_json::json!({
        "total_count": 1,
        "items": [
            {"full_name": "mock/app", "stargazers_count": 42, "language": "Rust",
             "description": "app of the day", "html_url": "https://github.com/mock/app"},
        ],
    })
    .to_string();
    let (url, _seen, handle) = serve(200, payload.into_bytes(), 0);
    let (registry, policy) = granted(&url);
    let body = fetch_text(&registry, &policy, &url).unwrap();
    let digest = render_digest(&body, 5).unwrap();
    assert!(digest.contains("mock/app ★42"));
    handle.join().unwrap();
}
