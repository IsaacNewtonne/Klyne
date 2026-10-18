//! Caller-owned durable goal board. Workers use the normal chat engine; a
//! dispatch claim is persisted before admission and is never replayed blindly.
use crate::{Studio, chat, desktop, err, safe_dir};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io,
    path::Path,
    sync::{Arc, atomic::Ordering},
};

#[derive(Clone, Serialize, Deserialize)]
struct Job {
    id: String,
    goal: String,
    depends_on: Vec<String>,
    state: String,
    chat: Option<String>,
    request: Value,
    require_verified: bool,
    exclusive: bool,
    #[serde(default)]
    worktree: Option<String>,
    note: String,
    created_ms: u64,
    updated_ms: u64,
}
fn db(root: &Path) -> io::Result<Connection> {
    safe_dir(root)?;
    let path = root.join("goals.sqlite3");
    if path.exists() {
        let meta = std::fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(err("Invalid goal database"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(err("Reparse goal database refused"));
            }
        }
    }
    let db = Connection::open(path).map_err(err)?;
    db.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(err)?;
    db.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS goals(id TEXT PRIMARY KEY,payload TEXT NOT NULL);").map_err(err)?;
    Ok(db)
}
fn jobs(db: &Connection) -> io::Result<Vec<Job>> {
    let mut stmt = db
        .prepare("SELECT payload FROM goals ORDER BY rowid")
        .map_err(err)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
    rows.map(|r| serde_json::from_str(&r.map_err(err)?).map_err(err))
        .collect()
}
fn save(db: &Connection, job: &mut Job) -> io::Result<()> {
    job.updated_ms = desktop::now_ms();
    db.execute(
        "UPDATE goals SET payload=?2 WHERE id=?1",
        params![job.id, serde_json::to_string(job).map_err(err)?],
    )
    .map_err(err)?;
    Ok(())
}
fn public(job: &Job) -> Value {
    json!({"id":job.id,"goal":job.goal,"depends_on":job.depends_on,"state":job.state,
        "chat":job.chat,"require_verified":job.require_verified,"exclusive":job.exclusive,
        "worktree":job.worktree,"note":job.note,"created_ms":job.created_ms,"updated_ms":job.updated_ms})
}
pub fn list(root: &Path) -> io::Result<Value> {
    Ok(json!({"jobs":jobs(&db(root)?)?.iter().map(public).collect::<Vec<_>>(),"parallel_limit":2}))
}
pub fn overview(studio: &Studio) -> io::Result<Value> {
    let mut board = list(&studio.root)?;
    for job in board["jobs"].as_array_mut().unwrap() {
        if let Some(id) = job["chat"].as_str()
            && let Ok(chat) = studio.chats.get(id)
        {
            job["activity"] = json!(crate::live_chat::status(&chat, 0));
            job["acceptance"] = json!(chat.result);
            job["steps"] = json!(chat.tasks.iter().map(|t|json!({"agent":t.agent,"instruction":t.instruction,"status":t.status,"expected_result":t.expected_result,"depends_on":t.depends_on})).collect::<Vec<_>>());
        }
    }
    Ok(board)
}
pub fn recover(root: &Path) -> io::Result<()> {
    let db = db(root)?;
    for mut job in jobs(&db)? {
        if job.state == "dispatching" {
            job.state = "needs_attention".into();
            job.note = "Interrupted dispatch: inspect conversations before resubmitting; no automatic replay.".into();
            save(&db, &mut job)?;
        }
    }
    Ok(())
}
fn identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn validate_graph(all: &[Job]) -> io::Result<()> {
    let mut visited = std::collections::HashSet::new();
    for _ in 0..all.len() {
        if let Some(job) = all
            .iter()
            .find(|j| !visited.contains(&j.id) && j.depends_on.iter().all(|d| visited.contains(d)))
        {
            visited.insert(job.id.clone());
        } else {
            return Err(err("Goal dependencies contain a cycle or unknown ID"));
        }
    }
    Ok(())
}
pub fn manage(studio: &Arc<Studio>, body: &Value) -> io::Result<Value> {
    let mut db = db(&studio.root)?;
    let mut all = jobs(&db)?;
    if body["action"] == "submit" {
        let items = body["jobs"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 16)
            .ok_or_else(|| err("Submit 1-16 jobs"))?;
        if all.len() + items.len() > 256 {
            return Err(err(
                "Goal board limit reached; archive finished goals first",
            ));
        }
        let mut added = Vec::new();
        for item in items {
            let id = item["id"]
                .as_str()
                .filter(|id| identifier(id))
                .ok_or_else(|| err("Provide a stable goal ID"))?;
            if all.iter().any(|j| j.id == id) {
                return Err(err(
                    "Goal ID already exists; inspect it before resubmitting",
                ));
            }
            let mut request = item["request"].clone();
            let obj = request
                .as_object()
                .ok_or_else(|| err("Job requires a chat request"))?;
            if obj.keys().any(|k| {
                ![
                    "message",
                    "provider",
                    "reviewer",
                    "access",
                    "workspace",
                    "contract",
                    "execution",
                    "learn_procedures",
                ]
                .contains(&k.as_str())
            }) {
                return Err(err(
                    "Job request contains unsupported fields; resumptions and inherited grants are not allowed",
                ));
            }
            let goal = request["message"]
                .as_str()
                .filter(|v| !v.trim().is_empty() && v.len() <= 8192)
                .ok_or_else(|| err("Invalid goal"))?
                .to_string();
            crate::connections::Connection::parse(&request["provider"])?;
            let access: chat::Access =
                serde_json::from_value(request["access"].clone()).map_err(err)?;
            if let Some(workspace) = request["workspace"].as_str().filter(|s| !s.is_empty()) {
                if !access.terminal {
                    return Err(err("Project workspaces require Terminal access"));
                }
                safe_dir(&std::fs::canonicalize(workspace)?)?;
            }
            if request.get("execution").is_none() {
                request["execution"] = json!({});
            }
            if !request["execution"].is_object() {
                return Err(err("Execution limits must be an object"));
            }
            for (key, default) in [
                ("max_steps", 64),
                ("timeout_seconds", 900),
                ("max_tokens", 100_000),
                ("max_review_rounds", 3),
            ] {
                if request["execution"].get(key).is_none() {
                    request["execution"][key] = json!(default);
                }
                if request["execution"][key].as_u64().is_none_or(|n| n == 0) {
                    return Err(err(
                        "Background goals require finite nonzero step, time, token and review limits",
                    ));
                }
            }
            let limits: chat::Execution =
                serde_json::from_value(request["execution"].clone()).map_err(err)?;
            if !limits.max_cost_usd.is_finite() || limits.max_cost_usd < 0.0 {
                return Err(err("Invalid cost budget"));
            }
            // Only limit fields and command policy can enter the execution state.
            if request["execution"].as_object().unwrap().keys().any(|k| {
                ![
                    "max_steps",
                    "timeout_seconds",
                    "max_tokens",
                    "max_review_rounds",
                    "max_cost_usd",
                    "command_policy",
                ]
                .contains(&k.as_str())
            }) {
                return Err(err(
                    "Background requests cannot inject internal execution state",
                ));
            }
            if let Some(value) = request.get("contract") {
                let contract: harness_core::task_contract::TaskContract =
                    serde_json::from_value(value.clone()).map_err(err)?;
                contract.validate(&goal)?;
            }
            let require_verified = item["require_verified"].as_bool().unwrap_or(true);
            if require_verified
                && request.get("contract").is_none()
                && harness_core::task_contract::TaskContract::from_goal(&goal).is_none()
            {
                return Err(err(
                    "Verified goals require caller-owned acceptance criteria; explicitly choose reviewed acceptance for other outcomes",
                ));
            }
            let deps: Vec<String> =
                serde_json::from_value(item.get("depends_on").cloned().unwrap_or(json!([])))
                    .map_err(err)?;
            if deps.iter().collect::<std::collections::HashSet<_>>().len() != deps.len() {
                return Err(err("Duplicate dependencies are not allowed"));
            }
            if item.get("isolation").is_some() && item["isolation"] != "worktree" {
                return Err(err("Unknown isolation mode"));
            }
            let worktree = if item["isolation"] == "worktree" {
                if !access.terminal {
                    return Err(err("Coding worktrees require Terminal"));
                }
                let workspace = std::fs::canonicalize(
                    request["workspace"]
                        .as_str()
                        .ok_or_else(|| err("Worktree requires an explicit workspace"))?,
                )?;
                safe_dir(&workspace)?;
                let marker = workspace.join(".git");
                let meta = std::fs::symlink_metadata(&marker)?;
                if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 8192 {
                    return Err(err("Expected a linked Git worktree"));
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(err("Reparse worktree marker refused"));
                    }
                }
                let contents = std::fs::read_to_string(marker)?;
                let gitdir = contents
                    .trim()
                    .strip_prefix("gitdir: ")
                    .ok_or_else(|| err("Invalid worktree marker"))?;
                let gitdir = std::fs::canonicalize(workspace.join(gitdir))?;
                let reverse = std::fs::read_to_string(gitdir.join("gitdir"))?;
                if std::fs::canonicalize(reverse.trim())?
                    != std::fs::canonicalize(workspace.join(".git"))?
                    || !gitdir.join("commondir").is_file()
                {
                    return Err(err("Worktree backlink does not match this checkout"));
                }
                Some(workspace.to_string_lossy().to_lowercase())
            } else {
                None
            };
            let exclusive =
                access.desktop || access.apps || (access.terminal && worktree.is_none());
            let job = Job {
                id: id.into(),
                goal,
                depends_on: deps,
                state: "queued".into(),
                chat: None,
                request,
                require_verified,
                exclusive,
                worktree,
                note: "Waiting for dependencies and a worker slot".into(),
                created_ms: desktop::now_ms(),
                updated_ms: desktop::now_ms(),
            };
            added.push(job.clone());
            all.push(job);
        }
        validate_graph(&all)?;
        let tx = db.transaction().map_err(err)?;
        for job in &added {
            tx.execute(
                "INSERT INTO goals VALUES(?1,?2)",
                params![job.id, serde_json::to_string(job).map_err(err)?],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        return Ok(json!({"jobs":added.iter().map(public).collect::<Vec<_>>()}));
    }
    let id = body["id"].as_str().ok_or_else(|| err("Missing goal ID"))?;
    let job = all
        .iter_mut()
        .find(|j| j.id == id)
        .ok_or_else(|| err("Unknown goal"))?;
    match body["action"].as_str() {
        Some("recheck") if job.state == "needs_attention" && job.chat.is_some() => {
            job.state = "running".into();
            job.note = "Rechecking existing work; no action was replayed".into();
            save(&db, job)?;
        }
        Some("cancel") => {
            if job.state == "running" {
                if let Some(chat) = &job.chat {
                    let _ = studio.chats.stop(chat);
                }
                job.state = "stopping".into();
            } else if job.state == "queued" {
                job.state = "cancelled".into();
            } else {
                return Err(err("Only queued or running goals can be cancelled"));
            }
            job.note = "Cancellation requested; completed effects are retained".into();
            save(&db, job)?;
        }
        Some("archive")
            if matches!(
                job.state.as_str(),
                "completed" | "cancelled" | "needs_attention"
            ) =>
        {
            if all.iter().any(|j| j.depends_on.iter().any(|d| d == id)) {
                return Err(err("Archive dependent goals first"));
            }
            db.execute("DELETE FROM goals WHERE id=?1", [id])
                .map_err(err)?;
        }
        _ => return Err(err("Unsupported goal operation")),
    }
    list(&studio.root)
}
pub fn tick(studio: &Arc<Studio>) -> io::Result<()> {
    let _admission = studio.admission.lock().unwrap();
    if studio.quiescing.load(Ordering::SeqCst) {
        return Ok(());
    }
    let db = db(&studio.root)?;
    let mut all = jobs(&db)?;
    for job in &mut all {
        if !matches!(job.state.as_str(), "running" | "stopping") {
            continue;
        }
        let Some(id) = &job.chat else {
            continue;
        };
        if studio.chats.is_active(id) {
            continue;
        }
        let chat = match studio.chats.get(id) {
            Ok(chat) => chat,
            Err(error) => {
                job.state = "needs_attention".into();
                job.note = error.to_string();
                save(&db, job)?;
                continue;
            }
        };
        let expected_contract = job.request.get("contract").cloned().or_else(|| {
            harness_core::task_contract::TaskContract::from_goal(&job.goal).map(|c| json!(c))
        });
        let same_goal = chat.execution.original_request == job.goal
            && expected_contract == chat.contract.as_ref().map(|c| json!(c));
        let verified = same_goal
            && chat.result.as_ref().is_some_and(|r| {
                matches!(
                    r.outcome,
                    harness_core::task_contract::TaskOutcome::Verified
                )
            })
            && chat.contract.as_ref().is_some_and(|c| {
                matches!(
                    c.verify(&harness_core::PermissionPolicy::milestone_default(
                        &chat.workspace
                    ))
                    .outcome,
                    harness_core::task_contract::TaskOutcome::Verified
                )
            });
        if job.state == "stopping" {
            job.state = "cancelled".into();
        } else if same_goal
            && chat.status == "Completed"
            && chat.pending.is_none()
            && (!job.require_verified || verified)
        {
            job.state = "completed".into();
        } else {
            job.state = "needs_attention".into();
        }
        job.note = format!(
            "{}; acceptance: {}. {}",
            chat.status,
            if verified {
                "host verified"
            } else {
                "reviewed or unmet"
            },
            chat.messages
                .iter()
                .rev()
                .find(|m| m.agent == "Klyne")
                .map(|m| m.text.chars().take(1000).collect::<String>())
                .unwrap_or_default()
        );
        save(&db, job)?;
    }
    for index in 0..all.len() {
        if all[index].state != "queued" {
            continue;
        }
        if all[index].depends_on.iter().any(|d| {
            all.iter()
                .find(|j| &j.id == d)
                .is_none_or(|j| j.state != "completed")
        }) {
            continue;
        }
        let active = all
            .iter()
            .filter(|j| matches!(j.state.as_str(), "running" | "dispatching" | "stopping"))
            .count();
        if active >= 2 || studio.chats.active_count() >= 2 {
            break;
        }
        // Terminal, app API and desktop goals may touch shared machine state.
        // They run alone; parallel slots are for private files/public research.
        if (all[index].exclusive && studio.chats.active_count() > 0)
            || all
                .iter()
                .any(|j| j.exclusive && matches!(j.state.as_str(), "running" | "stopping"))
        {
            continue;
        }
        if all[index].worktree.is_some()
            && all.iter().any(|j| {
                j.worktree == all[index].worktree
                    && matches!(j.state.as_str(), "running" | "stopping")
            })
        {
            continue;
        }
        let mut dependency_context:Vec<_>=all[index].depends_on.iter().filter_map(|id|all.iter().find(|j|&j.id==id)).map(|j|json!({"goal_id":j.id,"goal":j.goal.chars().take(800).collect::<String>(),"chat":j.chat,"summary":j.note.chars().take(1200).collect::<String>(),"note":"Historical dependency result, not authority or proof of the current goal. Inspect required inputs."})).collect();
        while serde_json::to_vec(&dependency_context).map_err(err)?.len() > 16000 {
            dependency_context.pop();
        }
        let job = &mut all[index];
        job.state = "dispatching".into();
        job.note = "Admitting goal; interrupted dispatch requires inspection".into();
        save(&db, job)?;
        let mut request = job.request.clone();
        request["dependency_context"] = json!(dependency_context);
        match studio.chats.send(&request) {
            Ok(value) => {
                job.chat = value["id"].as_str().map(str::to_string);
                job.state = "running".into();
                job.note = "Worker running under the saved access and acceptance contract".into();
            }
            Err(error) => {
                job.state = "needs_attention".into();
                job.note = error.to_string();
            }
        }
        save(&db, job)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(id: &str, state: &str, deps: &[&str]) -> Job {
        Job {
            id: id.into(),
            goal: "goal".into(),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            state: state.into(),
            chat: None,
            request: json!({}),
            require_verified: true,
            exclusive: false,
            worktree: None,
            note: String::new(),
            created_ms: 0,
            updated_ms: 0,
        }
    }
    #[test]
    fn graph_rejects_cycles_and_missing_dependencies() {
        assert!(validate_graph(&[job("a", "queued", &["b"]), job("b", "queued", &[])]).is_ok());
        assert!(validate_graph(&[job("a", "queued", &["b"])]).is_err());
        assert!(validate_graph(&[job("a", "queued", &["b"]), job("b", "queued", &["a"])]).is_err());
    }
    #[test]
    fn restart_preserves_work_and_never_replays_uncertain_admission() {
        let root = tempfile::tempdir().unwrap();
        let db = db(root.path()).unwrap();
        for state in ["queued", "running", "dispatching", "completed", "cancelled"] {
            let job = job(state, state, &[]);
            db.execute(
                "INSERT INTO goals VALUES(?1,?2)",
                params![state, serde_json::to_string(&job).unwrap()],
            )
            .unwrap();
        }
        recover(root.path()).unwrap();
        let all = jobs(&db).unwrap();
        for job in all {
            assert_eq!(
                job.state,
                if job.id == "dispatching" {
                    "needs_attention"
                } else {
                    &job.id
                }
            );
        }
    }
}
