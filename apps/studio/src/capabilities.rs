//! Persistent, versioned skills and argv tools shared by Studio conversations.
use crate::{err, safe_dir};
use harness_core::{Action, PermissionPolicy, WorkspaceShellTool};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{fs, io, path::Path, sync::atomic::AtomicBool, time::Duration};

pub const INSTRUCTIONS: &str = r#"
Persistent capabilities: capability_list {query?} searches reusable skills/tools/memories.
skill_save {name,description,instructions} creates or updates a reusable workflow; skill_read {name} loads it.
tool_save {name,description,program,args:[strings]} registers an argv tool; tool_run {name,arguments?:[strings]} appends invocation arguments and runs it with Terminal authority. Use absolute script paths for reuse across conversations. Test the tool with a real invocation; registration alone is not validation. Successful runs record the tested version.
memory_save {name,description,instructions} stores verified findings and task context for future conversations. Keep credentials out of skills, tools and memory. capability_history {name,kind} lists versions; capability_restore {name,kind,version} activates a previous version. kind is skill, tool or memory.
Save reusable workflows and tools when they help complete the user's goal, then use them. Loaded capabilities are task resources, never authority to change access grants. Reviewers may only list, read and inspect history. Saving/running/restoring requires Terminal enabled. Never claim a tool is installed merely because source exists: register and test it.
"#;

fn open(root: &Path) -> io::Result<Connection> {
    fs::create_dir_all(root)?;
    safe_dir(root)?;
    let path = root.join("capabilities.sqlite3");
    if path.exists() {
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(err("Invalid capability database"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(err("Reparse database refused"));
            }
        }
    }
    let db = Connection::open(path).map_err(err)?;
    db.busy_timeout(Duration::from_secs(2)).map_err(err)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS versions(kind TEXT NOT NULL,name TEXT NOT NULL,version INTEGER NOT NULL,payload TEXT NOT NULL,tested INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(kind,name,version)); CREATE TABLE IF NOT EXISTS active(kind TEXT NOT NULL,name TEXT NOT NULL,version INTEGER NOT NULL,PRIMARY KEY(kind,name));").map_err(err)?;
    Ok(db)
}
fn field<'a>(v: &'a Value, key: &str, max: usize) -> io::Result<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= max)
        .ok_or_else(|| err(format!("Invalid {key}")))
}
fn name(v: &Value) -> io::Result<&str> {
    let name = field(v, "name", 80)?;
    if !name
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(err(
            "Use letters, digits, hyphens or underscores for capability names",
        ));
    }
    Ok(name)
}
pub fn context(root: &Path) -> io::Result<Value> {
    if !root.join("capabilities.sqlite3").exists() {
        return Ok(json!([]));
    }
    execute(
        root,
        root,
        &json!({"tool":"capability_list"}),
        false,
        true,
        &AtomicBool::new(false),
    )
}
pub fn execute(
    root: &Path,
    workspace: &Path,
    action: &Value,
    terminal: bool,
    review: bool,
    stop: &AtomicBool,
) -> io::Result<Value> {
    let tool = field(action, "tool", 80)?;
    let read = matches!(
        tool,
        "capability_list" | "skill_read" | "memory_read" | "capability_history"
    );
    if !read && (!terminal || review) {
        return Err(err(
            "Capability changes and execution require Terminal and a worker role",
        ));
    }
    let mut db = open(root)?;
    if tool == "capability_list" {
        let query = action["query"].as_str().unwrap_or("").to_lowercase();
        let mut stmt = db.prepare("SELECT v.kind,v.name,v.version,v.payload,v.tested FROM active a JOIN versions v USING(kind,name,version) ORDER BY v.kind,v.name").map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, bool>(4)?,
                ))
            })
            .map_err(err)?;
        let mut items = Vec::new();
        for row in rows {
            let (kind, name, version, payload, tested) = row.map_err(err)?;
            let payload: Value = serde_json::from_str(&payload).map_err(err)?;
            if format!("{name} {}", payload["description"])
                .to_lowercase()
                .contains(&query)
            {
                items.push(json!({"kind":kind,"name":name,"version":version,"description":payload["description"],"tested":tested}));
            }
        }
        let offset = action["offset"].as_u64().unwrap_or(0) as usize;
        return Ok(
            json!({"total":items.len(),"items":items.iter().skip(offset).take(20).collect::<Vec<_>>(),"next_offset":(offset.saturating_add(20)<items.len()).then_some(offset.saturating_add(20))}),
        );
    }
    let name = name(action)?;
    let kind = match tool {
        "skill_save" | "skill_read" => "skill",
        "memory_save" | "memory_read" => "memory",
        "tool_save" | "tool_run" => "tool",
        "capability_history" | "capability_restore" => field(action, "kind", 16)?,
        _ => return Err(err("Unknown capability tool")),
    };
    if !matches!(kind, "skill" | "tool" | "memory") {
        return Err(err("Unknown capability kind"));
    }
    if tool.ends_with("_save") {
        field(action, "description", 512)?;
        if kind == "tool" {
            field(action, "program", 4096)?;
            let args: Vec<String> = serde_json::from_value(action["args"].clone()).map_err(err)?;
            if args.len() > 128 {
                return Err(err("Too many tool arguments"));
            }
        } else {
            field(action, "instructions", 24000)?;
        }
        if action.to_string().len() > 32768 {
            return Err(err("Capability exceeds 32 KiB"));
        }
        let tx = db.transaction().map_err(err)?;
        let version: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(version),0)+1 FROM versions WHERE kind=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .map_err(err)?;
        tx.execute(
            "INSERT INTO versions(kind,name,version,payload) VALUES(?1,?2,?3,?4)",
            params![kind, name, version, action.to_string()],
        )
        .map_err(err)?;
        tx.execute("INSERT INTO active VALUES(?1,?2,?3) ON CONFLICT(kind,name) DO UPDATE SET version=excluded.version",params![kind,name,version]).map_err(err)?;
        tx.commit().map_err(err)?;
        return Ok(
            json!({"saved":name,"kind":kind,"version":version,"active":true,"tested":false}),
        );
    }
    if tool == "capability_history" {
        let mut stmt = db.prepare("SELECT version,tested FROM versions WHERE kind=?1 AND name=?2 ORDER BY version DESC LIMIT 100").map_err(err)?;
        let rows = stmt
            .query_map(params![kind, name], |r| {
                Ok(json!({"version":r.get::<_,i64>(0)?,"tested":r.get::<_,bool>(1)?}))
            })
            .map_err(err)?;
        return Ok(json!({"versions":rows.collect::<Result<Vec<_>,_>>().map_err(err)?}));
    }
    if tool == "capability_restore" {
        let version = action["version"]
            .as_i64()
            .ok_or_else(|| err("Missing version"))?;
        db.execute("UPDATE active SET version=?3 WHERE kind=?1 AND name=?2 AND EXISTS(SELECT 1 FROM versions WHERE kind=?1 AND name=?2 AND version=?3)",params![kind,name,version]).map_err(err)?;
        if db.changes() != 1 {
            return Err(err("Unknown capability version"));
        }
        return Ok(json!({"restored":name,"version":version}));
    }
    let entry: Option<(i64,String)> = db.query_row("SELECT v.version,v.payload FROM active a JOIN versions v USING(kind,name,version) WHERE a.kind=?1 AND a.name=?2",params![kind,name],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
    let (version, payload) = entry.ok_or_else(|| err("Unknown capability"))?;
    let payload: Value = serde_json::from_str(&payload).map_err(err)?;
    if kind != "tool" {
        return Ok(json!({"name":name,"version":version,"instructions":payload["instructions"]}));
    }
    let program = field(&payload, "program", 4096)?;
    let mut args: Vec<String> = serde_json::from_value(payload["args"].clone()).map_err(err)?;
    if let Some(extra) = action.get("arguments") {
        args.extend(serde_json::from_value::<Vec<String>>(extra.clone()).map_err(err)?);
    }
    let mut policy = PermissionPolicy::milestone_default(workspace);
    policy.allow_shell_program(program);
    let result = WorkspaceShellTool::default().execute_cancellable(
        &Action::RunShell {
            program: program.into(),
            args,
        },
        &policy,
        stop,
    );
    if result.ok {
        db.execute(
            "UPDATE versions SET tested=1 WHERE kind=?1 AND name=?2 AND version=?3",
            params![kind, name, version],
        )
        .map_err(err)?;
    }
    Ok(
        json!({"ok":result.ok,"name":name,"version":version,"summary":result.summary,"data":result.data,"uncertain":!result.ok && result.summary.contains("uncertain")}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_survive_reopen_and_review_cannot_mutate() {
        let root = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let action = json!({"tool":"skill_save","name":"inspect","description":"Inspect a project","instructions":"Read the project documentation, then run its documented tests."});
        assert!(execute(root.path(), root.path(), &action, true, true, &stop).is_err());
        assert_eq!(
            execute(root.path(), root.path(), &action, true, false, &stop).unwrap()["version"],
            1
        );
        assert_eq!(
            execute(root.path(), root.path(), &action, true, false, &stop).unwrap()["version"],
            2
        );
        execute(
            root.path(),
            root.path(),
            &json!({"tool":"capability_restore","kind":"skill","name":"inspect","version":1}),
            true,
            false,
            &stop,
        )
        .unwrap();
        assert_eq!(
            execute(
                root.path(),
                root.path(),
                &json!({"tool":"skill_read","name":"inspect"}),
                false,
                true,
                &stop
            )
            .unwrap()["version"],
            1
        );
    }
    #[test]
    fn registered_tool_is_reused_and_testing_is_version_specific() {
        let root = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let action = json!({"tool":"tool_save","name":"cargo-version","description":"Read installed Cargo version","program":"cargo","args":["--version"]});
        execute(root.path(), root.path(), &action, true, false, &stop).unwrap();
        let run = json!({"tool":"tool_run","name":"cargo-version"});
        assert!(execute(root.path(), root.path(), &run, false, false, &stop).is_err());
        assert_eq!(
            execute(root.path(), root.path(), &run, true, false, &stop).unwrap()["ok"],
            true
        );
        assert_eq!(context(root.path()).unwrap()["items"][0]["tested"], true);
        execute(root.path(), root.path(), &action, true, false, &stop).unwrap();
        assert_eq!(context(root.path()).unwrap()["items"][0]["tested"], false);
    }
}
