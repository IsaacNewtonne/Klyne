//! Recognize redundant file writes without treating receipts as task completion.
use harness_core::{Action, PermissionDecision, PermissionPolicy, verification::SuccessCriterion};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

/// A candidate still requires a fresh host check before suppressing the write.
/// Only the newest receipt for this destination in the current goal is eligible.
pub fn repeated_write(
    workspace: &Path,
    evidence: &[Value],
    decision: &Value,
) -> Option<SuccessCriterion> {
    if !matches!(decision["decision"].as_str(), Some("act" | "verify"))
        || decision["action"]["tool"] != "write_file"
    {
        return None;
    }
    let path = crate::chat_paths::relative_path(workspace, decision["action"]["path"].as_str()?);
    let contents = decision["action"]["contents"].as_str()?;
    let policy = PermissionPolicy::milestone_default(workspace);
    if policy.check(&Action::WriteFile {
        path: path.clone(),
        contents: contents.into(),
    }) != PermissionDecision::Allow
    {
        return None;
    }
    let destination = policy.resolve_workspace_path(&path).ok()?;
    let receipt = evidence.iter().rev().find(|e| {
        e["receipt"]["effect"] == "file_write"
            && e["receipt"]["target"].as_str().is_some_and(|target| {
                policy.resolve_workspace_path(target).ok().as_ref() == Some(&destination)
            })
    })?;
    let sha256 = format!("{:x}", Sha256::digest(contents.as_bytes()));
    (receipt["ok"] == true && receipt["receipt"]["criterion"]["FileDigest"]["sha256"] == sha256)
        .then_some(SuccessCriterion::FileDigest { path, sha256 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::{
        ToolRegistry,
        verification::{FileEvidenceVerifier, Verifier},
    };
    use serde_json::json;

    #[test]
    fn repeats_require_matching_receipt_and_fresh_bytes() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("a.txt");
        std::fs::write(&file, "alpha").unwrap();
        let receipt = json!({"ok":true,"receipt":{"effect":"file_write","target":"a.txt","criterion":{"FileDigest":{"path":"a.txt","sha256":format!("{:x}",Sha256::digest(b"alpha"))}}}});
        let decision = json!({"decision":"act","action":{"tool":"write_file","path":"./a.txt","contents":"alpha"}});
        let evidence = vec![
            receipt.clone(),
            json!({"action":"read_file:other.txt","ok":true}),
        ];
        let criterion = repeated_write(root.path(), &evidence, &decision).unwrap();
        let action = criterion.observation_action();
        let policy = PermissionPolicy::milestone_default(root.path());
        let check = || {
            FileEvidenceVerifier
                .verify(
                    &criterion,
                    &action,
                    &ToolRegistry::milestone_default().execute(&action, &policy),
                )
                .passed
        };
        assert!(check());
        std::fs::write(&file, "changed externally").unwrap();
        assert!(
            !check(),
            "historical receipts cannot suppress a necessary repair"
        );
        assert!(repeated_write(root.path(), &[], &decision).is_none());
        let mut changed = decision.clone();
        changed["action"]["contents"] = json!("new revision");
        assert!(repeated_write(root.path(), &evidence, &changed).is_none());
        changed = decision.clone();
        changed["action"]["path"] = json!(file);
        assert!(repeated_write(root.path(), &evidence, &changed).is_some());
        changed["action"]["path"] = json!("../a.txt");
        assert!(repeated_write(root.path(), &evidence, &changed).is_none());
        changed = decision.clone();
        changed["action"]["tool"] = json!("desktop_type");
        assert!(repeated_write(root.path(), &evidence, &changed).is_none());
        let mut newer = receipt.clone();
        newer["ok"] = json!(false);
        assert!(repeated_write(root.path(), &[receipt, newer], &decision).is_none());
    }
}
