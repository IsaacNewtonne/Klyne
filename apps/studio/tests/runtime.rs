use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Supervisor {
    child: Child,
    root: tempfile::TempDir,
    port: u16,
}
impl Supervisor {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let child = Command::new(env!("CARGO_BIN_EXE_klyne-supervisor"))
            .args(["--port", &port.to_string(), "--root"])
            .arg(root.path())
            .args(["--binary", env!("CARGO_BIN_EXE_klyne-studio")])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let app = Self { child, root, port };
        app.wait(|| app.health().is_some());
        app
    }
    fn health(&self) -> Option<Value> {
        reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(500))
            .build()
            .unwrap()
            .get(format!("http://127.0.0.1:{}/api/runtime/ready", self.port))
            .send()
            .ok()?
            .json()
            .ok()
    }
    fn wait(&self, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !condition() {
            assert!(Instant::now() < deadline, "Runtime did not become ready");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    fn stage(&self, binary: &std::path::Path, hash: Option<&str>) {
        let digest = format!("{:x}", Sha256::digest(fs::read(binary).unwrap()));
        fs::write(
            self.root.path().join("runtime/pending.json"),
            json!({"binary":binary,"sha256":hash.unwrap_or(&digest)}).to_string(),
        )
        .unwrap();
    }
    fn result(&self) -> Option<Value> {
        serde_json::from_slice(&fs::read(self.root.path().join("runtime/last-result.json")).ok()?)
            .ok()
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::thread::sleep(Duration::from_millis(150));
    }
}
fn attest(app: &Supervisor, binary: &std::path::Path) {
    let digest = format!("{:x}", Sha256::digest(fs::read(binary).unwrap()));
    let command = json!([
        std::env::current_exe().unwrap(),
        "--ignored",
        "--exact",
        "attestation_test_fixture"
    ]);
    let result = Command::new(env!("CARGO_BIN_EXE_klyne-supervisor"))
        .current_dir(app.root.path())
        .arg("--root")
        .arg(app.root.path())
        .arg("--binary")
        .arg(binary)
        .args([
            "--attest-digest",
            &digest,
            "--attest-command",
            &command.to_string(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
#[ignore = "subprocess fixture"]
fn attestation_test_fixture() {
    fs::write("attestation-fixture-ran.txt", "fixture executed").unwrap();
}

#[cfg(windows)]
#[test]
fn supervisor_restarts_crashes_and_stops_a_crash_loop() {
    let mut app = Supervisor::new();
    for attempt in 1..=4 {
        let pid = app.health().unwrap()["pid"].as_u64().unwrap();
        let result = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert!(result.status.success());
        if attempt <= 3 {
            app.wait(|| app.health().is_some_and(|v| v["pid"].as_u64() != Some(pid)));
            assert!(app.child.try_wait().unwrap().is_none());
        } else {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.child.try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline, "crash loop must stop");
                std::thread::sleep(Duration::from_millis(30));
            }
        }
    }
    let state: Value = serde_json::from_slice(
        &fs::read(app.root.path().join("runtime/restart-status.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["exhausted"], true);
    assert_eq!(state["attempt"], 4);
}

#[test]
fn supervisor_rejects_bad_digest_activates_and_rolls_back_failed_startup() {
    let app = Supervisor::new();
    let original = app.health().unwrap()["pid"].clone();
    let binary = std::path::Path::new(env!("CARGO_BIN_EXE_klyne-studio"));
    app.stage(binary, Some("wrong"));
    app.wait(|| app.result().is_some_and(|v| v["rejected"].is_string()));
    assert_eq!(app.health().unwrap()["pid"], original);
    app.wait(|| !app.root.path().join("runtime/pending.json").exists());
    app.stage(binary, None);
    // Identity and protocol alone do not qualify: activation requires an
    // independently produced test attestation for the exact digest.
    app.wait(|| {
        app.result().is_some_and(|v| {
            v["rejected"]
                .as_str()
                .is_some_and(|reason| reason.contains("attestation"))
        })
    });
    app.wait(|| !app.root.path().join("runtime/pending.json").exists());
    attest(&app, binary);
    app.stage(binary, None);
    app.wait(|| app.result().is_some_and(|v| v["rollback"] == false));
    let activated = app.health().unwrap()["pid"].clone();
    assert_ne!(activated, original);
    app.wait(|| !app.root.path().join("runtime/pending.json").exists());
    let source = app.root.path().join("fails.rs");
    let candidate = app
        .root
        .path()
        .join(format!("fails{}", std::env::consts::EXE_SUFFIX));
    fs::write(&source,r#"fn main(){if std::env::args().any(|a|a=="--runtime-check"){println!("{{\"klyne_runtime_protocol\":1}}");}else{std::process::exit(17);}}"#).unwrap();
    let output = Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&candidate)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Every candidate needs its own attestation, including the failing one:
    // the gate binds attestation per digest, not per test.
    attest(&app, &candidate);
    app.stage(&candidate, None);
    app.wait(|| app.result().is_some_and(|v| v["rollback"] == true));
    app.wait(|| app.health().is_some());
    assert_ne!(app.health().unwrap()["pid"], activated);
}
