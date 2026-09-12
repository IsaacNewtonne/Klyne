//! Conservative guard for unsupported delivery claims, independent of access toggles.
pub fn delivery_claim(text: &str) -> bool {
    let text=text.to_lowercase();
    let message=["message","email","e-mail","sent to","delivered to"].iter().any(|s|text.contains(s));
    message && ["send", "sent", "deliver", "messaged", "emailed"].iter().any(|s|text.contains(s))
}
pub fn check(goal:&str, answer:&str)->std::io::Result<()> {
    if delivery_claim(goal) || delivery_claim(answer) {
        return Err(crate::err("Message delivery is unverified. Klyne has no supported delivery receipt for this task and cannot confirm that a message was sent. Inspect the destination before retrying. If no action occurred, enable the required app or desktop access before continuing."));
    }
    Ok(())
}
pub fn check_action_evidence(answer:&str,evidence:&[serde_json::Value])->std::io::Result<()> {
    let lower=answer.to_lowercase();
    let claims=["was opened","i opened","i saved","was saved","i created","file created","file is ready","i deleted","was deleted","i installed","was installed","i uploaded","was uploaded","i clicked","successfully sent","i sent"];
    let tool_evidence=evidence.iter().any(|e| e["ok"]==true && e["agent"]!="Route controller" && e["agent"]!="User reconciliation" && e["action"]!="sample_outline");
    if !tool_evidence && claims.iter().any(|claim|lower.contains(claim)) {
        return Err(crate::err("The proposed answer claims an app or file action, but this task has no successful tool evidence. That action is not confirmed. The task is incomplete."));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn invented_delivery_is_not_success() {
        assert!(super::check("open zalo, send a message to joidi","Task complete").is_err());
        assert!(super::check("hi","The message was successfully sent to him").is_err());
        assert!(super::check("hi","Hi! How can I help?").is_ok());
        assert!(super::check_action_evidence("Zalo was opened",&[]).is_err());
        assert!(super::check_action_evidence("Here is a project plan",&[]).is_ok());
    }
}
