//! Durable interval jobs. A committed dispatch claim precedes chat admission.
//! An unresolved claim is paused after restart, never blindly dispatched again.
use crate::{Studio, err};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io,
    path::Path,
    sync::{Arc, atomic::Ordering},
};

#[derive(Serialize, Deserialize)]
struct Job {
    id: String,
    name: String,
    interval_seconds: u64,
    next_run_ms: u64,
    enabled: bool,
    state: String,
    last_chat: Option<String>,
    note: String,
    request: Value,
}
fn database(root: &Path) -> io::Result<Connection> {
    crate::safe_dir(root)?;
    let path = root.join("schedules.sqlite3");
    if path.exists() {
        let meta = std::fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(err("Invalid schedule database"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(err("Reparse schedule database refused"));
            }
        }
    }
    let db = Connection::open(path).map_err(err)?;
    db.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(err)?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS schedules (id TEXT PRIMARY KEY, payload TEXT NOT NULL);",
    )
    .map_err(err)?;
    Ok(db)
}
fn jobs(db: &Connection) -> io::Result<Vec<Job>> {
    let mut stmt = db
        .prepare("SELECT payload FROM schedules ORDER BY id")
        .map_err(err)?;
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(err)?
        .map(|row| serde_json::from_str(&row.map_err(err)?).map_err(err))
        .collect()
}
fn save(db: &Connection, job: &Job) -> io::Result<()> {
    db.execute("INSERT INTO schedules VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
        params![job.id, serde_json::to_string(job).map_err(err)?]).map_err(err)?;
    Ok(())
}
pub fn list(root: &Path) -> io::Result<Value> {
    if !root.join("schedules.sqlite3").try_exists()? {
        return Ok(json!({"jobs":[]}));
    }
    let jobs = jobs(&database(root)?)?;
    Ok(
        json!({"jobs":jobs.iter().map(|j| json!({"id":j.id,"name":j.name,
        "interval_seconds":j.interval_seconds,"next_run_ms":j.next_run_ms,"enabled":j.enabled,
        "state":j.state,"last_chat":j.last_chat,"note":j.note})).collect::<Vec<_>>()}),
    )
}
/// Caller holds Studio admission, shared with scheduled dispatch and quiescing.
pub fn manage(studio: &Arc<Studio>, body: &Value) -> io::Result<Value> {
    let db = database(&studio.root)?;
    let now = crate::desktop::now_ms();
    if body["action"] == "create" {
        if jobs(&db)?.len() >= 100 {
            return Err(err("Schedule limit reached (100)"));
        }
        let id = body["chat_id"]
            .as_str()
            .ok_or_else(|| err("Choose a completed conversation"))?;
        let chat = studio.chats.get(id)?;
        if chat.status != "Completed" || chat.pending.is_some() || chat.prompt_maker {
            return Err(err(
                "Only completed general-assistant tasks can be scheduled",
            ));
        }
        let interval = body["interval_seconds"]
            .as_u64()
            .filter(|n| (60..=31_536_000).contains(n))
            .ok_or_else(|| err("Choose an interval between one minute and one year"))?;
        let name = body["name"].as_str().unwrap_or(&chat.title).trim();
        if name.is_empty() || name.len() > 200 {
            return Err(err("Schedule name must be 1–200 bytes"));
        }
        let bounded = |value: u64, default: u64| if value == 0 { default } else { value };
        // Capture settings, never approval grants, pending actions or model-written data.
        let mut request = json!({"message":chat.execution.original_request,"provider":chat.provider,
            "decisions":chat.execution.decisions,"learn_procedures":chat.execution.learn_procedures,
            "reviewer":chat.execution.reviewer,"access":chat.access,
            "execution":{"max_steps":bounded(chat.execution.max_steps,64),
                "timeout_seconds":bounded(chat.execution.timeout_seconds,600),
                "max_tokens":bounded(chat.execution.max_tokens,100_000),
                "max_review_rounds":bounded(chat.execution.max_review_rounds,3),
                "max_cost_usd":chat.execution.max_cost_usd,"command_policy":chat.execution.command_policy}});
        if chat.access.terminal {
            request["workspace"] = json!(chat.workspace);
        }
        if let Some(contract) = chat.contract {
            request["contract"] = json!(contract);
        }
        let job = Job {
            id: format!("{now}-{}", jobs(&db)?.len()),
            name: name.into(),
            interval_seconds: interval,
            next_run_ms: now.saturating_add(interval * 1000),
            enabled: true,
            state: "Waiting".into(),
            last_chat: None,
            note: "Runs while Klyne is open or running in background mode.".into(),
            request,
        };
        // Millisecond collisions after deletion must not overwrite a different job.
        db.execute(
            "INSERT INTO schedules VALUES (?1,?2)",
            params![job.id, serde_json::to_string(&job).map_err(err)?],
        )
        .map_err(err)?;
        return Ok(json!({"id":job.id}));
    }
    let id = body["id"]
        .as_str()
        .ok_or_else(|| err("Missing schedule ID"))?;
    let payload: String = db
        .query_row("SELECT payload FROM schedules WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(err)?
        .ok_or_else(|| err("Schedule not found"))?;
    let mut job: Job = serde_json::from_str(&payload).map_err(err)?;
    match body["action"].as_str() {
        Some("delete") => {
            db.execute("DELETE FROM schedules WHERE id=?1", [id])
                .map_err(err)?;
        }
        Some("pause") => {
            job.enabled = false;
            job.note =
                "Future runs paused. Any active conversation continues and can be stopped there."
                    .into();
            save(&db, &job)?;
        }
        Some("enable") => {
            if let Some(id) = &job.last_chat {
                let chat = studio.chats.get(id)?;
                if chat.pending.is_some()
                    || !matches!(
                        chat.status.as_str(),
                        "Completed" | "Stopped" | "Blocked" | "Needs input" | "Interrupted"
                    )
                {
                    return Err(err(
                        "Resolve or finish the previous conversation before enabling future runs",
                    ));
                }
            }
            job.enabled = true;
            job.state = "Waiting".into();
            job.last_chat = None;
            job.next_run_ms = now.saturating_add(job.interval_seconds * 1000);
            job.note = "Future runs enabled; previous work is not resumed.".into();
            save(&db, &job)?;
        }
        _ => return Err(err("Unknown schedule action")),
    }
    Ok(json!({"ok":true}))
}
pub fn recover(root: &Path) -> io::Result<()> {
    if !root.join("schedules.sqlite3").try_exists()? {
        return Ok(());
    }
    let db = database(root)?;
    for mut job in jobs(&db)? {
        if job.state == "Dispatching" {
            job.enabled = false;
            job.state = "Needs attention".into();
            job.note="Klyne restarted during dispatch. Inspect recent conversations before enabling future runs; this occurrence will not be replayed.".into();
            save(&db, &job)?;
        }
    }
    Ok(())
}
pub fn tick(studio: &Arc<Studio>, now: u64) -> io::Result<()> {
    if !studio.root.join("schedules.sqlite3").try_exists()? {
        return Ok(());
    }
    let _admission = studio.admission.lock().unwrap();
    if studio.quiescing.load(Ordering::SeqCst) {
        return Ok(());
    }
    let db = database(&studio.root)?;
    for mut job in jobs(&db)? {
        if !job.enabled {
            continue;
        }
        if job.state == "Running" {
            let result = job
                .last_chat
                .as_deref()
                .ok_or_else(|| err("Missing scheduled conversation"))
                .and_then(|id| studio.chats.get(id));
            match result {
                Ok(chat)
                    if matches!(
                        chat.status.as_str(),
                        "Planning" | "Working" | "Reviewing" | "Stopping" | "Upgrading"
                    ) =>
                {
                    continue;
                }
                Ok(chat) if chat.status == "Completed" => {
                    job.state = "Waiting".into();
                    job.next_run_ms = now.saturating_add(job.interval_seconds * 1000);
                    job.note = "Last run completed. Next run follows the saved interval.".into();
                }
                _ => {
                    job.enabled = false;
                    job.state = "Needs attention".into();
                    job.note="The last run needs attention. Review its conversation before enabling future runs.".into();
                }
            }
            save(&db, &job)?;
            continue;
        }
        if job.state != "Waiting" || now < job.next_run_ms || studio.chats.active_count() >= 4 {
            continue;
        }
        job.state = "Dispatching".into();
        job.last_chat = None;
        save(&db, &job)?;
        match studio.chats.send(&job.request) {
            Ok(result) => {
                job.last_chat = result["id"].as_str().map(str::to_owned);
                job.state = "Running".into();
                job.note = "Scheduled task started.".into();
            }
            Err(_) => {
                job.enabled = false;
                job.state = "Needs attention".into();
                job.note="Could not start this task. Check its saved provider and project; future runs are paused.".into();
            }
        }
        save(&db, &job)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_dispatch_is_never_replayed() {
        let root = tempfile::tempdir().unwrap();
        let db = database(root.path()).unwrap();
        let job = Job {
            id: "1".into(),
            name: "test".into(),
            interval_seconds: 60,
            next_run_ms: 0,
            enabled: true,
            state: "Dispatching".into(),
            last_chat: None,
            note: String::new(),
            request: json!({}),
        };
        save(&db, &job).unwrap();
        drop(db);
        recover(root.path()).unwrap();
        let value = list(root.path()).unwrap();
        assert_eq!(value["jobs"][0]["enabled"], false);
        assert_eq!(value["jobs"][0]["state"], "Needs attention");
        recover(root.path()).unwrap();
        assert_eq!(value, list(root.path()).unwrap());
    }
}
