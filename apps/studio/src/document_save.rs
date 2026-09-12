//! First save adapter: Notepad Ctrl+S plus caller-owned whole-file evidence.
use crate::desktop::DesktopAction;
use harness_core::{
    PermissionPolicy,
    task_contract::{TaskContract, TaskOutcome, TaskResult},
    verification::SuccessCriterion,
};
use serde_json::Value;
use std::path::Path;

pub fn capture(
    action: &DesktopAction,
    observation: Option<&Value>,
    contract: Option<&TaskContract>,
) -> Option<TaskContract> {
    let DesktopAction::Key { window, key } = action else {
        return None;
    };
    if !key.eq_ignore_ascii_case("CTRL+S") {
        return None;
    }
    let observation = observation?;
    if observation["foreground"] != *window {
        return None;
    }
    let mut windows = observation["windows"]
        .as_array()?
        .iter()
        .filter(|w| w["window"] == *window);
    let target = windows.next()?;
    if windows.next().is_some()
        || !target["process_name"]
            .as_str()?
            .eq_ignore_ascii_case("notepad")
    {
        return None;
    }
    let contract = contract?;
    if contract.criteria.is_empty()
        || !contract.criteria.iter().all(|c| {
            matches!(
                c,
                SuccessCriterion::FileContents { .. } | SuccessCriterion::FileDigest { .. }
            )
        })
    {
        return None;
    }
    Some(contract.clone())
}
pub fn verify(
    pending: &Value,
    workspace: &Path,
    current: Option<&TaskContract>,
    now: u64,
) -> Option<TaskResult> {
    if pending["save_adapter"] != "notepad" {
        return None;
    }
    let recorded = pending["recorded_at"].as_u64()?;
    if recorded > now || now - recorded > 600_000 {
        return None;
    }
    let action: DesktopAction = serde_json::from_value(pending["action"].clone()).ok()?;
    if !matches!(action,DesktopAction::Key{ref key,..} if key.eq_ignore_ascii_case("CTRL+S")) {
        return None;
    }
    let saved: TaskContract = serde_json::from_value(pending["save_contract"].clone()).ok()?;
    if serde_json::to_value(current?).ok()? != pending["save_contract"] {
        return None;
    }
    saved.validate(&saved.goal).ok()?;
    if !saved.criteria.iter().all(|c| {
        matches!(
            c,
            SuccessCriterion::FileContents { .. } | SuccessCriterion::FileDigest { .. }
        )
    }) {
        return None;
    }
    let mut policy = PermissionPolicy::milestone_default(workspace);
    policy.revoke_capability(harness_core::permissions::Capability::FilesystemWrite);
    let result = saved.verify(&policy);
    matches!(result.outcome, TaskOutcome::Verified).then_some(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn exact_saved_file_resolves_but_missing_partial_or_changed_files_do_not() {
        let root = tempfile::tempdir().unwrap();
        let contract = TaskContract::from_goal("write file note.txt :: exact contents").unwrap();
        let action = DesktopAction::Key {
            window: "42".into(),
            key: "CTRL+S".into(),
        };
        let observed =
            json!({"foreground":"42","windows":[{"window":"42","process_name":"Notepad"}]});
        let saved = capture(&action, Some(&observed), Some(&contract)).unwrap();
        let pending = json!({"action":action,"save_adapter":"notepad","save_contract":saved,"recorded_at":100});
        assert!(verify(&pending, root.path(), Some(&contract), 200).is_none());
        std::fs::write(root.path().join("note.txt"), "exact").unwrap();
        assert!(verify(&pending, root.path(), Some(&contract), 200).is_none());
        std::fs::write(root.path().join("note.txt"), "exact contents").unwrap();
        assert!(verify(&pending, root.path(), Some(&contract), 200).is_some());
        assert!(verify(&pending, root.path(), Some(&contract), 700_000).is_none());
        let other = TaskContract::from_goal("write file other.txt :: exact contents").unwrap();
        assert!(verify(&pending, root.path(), Some(&other), 200).is_none());
        assert!(
            capture(
                &action,
                Some(
                    &json!({"foreground":"42","windows":[{"window":"42","process_name":"browser"}]})
                ),
                Some(&contract)
            )
            .is_none()
        );
        let escape = TaskContract::from_goal("write file ../note.txt :: exact contents").unwrap();
        let escaped = json!({"action":action,"save_adapter":"notepad","save_contract":escape,"recorded_at":100});
        assert!(verify(&escaped, root.path(), Some(&escape), 200).is_none());
    }
}
