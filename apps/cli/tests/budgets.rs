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
            "harness-cli-budget-{}-{}",
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
fn cli_zero_budget_exits_unsuccessfully_without_writing() {
    let w = Workspace::new();
    let output = w.run(&[
        "--objective",
        "create file result.txt with content expected",
        "--max-tool-calls",
        "0",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("budget exhausted"));
    assert!(!w.0.join("result.txt").exists());
}

#[test]
fn cli_exact_budget_completes_and_resume_override_is_rejected() {
    let w = Workspace::new();
    let output = w.run(&[
        "--objective",
        "create file result.txt with content expected",
        "--max-tool-calls",
        "3",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(w.0.join("result.txt")).unwrap(),
        "expected"
    );
    assert!(
        !w.run(&["--resume", "--max-tool-calls", "100"])
            .status
            .success()
    );
    assert!(w.run(&["--resume"]).status.success());
}
