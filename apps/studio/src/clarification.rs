//! Clarify real identity ambiguity, not cosmetic differences in display labels.
pub const INSTRUCTIONS: &str = r#"
Use the user's latest clarification as an answer, not a reason to ask the same question again. Preserve explicitly requested message text exactly. For human-facing app, group, contact and profile display names, capitalization and repeated whitespace alone are not different identities. Inspect/search the available app to resolve the display label; use the actual observed label when interacting. Do not ask whether 'Team Chat' and 'TEAM CHAT' are different spellings before inspecting. This is a general display-name rule, not an app-specific rule. Never normalize passwords, paths, account addresses, IDs, selectors or message content. If inspection reveals multiple plausible destinations, ask one concise question identifying those observed choices. If the needed information is already in the conversation, proceed within existing authorization. Missing login/access and genuinely missing message content still require input. Speak directly to the user, not 'The user clarified...'. Restating the requested action is not completing it. If no action was attempted, request actual work rather than claiming a result needs verification.
"#;

pub fn cosmetic_name_question(question: &str) -> bool {
    let lower = question.to_lowercase();
    if !["group", "contact", "profile", "display name"]
        .iter()
        .any(|s| lower.contains(s))
        || [
            "password",
            "file path",
            "email address",
            "account id",
            "two matches",
            "multiple matches",
            "two groups",
            "two contacts",
        ]
        .iter()
        .any(|s| lower.contains(s))
    {
        return false;
    }
    let mut quoted = Vec::new();
    let mut quote = None;
    let mut current = String::new();
    for ch in question.chars() {
        if quote == Some(ch) {
            if !current.trim().is_empty() {
                quoted.push(current.clone());
            }
            current.clear();
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if quote.is_some() {
            current.push(ch);
        }
    }
    let normalize = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    quoted.iter().enumerate().any(|(i, a)| {
        quoted
            .iter()
            .skip(i + 1)
            .any(|b| a != b && normalize(a) == normalize(b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_case_is_not_identity_but_real_ambiguity_is_preserved() {
        assert!(cosmetic_name_question(
            "Is the group 'dA STREETS' or 'DA STREETS'?"
        ));
        assert!(cosmetic_name_question(
            "Contact 'Alice Smith' or 'ALICE  SMITH'?"
        ));
        assert!(!cosmetic_name_question(
            "Two contacts match: 'Alice' and 'ALICE'. Which one?"
        ));
        assert!(!cosmetic_name_question("Group 'Family' or 'Family work'?"));
        assert!(!cosmetic_name_question(
            "Profile password 'Secret' or 'SECRET'?"
        ));
        assert!(!cosmetic_name_question("What message should I send?"));
    }
}
