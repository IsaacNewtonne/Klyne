use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "harness-cli-inspect-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_harness-cli"))
            .arg("--workspace")
            .arg(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn inspect_reports_completed_run_as_json() {
    let w = Workspace::new();
    let completed = w.run(&[
        "--objective",
        "create file result.txt with content expected",
    ]);
    assert!(completed.status.success());
    let output = w.run(&["--inspect"]);
    assert!(output.status.success());
    let report: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(report["history"], serde_json::json!(3));
    assert_eq!(report["tools"]["used"], serde_json::json!(3));
    assert!(report["outcome"].get("Completed").is_some());
    assert_eq!(report["kinds"]["CognitiveStep"], serde_json::json!(3));
    assert_eq!(
        report["objective"]["text"],
        serde_json::json!("create file result.txt with content expected")
    );
}
