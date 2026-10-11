//! Bounded document ingestion; PDFs are parsed in a cancellable child process.
use crate::err;
use harness_core::{Action, PermissionDecision, PermissionPolicy};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
const FILE_CAP: u64 = 4 * 1024 * 1024;
const TEXT_CAP: usize = 60_000;
fn bounded(mut text: String) -> (String, bool) {
    let truncated = text.len() > TEXT_CAP;
    if truncated {
        let mut end = TEXT_CAP;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    (text, truncated)
}
pub fn helper() -> io::Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(FILE_CAP + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_CAP {
        return Err(err("PDF exceeds 4 MB"));
    }
    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|_| err("Could not extract PDF text"))?;
    let (text, truncated) = bounded(text);
    println!("{}", json!({"text":text,"truncated":truncated}));
    Ok(())
}
pub fn extract(workspace: &Path, path: &str, stop: &AtomicBool) -> io::Result<Value> {
    let policy = PermissionPolicy::milestone_default(workspace);
    if policy.check(&Action::ReadFile { path: path.into() }) != PermissionDecision::Allow {
        return Err(err("Document path is outside the permitted workspace"));
    }
    let resolved = policy.resolve_workspace_path(path).map_err(err)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&resolved)?
        .take(FILE_CAP + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_CAP {
        return Err(err("Document exceeds 4 MB"));
    }
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let pdf = bytes.starts_with(b"%PDF-");
    let (text, truncated) = if pdf {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--extract-pdf")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()?;
        let _job = match harness_core::process_job::ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let mut input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let writer = std::thread::spawn(move || input.write_all(&bytes));
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            output
                .take(512 * 1024 + 1)
                .read_to_end(&mut out)
                .map(|_| out)
        });
        let started = Instant::now();
        let status = loop {
            if stop.load(Ordering::SeqCst) || started.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                let _ = reader.join();
                return Err(err("Document extraction stopped or exceeded 10 seconds"));
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let _ = writer.join();
        let out = reader
            .join()
            .map_err(|_| err("Document output reader failed"))??;
        if !status.success() || out.len() > 512 * 1024 {
            return Err(err(
                "Could not extract PDF text; encrypted or unsupported PDFs need preprocessing",
            ));
        }
        let value: Value =
            serde_json::from_slice(&out).map_err(|_| err("Invalid PDF extraction result"))?;
        (
            value["text"].as_str().unwrap_or("").to_string(),
            value["truncated"] == true,
        )
    } else {
        bounded(
            String::from_utf8(bytes)
                .map_err(|_| err("Use a text document or PDF; images require OCR first"))?,
        )
    };
    if stop.load(Ordering::SeqCst) {
        return Err(err("Document extraction cancelled"));
    }
    if text.trim().is_empty() {
        return Err(err(
            "Document has no extractable text. Scanned PDFs require OCR first.",
        ));
    }
    Ok(
        json!({"text":text,"truncated":truncated,"sha256":digest,"format":if pdf{"pdf"}else{"text"}}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_paths_and_utf8_limits_are_enforced() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("input.txt"), "é".repeat(40000)).unwrap();
        let result = extract(root.path(), "input.txt", &AtomicBool::new(false)).unwrap();
        assert_eq!(result["truncated"], true);
        assert_eq!(result["text"].as_str().unwrap().len(), 60000);
        assert!(extract(root.path(), "../outside.txt", &AtomicBool::new(false)).is_err());
        assert!(extract(root.path(), "input.txt", &AtomicBool::new(true)).is_err());
    }
}
