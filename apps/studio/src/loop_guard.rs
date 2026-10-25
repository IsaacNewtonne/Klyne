//! Loop, repetition and stall detection for the worker loop (2026-10-25).
//!
//! The ideas (thresholds for identical calls, repeated failures, steps with
//! no action, near-identical replies, and a soft/hard stall watchdog) are
//! adapted from Mercury Agent (MIT, (c) 2026 Cosmic Stack), reimplemented
//! here in Rust. It complements, never replaces, the existing caps: the
//! step, token, spend and time limits in `guard()`, the file-operation
//! journal in `progress.rs` and the review-round limit.
//!
//! A soft trip injects one short warning for the model. A hard trip hands
//! the task to the reviewer once; a second hard trip, or an absolute cap,
//! ends the turn as Needs input with the reason.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Same tool with identical arguments this many times in a row.
pub const IDENTICAL: usize = 4;
/// Same tool failing this many times (not necessarily in a row).
pub const FAILING: usize = 6;
/// Read and fetch tools legitimately repeat more (paging, polling).
pub const IDENTICAL_READ: usize = 6;
pub const FAILING_READ: usize = 8;
/// Worker steps in a row that produced no tool action.
pub const NO_ACTION: u32 = 6;
/// Near-identical replies (word overlap above SIMILARITY).
pub const SIMILAR_REPLIES: usize = 3;
pub const SIMILARITY: f64 = 0.75;
/// Absolute caps per goal, on top of the user's step limit.
pub const MAX_CALLS: u32 = 75;
pub const MAX_FAILURES: u32 = 20;
const WINDOW: usize = 40;
const REPLY_WINDOW: usize = 15;
const KEY_CHARS: usize = 300;

const HIGH_TOLERANCE: &[&str] = &[
    "read_file",
    "read_range",
    "list_dir",
    "search_file",
    "stat_path",
    "hash_file",
    "fetch_url",
    "browser_read",
    "browser_screenshot",
    "document_read",
    "evidence_read",
    "desktop_observe",
    "desktop_apps",
    "video_transcript",
    "runtime_status",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Trip {
    None,
    /// Warn the model once and continue.
    Soft(String),
    /// Hand off to the reviewer (first time) or stop with Needs input.
    Hard(String),
    /// Absolute cap reached: stop with Needs input.
    Cap(String),
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LoopGuard {
    /// Recent (tool, argument key, failed).
    calls: Vec<(String, String, bool)>,
    pub total_calls: u32,
    pub total_failures: u32,
    no_action: u32,
    replies: Vec<String>,
    /// Soft warnings already given (by kind) since the last handoff.
    warned: Vec<String>,
    /// Hard trips handed to the reviewer this goal.
    pub handoffs: u32,
}

fn tolerant(tool: &str) -> bool {
    HIGH_TOLERANCE.contains(&tool)
}

fn key(action: &Value) -> String {
    let mut action = action.clone();
    if let Some(object) = action.as_object_mut() {
        object.remove("tool");
    }
    action.to_string().chars().take(KEY_CHARS).collect()
}

fn words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(str::to_owned)
        .collect()
}

/// Word overlap of two replies: shared words over the larger word set.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (words(a), words(b));
    let larger = a.len().max(b.len());
    if larger == 0 {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / larger as f64
}

impl LoopGuard {
    /// One executed tool action and whether it failed.
    pub fn record_action(&mut self, action: &Value, failed: bool) {
        let tool = action["tool"].as_str().unwrap_or("").to_owned();
        self.calls.push((tool, key(action), failed));
        if self.calls.len() > WINDOW {
            self.calls.remove(0);
        }
        self.total_calls = self.total_calls.saturating_add(1);
        if failed {
            self.total_failures = self.total_failures.saturating_add(1);
        }
        self.no_action = 0;
    }
    /// A worker step that produced no tool action (for example a rejected
    /// completion), with the text the model returned.
    pub fn record_no_action(&mut self, reply: &str) {
        self.no_action = self.no_action.saturating_add(1);
        self.record_reply(reply);
    }
    pub fn record_reply(&mut self, reply: &str) {
        if reply.trim().is_empty() {
            return;
        }
        self.replies.push(reply.chars().take(2000).collect());
        if self.replies.len() > REPLY_WINDOW {
            self.replies.remove(0);
        }
    }
    fn identical_run(&self) -> (usize, &str) {
        let Some((tool, key, _)) = self.calls.last() else {
            return (0, "");
        };
        let run = self
            .calls
            .iter()
            .rev()
            .take_while(|(t, k, _)| t == tool && k == key)
            .count();
        (run, tool)
    }
    fn failures_of_last(&self) -> (usize, &str) {
        let Some((tool, _, true)) = self.calls.last() else {
            return (0, "");
        };
        (
            self.calls
                .iter()
                .filter(|(t, _, f)| t == tool && *f)
                .count(),
            tool,
        )
    }
    fn similar_replies(&self) -> usize {
        let Some(last) = self.replies.last() else {
            return 0;
        };
        1 + self.replies[..self.replies.len() - 1]
            .iter()
            .filter(|r| similarity(r, last) > SIMILARITY)
            .count()
    }
    fn trip(
        &mut self,
        kind: &str,
        count: usize,
        threshold: usize,
        soft: String,
        hard: String,
    ) -> Trip {
        if count < threshold {
            return Trip::None;
        }
        if !self.warned.iter().any(|w| w == kind) {
            self.warned.push(kind.to_owned());
            return Trip::Soft(soft);
        }
        if count >= threshold + 2 {
            return Trip::Hard(hard);
        }
        Trip::None
    }
    /// The strongest condition now present.
    pub fn check(&mut self) -> Trip {
        if self.total_calls >= MAX_CALLS {
            return Trip::Cap(format!(
                "I stopped after {MAX_CALLS} tool calls in this goal without finishing."
            ));
        }
        if self.total_failures >= MAX_FAILURES {
            return Trip::Cap(format!(
                "I stopped after {MAX_FAILURES} failed tool calls in this goal."
            ));
        }
        let (run, tool) = self.identical_run();
        let tool = tool.to_owned();
        let limit = if tolerant(&tool) {
            IDENTICAL_READ
        } else {
            IDENTICAL
        };
        let trip = self.trip(
            "identical",
            run,
            limit,
            format!("You have called {tool} with the same arguments {run} times in a row. The result will not change. Use the result you already have, try a different approach, or finish."),
            format!("The worker kept calling {tool} with identical arguments ({run} times in a row)."),
        );
        if trip != Trip::None {
            return trip;
        }
        let (fails, tool) = self.failures_of_last();
        let tool = tool.to_owned();
        let limit = if tolerant(&tool) {
            FAILING_READ
        } else {
            FAILING
        };
        let trip = self.trip(
            "failing",
            fails,
            limit,
            format!("{tool} has failed {fails} times. Stop retrying it the same way: change the approach, use another tool, or report the blocker."),
            format!("{tool} kept failing ({fails} times)."),
        );
        if trip != Trip::None {
            return trip;
        }
        let idle = self.no_action as usize;
        let trip = self.trip(
            "no_action",
            idle,
            NO_ACTION as usize - 2,
            format!("{idle} steps in a row produced no tool action. Call the tool that does the next part of the task now."),
            format!("The worker went {idle} steps in a row without taking any action."),
        );
        if trip != Trip::None {
            return trip;
        }
        let similar = self.similar_replies();
        self.trip(
            "replies",
            similar,
            SIMILAR_REPLIES,
            "Your last replies are nearly identical. Repeating the same answer will not be accepted; do the missing work with a tool, or report what blocks it.".into(),
            format!("The worker repeated nearly the same reply {similar} times."),
        )
    }
    /// After a reviewer handoff: forget the recent pattern, keep the totals.
    pub fn handed_off(&mut self) {
        self.handoffs = self.handoffs.saturating_add(1);
        self.calls.clear();
        self.replies.clear();
        self.warned.clear();
        self.no_action = 0;
    }
}

/// Stall watchdog thresholds, from quality/config.json `"stall"`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StallLimits {
    pub soft_seconds: u64,
    pub hard_seconds: u64,
}
impl Default for StallLimits {
    fn default() -> Self {
        Self {
            soft_seconds: 180,
            hard_seconds: 480,
        }
    }
}
impl StallLimits {
    pub fn parse(value: &Value) -> Self {
        let soft = value["soft_seconds"]
            .as_u64()
            .unwrap_or(180)
            .clamp(10, 3600);
        let hard = value["hard_seconds"]
            .as_u64()
            .unwrap_or(480)
            .clamp(soft + 10, 7200);
        Self {
            soft_seconds: soft,
            hard_seconds: hard,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Shared heartbeat for one running goal. Any saved progress (model
/// stream, tool result, step change) calls `touch`.
#[derive(Debug)]
pub struct Watch {
    last: AtomicU64,
    pub soft: AtomicBool,
    pub hard: AtomicBool,
    pub done: AtomicBool,
}
impl Default for Watch {
    fn default() -> Self {
        Self {
            last: AtomicU64::new(now_ms()),
            soft: AtomicBool::new(false),
            hard: AtomicBool::new(false),
            done: AtomicBool::new(false),
        }
    }
}
#[derive(Debug, PartialEq)]
pub enum Tick {
    Quiet,
    Soft,
    Hard,
}
impl Watch {
    pub fn touch(&self) {
        self.last.store(now_ms(), Ordering::SeqCst);
        self.soft.store(false, Ordering::SeqCst);
    }
    pub fn silent_ms(&self) -> u64 {
        now_ms().saturating_sub(self.last.load(Ordering::SeqCst))
    }
    /// Evaluate at `now`; the soft notice fires once per silence.
    pub fn tick(&self, limits: StallLimits) -> Tick {
        let silent = self.silent_ms();
        if silent >= limits.hard_seconds * 1000 {
            self.hard.store(true, Ordering::SeqCst);
            return Tick::Hard;
        }
        if silent >= limits.soft_seconds * 1000 && !self.soft.swap(true, Ordering::SeqCst) {
            return Tick::Soft;
        }
        Tick::Quiet
    }
    #[cfg(test)]
    pub fn age(&self, ms: u64) {
        self.last.fetch_sub(ms, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn act(g: &mut LoopGuard, action: Value, failed: bool) -> Trip {
        g.record_action(&action, failed);
        g.check()
    }

    #[test]
    fn identical_calls_warn_then_hand_off() {
        let mut g = LoopGuard::default();
        let a = json!({"tool":"write_file","path":"a.txt","contents":"x"});
        for _ in 0..3 {
            assert_eq!(act(&mut g, a.clone(), false), Trip::None);
        }
        assert!(matches!(act(&mut g, a.clone(), false), Trip::Soft(m) if m.contains("write_file")));
        assert_eq!(act(&mut g, a.clone(), false), Trip::None);
        assert!(matches!(act(&mut g, a.clone(), false), Trip::Hard(_)));
        g.handed_off();
        assert_eq!(act(&mut g, a, false), Trip::None);
        assert_eq!(g.handoffs, 1);
    }

    #[test]
    fn read_tools_get_more_room_and_different_args_reset() {
        let mut g = LoopGuard::default();
        let r = json!({"tool":"read_file","path":"a.txt"});
        for _ in 0..5 {
            assert_eq!(act(&mut g, r.clone(), false), Trip::None);
        }
        assert!(matches!(act(&mut g, r, false), Trip::Soft(_)));
        let mut g = LoopGuard::default();
        for i in 0..10 {
            let t = act(
                &mut g,
                json!({"tool":"write_file","path":format!("{i}.txt")}),
                false,
            );
            assert_eq!(t, Trip::None);
        }
    }

    #[test]
    fn repeated_failures_of_one_tool_trip() {
        let mut g = LoopGuard::default();
        for i in 0..5 {
            let t = act(
                &mut g,
                json!({"tool":"run_shell","command":format!("x{i}")}),
                true,
            );
            assert_eq!(t, Trip::None);
        }
        assert!(
            matches!(act(&mut g, json!({"tool":"run_shell","command":"y"}), true), Trip::Soft(m) if m.contains("failed 6"))
        );
    }

    #[test]
    fn steps_without_action_and_similar_replies_trip() {
        let mut g = LoopGuard::default();
        g.record_no_action("first idea");
        g.record_no_action("a totally different second idea");
        g.record_no_action("and a third unrelated one");
        assert_eq!(g.check(), Trip::None);
        g.record_no_action("fourth: something else entirely");
        assert!(matches!(g.check(), Trip::Soft(m) if m.contains("no tool action")));
        let mut g = LoopGuard::default();
        let reply = "I saved the summary to summary.txt in the workspace folder";
        g.record_reply(reply);
        g.record_reply("I saved the summary to summary.txt in the workspace");
        assert_eq!(g.check(), Trip::None);
        g.record_reply(reply);
        assert!(matches!(g.check(), Trip::Soft(m) if m.contains("nearly identical")));
        assert!(similarity("a b c d", "w x y z") < 0.1);
    }

    #[test]
    fn absolute_caps_stop() {
        let mut g = LoopGuard::default();
        for i in 0..MAX_CALLS {
            g.record_action(&json!({"tool":"list_dir","path":format!("{i}")}), false);
        }
        assert!(matches!(g.check(), Trip::Cap(_)));
        let mut g = LoopGuard::default();
        for i in 0..MAX_FAILURES {
            g.record_action(&json!({"tool":format!("t{}", i % 7),"n":i}), true);
        }
        assert!(matches!(g.check(), Trip::Cap(m) if m.contains("failed")));
    }

    #[test]
    fn watchdog_soft_once_then_hard_and_activity_resets() {
        let limits = StallLimits {
            soft_seconds: 180,
            hard_seconds: 480,
        };
        let w = Watch::default();
        assert_eq!(w.tick(limits), Tick::Quiet);
        w.age(181_000);
        assert_eq!(w.tick(limits), Tick::Soft);
        assert_eq!(w.tick(limits), Tick::Quiet);
        w.touch();
        assert_eq!(w.tick(limits), Tick::Quiet);
        w.age(481_000);
        assert_eq!(w.tick(limits), Tick::Hard);
        assert!(w.hard.load(Ordering::SeqCst));
        assert_eq!(StallLimits::parse(&Value::Null), limits);
        let s = StallLimits::parse(&json!({"soft_seconds":60,"hard_seconds":30}));
        assert_eq!((s.soft_seconds, s.hard_seconds), (60, 70));
    }
}
