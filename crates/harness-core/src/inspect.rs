//! Machine-readable run inspection.
//!
//! A single JSON snapshot over the checkpoint plus the event log: identity,
//! lifecycle, budgets and spend, plan rollup, history shape, and terminal
//! outcome. Powers the CLI `--inspect` flag and external watchdogs that
//! must not parse human-oriented `--events` output.

use crate::event_store::EventStore;
use std::io;

pub fn inspect_run(events: &mut impl EventStore) -> io::Result<serde_json::Value> {
    let log = events.events()?;
    let mut count = std::collections::BTreeMap::new();
    for event in &log {
        *count.entry(event.kind.clone()).or_insert(0u64) += 1;
    }
    let Some(state) = events.load()? else {
        return Ok(serde_json::json!({
            "run": null,
            "events": log.len(),
            "kinds": count,
        }));
    };
    let tasks = state
        .plan
        .tasks
        .iter()
        .map(|task| {
            serde_json::json!({
                "id": task.id,
                "goal": task.goal_id,
                "status": task.status,
                "evidence": task.evidence,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "objective": {"id": state.objective.id, "text": state.objective.text},
        "workspace": state.workspace,
        "lifecycle": state.plan.lifecycle,
        "steps": {"used": state.steps, "max": state.max_steps},
        "tools": {
            "used": state.tool_budget.as_ref().map(|budget| budget.used),
            "limit": state.tool_budget.as_ref().map(|budget| budget.limit),
        },
        "tokens_used": state.used_tokens,
        "cost_usd": state.used_cost_usd,
        "limits": {
            "wall_clock_secs": state.resource_limits.wall_clock_secs,
            "token_limit": state.resource_limits.token_limit,
            "cost_limit_usd": state.resource_limits.cost_limit_usd,
            "max_consecutive_failures": state.resource_limits.max_consecutive_failures,
        },
        "started_at_ms": state.started_at_ms,
        "history": state.history.len(),
        "pending": state.pending,
        "outcome": state.outcome,
        "goals": state.plan.goals,
        "tasks": tasks,
        "amendments": state.plan.amendments,
        "events": log.len(),
        "kinds": count,
    }))
}
