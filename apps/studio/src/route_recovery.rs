//! Fallback is a change of controller route, never replay of a failed operation.
use serde_json::Value;
pub fn may_switch(
    result: &Value,
    desktop: bool,
    review: bool,
    stopped: bool,
    switched: bool,
) -> bool {
    desktop
        && !review
        && !stopped
        && !switched
        && result["ok"] == false
        && result["known_not_applied"] == true
        && result["route_unavailable"] == true
        && result["uncertain"] != true
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_pre_dispatch_unavailability_can_switch() {
        let safe = json!({"ok":false,"known_not_applied":true,"route_unavailable":true});
        assert!(may_switch(&safe, true, false, false, false));
        for (desktop, review, stop, switched) in [
            (false, false, false, false),
            (true, true, false, false),
            (true, false, true, false),
            (true, false, false, true),
        ] {
            assert!(!may_switch(&safe, desktop, review, stop, switched));
        }
        for value in [
            json!({"ok":false}),
            json!({"ok":false,"route_unavailable":true}),
            json!({"ok":false,"known_not_applied":true,"route_unavailable":true,"uncertain":true}),
        ] {
            assert!(!may_switch(&value, true, false, false, false));
        }
    }
}
