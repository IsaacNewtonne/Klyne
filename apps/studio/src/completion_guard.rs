//! Separate independently verified effects from evidence-backed model review.
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
pub fn has_attempted_action(evidence: &[serde_json::Value]) -> bool {
    evidence.iter().any(|e| {
        e["action"].is_string()
            && !matches!(e["agent"].as_str(), Some("Completion review" | "Reviewer"))
    })
}
pub fn observation_candidates(evidence: &[serde_json::Value]) -> Vec<usize> {
    // A later worker operation invalidates an earlier destination observation.
    // Unknown tools are conservatively treated as operations, not read checks.
    let last_operation = evidence.iter().rposition(|e| {
        !matches!(
            e["agent"].as_str(),
            Some(
                "Reviewer"
                    | "Host verifier"
                    | "Host reconciliation"
                    | "Runtime check"
                    | "Completion review"
            )
        ) && e["action"].is_string()
    });
    evidence
        .iter()
        .enumerate()
        .filter_map(|(index, e)| {
            let action = e["action"].as_str().unwrap_or("");
            (last_operation.is_none_or(|last| index > last)
                && e["agent"] == "Reviewer"
                && e["ok"] == true
                && (matches!(
                    action,
                    "desktop_observe" | "browser_read" | "browser_screenshot"
                ) || action.starts_with("fetch:"))
                && e["data"].as_str().is_some_and(|data| !data.is_empty()))
            .then_some(index)
        })
        .collect()
}

fn observed_effect(
    review: &serde_json::Value,
    evidence: &[serde_json::Value],
    effect: &str,
) -> bool {
    let candidates = observation_candidates(evidence);
    review["observed_results"]
        .as_array()
        .is_some_and(|results| {
            results.len() <= 32
                && results.iter().any(|result| {
                    result["effect"] == effect
                        && ["target", "observation"].iter().all(|key| {
                            result[key]
                                .as_str()
                                .is_some_and(|s| !s.trim().is_empty() && s.len() <= 4000)
                        })
                        && result["evidence_index"]
                            .as_u64()
                            .is_some_and(|i| candidates.contains(&(i as usize)))
                        && review["outcome"] == "achieved"
                })
        })
}

pub fn check_review(
    goal: &str,
    review: &serde_json::Value,
    evidence: &[serde_json::Value],
) -> std::io::Result<()> {
    let answer = review["summary"].as_str().unwrap_or("");
    if (delivery_claim(goal) || delivery_claim(answer)) && !has_attempted_action(evidence) {
        return Err(crate::err(
            "No tool action has been attempted for this goal. Restating the request is not completion. Use the supplied destination and message; request repair tasks to inspect the app and do the authorized work. Do not ask for capitalization confirmation.",
        ));
    }
    let app_action = evidence.iter().any(|e| {
        let action = e["action"].as_str().unwrap_or("");
        e["agent"] != "Reviewer"
            && (action.starts_with("desktop_") || action.starts_with("browser_"))
            && !matches!(
                action,
                "desktop_observe"
                    | "desktop_apps"
                    | "desktop_clipboard_get"
                    | "browser_read"
                    | "browser_screenshot"
                    | "browser_close"
            )
    });
    if app_action
        && ![
            "send", "open", "click", "delete", "install", "upload", "state",
        ]
        .iter()
        .any(|effect| observed_effect(review, evidence, effect))
    {
        return Err(crate::err(
            "Inspect the app destination with a read-only tool and cite observed_results before completing. Check the saved action's result; do not perform it again.",
        ));
    }
    if (delivery_claim(goal) || delivery_claim(answer))
        && !observed_effect(review, evidence, "send")
    {
        return Err(crate::err(
            "Check the destination with a read-only tool and cite the visible result in observed_results before finishing. Do not repeat the send. A visible outgoing message is an observed result, not proof of delivery or reading.",
        ));
    }
    check_action_evidence_with_review(answer, evidence, Some(review))
}
/// Bind supported completion claims to host-produced destination receipts.
/// Unstructured prose remains model-reviewed; it never creates a receipt.
#[cfg(test)]
pub fn check_action_evidence(answer: &str, evidence: &[serde_json::Value]) -> std::io::Result<()> {
    check_action_evidence_with_review(answer, evidence, None)
}
fn check_action_evidence_with_review(
    answer: &str,
    evidence: &[serde_json::Value],
    review: Option<&serde_json::Value>,
) -> std::io::Result<()> {
    let lower = answer.to_lowercase();
    let groups: &[(&str, &[&str])] = &[
        (
            "file_write",
            &[
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
            ],
        ),
        ("open", &["was opened", "been opened", "i opened"]),
        (
            "delete",
            &[
                "i deleted",
                "been deleted",
                "deleted successfully",
                "was deleted",
            ],
        ),
        (
            "install",
            &[
                "i installed",
                "been installed",
                "installed successfully",
                "was installed",
            ],
        ),
        (
            "upload",
            &[
                "i uploaded",
                "been uploaded",
                "uploaded successfully",
                "was uploaded",
            ],
        ),
        ("click", &["i clicked", "been clicked"]),
        ("send", &["successfully sent", "been sent", "i sent"]),
    ];
    for (effect, phrases) in groups {
        if !phrases.iter().any(|phrase| lower.contains(phrase)) {
            continue;
        }
        let receipts: Vec<_> = evidence
            .iter()
            .filter(|e| valid_receipt(e) && e["receipt"]["effect"] == *effect)
            .collect();
        if receipts.is_empty() {
            // A model-reviewed UI result is useful without pretending it is a
            // host receipt. File claims retain independent filesystem checks.
            if *effect != "file_write"
                && review.is_some_and(|r| observed_effect(r, evidence, effect))
            {
                continue;
            }
            return Err(crate::err(format!(
                "No verified {effect} receipt supports this completion. Inspect the destination and report only observed results."
            )));
        }
        if *effect == "file_write" {
            for word in answer.split_whitespace() {
                let path = word.trim_matches(|c: char| {
                    matches!(
                        c,
                        '`' | '\'' | '"' | '.' | ',' | '(' | ')' | ';' | '!' | '?'
                    )
                });
                let Some((_, extension)) = path.rsplit_once('.') else {
                    continue;
                };
                if extension.is_empty()
                    || extension.len() > 12
                    || !extension.chars().all(char::is_alphanumeric)
                {
                    continue;
                }
                if !receipts
                    .iter()
                    .any(|e| e["receipt"]["target"].as_str() == Some(path))
                {
                    return Err(crate::err(format!("No matching file receipt for {path}")));
                }
            }
        }
    }
    Ok(())
}

fn valid_receipt(entry: &serde_json::Value) -> bool {
    entry["ok"] == true
        && entry["receipt"]["version"] == 1
        && entry["receipt"]["target"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
        && matches!(
            entry["agent"].as_str(),
            Some("Runtime check" | "Host verifier" | "Host reconciliation")
        )
}

pub fn verify_claims(
    review: &serde_json::Value,
    evidence: &[serde_json::Value],
    policy: &harness_core::PermissionPolicy,
) -> std::io::Result<()> {
    use harness_core::verification::{FileEvidenceVerifier, Verifier};
    if let Some(claims) = review.get("claims") {
        let claims = claims
            .as_array()
            .filter(|c| c.len() <= 32)
            .ok_or_else(|| crate::err("claims must be a bounded array"))?;
        for claim in claims {
            if !claim["effect"].is_string()
                || !claim["target"].is_string()
                || !evidence.iter().any(|e| {
                    valid_receipt(e)
                        && e["receipt"]["effect"] == claim["effect"]
                        && e["receipt"]["target"] == claim["target"]
                })
            {
                return Err(crate::err(
                    "A completion claim has no matching host receipt",
                ));
            }
        }
    }
    // Recheck the latest receipt for each file. Earlier versions do not verify
    // later edits, and historical success does not verify a deleted file.
    let explicit = review.get("claims").and_then(serde_json::Value::as_array);
    let summary = review["summary"].as_str().unwrap_or("");
    let named: std::collections::HashSet<_> = evidence
        .iter()
        .filter_map(|entry| {
            let target = entry["receipt"]["target"].as_str()?;
            (summary.contains(target)
                || explicit.is_some_and(|claims| claims.iter().any(|c| c["target"] == target)))
            .then_some(target)
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    for entry in evidence
        .iter()
        .rev()
        .filter(|e| valid_receipt(e) && e["receipt"]["effect"] == "file_write")
    {
        let target = entry["receipt"]["target"].as_str().unwrap();
        if !named.is_empty() && !named.contains(target) {
            continue;
        }
        // An explicit empty claim set need not revalidate scratch files that
        // were deliberately removed. Caller-owned contracts still run.
        if named.is_empty() && explicit.is_some_and(|claims| claims.is_empty()) {
            continue;
        }
        if !seen.insert(target) {
            continue;
        }
        let criterion: harness_core::verification::SuccessCriterion =
            serde_json::from_value(entry["receipt"]["criterion"].clone()).map_err(crate::err)?;
        let action = criterion.observation_action();
        let observation = harness_core::ToolRegistry::milestone_default().execute(&action, policy);
        if !FileEvidenceVerifier
            .verify(&criterion, &action, &observation)
            .passed
        {
            return Err(crate::err(format!(
                "File receipt is stale for {target}; verify the current result before completing"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn receipt(path: &str) -> serde_json::Value {
        json!({"agent":"Runtime check","ok":true,"receipt":{"version":1,"effect":"file_write","target":path}})
    }
    #[test]
    fn unrelated_actions_and_targets_never_confirm_saves() {
        for action in [
            "shell:pytest tests",
            "browser_click",
            "tool_test",
            "list_dir:.",
            "stat_path:x",
            "desktop_clipboard_get",
            "unknown_future_tool",
        ] {
            assert!(
                check_action_evidence(
                    "I saved greeting.txt.",
                    &[json!({"agent":"Worker","action":action,"ok":true})]
                )
                .is_err()
            );
        }
        assert!(check_action_evidence("I saved greeting.txt.", &[receipt("other.txt")]).is_err());
        assert!(check_action_evidence("I saved greeting.txt.", &[receipt("greeting.txt")]).is_ok());
        assert!(
            check_action_evidence(
                "The package has been installed.",
                &[receipt("greeting.txt")]
            )
            .is_err()
        );
        assert!(check_action_evidence("Here is a project plan", &[]).is_ok());
    }
    #[test]
    fn claims_require_exact_effect_and_target_and_fresh_file() {
        let root = tempfile::tempdir().unwrap();
        let policy = harness_core::PermissionPolicy::milestone_default(root.path());
        let mut proof = receipt("a.txt");
        proof["receipt"]["criterion"] =
            json!({"FileContents":{"path":"a.txt","expected":"checked"}});
        std::fs::write(root.path().join("a.txt"), "checked").unwrap();
        assert!(
            verify_claims(
                &json!({"claims":[{"effect":"file_write","target":"a.txt"}]}),
                &[proof.clone()],
                &policy
            )
            .is_ok()
        );
        assert!(
            verify_claims(
                &json!({"claims":[{"effect":"file_write","target":"b.txt"}]}),
                &[proof.clone()],
                &policy
            )
            .is_err()
        );
        std::fs::write(root.path().join("a.txt"), "changed later").unwrap();
        assert!(
            verify_claims(
                &json!({"summary":"Saved a.txt", "claims":[]}),
                &[proof.clone()],
                &policy
            )
            .is_err()
        );
        assert!(verify_claims(&json!({}), &[proof], &policy).is_err());
    }
    #[test]
    fn invented_delivery_is_not_success() {
        assert!(check_review("send a message", &json!({"summary":"Task complete"}), &[]).is_err());
        assert!(
            check_review(
                "hi",
                &json!({"summary":"The message was successfully sent to him"}),
                &[]
            )
            .is_err()
        );
        assert!(check_review("hi", &json!({"summary":"Hi!"}), &[]).is_ok());
    }
    #[test]
    fn app_observations_are_reviewed_not_fabricated_receipts() {
        let operation =
            json!({"agent":"Assistant","action":"desktop_key","ok":true,"data":"key applied"});
        let observation = json!({"agent":"Reviewer","action":"desktop_observe","ok":true,"data":"destination observation"});
        for target in [
            "Chat app / recipient",
            "Mail app / sent folder",
            "Browser chat / recipient",
        ] {
            let review = json!({"summary":"The outgoing message is visible in the requested conversation.","outcome":"achieved","observed_results":[{"effect":"send","target":target,"observation":"The requested text is visible as an outgoing message.","evidence_index":1}]});
            assert!(
                check_review(
                    "send a message",
                    &review,
                    &[operation.clone(), observation.clone()]
                )
                .is_ok()
            );
            assert!(
                check_review(
                    "send a message",
                    &review,
                    &[observation.clone(), operation.clone()]
                )
                .is_err()
            );
            assert!(
                check_review(
                    "send a message",
                    &review,
                    &[operation.clone(), operation.clone()]
                )
                .is_err()
            );
            let mut failed = observation.clone();
            failed["ok"] = json!(false);
            assert!(check_review("send a message", &review, &[operation.clone(), failed]).is_err());
        }
        for effect in ["open", "delete", "install", "upload", "click"] {
            let review = json!({"outcome":"achieved","observed_results":[{"effect":effect,"target":"Any app / destination","observation":"Expected result visible","evidence_index":1}]});
            assert!(observed_effect(
                &review,
                &[operation.clone(), observation.clone()],
                effect
            ));
        }
    }
}
