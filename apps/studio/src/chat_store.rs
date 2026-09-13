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
    let after = chat.pending.as_ref().unwrap_or(&Value::Null);
    if before != after {
        if !before.is_null() && !after.is_null() {
            return Err(err("Cannot replace an unresolved action"));
        }
        let (action, state, evidence) = if before.is_null() {
            (after.clone(), "dispatched", Value::Null)
        } else {
            // A returned tool call is not proof of task success. Preserve the
            // observation or explicit reconciliation without calling it verified.
            let mut evidence = chat.evidence.last().cloned().unwrap_or(Value::Null);
            let secrets = crate::broker::secret_values(&chat.execution.secret_grants);
            crate::broker::scrub_value(&mut evidence, &secrets);
            let state = if evidence["agent"] == "User reconciliation" {
                "reconciled"
            } else {
                "returned"
            };
            let mut action = before.clone();
            crate::broker::scrub_value(&mut action, &secrets);
            (action, state, evidence)
        };
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
    crate::broker::scrub_value(
        &mut metadata,
        &crate::broker::secret_values(&chat.execution.secret_grants),
    );
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
    let data: Value = serde_json::from_str(&payload).map_err(err)?;
    Ok(serde_json::json!({"id":data["id"],"title":data["title"],"status":data["status"]}))
}
