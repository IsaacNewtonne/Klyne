//! Controlled tool benchmark. Runtime integration is covered separately in tests.
use harness_core::{Action, PermissionPolicy, ToolRegistry};
use std::{fs, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let mut expected = vec![b'x'; 8 * 1024 * 1024];
    expected[1024..1027].copy_from_slice(b"old");
    fs::write(root.path().join("data"), &expected)?;
    let tools = ToolRegistry::milestone_default();
    let policy = PermissionPolicy::milestone_default(root.path());
    let call = |action| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let result = tools.execute(&action, &policy);
        if !result.ok {
            return Err(result.data.into());
        }
        Ok(serde_json::from_str(&result.data)?)
    };
    let started = Instant::now();
    let hash = call(Action::HashFile {
        path: "data".into(),
    })?;
    let patched = call(Action::PatchFile {
        path: "data".into(),
        offset: 1024,
        expected: "old".into(),
        replacement: "new".into(),
        expected_sha256: hash["sha256"].as_str().ok_or("missing digest")?.into(),
    })?;
    let range = call(Action::ReadFileRange {
        path: "data".into(),
        offset: 1024,
        length: 3,
    })?;
    let tool_seconds = started.elapsed().as_secs_f64();
    expected[1024..1027].copy_from_slice(b"new");
    if fs::read(root.path().join("data"))? != expected || range["text"] != "new" {
        return Err("independent byte verification failed".into());
    }
    println!(
        "{}",
        serde_json::json!({"suite":"large-files-v1", "file_bytes":expected.len(), "tool_calls":3, "tool_seconds":tool_seconds, "independently_verified":true, "before_sha256":hash["sha256"], "after_sha256":patched["after_sha256"]})
    );
    Ok(())
}
