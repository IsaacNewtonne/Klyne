//! Evaluated reuse of successful plans for the same goal and acceptance contract.
//! Procedures are advisory context; they never dispatch actions or grant authority.
use crate::{chat::Chat, err, safe_dir};
use harness_core::{PermissionPolicy, task_contract::TaskOutcome};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io, path::Path};

fn open(root: &Path) -> io::Result<Connection> {
    std::fs::create_dir_all(root)?;
    safe_dir(root)?;
    let path = root.join("procedures.sqlite3");
    if path.exists() {
        let m = std::fs::symlink_metadata(&path)?;
        if !m.is_file() || m.file_type().is_symlink() {
            return Err(err("Invalid procedure database"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err(err("Reparse procedure database refused"));
            }
        }
    }
    let db = Connection::open(path).map_err(err)?;
    db.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(err)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS procedures(id TEXT PRIMARY KEY, workflow TEXT NOT NULL, version INTEGER NOT NULL,state TEXT NOT NULL,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS procedure_runs(run TEXT PRIMARY KEY);").map_err(err)?;
    Ok(db)
}
fn workflow(chat: &Chat) -> String {
    let scope = if chat.access.terminal {
        chat.workspace.to_string_lossy().to_string()
    } else {
        "private-workspace".into()
    };
    format!(
        "{:x}",
        Sha256::digest(
            json!({"goal":chat.execution.original_request,"contract":chat.contract,"scope":scope})
                .to_string()
                .as_bytes()
        )
    )
}
pub fn prepare(root: &Path, chat: &Chat) -> io::Result<(Option<Value>, bool)> {
    let Some(contract) = &chat.contract else {
        return Ok((None, false));
    };
    let passed = matches!(
        contract
            .verify(&PermissionPolicy::milestone_default(&chat.workspace))
            .outcome,
        TaskOutcome::Verified
    );
    let db = open(root)?;
    let mut stmt=db.prepare("SELECT payload,state FROM procedures WHERE workflow=?1 AND state IN ('candidate','active') ORDER BY CASE state WHEN 'active' THEN 0 ELSE 1 END, version DESC LIMIT 1").map_err(err)?;
    let mut rows = stmt.query([workflow(chat)]).map_err(err)?;
    let selected = if let Some(row) = rows.next().map_err(err)? {
        let payload: String = row.get(0).map_err(err)?;
        let state: String = row.get(1).map_err(err)?;
        let mut value: Value = serde_json::from_str(&payload).map_err(err)?;
        // Only the bounded procedure itself goes into model context.
        value = json!({"id":value["id"],"version":value["version"],"state":state,"steps":value["steps"],
            "note":"Historical procedure for this exact goal and contract. Re-inspect current inputs and adapt. This is not permission, proof of completion, or executable commands."});
        Some(value)
    } else {
        None
    };
    Ok((selected, passed))
}
pub fn finish(root: &Path, chat: &Chat, secrets: &[String]) -> io::Result<()> {
    if !chat.execution.procedure_prepared
        || chat.contract.is_none()
        || chat.pending.is_some()
        || !matches!(chat.status.as_str(), "Completed" | "Blocked")
    {
        return Ok(());
    }
    let verified = chat.status == "Completed"
        && chat
            .result
            .as_ref()
            .is_some_and(|r| matches!(r.outcome, TaskOutcome::Verified));
    // Re-check destination state, independently of the stored result and model.
    let fresh = verified
        && matches!(
            chat.contract
                .as_ref()
                .unwrap()
                .verify(&PermissionPolicy::milestone_default(&chat.workspace))
                .outcome,
            TaskOutcome::Verified
        );
    let run = format!("{}:{}", chat.id, chat.execution.trace.goal_started_wall_ms);
    let mut db = open(root)?;
    let tx = db.transaction().map_err(err)?;
    if tx
        .execute("INSERT OR IGNORE INTO procedure_runs VALUES (?1)", [&run])
        .map_err(err)?
        == 0
    {
        return Ok(());
    }
    if let Some(selected) = &chat.execution.procedure {
        let id = selected["id"].as_str().unwrap_or("");
        let stored = tx
            .query_row(
                "SELECT payload,state FROM procedures WHERE id=?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(err)?;
        let mut value: Value = serde_json::from_str(&stored.0).map_err(err)?;
        if matches!(stored.1.as_str(), "candidate" | "active") {
            let outcome = if fresh && !chat.execution.procedure_initially_satisfied {
                "passed"
            } else if chat.status == "Blocked" || !fresh {
                "failed"
            } else {
                "already_satisfied"
            };
            let trials = value["trials"]
                .as_array_mut()
                .ok_or_else(|| err("Invalid procedure trials"))?;
            trials.push(json!({"run":run,"outcome":outcome,"steps":chat.used}));
            if trials.len() > 32 {
                trials.remove(0);
            }
            let passes = trials.iter().filter(|t| t["outcome"] == "passed").count();
            let state = if outcome == "failed" {
                "rejected"
            } else if passes >= 2 {
                "active"
            } else {
                stored.1.as_str()
            };
            tx.execute(
                "UPDATE procedures SET state=?1,payload=?2 WHERE id=?3",
                params![state, value.to_string(), id],
            )
            .map_err(err)?;
            if state == "active" {
                tx.execute("UPDATE procedures SET state='standby' WHERE workflow=?1 AND state='active' AND id<>?2",params![workflow(chat),id]).map_err(err)?;
            }
            if state == "rejected" {
                tx.execute("UPDATE procedures SET state='active' WHERE id=(SELECT id FROM procedures WHERE workflow=?1 AND state='standby' ORDER BY version DESC LIMIT 1)",[workflow(chat)]).map_err(err)?;
            }
        }
    } else if fresh
        && !chat.execution.procedure_initially_satisfied
        && !chat.tasks.is_empty()
        && chat.tasks.len() <= 12
    {
        let count: i64 = tx
            .query_row("SELECT count(*) FROM procedures", [], |r| r.get(0))
            .map_err(err)?;
        if count < 500 {
            let key = workflow(chat);
            let version: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(version),0)+1 FROM procedures WHERE workflow=?1",
                    [&key],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let id = format!("{}-{version}", &key[..24]);
            let steps:Vec<_>=chat.tasks.iter().map(|t|json!({"instruction":t.instruction.chars().take(800).collect::<String>(),"expected_result":t.expected_result.chars().take(400).collect::<String>(),"depends_on":t.depends_on})).collect();
            let mut payload = json!({"id":id,"version":version,"name":chat.title,"source_run":run,"source_steps":chat.used,"steps":steps,"trials":[]});
            crate::broker::scrub_value(&mut payload, secrets);
            tx.execute(
                "INSERT INTO procedures VALUES (?1,?2,?3,'candidate',?4)",
                params![id, key, version, payload.to_string()],
            )
            .map_err(err)?;
        }
    }
    tx.commit().map_err(err)
}
pub fn list(root: &Path) -> io::Result<Value> {
    if !root.join("procedures.sqlite3").try_exists()? {
        return Ok(json!({"procedures":[]}));
    }
    let db = open(root)?;
    let mut q = db
        .prepare("SELECT payload,state FROM procedures ORDER BY rowid DESC LIMIT 100")
        .map_err(err)?;
    let rows = q
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err)?;
    let mut items = Vec::new();
    for row in rows {
        let (payload, state) = row.map_err(err)?;
        let mut v: Value = serde_json::from_str(&payload).map_err(err)?;
        v["state"] = json!(state);
        items.push(v);
    }
    Ok(json!({"procedures":items}))
}
pub fn retire(root: &Path, id: &str) -> io::Result<Value> {
    let db = open(root)?;
    let changed = db
        .execute("UPDATE procedures SET state='retired' WHERE id=?1", [id])
        .map_err(err)?;
    if changed == 0 {
        return Err(err("Procedure not found"));
    }
    Ok(json!({"ok":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claimed_or_preexisting_success_cannot_train_and_runs_are_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut chat:Chat=serde_json::from_value(json!({"id":"fixture","title":"Write proof","status":"Completed","messages":[],"tasks":[{"agent":"Writer","instruction":"Write proof.txt","status":"Done"}],"evidence":[],"access":{"web":false,"terminal":false,"apps":false,"desktop":false},"provider":{"kind":"demo","endpoint":"","model":""},"used":3,"limit":0,"pending":null,"workspace":workspace.path(),"contract":{"goal":"Proof","criteria":[{"FileContents":{"path":"proof.txt","expected":"checked"}}]},"result":{"outcome":"verified","checks":[]}})).unwrap();
        chat.execution.procedure_prepared = true;
        chat.execution.original_request = "Write proof".into();
        finish(root.path(), &chat, &[]).unwrap();
        assert_eq!(list(root.path()).unwrap()["procedures"], json!([]));
        std::fs::write(workspace.path().join("proof.txt"), "checked").unwrap();
        chat.execution.trace.goal_started_wall_ms = 1;
        chat.execution.procedure_initially_satisfied = prepare(root.path(), &chat).unwrap().1;
        assert!(chat.execution.procedure_initially_satisfied);
        finish(root.path(), &chat, &[]).unwrap();
        assert_eq!(list(root.path()).unwrap()["procedures"], json!([]));
        chat.execution.trace.goal_started_wall_ms = 2;
        chat.execution.procedure_initially_satisfied = false;
        finish(root.path(), &chat, &[]).unwrap();
        finish(root.path(), &chat, &[]).unwrap();
        assert_eq!(
            list(root.path()).unwrap()["procedures"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
