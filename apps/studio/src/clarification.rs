//! Clarify real identity ambiguity, not cosmetic differences in display labels.
pub const INSTRUCTIONS: &str = r#"
For an existing-browser request, inspect the available profiles before asking which profile to use. Use the name already supplied in the conversation; do not ask for it again or claim there are multiple matches without observations. Listing profiles is part of the requested work and does not require another confirmation. Profile display names are not case-sensitive account identifiers. If inspection reveals multiple plausible matches or no match, explain what you actually found and ask only for the unresolved choice.
Do not ask the user to transcribe observable browser state such as the current URL, address bar, or page title when desktop inspection is enabled. Inspect it yourself if needed. When the user has specified the destination and the requested browser profile is identified, continue navigation in a new tab; knowing the old tab's URL is not a prerequisite. Verify the destination before performing the requested external action. Ask only for information that inspection cannot resolve, such as login assistance or genuinely ambiguous identities.
When the user requests a website in their existing browser/profile, inspect the existing browser, identify the requested profile, and open a new tab in that same window (for example, focus it and use Ctrl+T). Do not ask the user to choose CDP, attach versus browser_open, debugging ports, or internal tab IDs. Those are implementation choices. A fresh isolated profile does not satisfy an existing-profile request. Use desktop controls when no authorized matching CDP session is available. Never assume a loopback endpoint is the requested profile merely because it exists. If inspection finds genuinely ambiguous profiles, ask using their visible names; if desktop access is unavailable or an emergency stop remains active, report that specific blocker without bypassing it.
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

pub fn requests_tool_evidence(question: &str) -> bool {
    let q = question.to_lowercase();
    ["provide", "send me", "supply"]
        .iter()
        .any(|s| q.contains(s))
        && ["browser_read", "browser_screenshot", "desktop_observe"]
            .iter()
            .any(|s| q.contains(s))
}


pub fn browser_route_question(question: &str) -> bool {
    let q = question.to_lowercase();
    (q.contains("cdp") || q.contains("browser_open") || q.contains("isolated chrome"))
        && (q.contains("which session") || q.contains("attach or open") || q.contains("which tab") || q.contains("should i use"))
}

pub fn browser_state_question(question: &str) -> bool {
    let q = question.to_lowercase();
    let state = ["current url", "address bar", "address-bar", "current page title"]
        .iter()
        .any(|s| q.contains(s));
    let request = ["what", "tell me", "provide", "confirm", "which url"]
        .iter()
        .any(|s| q.contains(s));
    let blocker = ["sign in", "log in", "password", "permission", "access disabled", "cannot inspect", "can't inspect", "multiple profiles", "which profile"]
        .iter()
        .any(|s| q.contains(s));
    state && request && !blocker
}

pub fn profile_inspection_question(question: &str) -> bool {
    let q = question.to_lowercase();
    q.contains("profile")
        && ["which", "exact profile", "confirm", "list", "case-sensitive"]
            .iter().any(|s| q.contains(s))
        && !["password", "sign in", "log in", "access disabled", "cannot inspect", "can't inspect", "emergency stop"]
            .iter().any(|s| q.contains(s))
}

pub fn existing_browser_profile(goal: &str) -> bool {
    let goal = goal.to_lowercase();
    goal.contains("profile")
        && ["chrome", "edge", "browser"]
            .iter()
            .any(|s| goal.contains(s))
        && !["new profile", "isolated profile", "temporary profile"]
            .iter()
            .any(|s| goal.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_selection_requires_inspection_first() {
        assert!(profile_inspection_question("Which specific profile(s) should I open? Please provide the exact profile name(s) to open (case-sensitive, e.g., 'mrmuller'). I can list the visible profiles; please confirm."));
        assert!(profile_inspection_question("Which profile should I use?"));
        assert!(!profile_inspection_question("Please enter the profile password."));
        assert!(!profile_inspection_question("I cannot inspect profiles. Which one should I use?"));
    }
    #[test]
    fn observable_browser_state_is_not_user_input() {
        assert!(browser_state_question("What is the current URL showing in the address bar? I need to verify we're on the right page before navigating to whatsapp.com."));
        assert!(browser_state_question("Please tell me the current page title."));
        assert!(!browser_state_question("Which URL should I open?"));
        assert!(!browser_state_question("I cannot inspect the browser. Please provide the current URL."));
        assert!(!browser_state_question("Which profile should I use? Please confirm the address bar."));
        assert!(!browser_state_question("Please sign in to continue."));
    }
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
    #[test]
    fn existing_profiles_and_tool_requests_are_explicit() {
        assert!(browser_route_question("Which session should I use: CDP or browser_open?"));
        assert!(!browser_route_question("Which profile: Personal or Work?"));
        assert!(existing_browser_profile(
            "Open Chrome, my work profile, then the website"
        ));
        assert!(!existing_browser_profile("Open Chrome with a new profile"));
        assert!(!existing_browser_profile("Read the customer profile"));
        assert!(requests_tool_evidence(
            "Please provide fresh browser_read evidence"
        ));
        assert!(!requests_tool_evidence("Please sign in to continue"));
    }
}
