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
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Independently produced test attestation for one candidate digest.
/// Written by whoever ran the candidate's suite green (operator or harness
/// experiment flow) BEFORE staging; consumed by `preflight`. Self-issued
/// claims inside the candidate binary do not count.
#[derive(Clone, Serialize, Deserialize)]
pub struct Attestation {
    pub digest: String,
    pub test_command: String,
    pub test_result: String,
    pub produced_at_ms: u64,
}

/// Attestation freshness window: a week-old green suite says little about
/// today's candidate, and anything dated in the future is skew or forgery.
pub const ATTESTATION_MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

pub fn attestation_path(root: &Path, digest: &str) -> PathBuf {
    root.join("runtime")
        .join(format!("attestation-{digest}.json"))
}

/// Record a green test run for `digest`. Refuses empty commands and
/// anything but an explicit pass.
pub fn attest(root: &Path, digest: &str, test_command: &str) -> io::Result<Attestation> {
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("Attestation needs a SHA-256 hex digest"));
    }
    if test_command.trim().is_empty() || test_command.len() > 1024 {
        return Err(io::Error::other(
            "Attestation needs the exact test command that passed",
        ));
    }
    let attestation = Attestation {
        digest: digest.into(),
        test_command: test_command.into(),
        test_result: "pass".into(),
        produced_at_ms: now_ms(),
    };
    let path = attestation_path(root, digest);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    fs::write(
        &temp,
        serde_json::to_vec(&attestation).map_err(io::Error::other)?,
    )?;
    fs::rename(temp, path)?;
    Ok(attestation)
}

pub fn check_attestation(root: &Path, digest: &str, now: u64) -> io::Result<()> {
    let bytes = fs::read(attestation_path(root, digest)).map_err(|_| {
        io::Error::other(
            "No test attestation for this candidate digest. Run the candidate test suite green and record it with attest() before staging.",
        )
    })?;
    let attestation: Attestation =
        serde_json::from_slice(&bytes).map_err(|_| io::Error::other("Attestation is corrupt"))?;
    if !attestation.digest.eq_ignore_ascii_case(digest) {
        return Err(io::Error::other(
            "Attestation digest does not match the candidate",
        ));
    }
    if attestation.test_result != "pass" || attestation.test_command.trim().is_empty() {
        return Err(io::Error::other(
            "Attestation does not record a passing test command",
        ));
    }
    if attestation.produced_at_ms > now.saturating_add(3_600_000)
        || now.saturating_sub(attestation.produced_at_ms) > ATTESTATION_MAX_AGE_MS
    {
        return Err(io::Error::other(
            "Attestation is stale or dated in the future; re-run the candidate suite",
        ));
    }
    Ok(())
}
pub fn preflight(root: &Path, candidate: &Candidate) -> io::Result<()> {
    if digest(&candidate.binary)? != candidate.sha256 {
        return Err(io::Error::other("Runtime digest mismatch"));
    }
    // Identity and protocol checks prove what the candidate IS, not that it
    // is safe to run. Activation additionally requires an independently
    // produced test attestation for this exact digest (audit Phase 8):
    // the candidate's own suite, run green outside this process, recorded
    // with its command and timestamp before staging.
    check_attestation(root, &candidate.sha256, now_ms())?;
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
    preflight(root, &candidate)?;
    let temp = root.join("runtime/pending.tmp");
    fs::write(&temp, serde_json::to_vec(&candidate)?)?;
    fs::rename(temp, root.join("runtime/pending.json"))?;
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_of(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    #[test]
    fn attestation_binds_command_freshness_and_digest() {
        let root =
            std::env::temp_dir().join(format!("klyne-attest-{}-{}", std::process::id(), now_ms()));
        fs::create_dir_all(&root).unwrap();
        let digest = digest_of(b"candidate");
        // Nothing recorded: staging material is refused.
        assert!(check_attestation(&root, &digest, now_ms()).is_err());
        // Malformed attestations are refused at write time.
        assert!(attest(&root, "short", "cargo test").is_err());
        assert!(attest(&root, &digest, "  ").is_err());
        let attestation = attest(&root, &digest, "cargo test --workspace --locked").unwrap();
        assert_eq!(attestation.test_result, "pass");
        assert!(check_attestation(&root, &digest, now_ms()).is_ok());
        // Another digest cannot borrow it; stale and future stamps fail.
        assert!(check_attestation(&root, &digest_of(b"other"), now_ms()).is_err());
        assert!(check_attestation(&root, &digest, now_ms() + ATTESTATION_MAX_AGE_MS + 1).is_err());
        assert!(check_attestation(&root, &digest, 1).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
