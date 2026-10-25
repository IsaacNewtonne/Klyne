//! Host-side postcondition checks (2026-10-24 evidence round).
//!
//! Each new evidence entry gets a `delivery` status (confirmed, unknown or
//! failed) and, for common tools, a `verify` fact that Klyne computes itself
//! from the destination: the model never decides these. The reviewer may only
//! accept completion when no acting step is left `unsatisfied`.
use serde_json::{Value, json};
use std::path::Path;

/// Read-only tools: their result is the observation; nothing to verify.
const READS: &[&str] = &[
    "read_file",
    "list_dir",
    "stat_path",
    "search_file",
    "browser_read",
    "browser_open",
    "runtime_status",
    "memory_read",
    "skill_read",
    "capability_list",
    "desktop_observe",
    "desktop_apps",
    "app_list",
    "evidence_read",
    "result_review",
    "loop_check",
];

fn tool_of(action: &str) -> &str {
    action.split(':').next().unwrap_or(action)
}

/// Delivery status from the result alone.
pub fn delivery(entry: &Value) -> &'static str {
    let summary = entry["summary"].as_str().unwrap_or("").to_lowercase();
    if entry["ok"] == true {
        "confirmed"
    } else if summary.contains("uncertain")
        || entry["data"]
            .as_str()
            .is_some_and(|d| d.contains("\"uncertain\":true"))
    {
        "unknown"
    } else {
        "failed"
    }
}

fn result_data(entry: &Value) -> Value {
    let data = entry["data"]
        .as_str()
        .and_then(|d| serde_json::from_str::<Value>(d).ok())
        .unwrap_or(Value::Null);
    if data["data"].is_object() {
        data["data"].clone()
    } else {
        data
    }
}

/// The `launched` object of a desktop_launch result. Results carry a large
/// observation and may be truncated, so fall back to extracting the object.
fn launched_record(entry: &Value) -> Value {
    let data = result_data(entry);
    if data["launched"].is_object() {
        return data["launched"].clone();
    }
    let text = entry["data"].as_str().unwrap_or("");
    let Some(start) = text
        .find("\"launched\":{")
        .map(|i| i + "\"launched\":".len())
    else {
        return Value::Null;
    };
    let (mut depth, mut quoted, mut escaped) = (0, false, false);
    for (i, c) in text[start..].char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '{' if !quoted => depth += 1,
            '}' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&text[start..start + i + 1])
                        .unwrap_or(Value::Null);
                }
            }
            _ => {}
        }
    }
    Value::Null
}

/// Postcondition check for one evidence entry. `present(kind, name)` reports
/// whether a memory/skill/tool currently exists in Klyne's capability store.
pub fn check(entry: &Value, workspace: &Path, present: &dyn Fn(&str, &str) -> bool) -> Value {
    let action = entry["action"].as_str().unwrap_or("");
    let tool = tool_of(action);
    // Typed web errors: a blocked source is "blocked", not unfinished work
    // to retry. NETWORK stays unsatisfied (one retry) until it repeats.
    if let Some(code) = entry["error_code"]
        .as_str()
        .and_then(crate::web_reach::ErrorCode::parse)
        && (code.final_for_source() || entry["repeat_blocked"] == true)
    {
        return json!({"status":"blocked","check":format!("source blocked: {code}"),"hint":entry["hint"]});
    }
    if entry["agent"] == "User approval" || READS.contains(&tool) {
        return json!({"status":"not_applicable","check":"read-only or approval record"});
    }
    if entry["ok"] != true {
        let text = format!("{} {}", entry["summary"], entry["data"]).to_lowercase();
        if (tool.ends_with("_forget") && text.contains("no saved"))
            || text.contains("already removed")
        {
            // Forgetting something already absent still meets the postcondition.
            return json!({"status":"satisfied","check":"already absent"});
        }
        return json!({"status":"unsatisfied","check":"the tool reported failure"});
    }
    let target = action.split_once(':').map(|(_, t)| t).unwrap_or("");
    let fact = |ok: bool, check: String| json!({"status":if ok {"satisfied"} else {"unsatisfied"},"check":check});
    match tool {
        "write_file" | "apply_patch" | "append_file" | "copy_file" | "move_file"
            if !target.is_empty() =>
        {
            let path = workspace.join(target);
            let ok = path.is_file() && path.metadata().is_ok_and(|m| m.len() > 0);
            fact(ok, format!("file {target} exists and is not empty"))
        }
        "create_dir" | "make_dir" if !target.is_empty() => fact(
            workspace.join(target).is_dir(),
            format!("directory {target} exists"),
        ),
        "shell" | "run_shell" | "tool_run" | "tool_test" => {
            let text = format!("{} {}", entry["summary"], entry["data"]).to_lowercase();
            let ok = text.contains("exit code: 0")
                || text.contains("\"exit_code\":0")
                || text.contains("\\\"exit_code\\\":0")
                || (matches!(tool, "tool_run" | "tool_test") && !text.contains("\"ok\":false"));
            fact(ok, "command exit code 0".into())
        }
        "fetch" | "fetch_url" => {
            // An anti-bot challenge page is never accepted as evidence.
            let ok = entry["data"]
                .as_str()
                .is_some_and(|d| !d.trim().is_empty() && !crate::web_reach::is_challenge(d, ""));
            fact(
                ok,
                format!("fetched {target} with content (not a challenge page)"),
            )
        }
        "video_transcript" => {
            let ok = entry["data"].as_str().is_some_and(|d| !d.trim().is_empty());
            fact(ok, format!("transcript of {target} retrieved"))
        }
        "memory_save" | "skill_save" | "tool_save" => {
            let data = result_data(entry);
            let kind = tool.trim_end_matches("_save");
            match data["saved"].as_str() {
                Some(name) => fact(present(kind, name), format!("{kind} {name} present")),
                None => json!({"status":"unknown","check":"no saved name in result"}),
            }
        }
        "memory_forget" | "skill_forget" | "tool_forget" => {
            let data = result_data(entry);
            let kind = tool.trim_end_matches("_forget");
            match data["forgotten"].as_str() {
                Some(name) => fact(!present(kind, name), format!("{kind} {name} absent")),
                None => json!({"status":"unknown","check":"no forgotten name in result"}),
            }
        }
        "delete_path" => {
            let path = workspace.join(target);
            fact(!path.exists(), format!("{target} absent"))
        }
        "desktop_launch" => {
            let launched = launched_record(entry);
            let window = launched["window"]
                .as_str()
                .filter(|w| !w.is_empty() && *w != "0");
            // A launch counts only with a window handle that appeared after
            // it started (new_window); a pid alone or a pre-existing window
            // does not.
            let ok = window.is_some() && launched["new_window"] != false;
            fact(
                ok,
                match window {
                    Some(w) => format!("new window {w} appeared"),
                    None => "no new window appeared".into(),
                },
            )
        }
        _ => json!({"status":"unknown","check":"no host check for this tool"}),
    }
}

/// Adds `delivery` and `verify` to each entry that lacks them.
pub fn annotate(entries: &mut [Value], workspace: &Path, present: &dyn Fn(&str, &str) -> bool) {
    for entry in entries {
        if entry.get("verify").is_some() || !entry.is_object() {
            continue;
        }
        entry["delivery"] = json!(delivery(entry));
        entry["verify"] = check(entry, workspace, present);
    }
}

/// Acting steps whose latest host check is unsatisfied (by action string).
/// Attempts at the same effect share a key, so a later success supersedes
/// an earlier failure: command runs are one family, and path spellings
/// (`.\\a`, `./a`, `a`) are one target.
fn attempt_key(action: &str) -> String {
    let (tool, target) = action.split_once(':').unwrap_or((action, ""));
    let family = match tool {
        "tool_run" | "tool_test" | "shell" | "run_shell" => "command",
        other => other,
    };
    let target = target.replace('\\', "/");
    format!("{family}:{}", target.trim_start_matches("./"))
}

pub fn unsatisfied(evidence: &[Value]) -> Vec<String> {
    let mut latest: Vec<(String, bool)> = Vec::new();
    for entry in evidence {
        let status = entry["verify"]["status"].as_str().unwrap_or("");
        if status == "blocked" {
            // A blocked source supersedes earlier failed attempts at it.
            let key = attempt_key(entry["action"].as_str().unwrap_or(""));
            latest.retain(|(k, _)| *k != key);
            continue;
        }
        if !matches!(status, "satisfied" | "unsatisfied") || entry["agent"] == "Reviewer" {
            continue;
        }
        let key = attempt_key(entry["action"].as_str().unwrap_or(""));
        latest.retain(|(k, _)| *k != key);
        latest.push((key, status == "satisfied"));
    }
    latest
        .into_iter()
        .filter(|(_, ok)| !ok)
        .map(|(k, _)| k)
        .collect()
}

/// True when at least one acting step was verified and none is unsatisfied.
/// Pure answers: nothing unsatisfied and at least one successful
/// non-reviewer result (verify may be n/a for reads).
pub fn answer_supported(evidence: &[Value]) -> bool {
    unsatisfied(evidence).is_empty()
        && evidence
            .iter()
            .any(|e| e["ok"] == true && e["agent"] != "Reviewer" && e["agent"] != "Host verifier")
}

pub fn all_satisfied(evidence: &[Value]) -> bool {
    unsatisfied(evidence).is_empty()
        && evidence
            .iter()
            .any(|e| e["verify"]["status"] == "satisfied" && e["agent"] != "Reviewer")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_forget_of_absent_item_is_satisfied_and_reads_support_answers() {
        let none = |_: &str, _: &str| false;
        let again = json!({"agent":"Assistant","action":"memory_forget","ok":false,"summary":"No saved memory named 'x'"});
        assert_eq!(check(&again, Path::new("."), &none)["status"], "satisfied");
        let read = json!({"agent":"Planner","action":"browser_read","ok":true,"verify":{"status":"not_applicable"}});
        assert!(answer_supported(&[read]));
    }

    #[test]
    fn blocked_web_sources_are_blocked_and_challenges_are_never_evidence() {
        let none = |_: &str, _: &str| false;
        let ws = Path::new(".");
        let blocked = json!({"agent":"worker","action":"fetch:https://x.test","ok":false,"error_code":"PLATFORM_BLOCKED","hint":"h","source":"fetch_url:https://x.test"});
        assert_eq!(check(&blocked, ws, &none)["status"], "blocked");
        let network = json!({"agent":"worker","action":"fetch:https://x.test","ok":false,"error_code":"NETWORK"});
        assert_eq!(check(&network, ws, &none)["status"], "unsatisfied");
        let challenge = json!({"agent":"worker","action":"fetch:https://y.test","ok":true,"data":"<html><title>Just a moment...</title></html>"});
        assert_eq!(check(&challenge, ws, &none)["status"], "unsatisfied");
        let browser = json!({"agent":"worker","action":"browser_open","ok":false,"error_code":"PLATFORM_BLOCKED","source":"browser:https://y.test"});
        assert_eq!(check(&browser, ws, &none)["status"], "blocked");
        assert!(!answer_supported(&[browser]));
        let page =
            json!({"agent":"worker","action":"fetch:https://y.test","ok":true,"data":"Real text"});
        assert_eq!(check(&page, ws, &none)["status"], "satisfied");
        // A failed attempt followed by the host's blocked verdict is not
        // left as unfinished work that would force another retry.
        let mut evidence = vec![network.clone(), network, blocked];
        evidence[1]["repeat_blocked"] = json!(true);
        annotate(&mut evidence, ws, &none);
        assert_eq!(evidence[1]["verify"]["status"], "blocked");
        assert!(unsatisfied(&evidence).is_empty());
    }

    #[test]
    fn later_success_supersedes_a_failed_attempt() {
        let failed =
            json!({"agent":"Assistant","action":"tool_run","verify":{"status":"unsatisfied"}});
        let tested =
            json!({"agent":"Assistant","action":"tool_test","verify":{"status":"satisfied"}});
        assert!(unsatisfied(&[failed, tested]).is_empty());
        let a = json!({"agent":"Assistant","action":"write_file:tools\\a.json","verify":{"status":"unsatisfied"}});
        let b = json!({"agent":"Assistant","action":"write_file:./tools/a.json","verify":{"status":"satisfied"}});
        assert!(unsatisfied(&[a, b]).is_empty());
    }

    #[test]
    fn launch_needs_a_new_window_even_when_truncated_or_reused() {
        let none = |_: &str, _: &str| false;
        let check_data = |data: &str| {
            check(
                &json!({"agent":"A","action":"desktop_launch","ok":true,"data":data}),
                Path::new("."),
                &none,
            )["status"]
                .clone()
        };
        // Truncated observation after the launched object (UWP reuse).
        let truncated = r#"{"observation":{"x":"{"},"launched":{"focus_error":"Windows refused \"focus\"","pid":37440,"window":"36111150","new_window":true,"reused_process":true},"more":[1,2"#;
        assert_eq!(check_data(truncated), "satisfied");
        assert_eq!(check_data(r#"{"launched":{"pid":5}}"#), "unsatisfied");
        assert_eq!(
            check_data(r#"{"launched":{"window":null,"note":"No new window"}}"#),
            "unsatisfied"
        );
        assert_eq!(
            check_data(r#"{"launched":{"pid":5,"window":"9","new_window":false}}"#),
            "unsatisfied"
        );
    }

    #[test]
    fn delivery_and_file_checks() {
        let dir = std::env::temp_dir().join(format!("klyne_verify_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hi").unwrap();
        let none = |_: &str, _: &str| false;
        let ok = json!({"agent":"Assistant","action":"write_file:a.txt","ok":true});
        assert_eq!(delivery(&ok), "confirmed");
        assert_eq!(check(&ok, &dir, &none)["status"], "satisfied");
        let missing = json!({"agent":"Assistant","action":"write_file:b.txt","ok":true});
        assert_eq!(check(&missing, &dir, &none)["status"], "unsatisfied");
        let failed = json!({"agent":"Assistant","action":"write_file:a.txt","ok":false,"summary":"permission denied"});
        assert_eq!(delivery(&failed), "failed");
        let unsure = json!({"ok":false,"summary":"Artifact outcome uncertain"});
        assert_eq!(delivery(&unsure), "unknown");
        let read = json!({"agent":"Assistant","action":"read_file:a.txt","ok":true});
        assert_eq!(check(&read, &dir, &none)["status"], "not_applicable");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn capability_shell_and_launch_checks() {
        let ws = Path::new(".");
        let has_teal = |kind: &str, name: &str| kind == "memory" && name == "teal";
        let saved = json!({"agent":"Assistant","action":"memory_save","ok":true,"data":"{\"saved\":\"teal\",\"kind\":\"memory\"}"});
        assert_eq!(check(&saved, ws, &has_teal)["status"], "satisfied");
        let forgot = json!({"agent":"Assistant","action":"memory_forget","ok":true,"data":"{\"forgotten\":\"teal\",\"kind\":\"memory\"}"});
        assert_eq!(check(&forgot, ws, &has_teal)["status"], "unsatisfied");
        let gone = json!({"agent":"Assistant","action":"tool_forget","ok":true,"data":"{\"forgotten\":\"x\",\"kind\":\"tool\"}"});
        assert_eq!(check(&gone, ws, &has_teal)["status"], "satisfied");
        let shell = json!({"agent":"Assistant","action":"shell:cmd /c echo","ok":true,"summary":"cmd exited with exit code: 0"});
        assert_eq!(check(&shell, ws, &has_teal)["status"], "satisfied");
        let launch = json!({"agent":"Assistant","action":"desktop_launch","ok":true,"data":"{\"launched\":{\"pid\":5,\"window\":\"77\"}}"});
        assert_eq!(check(&launch, ws, &has_teal)["status"], "satisfied");
    }

    #[test]
    fn unsatisfied_tracks_the_latest_attempt() {
        let mut ev = vec![
            json!({"agent":"Assistant","action":"write_file:a","verify":{"status":"unsatisfied"}}),
            json!({"agent":"Assistant","action":"read_file:a","verify":{"status":"not_applicable"}}),
        ];
        assert_eq!(unsatisfied(&ev), vec!["write_file:a".to_owned()]);
        assert!(!all_satisfied(&ev));
        ev.push(
            json!({"agent":"Assistant","action":"write_file:a","verify":{"status":"satisfied"}}),
        );
        assert!(unsatisfied(&ev).is_empty());
        assert!(all_satisfied(&ev));
        assert!(!all_satisfied(&[
            json!({"agent":"Assistant","action":"read_file:a","verify":{"status":"not_applicable"}})
        ]));
    }
}

const NATIVE_BY_NAME: &[&str] = &[
    "memory_forget",
    "skill_forget",
    "tool_forget",
    "tool_run",
    "tool_test",
    "runtime_status",
    "memory_read",
    "skill_read",
    "capability_list",
];

/// Full command line of a run_shell action, unwrapping `powershell -Command`
/// and `cmd /c` wrappers.
fn shell_line(action: &Value) -> String {
    let program = action["program"].as_str().unwrap_or("");
    let args: Vec<String> = action["args"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let base = program
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".exe")
        .to_lowercase();
    if matches!(base.as_str(), "powershell" | "pwsh") {
        if let Some(i) = args
            .iter()
            .position(|a| a.eq_ignore_ascii_case("-command") || a.eq_ignore_ascii_case("-c"))
        {
            return args[i + 1..].join(" ");
        }
        return args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
    }
    if base == "cmd" && args.first().is_some_and(|a| a.eq_ignore_ascii_case("/c")) {
        return args[1..].join(" ");
    }
    std::iter::once(program.to_owned())
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_line(line: &str) -> String {
    line.replace(['\'', '"'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_start_matches("& ")
        .to_lowercase()
}

/// Strip `cmd /c`, `powershell [-flags] -Command` and `&` wrappers from a
/// normalized command line, repeatedly.
pub fn strip_wrappers(line: &str) -> String {
    let mut words: Vec<&str> = line.split_whitespace().collect();
    while let Some(first) = words.first() {
        let base = first
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or("")
            .trim_end_matches(".exe");
        if base == "cmd" && words.get(1).is_some_and(|w| *w == "/c" || *w == "/k") {
            words.drain(..2);
        } else if matches!(base, "powershell" | "pwsh") {
            match words.iter().position(|w| *w == "-command" || *w == "-c") {
                Some(i) => {
                    words.drain(..=i);
                }
                None => {
                    let rest = words.iter().skip(1).position(|w| !w.starts_with('-'));
                    match rest {
                        Some(i) => {
                            words.drain(..=i);
                        }
                        None => break,
                    }
                }
            }
        } else if *first == "&" {
            words.remove(0);
        } else {
            break;
        }
    }
    words.join(" ")
}

/// Whether the request asks to run, test or execute something (negated
/// phrases such as "do not test or run it" do not count).
pub fn asks_to_run(text: &str) -> bool {
    let mut t = format!(
        " {} ",
        text.to_lowercase().replace(['.', ',', ';', '!', '?'], " ")
    );
    for neg in ["do not", "don't", "dont", "without", "no need to", "never"] {
        while let Some(i) = t.find(neg) {
            let end = t[i..]
                .find(" it ")
                .map(|j| i + j + 3)
                .unwrap_or_else(|| (i + neg.len() + 20).min(t.len()));
            t.replace_range(i..end.min(t.len()), " ");
        }
    }
    [
        " run ",
        " test ",
        " execute ",
        " try it ",
        " running ",
        " testing ",
    ]
    .iter()
    .any(|w| t.contains(w))
}

/// Whether the request asks to see contents (show, read, list …), so a
/// read after a verified write is wanted.
pub fn asks_to_show(text: &str) -> bool {
    let t = format!(
        " {} ",
        text.to_lowercase()
            .replace(['.', ',', ';', '!', '?', ':'], " ")
    );
    [
        " show ",
        " display ",
        " print ",
        " contents ",
        " content ",
        " read ",
        " list ",
        " summarize ",
        " summarise ",
        " what's in ",
        " what is in ",
        " tell me ",
        " open ",
    ]
    .iter()
    .any(|w| t.contains(w))
}

const SAVES: &[&str] = &[
    "tool_save",
    "skill_save",
    "memory_save",
    "write_file",
    "append_file",
    "make_dir",
    "document_create",
    "document_save",
    "artifact_write",
];

/// Verified save/write steps in a task window, described for the answer;
/// None when there are none or any check is unmet.
pub fn verified_saves(window: &[Value]) -> Option<Vec<String>> {
    let saves: Vec<String> = window
        .iter()
        .filter(|e| {
            e["agent"] != "Reviewer"
                && e["ok"] == true
                && e["verify"]["status"] == "satisfied"
                && SAVES.contains(&tool_of(e["action"].as_str().unwrap_or("")))
        })
        .map(|e| {
            let action = e["action"].as_str().unwrap_or("");
            let data: Value = e["data"]
                .as_str()
                .and_then(|d| serde_json::from_str(d).ok())
                .unwrap_or(Value::Null);
            match data["name"].as_str() {
                Some(name) if !action.contains(':') => format!("{action} {name}"),
                _ => action.replacen(':', " ", 1),
            }
        })
        .collect();
    (!saves.is_empty() && all_satisfied(window)).then_some(saves)
}

/// A native tool covering a run_shell request, so no shell approval is
/// needed: version checks → runtime_status, a saved tool's own command →
/// tool_run, dir/ls → list_dir, type/cat/Get-Content → read_file,
/// mkdir → make_dir, Get-Item/Test-Path → stat_path. `saved` holds
/// `{name, program, args}` of tools saved in this conversation.
pub fn native_for_shell(action: &Value, saved: &[Value]) -> Option<Value> {
    if action["tool"] != "run_shell" {
        return None;
    }
    let line = normalize_line(&shell_line(action));
    let bare = strip_wrappers(&line);
    for tool in saved {
        let own = normalize_line(&shell_line(
            &json!({"program":tool["program"],"args":tool["args"]}),
        ));
        let full = normalize_line(
            &std::iter::once(tool["program"].as_str().unwrap_or("").to_owned())
                .chain(
                    tool["args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_owned)),
                )
                .collect::<Vec<_>>()
                .join(" "),
        );
        let stripped = line.split('|').next().unwrap_or("").trim().to_owned();
        let code = normalize_line(tool["code"].as_str().unwrap_or(""));
        let bare_stripped = strip_wrappers(&stripped);
        let forms = [strip_wrappers(&own), strip_wrappers(&full)];
        if (!code.is_empty() && (line == code || stripped == code || bare == strip_wrappers(&code)))
            || (tool["program"].is_string()
                && !own.is_empty()
                && (stripped == own
                    || stripped == full
                    || forms
                        .iter()
                        .any(|f| !f.is_empty() && (*f == bare || *f == bare_stripped))))
        {
            return Some(json!({"tool":"tool_run","name":tool["name"]}));
        }
    }
    // A native tool name typed as a shell command ("skill_forget x").
    let first: Vec<&str> = line.split_whitespace().collect();
    if let [verb, rest @ ..] = first.as_slice()
        && NATIVE_BY_NAME.contains(verb)
    {
        let mut native = json!({"tool":verb});
        if let Some(name) = rest.first() {
            native["name"] = json!(name);
        }
        return Some(native);
    }
    if line.contains("--version") && (line.contains("klyne") || line.contains("runtime")) {
        return Some(json!({"tool":"runtime_status"}));
    }
    if line.contains(['|', ';', '>', '&']) {
        return None;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let (verb, rest) = words.split_first()?;
    let path_arg = rest
        .iter()
        .rfind(|w| {
            !w.starts_with('-') && !w.starts_with('/') || w.contains('.') && !w.starts_with('-')
        } && !matches!(**w, "directory" | "file"))
        .map(|w| (*w).to_owned());
    let path = path_arg.clone().unwrap_or_else(|| ".".into());
    match *verb {
        "dir" | "ls" | "get-childitem" | "gci" => Some(json!({"tool":"list_dir","path":path})),
        "type" | "cat" | "get-content" | "gc" if path_arg.is_some() => {
            Some(json!({"tool":"read_file","path":path}))
        }
        "mkdir" | "md" if path_arg.is_some() => Some(json!({"tool":"make_dir","path":path})),
        "new-item" if line.contains("directory") && path_arg.is_some() => {
            Some(json!({"tool":"make_dir","path":path}))
        }
        "get-item" | "test-path" | "gi" if path_arg.is_some() => {
            Some(json!({"tool":"stat_path","path":path}))
        }
        _ => None,
    }
}

/// True when a delete/forget of the same target already ran in this
/// window and its postcondition held.
pub fn already_removed(evidence: &[Value], canonical: &Value) -> bool {
    let tool = canonical["tool"].as_str().unwrap_or("");
    let target = match tool {
        "skill_forget" | "tool_forget" => canonical["name"].as_str(),
        "delete_path" => canonical["path"].as_str(),
        _ => None,
    };
    let Some(target) = target.filter(|t| !t.is_empty()) else {
        return false;
    };
    let leaf = |s: &str| s.rsplit(['\\', '/']).next().unwrap_or(s).to_lowercase();
    evidence.iter().any(|e| {
        let action = e["action"].as_str().unwrap_or("");
        e["agent"] != "Reviewer"
            && e["ok"] == true
            && e["verify"]["status"] == "satisfied"
            && tool_of(action) == tool
            && match tool {
                "delete_path" => action
                    .split_once(':')
                    .is_some_and(|(_, p)| leaf(p) == leaf(target)),
                _ => result_data(e)["forgotten"].as_str() == Some(target),
            }
    })
}

const CLAIMS: &[(&[&str], &[&str])] = &[
    (
        &["removed", "deleted", "forgotten", "forgot", "erased"],
        &[
            "delete_path",
            "memory_forget",
            "skill_forget",
            "tool_forget",
        ],
    ),
    (
        &["saved", "stored", "remembered"],
        &[
            "memory_save",
            "skill_save",
            "tool_save",
            "write_file",
            "document_create",
            "artifact_write",
        ],
    ),
    (
        &["created", "wrote", "written"],
        &[
            "write_file",
            "append_file",
            "apply_patch",
            "copy_file",
            "make_dir",
            "create_dir",
            "memory_save",
            "skill_save",
            "tool_save",
            "document_create",
            "artifact_write",
        ],
    ),
    (
        &["ran", "executed"],
        &["run_shell", "shell", "tool_run", "tool_test"],
    ),
    (
        &["opened", "launched"],
        &["browser_open", "desktop_launch", "open_url", "app_open"],
    ),
];
const NEGATIONS: &[&str] = &[
    " not ",
    "n't",
    " no ",
    "nothing",
    "none",
    " never ",
    "failed",
    "unable",
    "could not",
    "skipped",
    "would ",
];

fn claim_targets(sentence: &str) -> Vec<String> {
    sentence
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '[' | ']'))
        .map(|w| w.trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | '.' | ':' | '!' | '?')))
        .filter(|w| {
            let leaf = w.rsplit(['\\', '/']).next().unwrap_or(w);
            !w.contains("://")
                && leaf.len() > 2
                && (leaf.contains('_')
                    || leaf.rsplit_once('.').is_some_and(|(stem, ext)| {
                        !stem.is_empty()
                            && (1..=5).contains(&ext.len())
                            && ext.chars().all(|c| c.is_ascii_alphabetic())
                    }))
        })
        .map(|w| w.rsplit(['\\', '/']).next().unwrap_or(w).to_lowercase())
        .collect()
}

/// "the saved skill", "a removed file": the participle names a thing rather
/// than claiming an action.
fn adjective_use(words: &[&str], i: usize) -> bool {
    const BEFORE: &[&str] = &["the", "a", "an", "your", "my", "no", "any", "previously"];
    const AFTER: &[&str] = &[
        "tool",
        "tools",
        "skill",
        "skills",
        "memory",
        "memories",
        "capability",
        "capabilities",
        "item",
        "items",
        "file",
        "files",
    ];
    (i > 0 && BEFORE.contains(&words[i - 1])) || words.get(i + 1).is_some_and(|w| AFTER.contains(w))
}

/// Answer for a repeated forget/delete: the latest user message names a
/// target that was already removed and verified in this conversation.
pub fn already_removed_answer(user_text: &str, evidence: &[Value]) -> Option<String> {
    for target in claim_targets(user_text) {
        for tool in ["tool_forget", "skill_forget"] {
            if already_removed(evidence, &json!({"tool":tool,"name":target})) {
                return Some(format!(
                    "Already removed: {target} was deleted and verified earlier. Nothing else to do."
                ));
            }
        }
        if already_removed(evidence, &json!({"tool":"delete_path","path":target})) {
            return Some(format!(
                "Already removed: {target} was deleted and verified earlier. Nothing else to do."
            ));
        }
    }
    None
}

/// Claimed actions in a final answer with no matching successful evidence.
/// Deterministic keyword check: each sentence claiming an effect (removed,
/// saved, created, wrote, ran, opened …) needs a successful, verified tool
/// result of that kind, and every named target (identifiers with `_` or a
/// file extension) must appear in one. "All"/counted removals also need a
/// successful listing after the last removal that no longer shows them.
pub fn unsupported_claims(text: &str, evidence: &[Value]) -> Vec<String> {
    let acted: Vec<&Value> = evidence
        .iter()
        .filter(|e| {
            e["agent"] != "Reviewer" && e["ok"] == true && e["verify"]["status"] != "unsatisfied"
        })
        .collect();
    let mut problems = Vec::new();
    for raw in text
        .split(['.', '!', '\n', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let sentence = format!(" {} ", raw.to_lowercase());
        if NEGATIONS.iter().any(|n| sentence.contains(n)) {
            continue;
        }
        for (verbs, tools) in CLAIMS {
            let words: Vec<&str> = sentence
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .filter(|w| !w.is_empty())
                .collect();
            if !words
                .iter()
                .enumerate()
                .any(|(i, w)| verbs.contains(w) && !adjective_use(&words, i))
            {
                continue;
            }
            let matching: Vec<&&Value> = acted
                .iter()
                .filter(|e| tools.contains(&tool_of(e["action"].as_str().unwrap_or(""))))
                .collect();
            if matching.is_empty() {
                problems.push(format!("\"{raw}\" has no successful {} result", tools[0]));
                continue;
            }
            if tools.contains(&"browser_open") {
                continue;
            }
            for target in claim_targets(&sentence) {
                let seen = matching.iter().any(|e| {
                    format!("{} {}", e["action"], e["data"])
                        .to_lowercase()
                        .contains(&target)
                });
                if !seen {
                    problems.push(format!(
                        "\"{raw}\" names {target}, which no successful result touched"
                    ));
                }
            }
            let listed = sentence.contains(" all ")
                || sentence.contains(" every ")
                || sentence
                    .split_whitespace()
                    .any(|w| w.chars().all(|c| c.is_ascii_digit()) && w.len() < 4 && w != "0")
                || [
                    "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
                ]
                .iter()
                .any(|n| sentence.contains(&format!(" {n} ")));
            if listed && tools.contains(&"delete_path") {
                let last = evidence.iter().rposition(|e| {
                    tools.contains(&tool_of(e["action"].as_str().unwrap_or(""))) && e["ok"] == true
                });
                let listing = last.and_then(|i| {
                    evidence[i + 1..].iter().rev().find(|e| {
                        matches!(
                            tool_of(e["action"].as_str().unwrap_or("")),
                            "capability_list" | "list_dir"
                        ) && e["ok"] == true
                    })
                });
                match listing {
                    None => problems.push(format!(
                        "\"{raw}\" needs a final listing after the removals"
                    )),
                    Some(list) => {
                        let shown = list["data"].as_str().unwrap_or("").to_lowercase();
                        for target in claim_targets(&sentence) {
                            if shown.contains(&format!("\"{target}\"")) {
                                problems.push(format!("\"{raw}\": {target} is still listed"));
                            }
                        }
                    }
                }
            }
        }
    }
    problems
}

#[cfg(test)]
mod claim_tests {
    use super::*;

    fn ok(action: &str, data: &str) -> Value {
        json!({"agent":"Assistant","action":action,"ok":true,"data":data,"verify":{"status":"satisfied"}})
    }

    #[test]
    fn claims_need_matching_evidence_and_listing() {
        let forget = ok(
            "tool_forget",
            r#"{"forgotten":"t70_qwen_date","kind":"tool"}"#,
        );
        let list = json!({"agent":"Assistant","action":"capability_list","ok":true,"data":"{\"items\":[]}","verify":{"status":"not_applicable"}});
        // Supported.
        assert!(
            unsupported_claims("Removed t70_qwen_date.", std::slice::from_ref(&forget)).is_empty()
        );
        // Named target never attempted.
        assert_eq!(
            unsupported_claims(
                "Removed t70_qwen_date and t72_q_echo.",
                std::slice::from_ref(&forget)
            )
            .len(),
            1
        );
        // Nothing attempted at all.
        assert_eq!(
            unsupported_claims("All test tools were removed.", &[]).len(),
            1
        );
        // Negated answers are fine.
        assert!(
            unsupported_claims("No test tools were found, so nothing was removed.", &[]).is_empty()
        );
        // "All" needs a listing after the last removal.
        assert_eq!(
            unsupported_claims("Removed all 2 tools.", std::slice::from_ref(&forget)).len(),
            1
        );
        assert!(unsupported_claims("Removed all 2 tools.", &[forget, list]).is_empty());
        // File claims match by file name.
        let write = ok("write_file:t72_q.txt", "{}");
        assert!(
            unsupported_claims("I created C:\\x\\files\\t72_q.txt with the text.", &[write])
                .is_empty()
        );
    }

    #[test]
    fn shell_requests_map_to_native_tools() {
        let shell = |program: &str, args: &[&str]| json!({"tool":"run_shell","program":program,"args":args});
        let saved = [json!({"name":"t72_echo","program":"cmd","args":["/c","echo","hello"]})];
        assert_eq!(
            native_for_shell(
                &shell(
                    "powershell",
                    &["-NoProfile", "-Command", "cmd /c echo hello | Out-String"]
                ),
                &saved
            )
            .unwrap()["tool"],
            "tool_run"
        );
        assert_eq!(
            native_for_shell(&shell("klyne", &["--version"]), &[]).unwrap()["tool"],
            "runtime_status"
        );
        assert_eq!(
            native_for_shell(
                &shell(
                    "powershell",
                    &["-Command", "& 'C:\\k\\runtime\\versions\\a.exe' --version"]
                ),
                &[]
            )
            .unwrap()["tool"],
            "runtime_status"
        );
        assert_eq!(
            native_for_shell(&shell("cmd", &["/c", "dir", "tools"]), &[]).unwrap(),
            json!({"tool":"list_dir","path":"tools"})
        );
        assert_eq!(
            native_for_shell(
                &shell("powershell", &["-Command", "Get-Content -Raw t72.txt"]),
                &[]
            )
            .unwrap(),
            json!({"tool":"read_file","path":"t72.txt"})
        );
        assert_eq!(
            native_for_shell(&shell("cmd", &["/c", "mkdir", "out"]), &[]).unwrap(),
            json!({"tool":"make_dir","path":"out"})
        );
        assert!(native_for_shell(&shell("cargo", &["build"]), &[]).is_none());
        let code = [json!({"name":"t74_echo","code":"cmd /c echo hello"})];
        assert_eq!(
            native_for_shell(
                &shell(
                    "powershell",
                    &[
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        "cmd /c echo hello"
                    ]
                ),
                &code
            )
            .unwrap()["name"],
            "t74_echo"
        );
        assert_eq!(
            native_for_shell(
                &shell(
                    "powershell",
                    &["-NoProfile", "-Command", "skill_forget t74_echo"]
                ),
                &[]
            )
            .unwrap(),
            json!({"tool":"skill_forget","name":"t74_echo"})
        );
        assert!(
            native_for_shell(&shell("powershell", &["-Command", "Remove-Item x"]), &[]).is_none()
        );
    }

    #[test]
    fn repeated_delete_of_verified_target_is_recognized() {
        let gone = ok("tool_forget", r#"{"forgotten":"x_tool","kind":"tool"}"#);
        assert!(already_removed(
            std::slice::from_ref(&gone),
            &json!({"tool":"tool_forget","name":"x_tool"})
        ));
        assert!(!already_removed(
            &[gone],
            &json!({"tool":"tool_forget","name":"y_tool"})
        ));
        let deleted = ok("delete_path:C:\\w\\old.txt", "{}");
        assert!(already_removed(
            &[deleted],
            &json!({"tool":"delete_path","path":"old.txt"})
        ));
    }
}

/// Read-only shell checks (content, size, hash, existence) that add nothing
/// once Klyne's own postcondition checks hold.
pub fn read_only_shell_check(action: &Value) -> bool {
    if action["tool"] != "run_shell" {
        return false;
    }
    let line = normalize_line(&shell_line(action));
    let mutating = [
        "remove-",
        "set-",
        "new-",
        "out-file",
        "add-content",
        "move-",
        "copy-",
        "del ",
        "rm ",
        ">",
        "start-",
        "stop-",
        "invoke-",
    ];
    let reading = [
        "get-content",
        "measure-object",
        "get-filehash",
        "get-item",
        "test-path",
        "type ",
        "findstr",
        "select-string",
        "certutil -hashfile",
    ];
    !mutating.iter().any(|m| line.contains(m))
        && reading.iter().any(|r| {
            line.starts_with(r)
                || line.contains(&format!("({r}"))
                || line.contains(&format!("; {r}"))
                || line.contains(&format!("| {r}"))
        })
}

#[cfg(test)]
mod read_only_tests {
    #[test]
    fn verification_shells_are_recognized() {
        let a = serde_json::json!({"tool":"run_shell","program":"powershell","args":["-NoProfile","-Command","(Get-Content -Raw -Path t72_o.txt) | Measure-Object -Character"]});
        assert!(super::read_only_shell_check(&a));
        let b = serde_json::json!({"tool":"run_shell","program":"powershell","args":["-Command","Remove-Item x; Get-Content y"]});
        assert!(!super::read_only_shell_check(&b));
    }
}

#[cfg(test)]
mod wrapper_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn cmd_wrapped_saved_command_maps_to_tool_run() {
        let saved = vec![json!({"name":"t_echo","program":"cmd","args":["/c","echo hi"]})];
        let action = json!({"tool":"run_shell","program":"powershell","args":["-NoProfile","-NonInteractive","-Command","cmd /c echo hi"]});
        assert_eq!(
            native_for_shell(&action, &saved).unwrap()["tool"],
            "tool_run"
        );
        let saved = vec![json!({"name":"t_echo","program":"cmd","args":"/c echo hi"})];
        let saved_code = vec![json!({"name":"t_echo","code":"echo hi"})];
        assert_eq!(
            native_for_shell(&action, &saved_code).unwrap()["tool"],
            "tool_run"
        );
        let _ = saved;
    }
    #[test]
    fn strip_wrappers_unwraps_nested() {
        assert_eq!(
            strip_wrappers("powershell -noprofile -command cmd /c echo hi"),
            "echo hi"
        );
        assert_eq!(strip_wrappers("cmd.exe /c & echo hi"), "echo hi");
        assert_eq!(strip_wrappers("echo hi"), "echo hi");
    }
    #[test]
    fn asks_to_run_ignores_negation() {
        assert!(!asks_to_run("Save a tool named x. Do not test or run it."));
        assert!(asks_to_run("Save a tool and run it."));
        assert!(!asks_to_run("Save a tool named t74_echo"));
    }
}

#[cfg(test)]
mod adjective_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn saved_as_adjective_is_not_a_claim() {
        let ev = vec![
            json!({"agent":"Assistant","action":"tool_forget","ok":true,"data":"{\"forgotten\":\"t_x\",\"kind\":\"tool\"}","verify":{"status":"satisfied"}}),
        ];
        assert!(unsupported_claims("The saved skill t_x has been forgotten", &ev).is_empty());
        assert!(!unsupported_claims("Saved t_y to memory", &ev).is_empty());
    }
}

#[cfg(test)]
mod save_finish_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn verified_save_ends_without_reads_unless_contents_are_asked() {
        let w = vec![
            json!({"agent":"Assistant","action":"write_file:a.txt","ok":true,"verify":{"status":"satisfied"}}),
        ];
        assert_eq!(verified_saves(&w).unwrap(), vec!["write_file a.txt"]);
        let bad = vec![
            json!({"agent":"Assistant","action":"write_file:a.txt","ok":true,"verify":{"status":"unsatisfied"}}),
        ];
        assert!(verified_saves(&bad).is_none());
        assert!(asks_to_show("Write a.txt and show me its contents"));
        assert!(!asks_to_show("Save a tool named t_x with program cmd"));
    }
}
