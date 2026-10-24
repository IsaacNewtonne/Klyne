//! Transactional metadata and separately addressed history. Legacy JSON snapshots
//! remain readable and migrate on the next successful write.
use crate::{chat::Chat, err};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::{io, path::Path, time::Duration};
fn connection(path: &Path, write: bool) -> io::Result<Connection> {
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(err("Invalid chat database"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(err("Reparse database refused"));
            }
        }
    }
    let db = if write {
        Connection::open(path)
    } else {
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    }
    .map_err(err)?;
    db.busy_timeout(Duration::from_secs(2)).map_err(err)?;
    if write {
        db.execute_batch("PRAGMA synchronous=FULL;").map_err(err)?;
    }
    Ok(db)
}
pub fn save(path: &Path, chat: &Chat) -> io::Result<()> {
    save_with_secrets(path, chat, &crate::chat::chat_secrets(chat))
}
pub(crate) fn save_with_secrets(path: &Path, chat: &Chat, secrets: &[String]) -> io::Result<()> {
    let mut db = connection(path, true)?;
    let tx = db.transaction().map_err(err)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS chat(id INTEGER PRIMARY KEY,payload TEXT NOT NULL);CREATE TABLE IF NOT EXISTS history(kind TEXT NOT NULL,seq INTEGER NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(kind,seq));").map_err(err)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS action_events(seq INTEGER PRIMARY KEY,action TEXT NOT NULL,state TEXT NOT NULL,evidence TEXT NOT NULL);").map_err(err)?;
    let prior: Option<String> = tx
        .query_row("SELECT payload FROM chat WHERE id=1", [], |r| r.get(0))
        .optional()
        .map_err(err)?;
    let prior: Value = prior
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(err)?
        .unwrap_or(Value::Null);
    let before = &prior["pending"];
    let mut after = chat.pending.clone().unwrap_or(Value::Null);
    crate::broker::scrub_value(&mut after, secrets);
    let after = &after;
    if before != after {
        if !before.is_null() && !after.is_null() {
            return Err(err("Cannot replace an unresolved action"));
        }
        let (mut action, state, evidence) = if before.is_null() {
            (
                after.clone(),
                if after.get("proposal").is_some() {
                    "proposed"
                } else {
                    "dispatched"
                },
                Value::Null,
            )
        } else {
            // A returned tool call is not proof of task success. Preserve the
            // observation or explicit reconciliation without calling it verified.
            let mut evidence = chat.evidence.last().cloned().unwrap_or(Value::Null);
            crate::broker::scrub_value(&mut evidence, secrets);
            let state = if before.get("proposal").is_some() {
                if chat
                    .execution
                    .approval_queue
                    .iter()
                    .any(|p| p["request_id"] == before["request_id"])
                {
                    "queued"
                } else {
                    "resolved"
                }
            } else if evidence["agent"] == "User reconciliation" {
                "reconciled"
            } else {
                "returned"
            };
            let mut action = before.clone();
            crate::broker::scrub_value(&mut action, secrets);
            (action, state, evidence)
        };
        crate::broker::scrub_value(&mut action, secrets);
        tx.execute(
            "INSERT INTO action_events(action,state,evidence) VALUES(?1,?2,?3)",
            params![action.to_string(), state, evidence.to_string()],
        )
        .map_err(err)?;
    }
    let mut metadata = serde_json::to_value(chat).map_err(err)?;
    // Centralized secret hygiene (audit Phase 2): granted secret values
    // never persist. Values come from the process environment for names
    // the user bound, so only live secrets scrub — and only values long
    // enough to be unambiguous.
    crate::broker::scrub_value(&mut metadata, secrets);
    // Append in the same transaction as the snapshot. Short calls remain visible
    // between polls; IDs correlate dispatch and return without timing guesses.
    let old_count = tx
        .query_row(
            "SELECT count(*) FROM history WHERE kind='evidence'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(err)? as usize;
    let mut events = prior["activity_events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut seq = events.last().and_then(|e| e["seq"].as_u64()).unwrap_or(0);
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut emit = |kind: &str, data: Value| {
        seq += 1;
        events.push(serde_json::json!({"seq":seq,"at":at,"type":kind,"data":data}));
    };
    let mut operation = prior["visual_operation"].clone();
    if before != after {
        if !after.is_null() && after.get("proposal").is_none() {
            operation = serde_json::json!({"id":format!("{}:{}",chat.id,tx.last_insert_rowid()),"action":after.get("action").cloned().unwrap_or_else(||after.clone()),"agent":after["agent"],"direction":"outbound"});
            emit("tool:start", operation.clone());
        } else if !before.is_null() && before.get("proposal").is_none() && !operation.is_null() {
            let ok = chat.evidence.len() > old_count
                && chat.evidence.last().is_some_and(|e| e["ok"] == true);
            operation["direction"] = Value::String("inbound".into());
            emit("tool:result", operation.clone());
            emit(
                if ok { "tool:complete" } else { "tool:error" },
                operation.clone(),
            );
            operation = Value::Null;
        }
    }
    // Waiting is reported only from an explicit runtime wait, never a timer.
    if !operation.is_null()
        && prior["activity"] != metadata["activity"]
        && chat
            .activity
            .as_ref()
            .is_some_and(|a| a["kind"] == "waiting")
    {
        let mut waiting = operation.clone();
        waiting["status"] = Value::String("waiting".into());
        waiting["operation"] = Value::String("Awaiting response".into());
        emit("tool:waiting", waiting);
    }
    if prior["status"] != metadata["status"] {
        emit(
            "workflow:stage_changed",
            serde_json::json!({"status":chat.status}),
        );
        let mode = match chat.status.as_str() {
            "Completed" => Some("klyne:complete"),
            "Blocked" | "Interrupted" => Some("klyne:error"),
            "Needs input" => Some("klyne:input"),
            "Stopped" => Some("klyne:idle"),
            _ => None,
        };
        if let Some(mode) = mode {
            emit(mode, serde_json::json!({}));
        }
    }
    if prior["activity"] != metadata["activity"]
        && matches!(
            chat.status.as_str(),
            "Working" | "Planning" | "Reviewing" | "Upgrading" | "Stopping"
        )
    {
        emit(
            if chat.activity.is_some() {
                "klyne:waiting"
            } else {
                "klyne:thinking"
            },
            serde_json::json!({"activity":chat.activity}),
        );
    }
    if chat.evidence.len() > old_count {
        emit(
            "observation:new",
            serde_json::json!({"count":chat.evidence.len(),"added":chat.evidence.len()-old_count}),
        );
    }
    if events.len() > 256 {
        events.drain(..events.len() - 256);
    }
    metadata["activity_events"] = Value::Array(events);
    metadata["visual_operation"] = operation;
    crate::broker::scrub_value(&mut metadata, secrets);
    for kind in ["messages", "evidence"] {
        let entries = metadata.as_object_mut().unwrap().remove(kind).unwrap();
        let entries = entries.as_array().unwrap();
        tx.execute(
            "DELETE FROM history WHERE kind=?1 AND seq>=?2",
            params![kind, entries.len() as i64],
        )
        .map_err(err)?;
        let mut stmt=tx.prepare("INSERT INTO history VALUES(?1,?2,?3) ON CONFLICT(kind,seq) DO UPDATE SET payload=excluded.payload WHERE history.payload<>excluded.payload").map_err(err)?;
        for (seq, entry) in entries.iter().enumerate() {
            stmt.execute(params![kind, seq as i64, entry.to_string()])
                .map_err(err)?;
        }
    }
    tx.execute(
        "INSERT INTO chat VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
        [metadata.to_string()],
    )
    .map_err(err)?;
    tx.commit().map_err(err)
}
pub fn load(path: &Path) -> io::Result<Chat> {
    let mut db = connection(path, false)?;
    let tx = db.transaction().map_err(err)?;
    let payload: String = tx
        .query_row("SELECT payload FROM chat WHERE id=1", [], |r| r.get(0))
        .map_err(err)?;
    let mut data: Value = serde_json::from_str(&payload).map_err(err)?;
    for kind in ["messages", "evidence"] {
        if data.get(kind).is_none() {
            let mut stmt = tx
                .prepare("SELECT payload FROM history WHERE kind=?1 ORDER BY seq")
                .map_err(err)?;
            let rows = stmt
                .query_map([kind], |r| r.get::<_, String>(0))
                .map_err(err)?;
            let mut values = Vec::new();
            for row in rows {
                values.push(serde_json::from_str::<Value>(&row.map_err(err)?).map_err(err)?);
            }
            data[kind] = Value::Array(values);
        }
    }
    serde_json::from_value(data).map_err(err)
}
pub fn summary(path: &Path) -> io::Result<Value> {
    let db = connection(path, false)?;
    let payload: String = db
        .query_row("SELECT payload FROM chat WHERE id=1", [], |r| r.get(0))
        .map_err(err)?;
    let mut data: Value = serde_json::from_str(&payload).map_err(err)?;
    // Messages live in the history table; the last few are enough to tell
    // an unreachable model apart from other stops.
    if data["messages"].as_array().is_none_or(|m| m.is_empty()) {
        let mut stmt = db
            .prepare("SELECT payload FROM history WHERE kind='messages' ORDER BY seq DESC LIMIT 4")
            .map_err(err)?;
        let mut recent: Vec<Value> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .filter_map(Result::ok)
            .filter_map(|p| serde_json::from_str(&p).ok())
            .collect();
        recent.reverse();
        data["messages"] = Value::Array(recent);
    }
    Ok(
        serde_json::json!({"id":data["id"],"title":data["title"],"status":data["status"],"attention":attention(&data)}),
    )
}
/// Display-only sub-kind for a stopped conversation, so the sidebar can tell
/// "needs your approval" from "needs your answer", "couldn't finish" and
/// "error" without loading the full snapshot. Never used for decisions.
pub fn attention(data: &Value) -> &'static str {
    let status = data["status"].as_str().unwrap_or("");
    if matches!(
        status,
        "Planning" | "Working" | "Reviewing" | "Stopping" | "Upgrading"
    ) {
        return "";
    }
    if data["pending"]["proposal"].is_object() {
        return "approval";
    }
    if status == "Needs input" || data["execution"]["failure"]["kind"] == "missing_input" {
        return "input";
    }
    if data["pending"].is_object() {
        return "uncertain";
    }
    if status == "Completed" {
        return "";
    }
    let last = data["messages"]
        .as_array()
        .and_then(|m| m.iter().rev().find(|m| m["agent"] == "Klyne"))
        .and_then(|m| m["text"].as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let unreachable = [
        "connection failed",
        "could not connect",
        "connection refused",
        "not reachable",
        "error sending request",
    ];
    if unreachable.iter().any(|n| last.contains(n)) {
        return "error";
    }
    if matches!(status, "Blocked" | "Interrupted" | "Stopped") {
        return "blocked";
    }
    ""
}
/// Sequenced activity pulse: status, last event sequence, events after
/// `since`, and the content-free latency breakdown. Lets the UI skip full
/// snapshot fetches when nothing changed; the full snapshot remains the
/// fallback for rendering. Reads one row, never the history tables.
pub fn pulse(path: &Path, since: u64) -> io::Result<Value> {
    let db = connection(path, false)?;
    let payload: String = db
        .query_row("SELECT payload FROM chat WHERE id=1", [], |r| r.get(0))
        .map_err(err)?;
    let data: Value = serde_json::from_str(&payload).map_err(err)?;
    let events = data["activity_events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let seq = events.last().and_then(|e| e["seq"].as_u64()).unwrap_or(0);
    let fresh: Vec<Value> = events
        .into_iter()
        .filter(|e| e["seq"].as_u64().is_some_and(|s| s > since))
        .collect();
    let trace: crate::performance::Trace =
        serde_json::from_value(data["execution"]["trace"].clone()).unwrap_or_default();
    let model_calls = data["execution"]["model_timings"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    Ok(serde_json::json!({
        "id": data["id"],
        "status": data["status"],
        "seq": seq,
        "events": fresh,
        "trace": trace.report(
            model_calls,
            data["prompt_tokens"].as_u64().unwrap_or(0),
            data["completion_tokens"].as_u64().unwrap_or(0),
        ),
    }))
}

#[cfg(test)]
mod attention_tests {
    use super::attention;
    use serde_json::json;
    #[test]
    fn sub_kinds() {
        assert_eq!(
            attention(&json!({"status":"Interrupted","pending":{"proposal":{"kind":"shell"}}})),
            "approval"
        );
        assert_eq!(
            attention(&json!({"status":"Interrupted","pending":{"action":{}}})),
            "uncertain"
        );
        assert_eq!(attention(&json!({"status":"Needs input"})), "input");
        assert_eq!(
            attention(
                &json!({"status":"Blocked","messages":[{"agent":"Klyne","text":"Connection failed before request dispatch"}]})
            ),
            "error"
        );
        assert_eq!(
            attention(
                &json!({"status":"Blocked","messages":[{"agent":"Klyne","text":"I ran out of checks."}]})
            ),
            "blocked"
        );
        assert_eq!(
            attention(&json!({"status":"Working","pending":{"proposal":{}}})),
            ""
        );
        assert_eq!(attention(&json!({"status":"Completed"})), "");
    }
}
