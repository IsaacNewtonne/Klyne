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
        // 2026-10-24: a fallback model that Ollama reports as not installed
        // would only 404; skip it so the normal retry path is used instead.
        .filter(|connection| !missing_ollama_model(connection))
}

fn missing_ollama_model(connection: &Connection) -> bool {
    connection.kind == "ollama"
        && !connection.model.is_empty()
        && crate::connections::ollama_model_installed(&connection.endpoint, &connection.model)
            == Some(false)
}

/// Configured recovery/helper models that Ollama reports as not installed.
pub fn model_warnings(root: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(settings) = settings(root) {
        for (label, key) in [
            ("Recovery fallback", "fallback"),
            ("Recovery repair", "repair_provider"),
        ] {
            if let Ok(connection) = Connection::parse(&settings[key])
                && missing_ollama_model(&connection)
            {
                warnings.push(format!(
                    "{label} model {} is not installed in Ollama. Run ollama pull {} or choose an installed model in recovery/config.json.",
                    connection.model, connection.model
                ));
            }
        }
    }
    let quality = crate::quality::load(root);
    let models = &quality.models;
    for (label, model) in [
        ("planner", &models.planner),
        ("worker", &models.worker),
        ("reviewer", &models.reviewer),
        ("voice", &models.voice),
    ] {
        if let Some(model) = model.as_deref().filter(|m| *m != "current")
            && crate::connections::ollama_model_installed(&quality.helper.endpoint, model)
                == Some(false)
        {
            warnings.push(format!(
                "The {label} model {model} in quality/config.json is not installed in Ollama; that role falls back to an error until you pull it or clear the setting."
            ));
        }
    }
    let helper = quality.helper;
    if helper.enabled
        && crate::connections::ollama_model_installed(&helper.endpoint, &helper.model)
            == Some(false)
    {
        warnings.push(format!(
            "Helper model {} is not installed in Ollama; ask_model stays hidden. Change quality/config.json or pull the model.",
            helper.model
        ));
    }
    warnings
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
