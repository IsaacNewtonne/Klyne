//! Bounded, host-owned decision recovery journal. These records are not tool receipts.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Journal {
    pub next_id: u64,
    pub goal_start_id: u64,
    pub events: Vec<Value>,
}
impl Journal {
    pub fn begin_goal(&mut self) {
        self.goal_start_id = self.next_id;
    }
    pub fn record(
        &mut self,
        episode: u64,
        role: &str,
        evidence_count: usize,
        check: &str,
        problem: Option<&str>,
    ) {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        // Resolve only checks from this request episode. A later accepted model
        // decision does not prove that an earlier tool failure was repaired.
        if problem.is_none() {
            for e in &mut self.events {
                if e["episode"] == episode && e["state"] == "open" {
                    e["state"] = json!("decision_corrected");
                    e["resolved_by"] = json!(id);
                }
            }
        }
        self.events
            .push(json!({"id":id,"episode":episode,"role":role,
            "evidence_count":evidence_count,"check":check,
            "state":if problem.is_some() {"open"} else {"accepted"},
            "problem":problem.map(|p|p.chars().take(512).collect::<String>())}));
        if self.events.len() > 64 {
            self.events.drain(..self.events.len() - 64);
        }
    }
    pub fn context(&self) -> Value {
        json!({"recent_checks":self.events.iter().rev().filter(|e| e["id"].as_u64().is_some_and(|id| id >= self.goal_start_id)).take(12).collect::<Vec<_>>(),
            "interpretation":"Host validation findings, not a definitive root cause. decision_corrected means only that a replacement decision passed checks, not that the task or tool effect succeeded. Inspect linked tool evidence before diagnosing an unresolved failure. Never replay an uncertain effect. Correct the earliest unresolved prerequisite, preserve the goal, and verify the result."})
    }
}

/// Structural provenance check, deliberately not a claim that cited text proves
/// the model's semantic interpretation. Only environment questions trigger it.
pub fn provenance_problem(value: &Value, start: usize, count: usize) -> Option<&'static str> {
    value.get("question")?;
    if !matches!(
        value["blocker"]["kind"].as_str(),
        Some("authentication" | "access" | "observed_ambiguity" | "unavailable")
    ) {
        return None;
    }
    let ids = value["blocker"]["evidence_indices"].as_array()?;
    if ids.iter().any(|id| {
        id.as_u64()
            .is_none_or(|i| i < start as u64 || i >= count as u64)
    }) {
        Some(
            "The blocker cites evidence outside the current goal. Inspect the current resource before asking the user; historical observations do not establish its present state.",
        )
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_goal_keeps_history_but_excludes_old_findings_from_context() {
        let mut journal = Journal::default();
        journal.record(
            0,
            "worker",
            0,
            "blocker",
            Some("Old unresolved prerequisite"),
        );
        journal.begin_goal();
        assert!(
            journal.context()["recent_checks"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        journal.record(1, "worker", 0, "decision", None);
        assert_eq!(
            journal.context()["recent_checks"].as_array().unwrap().len(),
            1
        );
        assert_eq!(journal.events[0]["state"], "open");
        let restored: Journal =
            serde_json::from_value(serde_json::to_value(journal).unwrap()).unwrap();
        assert_eq!(restored.goal_start_id, 1);
    }
    #[test]
    fn corrected_failure_does_not_hide_another_episode() {
        let mut j = Journal::default();
        j.record(1, "worker", 0, "schema", Some("bad decision"));
        j.record(2, "worker", 1, "blocker", Some("unsupported"));
        j.record(2, "worker", 1, "decision", None);
        assert_eq!(j.events[0]["state"], "open");
        assert_eq!(j.events[1]["state"], "decision_corrected");
        assert_eq!(j.events[1]["resolved_by"], 2);
    }
    #[test]
    fn journal_roundtrips_and_remains_bounded() {
        let mut j = Journal::default();
        for n in 0..100 {
            j.record(n, "worker", n as usize, "schema", Some(&"x".repeat(1000)));
        }
        let restored: Journal = serde_json::from_value(serde_json::to_value(&j).unwrap()).unwrap();
        assert_eq!(restored.events.len(), 64);
        assert_eq!(restored.next_id, 100);
        assert_eq!(restored.events[0]["id"], 36);
        assert_eq!(restored.events[0]["problem"].as_str().unwrap().len(), 512);
    }
    #[test]
    fn blockers_need_current_goal_evidence_but_preferences_do_not() {
        let mut q = json!({"question":"Which account?","blocker":{"kind":"observed_ambiguity","evidence_indices":[0]}});
        assert!(provenance_problem(&q, 2, 4).is_some());
        q["blocker"]["evidence_indices"] = json!([3]);
        assert!(provenance_problem(&q, 2, 4).is_none());
        q["blocker"]["evidence_indices"] = json!([4]);
        assert!(provenance_problem(&q, 2, 4).is_some());
        q["blocker"]["kind"] = json!("user_preference");
        assert!(provenance_problem(&q, 2, 4).is_none());
    }
}
