//! Interaction gate (2026-10-25): a task that asks for a click or typing is
//! complete only when the matching action really happened, the click changed
//! the page, and quoted values in the answer come from the page after it.
//! A browser_open of a URL that is not a link on the current page, in a task
//! that says click, is refused with a hint to use browser_click instead.

use serde_json::{Value, json};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    pub click: bool,
    pub typing: bool,
}

fn normalized(text: &str) -> String {
    let mut out = String::from(" ");
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else {
            out.push(' ');
        }
    }
    let mut collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.insert(0, ' ');
    collapsed.push(' ');
    collapsed
}

/// Which interactions the user's words ask for. Only imperative tasks about
/// a page, link, browser or app count; how/why questions without a URL are
/// explanations, not interactions.
pub fn needs(text: &str) -> Needs {
    let n = normalized(text);
    let has = |words: &[&str]| words.iter().any(|w| n.contains(&format!(" {w} ")));
    let url = text.contains("http://") || text.contains("https://");
    let context = url
        || has(&[
            "page", "link", "button", "browser", "site", "website", "tab", "form", "field", "menu",
            "app", "window", "desktop", "screen", "icon", "open",
        ]);
    let explanation = has(&[
        "how do i",
        "how to",
        "how can i",
        "how would i",
        "why",
        "explain",
        "what happens",
    ]);
    if !context || (explanation && !url) {
        return Needs::default();
    }
    let click = has(&[
        "click",
        "clicks",
        "clicking",
        "tap the",
        "tap on",
        "press the",
        "press enter",
        "press on",
        "press ctrl",
        "press tab",
        "submit",
        "select the",
        "follow the link",
        "follow that link",
        "follow the first link",
    ]) || (has(&["follow"]) && has(&["link"]));
    let typing = has(&[
        "type into",
        "type in",
        "type the",
        "fill in",
        "fill out",
        "fill the",
        "enter into",
        "enter it into",
        "enter the text",
    ]);
    Needs { click, typing }
}

/// Parsed tool result of an evidence entry (browser data is a JSON string).
pub fn observation(entry: &Value) -> Value {
    match &entry["data"] {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        other => other.clone(),
    }
}

const CLICKS: &[&str] = &[
    "browser_click",
    "desktop_click",
    "desktop_invoke",
    "desktop_key",
];
const TYPES: &[&str] = &["browser_fill", "desktop_type", "desktop_fill"];

fn acted(entry: &Value, tools: &[&str]) -> bool {
    entry["ok"] == true
        && entry["agent"] != "Reviewer"
        && entry["action"].as_str().is_some_and(|a| tools.contains(&a))
}

fn is_page(entry: &Value) -> bool {
    entry["ok"] == true
        && entry["action"].as_str().is_some_and(|a| {
            matches!(
                a,
                "browser_open" | "browser_read" | "browser_click" | "browser_fill"
            )
        })
}

fn page_text(entry: &Value) -> String {
    let o = observation(entry);
    format!(
        "{}\n{}",
        o["title"].as_str().unwrap_or(""),
        o["text"].as_str().unwrap_or("")
    )
    .to_lowercase()
}

fn quoted(answer: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (open, close) in [('"', '"'), ('\u{201c}', '\u{201d}')] {
        let mut rest = answer;
        while let Some(start) = rest.find(open) {
            let after = &rest[start + open.len_utf8()..];
            let Some(end) = after.find(close) else { break };
            let q = after[..end].trim();
            if q.chars().count() >= 3 && q.chars().count() <= 120 {
                out.push(q.to_lowercase());
            }
            rest = &after[end + close.len_utf8()..];
        }
    }
    out
}

/// Why the completion is not accepted yet, or None. `can_act` is false when
/// neither Web nor Desktop access is on (the model can only explain then).
pub fn problem(request: &str, answer: &str, window: &[Value], can_act: bool) -> Option<String> {
    let need = needs(request);
    if !can_act || (!need.click && !need.typing) {
        return None;
    }
    if need.typing && !window.iter().any(|e| acted(e, TYPES)) {
        return Some("Interaction check: the task asks to type into the page, but no browser_fill or desktop_type step succeeded. Type with browser_fill {ref or selector, text}, then answer from the page after it.".into());
    }
    if !need.click {
        return None;
    }
    let Some(first) = window.iter().position(|e| acted(e, CLICKS)) else {
        let opened = window.iter().any(|e| e["action"] == "browser_open");
        return Some(format!(
            "Interaction check: the task asks to click, but no browser_click step succeeded{}. Use browser_click with the ref of the named link from links (browser_read lists them), then answer from the page after the click.",
            if opened {
                "; opening a URL directly is not a click"
            } else {
                ""
            }
        ));
    };
    // The latest browser click must have changed the page, or a later page
    // observation must differ from what the click saw.
    if let Some(last) = window.iter().rposition(|e| acted(e, &["browser_click"])) {
        let click = observation(&window[last]);
        if click["changed"] == false {
            let seen = (
                click["url"].clone(),
                click["title"].clone(),
                click["text"].clone(),
            );
            let moved = window[last + 1..].iter().filter(|e| is_page(e)).any(|e| {
                let o = observation(e);
                (o["url"].clone(), o["title"].clone(), o["text"].clone()) != seen
            });
            if !moved {
                return Some(format!(
                    "Interaction check: browser_click on {} did not change the page (same URL, title and text). Click the ref of the link the task names, then read the page it lands on.",
                    click["clicked"]
                        .as_str()
                        .map_or_else(|| "the target".to_owned(), |c| format!("\"{c}\""))
                ));
            }
        }
    }
    // Quoted values must come from the page after the interaction, not the
    // page before it. Link labels and the user's own words are exempt.
    let request_lower = request.to_lowercase();
    let mut before = String::new();
    let mut labels = Vec::new();
    for e in &window[..first] {
        if is_page(e) {
            before.push_str(&page_text(e));
            if let Some(links) = observation(e)["links"].as_array() {
                labels.extend(
                    links
                        .iter()
                        .filter_map(|l| l["text"].as_str())
                        .map(str::to_lowercase),
                );
            }
        }
    }
    let after: String = window[first..]
        .iter()
        .filter(|e| is_page(e))
        .map(page_text)
        .collect();
    let clicked: Vec<String> = window[first..]
        .iter()
        .filter_map(|e| observation(e)["clicked"].as_str().map(str::to_lowercase))
        .collect();
    let stale: Vec<String> = quoted(answer)
        .into_iter()
        .filter(|q| {
            before.contains(q.as_str())
                && !after.contains(q.as_str())
                && !request_lower.contains(q.as_str())
                && !labels
                    .iter()
                    .chain(&clicked)
                    .any(|l| l.contains(q.as_str()))
        })
        .collect();
    if !stale.is_empty() {
        return Some(format!(
            "Interaction check: \"{}\" appears only on the page before the click, not on the page after it. Answer from the page the click landed on (read it with browser_read).",
            stale.join("\", \"")
        ));
    }
    None
}

fn same_url(a: &str, b: &str) -> bool {
    let norm = |u: &str| {
        let u = u.split('#').next().unwrap_or(u);
        u.trim_end_matches('/')
            .replacen("://www.", "://", 1)
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

pub enum OpenRoute {
    /// The URL is a link on the current page: click it instead.
    Click(Value),
    /// A guessed URL in a click task: refused with this hint.
    Refuse(String),
}

/// A browser_open in a task that says click: a URL that is a link on the
/// current page becomes browser_click on its ref; a URL that is neither a
/// link, the current page nor in the user's words is refused.
pub fn route_open(request: &str, window: &[Value], action: &Value) -> Option<OpenRoute> {
    if action["tool"] != "browser_open" || !needs(request).click {
        return None;
    }
    let url = action["url"].as_str()?;
    let page = window.iter().rev().find(|e| is_page(e))?;
    let current = observation(page);
    let links = current["links"].as_array()?;
    let here = current["url"]
        .as_str()
        .or_else(|| page["page_url"].as_str())
        .unwrap_or("");
    if same_url(url, here)
        || request.split_whitespace().any(|w| {
            same_url(
                w.trim_matches(|c: char| {
                    matches!(c, '"' | '\'' | ',' | '.' | ')' | '(' | '<' | '>')
                }),
                url,
            )
        })
    {
        return None;
    }
    if let Some(link) = links
        .iter()
        .find(|l| l["href"].as_str().is_some_and(|h| same_url(h, url)))
    {
        return Some(OpenRoute::Click(
            json!({"tool":"browser_click","ref":link["ref"]}),
        ));
    }
    let listed: Vec<String> = links
        .iter()
        .take(12)
        .map(|l| {
            format!(
                "{} \"{}\"",
                l["ref"].as_str().unwrap_or(""),
                l["text"].as_str().unwrap_or("")
            )
        })
        .collect();
    Some(OpenRoute::Refuse(format!(
        "Not opened: the task asks to click, and {url} is not a link on the current page ({here}). Use browser_click with the ref of the named link instead of guessing a URL; if the exact text is not listed, click the link that clearly matches it and say which one. Links on this page: {}.",
        if listed.is_empty() {
            "none listed; call browser_read".to_owned()
        } else {
            listed.join(", ")
        }
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(action: &str, data: Value) -> Value {
        json!({"agent":"Assistant","action":action,"ok":true,"data":data.to_string()})
    }

    fn example() -> Value {
        page(
            "browser_open",
            json!({"url":"https://example.com/","title":"Example Domain","text":"This domain is for use in documentation examples. Learn more","links":[{"ref":"L1","text":"Learn more","href":"https://iana.org/domains/example"}]}),
        )
    }

    const TASK: &str = r#"Open https://example.com in the browser, click the "More information..." link, and tell me the title of the page it lands on."#;

    #[test]
    fn needs_reads_imperative_interactions_only() {
        assert!(needs(TASK).click);
        assert!(!needs(TASK).typing);
        assert!(needs("Open the app and type into the search field: cats").typing);
        assert!(needs("follow the first link on https://x.test and summarize").click);
        assert_eq!(needs("Summarize https://example.com"), Needs::default());
        assert_eq!(
            needs("How do I click a button in Selenium?"),
            Needs::default()
        );
        assert_eq!(needs("Write a press release"), Needs::default());
        assert_eq!(
            needs("Open Word notes and write a press release"),
            Needs::default()
        );
        assert!(needs("Open Notepad and press the Save button").click);
        assert_eq!(
            needs("Fetch the GitHub issue rust-lang/rust#100 and summarize it"),
            Needs::default()
        );
    }

    #[test]
    fn a_guessed_url_is_not_a_click() {
        let guessed = page(
            "browser_open",
            json!({"url":"https://example.com/learn-more","title":"Example Domain","text":"This domain is for use in documentation examples. Learn more","links":[]}),
        );
        let window = vec![example(), guessed];
        let p = problem(TASK, r#"The title is "Example Domain"."#, &window, true).unwrap();
        assert!(p.contains("no browser_click"), "{p}");
        assert!(p.contains("not a click"));
        // Without Web or Desktop access the gate does not apply.
        assert!(problem(TASK, "x", &window, false).is_none());
        // Pure read tasks are untouched.
        assert!(problem("Summarize https://example.com", "x", &window, true).is_none());

        match route_open(
            TASK,
            &window[..1],
            &json!({"tool":"browser_open","url":"https://example.com/learn-more"}),
        ) {
            Some(OpenRoute::Refuse(hint)) => {
                assert!(hint.contains("L1 \"Learn more\""), "{hint}");
                assert!(hint.contains("browser_click"));
            }
            _ => panic!("guessed URL must be refused"),
        }
        match route_open(
            TASK,
            &window[..1],
            &json!({"tool":"browser_open","url":"https://www.iana.org/domains/example/"}),
        ) {
            Some(OpenRoute::Click(action)) => {
                assert_eq!(action, json!({"tool":"browser_click","ref":"L1"}))
            }
            _ => panic!("a listed link becomes a click"),
        }
        // The user's own URL and reloads are allowed; non-click tasks too.
        assert!(
            route_open(
                TASK,
                &window[..1],
                &json!({"tool":"browser_open","url":"https://example.com"})
            )
            .is_none()
        );
        assert!(
            route_open(
                "Open https://example.com and https://a.test/x then compare",
                &window[..1],
                &json!({"tool":"browser_open","url":"https://b.test"})
            )
            .is_none()
        );
        assert!(route_open(TASK, &[], &json!({"tool":"browser_open","url":"https://x"})).is_none());
    }

    #[test]
    fn a_click_must_change_the_page_and_back_the_answer() {
        let landed = page(
            "browser_click",
            json!({"url":"https://www.iana.org/help/example-domains","url_before":"https://example.com/","changed":true,"clicked":"Learn more","title":"Example Domains","text":"Example Domains. As described in RFC 2606...","links":[]}),
        );
        let ok = vec![example(), landed.clone()];
        assert!(
            problem(
                TASK,
                r#"I clicked "Learn more"; the title is "Example Domains"."#,
                &ok,
                true
            )
            .is_none()
        );
        // A quoted value seen only before the click is stale.
        let stale = problem(TASK, r#"The title is "This domain is for use"."#, &ok, true).unwrap();
        assert!(
            stale.contains("only on the page before the click"),
            "{stale}"
        );

        let unchanged = page(
            "browser_click",
            json!({"url":"https://example.com/","url_before":"https://example.com/","changed":false,"clicked":"Learn more","title":"Example Domain","text":"This domain is for use in documentation examples. Learn more","links":[]}),
        );
        let p = problem(TASK, "done", &[example(), unchanged.clone()], true).unwrap();
        assert!(p.contains("did not change the page"), "{p}");
        // A later read showing a different page settles it.
        let later = page(
            "browser_read",
            json!({"url":"https://www.iana.org/help/example-domains","title":"Example Domains","text":"Example Domains","links":[]}),
        );
        assert!(
            problem(
                TASK,
                r#""Example Domains""#,
                &[example(), unchanged, later],
                true
            )
            .is_none()
        );

        // Typing tasks need a fill.
        let typing = "Open https://x.test and type into the search field: cats";
        assert!(
            problem(typing, "done", &[example()], true)
                .unwrap()
                .contains("browser_fill")
        );
        let filled = json!({"agent":"Assistant","action":"browser_fill","ok":true,"data":"{}"});
        assert!(problem(typing, "done", &[example(), filled], true).is_none());
        // Desktop clicks count.
        let desktop = json!({"agent":"Assistant","action":"desktop_click","ok":true,"data":"{}"});
        assert!(
            problem(
                "Open the Settings app and click Bluetooth",
                "done",
                &[desktop],
                true
            )
            .is_none()
        );
    }
}
