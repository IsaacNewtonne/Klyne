//! Staged runtime replacement. The supervisor owns activation and rollback.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub binary: PathBuf,
    pub sha256: String,
}
pub fn digest(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn preflight(candidate: &Candidate) -> io::Result<()> {
    if digest(&candidate.binary)? != candidate.sha256 {
        return Err(io::Error::other("Runtime digest mismatch"));
    }
    let mut policy =
        harness_core::PermissionPolicy::milestone_default(candidate.binary.parent().unwrap());
    let program = candidate.binary.to_string_lossy().into_owned();
    policy.allow_shell_program(&program);
    let tool = harness_core::WorkspaceShellTool::new(harness_core::ProcessLimits {
        timeout: std::time::Duration::from_secs(15),
        max_output_bytes: 8192,
    });
    let result = tool.execute_cancellable(
        &harness_core::Action::RunShell {
            program,
            args: vec!["--runtime-check".into()],
        },
        &policy,
        &AtomicBool::new(false),
    );
    if !result.ok
        || !result.data.lines().any(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .is_ok_and(|v| v["klyne_runtime_protocol"] == 1)
        })
    {
        return Err(io::Error::other(format!(
            "Candidate runtime preflight failed: {}",
            result.summary
        )));
    }
    Ok(())
}
#[allow(dead_code)] // Shared with the supervisor, which only consumes candidates.
pub fn stage(root: &Path, binary: &Path, expected: &str) -> io::Result<Candidate> {
    if std::env::var("KLYNE_SUPERVISED").as_deref() != Ok("1") {
        return Err(io::Error::other(
            "Start klyne-supervisor before staging a runtime upgrade",
        ));
    }
    let binary = fs::canonicalize(binary)?;
    if digest(&binary)? != expected {
        return Err(io::Error::other("Candidate digest does not match"));
    }
    let versions = root.join("runtime/versions");
    fs::create_dir_all(&versions)?;
    let stage_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("runtime/stage.lock"))?;
    stage_lock.try_lock().map_err(io::Error::other)?;
    if root.join("runtime/pending.json").exists() {
        return Err(io::Error::other("A runtime upgrade is already pending"));
    }
    let target = versions.join(format!("{expected}{}", std::env::consts::EXE_SUFFIX));
    if !target.exists() {
        fs::copy(&binary, &target)?;
    }
    let candidate = Candidate {
        binary: target,
        sha256: expected.into(),
    };
    preflight(&candidate)?;
    let temp = root.join("runtime/pending.tmp");
    fs::write(&temp, serde_json::to_vec(&candidate)?)?;
    fs::rename(temp, root.join("runtime/pending.json"))?;
    Ok(candidate)
}
