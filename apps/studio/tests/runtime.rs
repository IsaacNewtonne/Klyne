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
    app.stage(&candidate, None);
    app.wait(|| app.result().is_some_and(|v| v["rollback"] == true));
    app.wait(|| app.health().is_some());
    assert_ne!(app.health().unwrap()["pid"], activated);
}
