//! Token-budgeted context window (2026-10-25).
//!
//! Idea adapted from Mercury Agent (MIT), reimplemented for Klyne: the
//! chat history is filled newest-first inside a token budget sized for
//! Ollama's num_ctx 16384, the latest exchange is always kept whole,
//! older tool activity is shown as one-line traces instead of raw
//! outputs, and memory/warnings travel in a separate system block.
//! Receipts and evidence are untouched; this only shapes the prompt.

use serde_json::{Value, json};

/// History budget (tokens, chars/4) in compact and full mode. With the
/// system prompt, tools and observations this keeps a 9B model inside
/// num_ctx 16384.
pub const COMPACT_HISTORY_TOKENS: usize = 3000;
pub const FULL_HISTORY_TOKENS: usize = 6000;
/// Raw observations from the current goal.
pub const RAW_OBSERVATIONS: usize = 6;
/// Newest successful entries of the previous goal kept raw for follow-ups.
pub const CARRY_OVER: usize = 3;
/// One-line traces of older tool activity.
pub const TRACE_LINES: usize = 12;
/// Older (not latest exchange) messages are clipped to this many chars.
const OLDER_MESSAGE_CHARS: usize = 1500;

pub fn tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        format!(
            "{}... [truncated]",
            text.chars().take(limit).collect::<String>()
        )
    }
}

/// Newest-first history inside `budget` tokens. The last user message and
/// everything after it are kept whole; older messages are clipped and
/// added until the budget is spent.
pub fn messages(history: &[Value], budget: usize) -> Vec<Value> {
    let last_user = history
        .iter()
        .rposition(|m| m["role"] == "user")
        .unwrap_or(history.len().saturating_sub(1).min(history.len()));
    let mut kept: Vec<Value> = history[last_user..].to_vec();
    let mut used: usize = kept
        .iter()
        .map(|m| tokens(m["text"].as_str().unwrap_or("")) + 4)
        .sum();
    let mut older = Vec::new();
    for message in history[..last_user].iter().rev() {
        let mut message = message.clone();
        let text = clip(message["text"].as_str().unwrap_or(""), OLDER_MESSAGE_CHARS);
        let cost = tokens(&text) + 4;
        if used + cost > budget {
            break;
        }
        used += cost;
        message["text"] = json!(text);
        older.push(message);
    }
    older.reverse();
    older.append(&mut kept);
    older
}

/// One line for one evidence entry: `tool arg → ok (N lines)` or
/// `tool arg → ✗ error`.
pub fn trace(entry: &Value) -> String {
    let action = entry["action"].as_str().unwrap_or("?");
    let (tool, arg) = action.split_once(':').unwrap_or((action, ""));
    let arg = arg.trim();
    let head = if arg.is_empty() {
        tool.to_owned()
    } else {
        format!("{tool} {}", clip(arg, 60))
    };
    if entry["ok"] == false {
        let why = entry["summary"]
            .as_str()
            .or_else(|| entry["data"].as_str())
            .unwrap_or("failed");
        format!("{head} → ✗ {}", clip(why.lines().next().unwrap_or(""), 80))
    } else {
        let lines = match &entry["data"] {
            Value::String(text) => text.lines().count(),
            Value::Null => 0,
            other => other.to_string().lines().count(),
        };
        format!("{head} → ok ({lines} lines)")
    }
}

/// Raw observations (current goal only, newest `RAW_OBSERVATIONS`) and
/// one-line traces of everything older (earlier goals included).
pub fn observations(evidence: &[Value], goal_start: usize) -> (Vec<Value>, Vec<String>) {
    let goal_start = goal_start.min(evidence.len());
    let raw_from = evidence
        .len()
        .saturating_sub(RAW_OBSERVATIONS)
        .max(goal_start);
    let mut raw = evidence[raw_from..].to_vec();
    // A follow-up ("personalize it") still needs the previous goal's
    // result: carry its newest successful entries raw while this goal has
    // little evidence of its own.
    let mut carried = 0;
    if raw.len() < CARRY_OVER {
        let carry: Vec<Value> = evidence[..goal_start]
            .iter()
            .rev()
            .filter(|e| e["ok"] == true)
            .take(CARRY_OVER - raw.len())
            .cloned()
            .collect();
        carried = carry.len();
        for entry in carry {
            raw.insert(0, entry);
        }
    }
    let mut traces: Vec<String> = evidence[..raw_from]
        .iter()
        .rev()
        .filter(|e| !(carried > 0 && e["ok"] == true && raw.contains(e)))
        .take(TRACE_LINES)
        .map(trace)
        .collect();
    traces.reverse();
    (raw, traces)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_the_last_exchange_whole_and_fills_newest_first() {
        let mut history: Vec<Value> = (0..30)
            .map(|i| json!({"role": if i % 2 == 0 {"user"} else {"assistant"}, "text": format!("message {i} {}", "x".repeat(800))}))
            .collect();
        history.push(json!({"role":"user","text":"y".repeat(20_000)}));
        history.push(json!({"role":"assistant","text":"working"}));
        let kept = messages(&history, 3000);
        // The huge last user message survives whole even past the budget.
        assert_eq!(kept[kept.len() - 2]["text"].as_str().unwrap().len(), 20_000);
        assert_eq!(kept.last().unwrap()["text"], "working");
        let small = messages(&history[..30], 3000);
        assert!(small.len() < 30 && small.len() >= 10, "{}", small.len());
        assert!(
            small.last().unwrap()["text"]
                .as_str()
                .unwrap()
                .starts_with("message 29")
        );
        let total: usize = small
            .iter()
            .map(|m| tokens(m["text"].as_str().unwrap()) + 4)
            .sum();
        assert!(total <= 3000);
    }

    #[test]
    fn older_tool_activity_becomes_one_line_traces() {
        let mut evidence = vec![
            json!({"action":"read_file:notes.txt","ok":true,"data":"a\nb\nc"}),
            json!({"action":"fetch:https://example.com","ok":false,"summary":"HTTP 404 Not Found\nmore"}),
        ];
        for i in 0..7 {
            evidence.push(json!({"action":format!("list_dir:d{i}"),"ok":true,"data":"x"}));
        }
        let (raw, traces) = observations(&evidence, 2);
        assert_eq!(raw.len(), 6);
        assert_eq!(traces[0], "read_file notes.txt → ok (3 lines)");
        assert_eq!(
            traces[1],
            "fetch https://example.com → ✗ HTTP 404 Not Found"
        );
        assert_eq!(traces[2], "list_dir d0 → ok (1 lines)");
        // A fresh follow-up goal carries the previous goal's newest
        // successful results raw; failures stay traces.
        let (raw, traces) = observations(&evidence[..2], 2);
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0]["action"], "read_file:notes.txt");
        assert_eq!(
            traces,
            vec!["fetch https://example.com → ✗ HTTP 404 Not Found"]
        );
    }
}
