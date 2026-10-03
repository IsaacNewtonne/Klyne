//! Provider settings regression: both Studio views preserve per-provider choices.
use harness_browser::{BrowserLimits, ControlledBrowser};
use serde_json::{Value, json};
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait(b: &mut ControlledBrowser, expression: &str) {
    let start = Instant::now();
    while b.eval(expression).unwrap() != true {
        assert!(start.elapsed() < Duration::from_secs(20), "{expression}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn provider_choice_survives_reload_and_toggle() {
    let root = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_klyne-studio"))
            .args(["--port", &port.to_string(), "--root"])
            .arg(root.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let origin = format!("http://127.0.0.1:{port}");
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let start = Instant::now();
    loop {
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "Fixture server exited"
        );
        if let Ok(response) = client.get(format!("{origin}/api/runtime/ready")).send()
            && response
                .json::<Value>()
                .ok()
                .is_some_and(|v| v["pid"] == server.0.id())
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Fixture server did not start"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
    let mut b = ControlledBrowser::launch_isolated(BrowserLimits::default()).unwrap();
    for view in ["/", "/files"] {
        let url = format!("{origin}{view}");
        b.navigate(&url).unwrap();
        wait(&mut b, "typeof configureProvider==='function'");
        // Seed the old single-provider storage format to cover migration too.
        b.eval("localStorage.clear();localStorage.setItem('klyne-provider',JSON.stringify({kind:'ollama',endpoint:'http://127.0.0.1:11435',model:'persist-model'}))").unwrap();
        b.navigate(&url).unwrap();
        wait(
            &mut b,
            "document.querySelector('#provider-model').value==='persist-model'",
        );
        assert_eq!(
            b.eval("providerConfig()").unwrap(),
            json!({"kind":"ollama","endpoint":"http://127.0.0.1:11435","model":"persist-model"}),
            "legacy reload in {view}"
        );
        b.eval("(()=>{const kind=document.querySelector('#provider-kind');kind.value='opencode';kind.dispatchEvent(new Event('change'));const endpoint=document.querySelector('#provider-endpoint');endpoint.value='http://127.0.0.1:4097';endpoint.dispatchEvent(new Event('input'));const model=document.querySelector('#provider-model');model.value='provider/second-model';model.dispatchEvent(new Event('input'));kind.value='ollama';kind.dispatchEvent(new Event('change'));})()").unwrap();
        assert_eq!(
            b.eval("providerConfig()").unwrap(),
            json!({"kind":"ollama","endpoint":"http://127.0.0.1:11435","model":"persist-model"}),
            "toggle in {view}"
        );
        b.navigate(&url).unwrap();
        wait(
            &mut b,
            "document.querySelector('#provider-model').value==='persist-model'",
        );
        b.eval("(()=>{const kind=document.querySelector('#provider-kind');kind.value='opencode';kind.dispatchEvent(new Event('change'));})()").unwrap();
        assert_eq!(
            b.eval("providerConfig()").unwrap(),
            json!({"kind":"opencode","endpoint":"http://127.0.0.1:4097","model":"provider/second-model"}),
            "other provider retained in {view}"
        );
        b.navigate(&url).unwrap();
        wait(
            &mut b,
            "document.querySelector('#provider-kind').value==='opencode' && document.querySelector('#provider-model').value==='provider/second-model'",
        );
    }
}
