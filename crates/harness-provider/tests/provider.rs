use harness_core::event_store::EventStore;
use harness_core::verification::SuccessCriterion;
use harness_core::{
    Action, AgentRuntime, Model, Objective, PermissionPolicy, RunOutcome, SqliteEventStore,
    StepDecision, ToolRegistry,
};
use harness_provider::{OpenAiCompat, ProviderConfig, ProviderLimits};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct MockResponse {
    status: u16,
    body: String,
    delay_ms: u64,
}

struct SeenRequest {
    auth: Option<String>,
    body: String,
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        401 => "Unauthorized",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> SeenRequest {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).unwrap();
        raw.extend_from_slice(&chunk[..count]);
        if let Some(position) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&raw[..header_end]).into_owned();
    let auth = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("authorization")
            .then(|| value.trim().to_string())
    });
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0);
    while raw.len() < header_end + length {
        let count = stream.read(&mut chunk).unwrap();
        raw.extend_from_slice(&chunk[..count]);
    }
    SeenRequest {
        auth,
        body: String::from_utf8_lossy(&raw[header_end..header_end + length]).into_owned(),
    }
}

/// Serve queued responses on 127.0.0.1; returns the chat-completions URL and
/// the requests the mock observed.
fn serve(
    responses: Vec<MockResponse>,
) -> (
    String,
    mpsc::Receiver<SeenRequest>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/v1/chat/completions",
        listener.local_addr().unwrap().port()
    );
    let (send, receive) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            send.send(read_request(&mut stream)).unwrap();
            if response.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(response.delay_ms));
            }
            let reply = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.status,
                reason(response.status),
                response.body.len(),
                response.body
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    (url, receive, handle)
}

fn ok(body: String) -> MockResponse {
    MockResponse {
        status: 200,
        body,
        delay_ms: 0,
    }
}

fn envelope(decision: serde_json::Value) -> String {
    serde_json::json!({
        "choices": [{"message": {"content": decision.to_string()}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5},
    })
    .to_string()
}

fn act(tool: &str, fields: serde_json::Value) -> serde_json::Value {
    let mut action = serde_json::json!({"tool": tool});
    for (key, value) in fields.as_object().unwrap() {
        action[key] = value.clone();
    }
    serde_json::json!({"decision": "act", "action": action})
}

fn provider(url: &str, mutate: impl Fn(ProviderLimits) -> ProviderLimits) -> OpenAiCompat {
    OpenAiCompat::new(
        ProviderConfig::new(url, "mock-model").with_limits(mutate(ProviderLimits::default())),
    )
    .unwrap()
}

fn decide_once(
    url: &str,
    mutate: impl Fn(ProviderLimits) -> ProviderLimits,
) -> (StepDecision, harness_provider::Usage) {
    let mut model = provider(url, mutate);
    let decision = model.decide(&Objective::new("test objective"), &[]);
    let usage = model.telemetry();
    (decision, usage)
}

#[test]
fn usage_reports_tokens_and_priced_cost() {
    let body = serde_json::json!({
        "choices": [{"message": {"content": serde_json::json!({"decision": "fail", "reason": "done"}).to_string()}}],
        "usage": {"prompt_tokens": 2000, "completion_tokens": 1000},
    })
    .to_string();
    let (url, _seen, handle) = serve(vec![MockResponse {
        status: 200,
        body,
        delay_ms: 0,
    }]);
    let mut model = OpenAiCompat::new(ProviderConfig::new(&url, "mock-model").with_pricing(
        harness_provider::Pricing {
            usd_per_1k_prompt_tokens: 0.01,
            usd_per_1k_completion_tokens: 0.03,
        },
    ))
    .unwrap();
    let (decision, _) = {
        let decision = model.decide(&Objective::new("objective"), &[]);
        let usage = model.usage();
        (decision, usage)
    };
    assert!(matches!(decision, StepDecision::Fail(_)));
    let usage = model.usage();
    assert_eq!(usage.prompt_tokens, 2000);
    assert_eq!(usage.completion_tokens, 1000);
    assert!((usage.cost_usd - 0.05).abs() < 1e-9);
    handle.join().unwrap();
    // Without pricing, tokens accrue and cost stays zero.
    let (url, _seen, handle) = serve(vec![MockResponse {
        status: 200,
        body: serde_json::json!({
            "choices": [{"message": {"content": serde_json::json!({"decision": "fail", "reason": "done"}).to_string()}}],
            "usage": {"prompt_tokens": 7, "completion_tokens": 8},
        })
        .to_string(),
        delay_ms: 0,
    }]);
    let mut plain = OpenAiCompat::new(ProviderConfig::new(&url, "mock-model")).unwrap();
    plain.decide(&Objective::new("objective"), &[]);
    assert_eq!(plain.usage().prompt_tokens, 7);
    assert_eq!(plain.usage().cost_usd, 0.0);
    handle.join().unwrap();
}

#[test]
fn config_rejects_bad_endpoints_and_models() {
    assert!(OpenAiCompat::new(ProviderConfig::new("ftp://host/v1", "m")).is_err());
    assert!(OpenAiCompat::new(ProviderConfig::new("http://host/v1", "")).is_err());
    assert!(OpenAiCompat::new(ProviderConfig::new("not a url", "m")).is_err());
    assert!(OpenAiCompat::new(ProviderConfig::new("http://127.0.0.1:9/v1", "m")).is_ok());
}

#[test]
fn unknown_tool_and_malformed_payloads_become_fail() {
    for content in [
        serde_json::json!({"decision": "act", "action": {"tool": "format_disk"}}),
        serde_json::json!({"decision": "act", "action": {"tool": "read_file"}}),
        serde_json::json!({"decision": "act"}),
        serde_json::json!({"decision": "launch"}),
        serde_json::json!({"decision": "complete"}),
        serde_json::json!({"unrelated": true}),
    ] {
        let (url, _seen, handle) = serve(vec![MockResponse {
            status: 200,
            body: envelope(content),
            delay_ms: 0,
        }]);
        let (decision, _) = decide_once(&url, |limits| limits);
        assert!(matches!(decision, StepDecision::Fail(_)), "{decision:?}");
        handle.join().unwrap();
    }
    let (url, _seen, handle) = serve(vec![MockResponse {
        status: 200,
        body: "this is not json".into(),
        delay_ms: 0,
    }]);
    let (decision, _) = decide_once(&url, |limits| limits);
    assert!(matches!(decision, StepDecision::Fail(_)));
    handle.join().unwrap();
}

#[test]
fn retryable_status_recovers_and_counts_usage() {
    let valid = envelope(act("read_file", serde_json::json!({"path": "a.txt"})));
    let (url, _seen, handle) = serve(vec![
        MockResponse {
            status: 500,
            body: "{}".into(),
            delay_ms: 0,
        },
        MockResponse {
            status: 200,
            body: valid,
            delay_ms: 0,
        },
    ]);
    let (decision, usage) = decide_once(&url, |limits| limits);
    assert!(
        matches!(decision, StepDecision::Act(Action::ReadFile { .. })),
        "{decision:?}"
    );
    assert_eq!(usage.http_calls, 2);
    assert_eq!(usage.retries, 1);
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    handle.join().unwrap();
}

#[test]
fn persistent_failure_fails_after_bounded_retries() {
    let (url, _seen, handle) = serve(vec![
        MockResponse {
            status: 500,
            body: "{}".into(),
            delay_ms: 0,
        },
        MockResponse {
            status: 503,
            body: "{}".into(),
            delay_ms: 0,
        },
        MockResponse {
            status: 500,
            body: "{}".into(),
            delay_ms: 0,
        },
    ]);
    let (decision, usage) = decide_once(&url, |mut limits| {
        limits.max_retries = 2;
        limits
    });
    assert!(matches!(decision, StepDecision::Fail(_)));
    assert_eq!(usage.http_calls, 3);
    assert_eq!(usage.retries, 2);
    handle.join().unwrap();
}

#[test]
fn client_error_does_not_retry() {
    let (url, _seen, handle) = serve(vec![MockResponse {
        status: 401,
        body: "{}".into(),
        delay_ms: 0,
    }]);
    let (decision, usage) = decide_once(&url, |limits| limits);
    assert!(matches!(decision, StepDecision::Fail(_)));
    assert_eq!(usage.http_calls, 1);
    assert_eq!(usage.retries, 0);
    handle.join().unwrap();
}

#[test]
fn oversized_response_is_rejected() {
    let (url, _seen, handle) = serve(vec![MockResponse {
        status: 200,
        body: "x".repeat(300 * 1024),
        delay_ms: 0,
    }]);
    let (decision, _) = decide_once(&url, |mut limits| {
        limits.max_response_bytes = 64 * 1024;
        limits
    });
    assert!(
        matches!(&decision, StepDecision::Fail(reason) if reason.contains("exceeded")),
        "{decision:?}"
    );
    handle.join().unwrap();
}

#[test]
fn timeout_fails_fast_without_hanging() {
    let valid = envelope(act("read_file", serde_json::json!({"path": "a.txt"})));
    let (url, _seen, handle) = serve(vec![MockResponse {
        status: 200,
        body: valid,
        delay_ms: 10_000,
    }]);
    let start = std::time::Instant::now();
    let (decision, usage) = decide_once(&url, |mut limits| {
        limits.timeout_per_attempt = Duration::from_secs(1);
        limits.max_retries = 0;
        limits
    });
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "must not wait out the server"
    );
    assert!(matches!(decision, StepDecision::Fail(_)), "{decision:?}");
    assert_eq!(usage.http_calls, 1);
    handle.join().unwrap();
}

const REPO: &str = "// service configuration\nfn answer() -> i32 {\n    return 41;\n}\n";

#[test]
fn real_model_repairs_controlled_repo_through_runtime_boundaries() {
    unsafe { std::env::set_var("HARNESS_PROVIDER_TEST_KEY_XYZ", "super-secret-xyz") };
    let root = std::env::temp_dir().join(format!(
        "harness-provider-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("service.rs"), REPO).unwrap();
    let db = root.join("run.sqlite3");
    let offset = REPO.find("41").unwrap() as u64;
    let policy = PermissionPolicy::milestone_default(&root);
    let digest = serde_json::from_str::<serde_json::Value>(
        &ToolRegistry::milestone_default()
            .execute(
                &Action::HashFile {
                    path: "service.rs".into(),
                },
                &policy,
            )
            .data,
    )
    .unwrap()["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    let (url, seen, handle) = serve(vec![
        ok(envelope(act(
            "search_file",
            serde_json::json!({"path": "service.rs", "needle": "41", "max_matches": 10}),
        ))),
        ok(envelope(
            serde_json::json!({"decision": "act", "action": {"tool": "read_range", "path": "service.rs", "offset": offset, "length": 2}}),
        )),
        ok(envelope(act(
            "hash_file",
            serde_json::json!({"path": "service.rs"}),
        ))),
        ok(envelope(
            serde_json::json!({"decision": "act", "action": {"tool": "patch_file", "path": "service.rs", "offset": offset, "expected": "41", "replacement": "42", "expected_sha256": digest}}),
        )),
        ok(envelope(
            serde_json::json!({"decision": "complete", "summary": "repaired"}),
        )),
    ]);
    let config =
        ProviderConfig::new(&url, "mock-model").with_api_key_env("HARNESS_PROVIDER_TEST_KEY_XYZ");
    let model = OpenAiCompat::new(config).unwrap();
    let mut runtime = AgentRuntime::new(
        model,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&root),
        SqliteEventStore::open(&db).unwrap(),
    );
    let outcome = runtime
        .run_with_criterion(
            Objective::new("repair service.rs: replace 41 with 42"),
            Some(SuccessCriterion::FileRange {
                path: "service.rs".into(),
                offset,
                expected: "42".into(),
            }),
        )
        .unwrap();
    assert!(matches!(outcome, RunOutcome::Completed(_)), "{outcome:?}");
    assert!(
        fs::read_to_string(root.join("service.rs"))
            .unwrap()
            .contains("return 42;")
    );
    drop(runtime);
    handle.join().unwrap();
    // Five model calls (four actions plus completion); the runtime-owned
    // verification needs no model call. The secret only left in headers.
    let requests: Vec<SeenRequest> = seen.try_iter().collect();
    assert_eq!(requests.len(), 5);
    assert!(
        requests
            .iter()
            .all(|request| request.auth.as_deref() == Some("Bearer super-secret-xyz"))
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.body.contains("super-secret-xyz"))
    );
    let mut store = SqliteEventStore::open(&db).unwrap();
    for event in store.events().unwrap() {
        assert!(!event.detail.contains("super-secret-xyz"), "{}", event.kind);
    }
    let snapshot = serde_json::to_string(&store.load().unwrap().unwrap()).unwrap();
    assert!(!snapshot.contains("super-secret-xyz"));
    drop(store);
    unsafe { std::env::remove_var("HARNESS_PROVIDER_TEST_KEY_XYZ") };
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn provider_model_completes_file_objective_end_to_end() {
    unsafe { std::env::set_var("HARNESS_PROVIDER_E2E_KEY_XYZ", "e2e-secret-xyz") };
    let root = std::env::temp_dir().join(format!(
        "harness-provider-e2e-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let db = root.join("run.sqlite3");
    let (url, seen, handle) = serve(vec![
        ok(envelope(act(
            "write_file",
            serde_json::json!({"path": "result.txt", "contents": "correct"}),
        ))),
        ok(envelope(act(
            "read_file",
            serde_json::json!({"path": "result.txt"}),
        ))),
        ok(envelope(
            serde_json::json!({"decision": "complete", "summary": "done"}),
        )),
    ]);
    let config =
        ProviderConfig::new(&url, "mock-model").with_api_key_env("HARNESS_PROVIDER_E2E_KEY_XYZ");
    let model = OpenAiCompat::new(config).unwrap();
    let mut runtime = AgentRuntime::new(
        model,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&root),
        SqliteEventStore::open(&db).unwrap(),
    );
    let outcome = runtime
        .run(Objective::new(
            "create file result.txt with content correct",
        ))
        .unwrap();
    assert!(matches!(outcome, RunOutcome::Completed(_)), "{outcome:?}");
    assert_eq!(
        fs::read_to_string(root.join("result.txt")).unwrap(),
        "correct"
    );
    drop(runtime);
    handle.join().unwrap();
    // The secret traveled only in the Authorization header, never in
    // persisted task data.
    let requests: Vec<SeenRequest> = seen.try_iter().collect();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| request.auth.as_deref() == Some("Bearer e2e-secret-xyz")),
        "every call authenticates"
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.body.contains("e2e-secret-xyz")),
        "prompts never carry the credential"
    );
    let mut store = SqliteEventStore::open(&db).unwrap();
    for event in store.events().unwrap() {
        assert!(!event.detail.contains("e2e-secret-xyz"), "{}", event.kind);
    }
    let state = store.load().unwrap().unwrap();
    let snapshot = serde_json::to_string(&state).unwrap();
    assert!(!snapshot.contains("e2e-secret-xyz"));
    drop(store);
    unsafe { std::env::remove_var("HARNESS_PROVIDER_E2E_KEY_XYZ") };
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn provider_invalid_action_fails_without_side_effects() {
    let root = std::env::temp_dir().join(format!(
        "harness-provider-bad-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let db = root.join("run.sqlite3");
    let (url, _seen, handle) = serve(vec![MockResponse {
        status: 200,
        body: envelope(serde_json::json!({
            "decision": "act",
            "action": {"tool": "shell_exec", "program": "rm", "args": ["-rf", "/"]},
        })),
        delay_ms: 0,
    }]);
    let model = OpenAiCompat::new(ProviderConfig::new(&url, "mock-model")).unwrap();
    let mut runtime = AgentRuntime::new(
        model,
        ToolRegistry::milestone_default(),
        PermissionPolicy::milestone_default(&root),
        SqliteEventStore::open(&db).unwrap(),
    );
    let outcome = runtime
        .run(Objective::new(
            "create file result.txt with content correct",
        ))
        .unwrap();
    assert!(matches!(outcome, RunOutcome::Failed(_)), "{outcome:?}");
    assert!(!root.join("result.txt").exists());
    drop(runtime);
    let events = SqliteEventStore::open(&db).unwrap().events().unwrap();
    assert!(!events.iter().any(|e| e.kind == "ToolCalled"));
    handle.join().unwrap();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn unicode_history_truncates_at_char_boundary_without_panicking() {
    use harness_core::{Objective, Observation};
    use harness_provider::truncate_to_char_boundary as truncate_helper;

    // Direct helper coverage: Vietnamese, emoji, and mixed scripts at every
    // awkward byte offset must stay valid UTF-8 and never panic.
    for source in [
        "Tiếng Việt có dấu ễ ộ ữ — test ".repeat(8),
        "😀🎉🧪🚀✨ mixed ".repeat(8),
        "hello Tiếng Việt 😀 — mixed 123 ".repeat(8),
    ] {
        for limit in [1usize, 2, 3, 5, 7, 10, 25] {
            let mut data = source.clone();
            truncate_helper(&mut data, limit);
            assert!(data.len() <= limit, "limit {limit}");
            assert!(std::str::from_utf8(data.as_bytes()).is_ok());
        }
    }

    // End-to-end: oversized multibyte observations travel through
    // render_history into the mock provider request without panicking, arrive
    // as valid UTF-8, and carry the truncation marker.
    let unicode_observation = "Tiếng Việt 😀🎉 — ".repeat(200);
    assert!(unicode_observation.len() > 64);
    let history = vec![(
        Action::ReadFile {
            path: "a.txt".into(),
        },
        Observation {
            ok: true,
            summary: "ok".into(),
            data: unicode_observation,
        },
    )];
    let (url, seen, handle) = serve(vec![ok(envelope(
        serde_json::json!({"decision": "fail", "reason": "done"}),
    ))]);
    let mut model = OpenAiCompat::new(ProviderConfig::new(&url, "mock-model").with_limits(
        ProviderLimits {
            max_observation_chars: 25,
            ..ProviderLimits::default()
        },
    ))
    .unwrap();
    let decision = model.decide(&Objective::new("unicode"), &history);
    assert!(matches!(decision, StepDecision::Fail(_)), "{decision:?}");
    let request = seen.try_iter().next().expect("mock saw one request");
    assert!(
        std::str::from_utf8(request.body.as_bytes()).is_ok(),
        "request must stay valid UTF-8"
    );
    assert!(
        request.body.contains("[truncated]"),
        "oversized observation must be marked"
    );
    // No half-cut emoji may leak into the transmitted history.
    assert!(!request.body.contains('\u{FFFD}'));
    handle.join().unwrap();
}
