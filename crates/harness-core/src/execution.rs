//! Unified execution contract (Phase 1).
//!
//! Every tool route — core filesystem/shell tools, Studio browser, desktop,
//! MCP, capabilities, and local APIs — must answer the same question after a
//! call: **what happened to the outside world?** This module gives that
//! answer one shared vocabulary so dispatchers stop reimplementing it with
//! per-branch substring gates.
//!
//! | [`EffectState`] | Meaning | Pending action |
//! |---|---|---|
//! | `NotApplied` | Proven no external effect (validation refusal, known-not-applied) | Clear; safe to plan a different action |
//! | `Applied` | Effect verified through the route's observation | Clear; advance the task |
//! | `Unknown` | Dispatch may have mutated state but the receipt is missing | **Keep** pending; inspect before any retry or route switch |
//!
//! The rule is absolute: any post-dispatch observation failure preserves
//! uncertainty. A failed read-back after a successful mutation is `Unknown`,
//! never `NotApplied`.

use crate::types::Observation;
use serde::{Deserialize, Serialize};

/// What a tool call did to the outside world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectState {
    /// Proven to have no external effect. Safe to try something else.
    NotApplied,
    /// Applied and verified through the route's observation.
    Applied,
    /// May have applied; the receipt is missing. Never blind-replay.
    Unknown,
}

/// Typed tool result: the effect plus the human/model-facing detail.
/// This is the shape every adapter should converge on; [`Observation`]
/// converts losslessly via [`ToolResult::from_observation`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub effect: EffectState,
    pub ok: bool,
    pub summary: String,
    pub data: String,
}

impl ToolResult {
    pub fn from_observation(observation: Observation, mutating: bool) -> Self {
        let effect = effect_of_observation(&observation, mutating);
        Self {
            effect,
            ok: observation.ok,
            summary: observation.summary,
            data: observation.data,
        }
    }

    /// Whether the dispatcher must keep the pending action unresolved.
    pub fn keep_pending(&self) -> bool {
        self.effect == EffectState::Unknown
    }
}

/// Single uncertainty classifier shared by all routes. Any summary produced
/// after a mutation dispatch that contains an uncertainty marker means the
/// receipt is missing. Case-insensitive; matches the marker strings emitted
/// by the supervised adapters (`*_outcome *uncertain*`, `*may be uncertain*`).
pub fn is_uncertain_text(summary: &str) -> bool {
    summary.to_ascii_lowercase().contains("uncertain")
}

/// Classify a typed core [`Observation`].
///
/// - `ok == false` + uncertainty marker → `Unknown` (keep pending).
/// - `ok == false` otherwise → `NotApplied` (validation refusal, denied
///   permission, known failure before dispatch).
/// - `ok == true` + mutating action → `Applied`.
/// - `ok == true` + read-only action → `NotApplied` (reads change nothing).
pub fn effect_of_observation(observation: &Observation, mutating: bool) -> EffectState {
    if !observation.ok {
        if is_uncertain_text(&observation.summary) {
            return EffectState::Unknown;
        }
        return EffectState::NotApplied;
    }
    if mutating {
        EffectState::Applied
    } else {
        EffectState::NotApplied
    }
}

/// Classify an untyped JSON route result (`browser_*`, `mcp_*`, `desktop_*`,
/// `app_*`, capability `tool_run`).
///
/// - `uncertain == true`, or an `error` string with an uncertainty marker
///   (covers post-dispatch read-back failures mapped into `error`), →
///   `Unknown`.
/// - `ok == false` otherwise → `NotApplied`. Note: several read routes omit
///   `ok` on success shapes (`{title,text}`, `{screenshot}`, `{closed}`),
///   so a missing `ok` counts as success here; callers that need strictness
///   must check their own schema first.
/// - success + `mutating` → `Applied`, else `NotApplied`.
pub fn effect_of_value(value: &serde_json::Value, mutating: bool) -> EffectState {
    if value.get("uncertain").and_then(serde_json::Value::as_bool) == Some(true) {
        return EffectState::Unknown;
    }
    if let Some(error) = value.get("error").and_then(serde_json::Value::as_str)
        && is_uncertain_text(error)
    {
        return EffectState::Unknown;
    }
    let ok = value
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    if !ok {
        return EffectState::NotApplied;
    }
    if mutating {
        EffectState::Applied
    } else {
        EffectState::NotApplied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(ok: bool, summary: &str) -> Observation {
        Observation {
            ok,
            summary: summary.into(),
            data: String::new(),
        }
    }

    #[test]
    fn uncertain_summaries_keep_pending() {
        for summary in [
            "Browser navigation outcome uncertain: reset",
            "Desktop call stopped or timed out; any pending input has an uncertain outcome.",
            "tool timed out; process stopped; external outcome may be uncertain",
            "MCP request stopped or timed out; outcome is uncertain",
            "BROWSER OUTCOME UNCERTAIN",
        ] {
            assert!(is_uncertain_text(summary), "{summary}");
            assert_eq!(
                effect_of_observation(&obs(false, summary), true),
                EffectState::Unknown
            );
        }
    }

    #[test]
    fn plain_failures_are_known_not_applied() {
        for summary in [
            "permission denied",
            "invalid path",
            "tool not found: workspace_fs",
            "Browser requires Web access",
            "Unknown browser tool",
            "No browser session; use browser_open or browser_attach",
        ] {
            assert!(!is_uncertain_text(summary), "{summary}");
            assert_eq!(
                effect_of_observation(&obs(false, summary), true),
                EffectState::NotApplied
            );
        }
    }

    #[test]
    fn success_maps_by_mutation() {
        assert_eq!(
            effect_of_observation(&obs(true, "wrote a.txt"), true),
            EffectState::Applied
        );
        assert_eq!(
            effect_of_observation(&obs(true, "read a.txt"), false),
            EffectState::NotApplied
        );
    }

    #[test]
    fn value_routes_cover_all_branch_shapes() {
        // MCP/capability/app explicit flag.
        assert_eq!(
            effect_of_value(&serde_json::json!({"ok": false, "uncertain": true}), true),
            EffectState::Unknown
        );
        // Browser post-dispatch read-back failure mapped into `error`
        // (browser_tools tags these `uncertain`; see Browsers::execute).
        assert_eq!(
            effect_of_value(
                &serde_json::json!({"ok": false, "error": "Browser observation outcome uncertain after navigation: connection reset"}),
                true
            ),
            EffectState::Unknown
        );
        // Plain dispatch refusal.
        assert_eq!(
            effect_of_value(
                &serde_json::json!({"ok": false, "error": "No browser session; use browser_open"}),
                true
            ),
            EffectState::NotApplied
        );
        // Browser success shapes omit `ok`.
        assert_eq!(
            effect_of_value(&serde_json::json!({"title": "t", "text": "x"}), true),
            EffectState::Applied
        );
        assert_eq!(
            effect_of_value(&serde_json::json!({"screenshot": "browser.png"}), false),
            EffectState::NotApplied
        );
        // Desktop known-not-applied recovery signal stays clearable.
        assert_eq!(
            effect_of_value(
                &serde_json::json!({"ok": false, "known_not_applied": true}),
                true
            ),
            EffectState::NotApplied
        );
    }

    #[test]
    fn tool_result_round_trips_observation() {
        let result =
            ToolResult::from_observation(obs(false, "timed out; outcome may be uncertain"), true);
        assert!(result.keep_pending());
        assert!(!result.ok);
        let result = ToolResult::from_observation(obs(false, "permission denied"), true);
        assert!(!result.keep_pending());
    }
}
