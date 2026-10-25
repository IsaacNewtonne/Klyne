//! Unrequested file writes (2026-10-25). A read or summarize request should
//! be answered in chat; a small model sometimes also saves the answer to a
//! new file. Only clear read-only requests are affected: any word about
//! files, saving, editing or building keeps writes allowed.

use serde_json::{Value, json};

fn words(text: &str) -> String {
    let mut out = String::from(" ");
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else {
            out.push(' ');
        }
    }
    out.push(' ');
    out.split_whitespace()
        .fold(String::from(" "), |mut acc, w| {
            acc.push_str(w);
            acc.push(' ');
            acc
        })
}

/// Words that mean the user wants something written, changed or built.
const WRITE_WORDS: &[&str] = &[
    "save",
    "saves",
    "saved",
    "saving",
    "write",
    "writes",
    "written",
    "writing",
    "create",
    "creates",
    "make",
    "makes",
    "export",
    "exports",
    "file",
    "files",
    "folder",
    "download",
    "store",
    "keep",
    "record",
    "note",
    "notes",
    "jot",
    "log",
    "append",
    "add",
    "edit",
    "rewrite",
    "revise",
    "revision",
    "update",
    "fix",
    "change",
    "rename",
    "move",
    "copy",
    "generate",
    "draft",
    "build",
    "implement",
    "patch",
    "remove",
    "delete",
    "document",
    "workspace",
    "project",
    "repo",
    "code",
    "script",
    "csv",
    "spreadsheet",
    "pdf",
    "doc",
    "docx",
    "markdown",
    "txt",
    "json",
    "html",
    "md",
    "py",
    "rs",
    "js",
    "ts",
    "xlsx",
    "pptx",
    "yaml",
    "yml",
    "toml",
    "xml",
    "output",
    "put",
    "place",
    "attach",
    "compose",
    "poem",
    "story",
    "lyrics",
    "essay",
    "letter",
    "translate",
    "convert",
    "format",
    "install",
    "setup",
    "set",
    "remember",
    "skill",
    "memory",
];

/// Words that mark a read, lookup or summary request.
const READ_WORDS: &[&str] = &[
    "summarize",
    "summarise",
    "summary",
    "summaries",
    "tldr",
    "explain",
    "describe",
    "tell",
    "what",
    "whats",
    "who",
    "when",
    "where",
    "which",
    "why",
    "how",
    "list",
    "find",
    "look",
    "lookup",
    "check",
    "read",
    "fetch",
    "search",
    "compare",
    "show",
    "give",
    "title",
    "is",
    "are",
    "does",
    "did",
    "can",
];

/// True only when the user's own words ask to read, look up or summarize
/// and say nothing about files, saving, editing or building.
pub fn read_only(user_words: &str) -> bool {
    let n = words(user_words);
    let has = |list: &[&str]| list.iter().any(|w| n.contains(&format!(" {w} ")));
    !n.trim().is_empty() && has(READ_WORDS) && !has(WRITE_WORDS)
}

fn scratch(path: &str) -> bool {
    let p = path
        .trim_start_matches("./")
        .replace('\\', "/")
        .to_lowercase();
    ["scratch/", "tmp/", ".klyne/"]
        .iter()
        .any(|d| p.starts_with(d))
}

/// The evidence entry for a skipped write, or None to let it run. Only a
/// worker `write_file` to a new file outside scratch/ or tmp/ on a
/// read-only request is skipped; existing files keep their normal checks.
pub fn skip_write(
    user_words: &str,
    action: &Value,
    exists: &dyn Fn(&str) -> bool,
    agent: &str,
) -> Option<Value> {
    if action["tool"] != "write_file" {
        return None;
    }
    let path = action["path"].as_str()?;
    if path.trim().is_empty() || scratch(path) || exists(path) || !read_only(user_words) {
        return None;
    }
    let hint = format!(
        "Not written: the request does not ask to save or create a file, so {path} was not created. Answer in chat: put the result in your completion summary. Write files only when the user asks to save, write, create or export one."
    );
    Some(json!({
        "agent": agent,
        "action": format!("write_file:{path}"),
        "ok": false,
        "skipped": true,
        "route": "not_requested",
        "summary": hint.clone(),
        "data": hint,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &str) -> Value {
        json!({"tool":"write_file","path":path,"contents":"x"})
    }

    #[test]
    fn read_requests_are_read_only_and_save_requests_are_not() {
        for read in [
            "Summarize GitHub issue https://github.com/rust-lang/rust/issues/100 in two sentences: what failed and how it was resolved.",
            "What is the title of https://example.com?",
            "Fetch the GitHub issue rust-lang/rust#100 and summarize it",
            "Open https://example.com in the browser, click the \"More information...\" link, and tell me the title of the page it lands on.",
        ] {
            assert!(read_only(read), "{read}");
        }
        for write_task in [
            "Summarize rust-lang/rust#100 and save the summary to summary.txt",
            "Write a summary of https://example.com to notes.md",
            "Create a file with today's headlines",
            "Export the issue list as CSV",
            "Read poem.txt and rewrite it as a rap",
            "Explain this code and fix the bug",
            "Remember that my favourite colour is blue",
            "hello there",
            "",
        ] {
            assert!(!read_only(write_task), "{write_task}");
        }
    }

    #[test]
    fn only_new_files_on_read_requests_are_skipped() {
        let read = "Summarize https://github.com/rust-lang/rust/issues/100";
        let none = |_: &str| false;
        let skip = skip_write(read, &write("summary.txt"), &none, "Assistant").unwrap();
        assert_eq!(skip["ok"], false);
        assert_eq!(skip["route"], "not_requested");
        assert!(skip["summary"].as_str().unwrap().contains("Answer in chat"));
        // Saves the user asked for, scratch space, existing files and other
        // tools are untouched.
        assert!(
            skip_write(
                "Summarize it and save to summary.txt",
                &write("summary.txt"),
                &none,
                "Assistant"
            )
            .is_none()
        );
        assert!(skip_write(read, &write("scratch/notes.txt"), &none, "Assistant").is_none());
        assert!(skip_write(read, &write("tmp\\a.txt"), &none, "Assistant").is_none());
        assert!(skip_write(read, &write("summary.txt"), &|_| true, "Assistant").is_none());
        assert!(
            skip_write(
                read,
                &json!({"tool":"read_file","path":"a.txt"}),
                &none,
                "Assistant"
            )
            .is_none()
        );
        assert!(
            skip_write(
                read,
                &json!({"tool":"memory_save","instructions":"x"}),
                &none,
                "Assistant"
            )
            .is_none()
        );
        assert!(skip_write(read, &json!({"tool":"write_file"}), &none, "Assistant").is_none());
    }
}
