//! Conservative guard for unsupported delivery claims, independent of access toggles.
pub fn delivery_claim(text: &str) -> bool {
    let text = text.to_lowercase();
    let message = ["message", "email", "e-mail", "sent to", "delivered to"]
        .iter()
        .any(|s| text.contains(s));
    message
        && ["send", "sent", "deliver", "messaged", "emailed"]
            .iter()
            .any(|s| text.contains(s))
}
pub fn check(goal: &str, answer: &str) -> std::io::Result<()> {
    if delivery_claim(goal) || delivery_claim(answer) {
        return Err(crate::err(
            "Message delivery is unverified. Klyne has no supported delivery receipt for this task and cannot confirm that a message was sent. Inspect the destination before retrying. If no action occurred, enable the required app or desktop access before continuing.",
        ));
    }
    Ok(())
}
pub fn check_action_evidence(answer: &str, evidence: &[serde_json::Value]) -> std::io::Result<()> {
    let lower = answer.to_lowercase();
    let claims = [
        "was opened",
        "been opened",
        "i opened",
        "i saved",
        "been saved",
        "saved successfully",
        "was saved",
        "i created",
        "was created",
        "been created",
        "created successfully",
        "file created",
        "file is ready",
        "i deleted",
        "been deleted",
        "deleted successfully",
        "was deleted",
        "i installed",
        "been installed",
        "installed successfully",
        "was installed",
        "i uploaded",
        "been uploaded",
        "uploaded successfully",
        "was uploaded",
        "i clicked",
        "been clicked",
        "successfully sent",
        "been sent",
        "i sent",
    ];
    if !claims.iter().any(|claim| lower.contains(claim)) {
        return Ok(());
    }
    // Operation-bound receipts (audit Phase 3): an effect claim needs a
    // successful *effect* receipt, not just any ok evidence. Pure
    // observations — file reads, searches, hashes, browser reads/screenshots,
    // desktop observations, listings — can never confirm a mutation, so a
    // read-only trail claiming "saved" is rejected. Unknown future effect
    // tools stay lenient; host verification receipts always qualify.
    // Residual risk (documented): paraphrase outside the claim list still
    // evades; broker-issued receipts (deferred Phase 2) close that fully.
    if evidence.iter().any(is_effect_receipt) {
        return Ok(());
    }
    Err(crate::err(
        "The proposed answer claims an app or file action, but this task has no successful effect evidence. That action is not confirmed. The task is incomplete.",
    ))
}

/// Whether one evidence entry can serve as a receipt for a claimed mutation:
/// successful, not from an excluded controller, and recording an effect
/// rather than a read-only observation.
fn is_effect_receipt(entry: &serde_json::Value) -> bool {
    if entry["ok"] != true {
        return false;
    }
    match entry["agent"].as_str() {
        // Host verification and reconciliation are bound receipts by
        // construction; route-control and approval bookkeeping carry no
        // effects (an approval must never satisfy the claim it approved).
        Some("Host reconciliation") | Some("Runtime check") | Some("Host verifier") => return true,
        Some("Route controller") | Some("User reconciliation") | Some("User approval") => {
            return false;
        }
        _ => {}
    }
    let action = entry["action"].as_str().unwrap_or("");
    // Read-only observations can never confirm a mutation.
    const OBSERVATIONAL: [&str; 11] = [
        "read_file:",
        "read_range:",
        "hash_file:",
        "search_file:",
        "browser_read",
        "browser_screenshot",
        "desktop_observe",
        "app_list",
        "app_operations",
        "runtime_status",
        "sample_outline",
    ];
    !OBSERVATIONAL
        .iter()
        .any(|prefix| action.starts_with(prefix))
}
#[cfg(test)]
mod tests {
    use serde_json::json;

    fn receipt(agent: &str, action: &str) -> serde_json::Value {
        json!({"agent": agent, "action": action, "ok": true})
    }

    #[test]
    fn invented_delivery_is_not_success() {
        assert!(super::check("open zalo, send a message to joidi", "Task complete").is_err());
        assert!(super::check("hi", "The message was successfully sent to him").is_err());
        assert!(super::check("hi", "Hi! How can I help?").is_ok());
        assert!(super::check_action_evidence("Zalo was opened", &[]).is_err());
        assert!(super::check_action_evidence("Here is a project plan", &[]).is_ok());
    }

    #[test]
    fn observations_alone_cannot_confirm_mutations() {
        // The audit hole: any successful tool evidence satisfied any claim.
        // A read-only trail must not confirm "saved".
        let reads = vec![
            receipt("Worker", "read_file:greeting.txt"),
            receipt("Worker", "browser_read"),
            receipt("Worker", "desktop_observe"),
            receipt("Worker", "browser_screenshot"),
        ];
        assert!(super::check_action_evidence("I saved greeting.txt.", &reads).is_err());
        assert!(super::check_action_evidence("Zalo was opened", &reads).is_err());
        // Failed writes are not receipts either.
        let failed = vec![json!({"agent": "Worker", "action": "write_file:x.txt", "ok": false})];
        assert!(super::check_action_evidence("I saved x.", &failed).is_err());
    }

    #[test]
    fn effect_receipts_confirm_claims() {
        for action in [
            "write_file:greeting.txt",
            "patch_file:greeting.txt:0",
            "shell:pytest tests",
            "browser_click",
            "desktop_fill",
            "desktop_drag",
            "desktop_clipboard_set",
            "tool_run",
            "tool_test",
            "app_call",
            "mcp_call",
        ] {
            assert!(
                super::check_action_evidence("I saved the document.", &[receipt("Worker", action)])
                    .is_ok(),
                "{action} should confirm a save claim"
            );
        }
        // Host verification and reconciliation are bound receipts.
        assert!(
            super::check_action_evidence(
                "I saved the document.",
                &[receipt("Host reconciliation", "document_save_reconcile")]
            )
            .is_ok()
        );
        assert!(
            super::check_action_evidence(
                "The file was created.",
                &[receipt("Runtime check", "read_file:greeting.txt")]
            )
            .is_ok()
        );
        // Controller bookkeeping is not evidence.
        let control = vec![receipt("Route controller", "route_switch")];
        assert!(super::check_action_evidence("I saved the file.", &control).is_err());
    }

    #[test]
    fn paraphrase_variants_still_trigger() {
        let proof = vec![receipt("Worker", "write_file:x.txt")];
        for claim in [
            "The file has been saved.",
            "Created successfully.",
            "It was successfully sent.",
            "The package has been installed.",
        ] {
            assert!(super::check_action_evidence(claim, &[]).is_err(), "{claim}");
            assert!(
                super::check_action_evidence(claim, &proof).is_ok(),
                "{claim}"
            );
        }
    }
}
