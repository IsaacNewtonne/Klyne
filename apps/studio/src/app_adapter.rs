//! Transport-independent capability descriptions. Connectivity is not task proof.
use serde::Serialize;
use serde_json::{Value, json};
use std::{io, path::Path};

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Mcp,
    Api,
    Desktop,
}

#[derive(Debug, Serialize)]
pub struct Adapter {
    pub version: u32,
    pub transport: Transport,
    pub target: String,
    pub discovery_tool: &'static str,
    pub action_tools: Vec<&'static str>,
    pub connection_check: &'static str,
    pub verification: &'static str,
    pub reconciliation: &'static str,
    pub automatic_replay: bool,
}

impl Adapter {
    pub fn new(transport: Transport, target: &str, checked: bool) -> Self {
        let (discovery, actions, check) = match transport {
            Transport::Mcp => (
                "mcp_tools",
                vec!["mcp_call"],
                if checked { "probe_passed" } else { "unchecked" },
            ),
            Transport::Api => (
                "app_operations",
                vec!["app_invoke", "app_call"],
                "operations_untested",
            ),
            Transport::Desktop => (
                "desktop_observe",
                vec![
                    "desktop_focus",
                    "desktop_launch",
                    "desktop_click",
                    "desktop_type",
                    "desktop_key",
                    "desktop_scroll",
                    "desktop_invoke",
                    "desktop_fill",
                ],
                "observation_required",
            ),
        };
        Self {
            version: 1,
            transport,
            target: target.into(),
            discovery_tool: discovery,
            action_tools: actions,
            connection_check: check,
            verification: "Define the requested postcondition before acting; check destination state afterward. Tool return and connectivity do not verify task success.",
            reconciliation: "After an uncertain effect, inspect destination state. Do not repeat through this or another transport without establishing the effect was not applied.",
            automatic_replay: false,
        }
    }
}

/// API identity is supplied explicitly; display-name similarity is not a binding.
pub fn select(root: &Path, app_name: &str, app_id: &str, api: Option<&str>) -> io::Result<Value> {
    if let Some(name) = api {
        // Reads saved definitions only; selecting an adapter must not test a write.
        let operations = crate::local_apps::execute(
            root,
            &json!({"tool":"app_operations","name":name}),
            true,
            true,
        )?;
        if operations["total"].as_u64().unwrap_or(0) == 0 {
            return Err(crate::err("The linked API has no saved operations"));
        }
        let adapter = Adapter::new(Transport::Api, name, false);
        return Ok(
            json!({"route":"api","app":app_name,"app_id":app_id,"connection":name,"adapter":adapter,
            "instruction":format!("Use the explicitly linked API {name}: discover operations with app_operations, then use app_invoke. Operations are untested; verify the requested result."),
            "fallback":{"route":"desktop","app_id":app_id,"requires":"desktop access and a known-not-applied outcome before retrying an effect"}}),
        );
    }
    let mut route = crate::mcp::route(root, app_name);
    attach(&mut route, app_id);
    Ok(route)
}

fn attach(route: &mut Value, app_id: &str) {
    let mcp = route["route"] == "mcp" && route["probe"]["ok"] == true;
    let target = if mcp {
        route["server"].as_str().unwrap_or("")
    } else {
        app_id
    };
    let adapter = Adapter::new(
        if mcp {
            Transport::Mcp
        } else {
            Transport::Desktop
        },
        target,
        mcp,
    );
    route["instruction"] = json!(if mcp {
        format!(
            "Use the checked {target} MCP connection: discover with mcp_tools, invoke with mcp_call. It uses a separate browser profile. Verify the requested result."
        )
    } else {
        "Use desktop controls. Observe all windows, restore the target if minimized, and verify the requested result.".into()
    });
    route["route"] = json!(if mcp { "mcp" } else { "desktop" });
    route["app_id"] = json!(app_id);
    route["adapter"] = serde_json::to_value(adapter).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_require_evidence_and_never_promise_replay() {
        for name in ["editor", "spreadsheet", "messenger"] {
            let mut route = json!({"route":"mcp","server":name,"probe":{"ok":false}});
            attach(&mut route, name);
            assert_eq!(route["route"], "desktop");
            assert_eq!(route["adapter"]["target"], name);
            assert_eq!(route["adapter"]["automatic_replay"], false);
        }
        let mut route = json!({"route":"mcp","server":"chrome","probe":{"ok":true}});
        attach(&mut route, "installed-id");
        assert_eq!(route["adapter"]["connection_check"], "probe_passed");
        assert_eq!(
            Adapter::new(Transport::Api, "docs", true).connection_check,
            "operations_untested"
        );
    }
    #[test]
    fn missing_explicit_api_does_not_silently_switch_apps() {
        let root = tempfile::tempdir().unwrap();
        assert!(select(root.path(), "Editor", "editor-id", Some("missing")).is_err());
    }
}
