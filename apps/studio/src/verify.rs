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

/// Postcondition check for one evidence entry. `present(kind, name)` reports
/// whether a memory/skill/tool currently exists in Klyne's capability store.
pub fn check(entry: &Value, workspace: &Path, present: &dyn Fn(&str, &str) -> bool) -> Value {
    let action = entry["action"].as_str().unwrap_or("");
    let tool = tool_of(action);
    if entry["agent"] == "User approval" || READS.contains(&tool) {
        return json!({"status":"not_applicable","check":"read-only or approval record"});
    }
    if entry["ok"] != true {
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
            let ok = entry["data"].as_str().is_some_and(|d| !d.trim().is_empty());
            fact(ok, format!("fetched {target} with content"))
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
        "desktop_launch" => {
            let data = result_data(entry);
            let ok = data["launched"]["pid"].as_u64().is_some();
            fact(ok, "launched process reported".into())
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
pub fn unsatisfied(evidence: &[Value]) -> Vec<String> {
    let mut latest: Vec<(String, bool)> = Vec::new();
    for entry in evidence {
        let status = entry["verify"]["status"].as_str().unwrap_or("");
        if !matches!(status, "satisfied" | "unsatisfied") || entry["agent"] == "Reviewer" {
            continue;
        }
        let key = entry["action"].as_str().unwrap_or("").to_owned();
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
        let launch = json!({"agent":"Assistant","action":"desktop_launch","ok":true,"data":"{\"launched\":{\"pid\":5}}"});
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
