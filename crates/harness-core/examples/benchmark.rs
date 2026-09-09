//! Deterministic file-task benchmark; measures harness execution, not model intelligence.
use harness_core::{
    AgentRuntime, HeuristicModel, Objective, PermissionPolicy, RunOutcome, SqliteEventStore,
    ToolRegistry,
};
use std::{
    fs,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!(
        "harness-benchmark-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir_all(&root)?;
    let started = Instant::now();
    let mut completed = 0;
    let mut verified = 0;
    let mut denied = 0;
    let mut tool_calls = 0;
    let mut model_calls = 0;
    for index in 0..20 {
        let workspace = root.join(index.to_string());
        fs::create_dir_all(&workspace)?;
        let database = workspace.join(".harness/run.sqlite3");
        let objective = if index < 10 {
            "create file result.txt with content benchmark"
        } else {
            "create file ../escape.txt with content forbidden"
        };
        let outcome = {
            let store = SqliteEventStore::open(&database)?;
            let mut runtime = AgentRuntime::new(
                HeuristicModel,
                ToolRegistry::milestone_default(),
                PermissionPolicy::milestone_default(&workspace),
                store,
            );
            runtime.run(Objective::new(objective))?
        };
        if matches!(outcome, RunOutcome::Completed(_)) {
            completed += 1;
        }
        if index < 10
            && matches!(outcome, RunOutcome::Completed(_))
            && fs::read_to_string(workspace.join("result.txt"))? == "benchmark"
        {
            verified += 1;
        }
        let events = SqliteEventStore::open(&database)?.events()?;
        denied += events
            .iter()
            .filter(|e| e.kind == "PermissionDenied")
            .count();
        tool_calls += events.iter().filter(|e| e.kind == "ToolCalled").count();
        model_calls += events.iter().filter(|e| e.kind == "CognitiveStep").count();
    }
    let escaped = root.join("escape.txt").exists();
    let elapsed = started.elapsed().as_secs_f64();
    let report = serde_json::json!({"suite":"file-core-v1", "cases":20, "positive_cases":10, "negative_cases":10, "completed":completed, "independently_verified":verified, "permission_denials":denied, "tool_calls":tool_calls, "model_calls":model_calls, "tokens":0, "cost_usd":0, "retries":0, "human_interventions":0, "runtime_seconds":elapsed, "escape_created":escaped, "scope":"deterministic file tasks; no general coding or intelligence claim"});
    println!("{}", serde_json::to_string_pretty(&report)?);
    fs::remove_dir_all(root)?;
    if completed != 10 || verified != 10 || denied != 10 || escaped {
        return Err("benchmark acceptance failed".into());
    }
    Ok(())
}
