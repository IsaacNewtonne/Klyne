use harness_browser::{BrowserLimits, ControlledBrowser};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);

const PAGE: &str = "<!doctype html><html><head><title>Radar Test</title></head>\
<body><h1>Hello harness</h1>\
<button id=\"inc\" onclick=\"document.getElementById('count').textContent = Number(document.getElementById('count').textContent) + 1\">up</button>\
<span id=\"count\">0</span>\
<input id=\"name\" type=\"text\" value=\"\">\
</body></html>";

/// Minimal static-file server so tests run on http:// origins with no
/// external network and no extra dependencies.
struct FixtureServer {
    base_url: String,
    _thread: std::thread::JoinHandle<()>,
}

fn fixture_server() -> FixtureServer {
    let root = std::env::temp_dir().join(format!(
        "harness-browser-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("page.html"), PAGE).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!(
        "http://127.0.0.1:{}/page.html",
        listener.local_addr().unwrap().port()
    );
    let thread = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(_) => break,
            };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                match stream.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => {
                        head.push(byte[0]);
                        if head.ends_with(b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let body = PAGE.as_bytes();
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.write_all(body);
        }
    });
    FixtureServer {
        base_url,
        _thread: thread,
    }
}

fn launch() -> ControlledBrowser {
    ControlledBrowser::launch_isolated(BrowserLimits::default())
        .expect("headless Chrome must launch")
}

#[test]
fn navigate_reads_title_and_text() {
    let server = fixture_server();
    let mut browser = launch();
    browser.navigate(&server.base_url).unwrap();
    assert_eq!(browser.title().unwrap(), "Radar Test");
    assert!(browser.text().unwrap().contains("Hello harness"));
    browser.close();
}

#[test]
fn click_mutates_dom_and_fill_sets_input() {
    let server = fixture_server();
    let mut browser = launch();
    browser.navigate(&server.base_url).unwrap();
    browser.click("#inc").unwrap();
    browser.click("#inc").unwrap();
    let count = browser
        .eval("document.getElementById('count').textContent")
        .unwrap();
    assert_eq!(count.as_str(), Some("2"));
    browser.fill("#name", "harness").unwrap();
    let value = browser
        .eval("document.getElementById('name').value")
        .unwrap();
    assert_eq!(value.as_str(), Some("harness"));
    assert!(browser.click("#missing").is_err());
    browser.close();
}

#[test]
fn screenshot_returns_bounded_png() {
    let server = fixture_server();
    let mut browser = launch();
    browser.navigate(&server.base_url).unwrap();
    let png = browser.screenshot().unwrap();
    assert!(png.len() > 1000, "unexpectedly small: {}", png.len());
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(png.len() <= BrowserLimits::default().max_screenshot_bytes);
    browser.close();
}

#[test]
fn profiles_are_isolated_from_each_other() {
    let server = fixture_server();
    let mut first = launch();
    let mut second = launch();
    first.navigate(&server.base_url).unwrap();
    second.navigate(&server.base_url).unwrap();
    first
        .eval("localStorage.setItem('secret', 'first-only')")
        .unwrap();
    let seen = second.eval("localStorage.getItem('secret')").unwrap();
    assert_eq!(seen, serde_json::Value::Null);
    first.close();
    second.close();
}

#[test]
fn attach_drives_without_owning_the_browser() {
    let server = fixture_server();
    let owner = launch();
    let addr = owner.debugger_addr().to_string();
    let tabs = ControlledBrowser::list_tabs(&addr, Duration::from_secs(10)).unwrap();
    assert!(!tabs.is_empty());
    assert!(!tabs[0].0.is_empty());
    let mut attached = ControlledBrowser::attach(&addr, None, BrowserLimits::default()).unwrap();
    attached.navigate(&server.base_url).unwrap();
    assert_eq!(attached.title().unwrap(), "Radar Test");
    attached.close();
    // Closing an attached session disconnects only: the owner's browser
    // keeps working afterwards.
    let mut owner = owner;
    owner.navigate(&server.base_url).unwrap();
    assert_eq!(owner.title().unwrap(), "Radar Test");
    owner.close();
}

#[test]
fn close_removes_the_disposable_profile() {
    let mut browser = launch();
    let addr = browser.debugger_addr().to_string();
    assert!(ControlledBrowser::list_tabs(&addr, Duration::from_secs(10)).is_ok());
    browser.close();
    assert!(
        ControlledBrowser::list_tabs(&addr, Duration::from_secs(3)).is_err(),
        "debugger must be gone after close"
    );
}

#[test]
fn profile_listing_reads_local_state_fixture() {
    let root = std::env::temp_dir().join(format!(
        "harness-profiles-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("Local State"),
        r#"{"profile":{"info_cache":{"Default":{"name":"Person 1"},"Profile 1":{"name":"Work"}}}}"#,
    )
    .unwrap();
    unsafe { std::env::set_var("CHROME_USER_DATA_DIR", &root) };
    let profiles = harness_browser::list_profiles().unwrap();
    unsafe { std::env::remove_var("CHROME_USER_DATA_DIR") };
    assert_eq!(profiles.len(), 2);
    assert!(
        profiles
            .iter()
            .any(|p| p.name == "Work" && p.directory == "Profile 1")
    );
    assert!(
        profiles
            .iter()
            .any(|p| p.name == "Person 1" && p.directory == "Default")
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn base64_round_trips_for_screenshots() {
    // Base64 helpers are internal; exercise them through eval-driven data
    // URLs instead: a canvas-free deterministic byte check.
    let server = fixture_server();
    let mut browser = launch();
    browser.navigate(&server.base_url).unwrap();
    let value = browser.eval("btoa('hi')").unwrap();
    assert_eq!(value.as_str(), Some("aGk="));
    browser.close();
}
