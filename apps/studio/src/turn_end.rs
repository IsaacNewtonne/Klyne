//! Turn-end verdict and narration guard (2026-10-25).
//!
//! Adapted from Mercury Agent (MIT, (c) 2026 Cosmic Stack): a turn ends
//! for a reason, and only a deliberate stop can become Done. Running out
//! of steps is a pause with a "you are NOT done" nudge for the resumed
//! run. The narration guard refuses a finish for a change request when no
//! write or command ran, and makes the next step offer only acting tools.
use serde_json::Value;

/// Why a turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The model chose to finish and the host gates accepted it.
    ChoseToStop,
    /// The step budget ran out mid-task: a pause, never Done.
    StepsExhausted,
    /// The model connection failed or the stall watchdog stopped it.
    Interrupted,
    /// The model's output was cut off.
    Truncated,
    /// The user pressed Stop.
    Aborted,
    /// Anything else that blocked the turn.
    Blocked,
}
impl Ending {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChoseToStop => "chose_to_stop",
            Self::StepsExhausted => "steps_exhausted",
            Self::Interrupted => "interrupted",
            Self::Truncated => "truncated",
            Self::Aborted => "aborted",
            Self::Blocked => "blocked",
        }
    }
}

/// Marker the Ollama adapter puts on a cut-off reply.
pub const TRUNCATED_MARKER: &str = "model output was cut off";

/// Classify a turn that ended with an error, in priority order.
pub fn classify_error(stopped: bool, steps_exhausted: bool, error: &str) -> Ending {
    let error = error.to_lowercase();
    if stopped {
        Ending::Aborted
    } else if error.contains(TRUNCATED_MARKER) {
        Ending::Truncated
    } else if steps_exhausted {
        Ending::StepsExhausted
    } else if [
        "model server",
        "connection",
        "timed out",
        "could not reach",
        "error sending request",
        "ollama reported an error",
    ]
    .iter()
    .any(|m| error.contains(m))
    {
        Ending::Interrupted
    } else {
        Ending::Blocked
    }
}

/// Shown to the user when the step budget ends a turn.
pub fn steps_paused_message(used: u64, limit: u64) -> String {
    format!(
        "Paused: this goal used its step budget ({used} of {limit} steps). The task is NOT done. Raise the step limit and press Resume work to continue where I left off."
    )
}

/// One-shot nudge for the first model call after such a pause.
pub const STEPS_EXHAUSTED_NUDGE: &str = "[STEP BUDGET] The previous run stopped because the step budget ran out. The task is NOT done: do not summarize or wrap up. Resume exactly where you left off and take the next concrete action.";

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '.' && c != '_' && c != '-')
        .map(|w| w.trim_matches('.').to_owned())
        .filter(|w| !w.is_empty())
        .collect()
}

fn file_target(text: &str, words: &[String]) -> bool {
    let lower = text.to_lowercase();
    words.iter().any(|w| {
        matches!(
            w.as_str(),
            "file" | "files" | "folder" | "folders" | "directory" | "document" | "script" | "disk"
        ) || w.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        })
    }) || lower.contains("to the workspace")
}

/// Whether the user's words ask for a change that needs a write or a
/// command (not just an answer in chat).
pub fn action_request(text: &str, terminal: bool) -> bool {
    let w = words(text);
    let lower = text.to_lowercase();
    if lower.starts_with("how do i") || lower.starts_with("how to") || lower.starts_with("explain")
    {
        return false;
    }
    let has = |list: &[&str]| w.iter().any(|x| list.contains(&x.as_str()));
    if has(&["save", "export", "download", "install"]) {
        return true;
    }
    if has(&[
        "write", "create", "make", "generate", "append", "rename", "move", "copy", "edit",
        "modify", "update", "patch",
    ]) && file_target(text, &w)
    {
        return true;
    }
    terminal && has(&["run", "execute", "build", "compile"])
}

const MUTATING: &[&str] = &[
    "write_file",
    "patch_file",
    "make_dir",
    "copy_file",
    "move_file",
    "delete_path",
    "run_shell",
    "document_create",
    "artifact_write",
    "tool_save",
    "tool_run",
    "skill_save",
    "memory_save",
    "browser_fill",
    "browser_click",
    "desktop_click",
    "desktop_type",
    "desktop_key",
    "desktop_fill",
    "desktop_invoke",
    "desktop_launch",
    "desktop_drag",
    "desktop_clipboard_set",
    "app_invoke",
    "app_call",
];

/// Tools the narration guard offers (compact mode) for the forced step.
pub fn mutating_tool(name: &str) -> bool {
    MUTATING.contains(&name)
}

/// Whether any write, command or other acting tool ran (or was tried) in
/// this window, or a host receipt exists.
pub fn has_attempted_change(evidence: &[Value]) -> bool {
    evidence.iter().any(|e| {
        e.get("receipt").is_some_and(|r| !r.is_null())
            || e["action"]
                .as_str()
                .map(|a| a.split(':').next().unwrap_or(a))
                .is_some_and(|tool| tool == "shell" || mutating_tool(tool))
    })
}

/// The narration guard: a change request whose finish has no write or
/// command behind it.
pub fn narrated(request: &str, terminal: bool, evidence: &[Value]) -> bool {
    action_request(request, terminal) && !has_attempted_change(evidence)
}

/// Words that mean the request needs live data, a source or an action,
/// so a tool-free answer is not enough.
const NEEDS_SOURCE: &[&str] = &[
    "search",
    "look",
    "lookup",
    "find",
    "fetch",
    "browse",
    "open",
    "click",
    "download",
    "latest",
    "current",
    "currently",
    "today",
    "tonight",
    "tomorrow",
    "yesterday",
    "news",
    "price",
    "prices",
    "weather",
    "score",
    "stock",
    "summarize",
    "summarise",
    "summary",
    "read",
    "check",
    "verify",
    "send",
    "email",
    "post",
    "run",
    "install",
    "list",
    "show",
    "github",
    "issue",
    "repo",
    "website",
    "site",
    "page",
    "link",
    "url",
    "screen",
    "app",
    "window",
    "desktop",
    "browser",
    "workspace",
];

/// A knowledge or opinion question that a direct answer can settle: no
/// URL, file, action or live data (2026-10-25, reviewer meta-question fix).
pub fn direct_question(request: &str) -> bool {
    let lower = request.to_lowercase();
    let w = words(request);
    !request.trim().is_empty()
        && !lower.contains("http")
        && !lower.contains("www.")
        && !request.contains('/')
        && !request.contains('\\')
        && !file_target(request, &w)
        && !action_request(request, true)
        && !w.iter().any(|x| NEEDS_SOURCE.contains(&x.as_str()))
}

/// A reviewer question that only asks the user to confirm the answer is
/// good enough. The reviewer's job is to decide that itself.
pub fn sufficiency_question(question: &str) -> bool {
    let lower = question.to_lowercase().replace('\u{2019}', "'");
    [
        "is this sufficient",
        "is that sufficient",
        "is this enough",
        "is that enough",
        "is the answer sufficient",
        "is this answer sufficient",
        "sufficient, or",
        "sufficient or do you",
        "additional verification",
        "further verification",
        "need me to verify",
        "like me to verify",
        "should i verify",
        "want me to verify",
        "is this acceptable",
        "is this answer acceptable",
        "is this satisfactory",
        "does this answer your",
        "does this meet your",
        "are you satisfied",
        "is this what you wanted",
        "anything else you need",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

/// The user states a fact or preference to keep ("Remember that I prefer
/// ..."). Auto-memory records it; the turn only needs a brief confirmation.
pub fn memory_statement(request: &str) -> bool {
    let lower = request.to_lowercase().replace('\u{2019}', "'");
    let lower = lower.trim();
    if lower.ends_with('?')
        || [
            "do you remember",
            "what do you remember",
            "can you remember",
            "what did i",
        ]
        .iter()
        .any(|p| lower.contains(p))
    {
        return false;
    }
    [
        "remember that",
        "remember i ",
        "remember my ",
        "remember: ",
        "please remember",
        "from now on",
        "note that i",
        "keep in mind",
        "for future reference",
        "don't forget that",
    ]
    .iter()
    .any(|p| {
        lower.starts_with(p)
            || lower.contains(&format!(" {p}"))
            || lower.contains(&format!(". {p}"))
    })
}

/// A short, plain confirmation reply (no refusal, no question).
pub fn acknowledgement_reply(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && t.chars().count() <= 300
        && !t.starts_with('{')
        && !t.contains('?')
        && !crate::native_tools::refusal(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn errors_classify_in_priority_order() {
        assert_eq!(classify_error(true, true, "x"), Ending::Aborted);
        assert_eq!(
            classify_error(false, true, "Model output was cut off at the length limit"),
            Ending::Truncated
        );
        assert_eq!(
            classify_error(false, true, "This goal reached its configured step limit."),
            Ending::StepsExhausted
        );
        assert_eq!(
            classify_error(false, false, "error sending request for url"),
            Ending::Interrupted
        );
        assert_eq!(
            classify_error(false, false, "Desktop result was not confirmed"),
            Ending::Blocked
        );
        assert!(steps_paused_message(12, 12).contains("NOT done"));
        assert!(STEPS_EXHAUSTED_NUDGE.contains("NOT done"));
    }

    #[test]
    fn change_requests_are_told_apart_from_answers() {
        assert!(action_request(
            "Summarize the issue and save the summary to summary.txt",
            false
        ));
        assert!(action_request("Write the list to notes.md", false));
        assert!(action_request("create a folder called reports", false));
        assert!(action_request("run the tests", true));
        assert!(!action_request("run the tests", false));
        assert!(!action_request("Write a short poem about rain", false));
        assert!(!action_request(
            "Summarize GitHub issue https://github.com/rust-lang/rust/issues/100 in two sentences",
            false
        ));
        assert!(!action_request("How do I save a file in vim?", false));
        assert!(!action_request("What version is 1.2.3?", false));
    }

    #[test]
    fn narration_needs_an_attempted_change() {
        let ask = "Save the summary to summary.txt";
        let read = [json!({"action":"fetch_url:https://x","ok":true})];
        assert!(narrated(ask, false, &read));
        let wrote = [
            json!({"action":"write_file:summary.txt","ok":true,"receipt":{"effect":"file_write"}}),
        ];
        assert!(!narrated(ask, false, &wrote));
        let failed = [json!({"action":"write_file","ok":false})];
        assert!(!narrated(ask, false, &failed));
        assert!(!narrated("Summarize the page", false, &read));
        assert!(mutating_tool("run_shell") && !mutating_tool("read_file"));
    }
    #[test]
    fn direct_questions_memory_statements_and_meta_questions() {
        assert!(direct_question(
            "What colour should I paint a small north-facing room? Answer briefly."
        ));
        assert!(direct_question("Why is the sky blue?"));
        assert!(!direct_question(
            "Summarize https://github.com/rust-lang/rust/issues/100"
        ));
        assert!(!direct_question("What is the weather today in Hanoi?"));
        assert!(!direct_question("Write greeting.txt with hello"));
        assert!(!direct_question("What's in notes.txt?"));
        assert!(!direct_question("Inspect /tmp/x/Private album"));
        assert!(sufficiency_question(
            "The Assistant responded with advice. Is this sufficient, or do you need me to provide additional verification (e.g., checking web sources)?"
        ));
        assert!(!sufficiency_question(
            "Which city should the forecast be for?"
        ));
        assert!(memory_statement(
            "Remember that I prefer answers in British English and I like short replies."
        ));
        assert!(memory_statement("From now on, call me Zig."));
        assert!(!memory_statement("Do you remember what I prefer?"));
        assert!(!memory_statement("Summarize this page"));
        assert!(acknowledgement_reply(
            "Understood. I'll respond in British English with concise replies, as you prefer."
        ));
        assert!(!acknowledgement_reply("What should I remember?"));
        assert!(!acknowledgement_reply(
            "I don't have access to your file system."
        ));
    }
}
