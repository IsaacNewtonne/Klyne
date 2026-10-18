//! Recognize redundant file writes without treating receipts as task completion.
use harness_core::{Action, PermissionDecision, PermissionPolicy, verification::SuccessCriterion};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Small durable history of completed file operations. Hashes retain no content.
/// This is a stall signal, never evidence of task completion or permission to replay.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Journal {
    task: Option<usize>,
    recent: Vec<(String, String)>,
    pub reviews: u32,
}
impl Journal {
    fn key(decision: &Value) -> Option<String> {
        if !matches!(decision["decision"].as_str(), Some("act" | "verify")) {
            return None;
        }
        // Polling remote jobs and desktop observations have different progress
        // semantics. Leave those and all uncertain effects to their own controllers.
        if !matches!(
            decision["action"]["tool"].as_str(),
            Some(
                "read_file"
                    | "read_range"
                    | "hash_file"
                    | "search_file"
                    | "list_dir"
                    | "stat_path"
                    | "write_file"
                    | "patch_file"
                    | "document_read"
                    | "document_create"
                    | "resource_route"
                    | "artifact_read"
                    | "artifact_write"
                    | "artifact_inspect"
                    | "artifact_verify"
                    | "document_classify"
                    | "capability_list"
                    | "capability_history"
                    | "memory_read"
                    | "skill_read"
            )
        ) {
            return None;
        }
        Some(format!(
            "{:x}",
            Sha256::digest(decision["action"].to_string().as_bytes())
        ))
    }
    pub fn record(&mut self, task: usize, decision: &Value, evidence: &[Value]) {
        if self.task != Some(task) {
            *self = Self {
                task: Some(task),
                ..Self::default()
            };
        }
        let Some(key) = Self::key(decision) else {
            self.recent.clear();
            return;
        };
        if evidence.is_empty() {
            return;
        }
        let outcomes: Vec<_> = evidence
            .iter()
            .map(|e| {
                serde_json::json!({
                    "ok":e["ok"], "data":e["data"], "receipt":e["receipt"]
                })
            })
            .collect();
        let outcome = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&outcomes).unwrap())
        );
        if !self.recent.contains(&(key.clone(), outcome.clone())) {
            self.reviews = 0;
        }
        self.recent.push((key, outcome));
        // Three observations of each operation in a cycle of up to eight.
        if self.recent.len() > 24 {
            self.recent.remove(0);
        }
    }
    pub fn stalled(&self, task: usize, decision: &Value) -> bool {
        if self.task != Some(task) {
            return false;
        }
        let Some(key) = Self::key(decision) else {
            return false;
        };
        let Some(last) = self.recent.iter().rev().find(|(k, _)| *k == key) else {
            return false;
        };
        self.recent.iter().filter(|entry| *entry == last).count() >= 3
    }
}

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
    fn multi_file_and_discovery_cycles_survive_reload() {
        for length in [4, 8] {
            let mut journal = Journal::default();
            let actions: Vec<_> = (0..length).map(|i| json!({"decision":"act","action":{"tool":if i % 2 == 0 {"document_read"} else {"memory_read"},"path":format!("{i}.txt"),"name":format!("note-{i}")}})).collect();
            for _ in 0..3 {
                for action in &actions {
                    journal.record(0, action, &[json!({"ok":true,"data":"unchanged"})]);
                }
            }
            let saved: Journal =
                serde_json::from_value(serde_json::to_value(&journal).unwrap()).unwrap();
            assert!(saved.stalled(0, &actions[0]));
            journal.record(
                0,
                &actions[0],
                &[json!({"ok":true,"data":"new information"})],
            );
            assert!(!journal.stalled(0, &actions[0]));
        }
    }

    #[test]
    fn recurring_reads_and_alternating_writes_survive_restart() {
        let mut journal = Journal::default();
        let read = json!({"decision":"act","action":{"tool":"read_file","path":"a"}});
        let result = [json!({"ok":true,"data":"same"})];
        for _ in 0..3 {
            journal.record(0, &read, &result);
        }
        let saved: Journal =
            serde_json::from_value(serde_json::to_value(&journal).unwrap()).unwrap();
        assert!(saved.stalled(0, &read));
        assert!(!saved.stalled(1, &read));
        journal.record(0, &read, &[json!({"ok":true,"data":"changed"})]);
        assert!(!journal.stalled(0, &read));
        let a = json!({"decision":"act","action":{"tool":"write_file","path":"a","contents":"A"}});
        let b = json!({"decision":"act","action":{"tool":"write_file","path":"a","contents":"B"}});
        for _ in 0..3 {
            journal.record(1, &a, &result);
            journal.record(1, &b, &result);
        }
        assert!(journal.stalled(1, &a));
        let shell = json!({"decision":"act","action":{"tool":"run_shell","program":"example"}});
        assert!(!journal.stalled(1, &shell));
        journal.record(1, &shell, &result);
        assert!(!journal.stalled(1, &a));
    }

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
