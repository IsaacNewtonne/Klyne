//! Small on-disk bridge to the independent recovery service. No model or
//! repair code runs in this module; a broken chat cannot own its watchdog.
use crate::connections::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn settings(root: &Path) -> Option<Value> {
    let bytes = fs::read(root.join("recovery/config.json")).ok()?;
    if bytes.len() > 16384 {
        return None;
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    (value["enabled"] == true).then_some(value)
}

pub fn fallback(root: &Path) -> Option<Connection> {
    Connection::parse(&settings(root)?["fallback"])
        .ok()
        .filter(|connection| connection.kind != "demo")
}

pub fn record(root: &Path, chat_id: &str, incident: Value) -> std::io::Result<()> {
    if settings(root).is_none() {
        return Ok(());
    }
    let directory = root.join("recovery/incidents");
    fs::create_dir_all(&directory)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let name = format!("{chat_id}-{timestamp}");
    let data = json!({"id":name,"chat_id":chat_id,"created_at":timestamp,"failure":incident});
    let temporary = directory.join(format!("{name}.tmp"));
    fs::write(&temporary, serde_json::to_vec(&data)?)?;
    fs::rename(temporary, directory.join(format!("{name}.json")))
}
