//! Restart checks require fresh state and the original process lifetime.
use crate::desktop::DesktopAction;
use harness_core::operation_check::CheckOutcome;
use serde_json::{Value, json};

pub fn identity(action: &DesktopAction, observation: Option<&Value>) -> Option<Value> {
    let window = action.window()?;
    let rows = observation?.get("windows")?.as_array()?;
    let mut matches = rows.iter().filter(|r| r["window"] == window);
    let row = matches.next()?;
    if matches.next().is_some()
        || row["process_id"].as_u64().unwrap_or(0) == 0
        || row["process_started"].as_str().unwrap_or("").is_empty()
    {
        return None;
    }
    Some(
        json!({"window":window,"process_id":row["process_id"],"process_started":row["process_started"],"title":row["title"]}),
    )
}
pub fn supported(pending: &Value, now: u64) -> bool {
    let Some(recorded) = pending["recorded_at"].as_u64() else {
        return false;
    };
    if recorded > now || now - recorded > 600_000 || !pending["target_identity"].is_object() {
        return false;
    }
    serde_json::from_value::<DesktopAction>(pending["action"].clone())
        .ok()
        .is_some_and(|a| a.postcondition().is_some())
}
pub fn verified(pending: &Value, observation: &Value, now: u64) -> bool {
    if !supported(pending, now) {
        return false;
    }
    let Ok(action) = serde_json::from_value::<DesktopAction>(pending["action"].clone()) else {
        return false;
    };
    if identity(&action, Some(observation)).as_ref() != Some(&pending["target_identity"]) {
        return false;
    }
    action
        .postcondition()
        .is_some_and(|check| check.evaluate(observation).outcome == CheckOutcome::Verified)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recycled_handle_expired_intent_and_legacy_actions_stay_unresolved() {
        let observation = json!({"foreground":"42","windows":[{"window":"42","title":"Editor","process_id":123,"process_started":"1000","minimized":false}]});
        let action = DesktopAction::Focus {
            window: "42".into(),
        };
        let pending = json!({"action":action,"target_identity":identity(&action,Some(&observation)),"recorded_at":100});
        assert!(verified(&pending, &observation, 200));
        for field in ["process_id", "process_started", "title"] {
            let mut changed = observation.clone();
            changed["windows"][0][field] = json!("different");
            assert!(!verified(&pending, &changed, 200));
        }
        assert!(!verified(&pending, &observation, 700_000));
        assert!(!verified(&pending, &observation, 99));
        assert!(!verified(&json!({"action":action}), &observation, 200));
        let unsafe_action = json!({"action":{"tool":"desktop_key","window":"42","key":"ENTER"},"target_identity":pending["target_identity"],"recorded_at":100});
        assert!(!verified(&unsafe_action, &observation, 200));
    }
}
