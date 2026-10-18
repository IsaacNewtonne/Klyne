//! Advisory host routing. Every dispatched action still passes its own policy.
use crate::{chat::Access, err};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io, path::Path, sync::atomic::AtomicBool};

pub const INSTRUCTIONS: &str = "resource_route {operation,path?,format?,existing_session?:false} discovers ordered host-supported routes for create_file/read_file/edit_file/convert_file/open_file/research/browse/api/command/recall/build_tool/runtime. It does not execute or grant access. Inspect prerequisites and destination content. Existing signed-in sessions require Desktop or an explicitly authorized known CDP endpoint; never substitute an isolated profile. tool_save supports formats:[lowercase extensions]; capability_list {format?,query?,offset?} filters before pagination. Declared format support and stored tested flags are advisory; tool_run requalifies executable, argv and input digests.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    tool: String,
    operation: String,
    path: Option<String>,
    format: Option<String>,
    #[serde(default)]
    existing_session: bool,
}

pub fn resolve(
    root: &Path,
    workspace: &Path,
    action: &Value,
    access: &Access,
    pending: bool,
    existing_session: bool,
    stop: &AtomicBool,
) -> io::Result<Value> {
    let request: Request = serde_json::from_value(action.clone()).map_err(err)?;
    if request.tool != "resource_route" {
        return Err(err("Expected resource_route"));
    }
    if pending {
        return Ok(
            json!({"ok":false,"blocked":"Reconcile unresolved effects before selecting another route","routes":[]}),
        );
    }
    let format = request
        .format
        .unwrap_or_else(|| crate::artifacts::extension(request.path.as_deref().unwrap_or("")));
    if !format.is_empty() {
        crate::capabilities::format_name(&json!(format))?;
    }
    let mut routes = Vec::new();
    let mut add = |tools: &[&str], prerequisite: &str| {
        routes.push(json!({"tools":tools,"prerequisite":prerequisite}))
    };
    match request.operation.as_str() {
        "create_file" | "read_file" | "edit_file" | "convert_file" | "open_file" => {
            match request.operation.as_str() {
                "create_file" if format == "docx" => add(
                    &["document_create", "document_read"],
                    "Native DOCX text creation; explicit destination; no overwrite",
                ),
                "create_file"
                    if matches!(
                        format.as_str(),
                        "txt"
                            | "md"
                            | "markdown"
                            | "csv"
                            | "tsv"
                            | "json"
                            | "xml"
                            | "svg"
                            | "html"
                            | "css"
                            | "js"
                            | "py"
                            | "rs"
                            | "yaml"
                            | "yml"
                    ) =>
                {
                    add(
                        &["write_file", "read_file"],
                        "Use only for actual UTF-8 text formats",
                    )
                }
                "read_file" => add(
                    &["artifact_inspect", "artifact_read", "document_read"],
                    "Byte pages are transport; document extraction supports text, PDF and DOCX",
                ),
                "edit_file" | "convert_file" => add(
                    &["artifact_inspect", "capability_list"],
                    "Inspect actual input; preserve source; select a real encoder and explicit output path",
                ),
                _ => {}
            }
            if access.desktop {
                add(
                    &["desktop_apps", "desktop_observe", "desktop_launch"],
                    "Windows interactive desktop and an installed format-capable app; verify exact document state",
                );
            }
            if access.apps {
                add(
                    &["app_list", "app_operations", "app_invoke"],
                    "Configured integration with supported operations and normal authentication grants",
                );
            }
            if access.terminal {
                add(
                    &["capability_list", "tool_test", "tool_run", "run_shell"],
                    "Installed encoder or bounded tool development; untested tools require tool_test; tested tools are requalified at execution; normal argv approval; inspect output independently",
                );
            }
            if request.operation != "open_file" {
                add(
                    &["artifact_write", "artifact_verify"],
                    "Actual encoded bytes required; max 16 MiB; digest proves integrity only",
                );
            }
        }
        "research" if access.web => add(
            &["fetch_url"],
            "Public HTTPS source; preserve citations in output",
        ),
        "browse" if request.existing_session || existing_session => {
            if access.desktop {
                add(
                    &["desktop_observe", "desktop_apps"],
                    "Observe the user's requested signed-in session",
                );
            }
            if access.web && access.terminal {
                add(
                    &["browser_attach"],
                    "Explicitly authorized known CDP endpoint only; no cookie/profile copying",
                );
            }
        }
        "browse" if access.web => add(
            &["browser_open", "browser_read"],
            "Installed Chrome/Edge; isolated session; verify destination",
        ),
        "api" if access.apps => add(
            &["app_list", "app_operations", "app_invoke"],
            "Configured supported operations, authentication and origin-bound grants; verify destination",
        ),
        "command" if access.terminal => add(
            &["run_shell"],
            "Structured argv under configured command policy; independently inspect outputs",
        ),
        "recall" => add(
            &["capability_list", "memory_read", "skill_read"],
            "Advisory provenance only; revalidate machine-specific facts",
        ),
        "build_tool" if access.terminal => add(
            &["tool_save", "tool_test", "tool_run"],
            "Bounded development, normal command approval and qualification against exact executable/argv/inputs",
        ),
        "runtime" => {
            add(&["runtime_status"], "Inspect actual activation state");
            if access.terminal {
                add(
                    &["run_shell", "runtime_attest", "runtime_stage"],
                    "Build/test/attest/stage; preserve pending effects and durable task progress",
                );
            }
        }
        "research" | "browse" | "api" | "command" | "build_tool" => {}
        _ => return Err(err("Unknown resource operation")),
    }
    let candidates = if format.is_empty() {
        Value::Null
    } else {
        crate::capabilities::execute(
            root,
            workspace,
            &json!({"tool":"capability_list","format":format}),
            false,
            true,
            stop,
        )?
    };
    Ok(
        json!({"ok":true,"operation":request.operation,"format":format,"available":!routes.is_empty(),"routes":routes,"format_tools":candidates,"execution_access_required":{"terminal":access.terminal,"apps":access.apps,"web":access.web,"desktop":access.desktop},"verification":"Inspect exact destination bytes and meaningful content using the actual adapter. Structural checks and digest receipts do not certify rendering or task completion.","grants_access":false,"unavailable_reason":"Missing access, installed application, encoder or authentication must be reported, never invented"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_operations_respect_access_session_and_pending_effects() {
        let root = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let none = Access {
            web: false,
            terminal: false,
            apps: false,
            desktop: false,
        };
        for operation in [
            "create_file",
            "read_file",
            "edit_file",
            "convert_file",
            "open_file",
            "research",
            "browse",
            "api",
            "command",
            "recall",
            "build_tool",
            "runtime",
        ] {
            let action = json!({"tool":"resource_route","operation":operation});
            let result = resolve(
                root.path(),
                root.path(),
                &action,
                &none,
                false,
                false,
                &stop,
            )
            .unwrap();
            if [
                "open_file",
                "research",
                "browse",
                "api",
                "command",
                "build_tool",
            ]
            .contains(&operation)
            {
                assert_eq!(result["available"], false, "{operation}");
            }
            assert_eq!(
                resolve(root.path(), root.path(), &action, &none, true, false, &stop).unwrap()["ok"],
                false
            );
        }
        let all = Access {
            web: true,
            terminal: true,
            apps: true,
            desktop: true,
        };
        let result = resolve(
            root.path(),
            root.path(),
            &json!({"tool":"resource_route","operation":"browse"}),
            &all,
            false,
            true,
            &stop,
        )
        .unwrap();
        assert!(result.to_string().contains("browser_attach"));
        assert!(!result.to_string().contains("browser_open"));
        assert!(!result.to_string().contains("grants_access\":true"));
    }
    #[test]
    fn format_discovery_filters_before_pagination_and_requires_qualification() {
        let root = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        for index in 0..25 {
            crate::capabilities::execute(root.path(), root.path(), &json!({"tool":"tool_save","name":format!("adapter{index:02}"),"description":"fixture","program":"encoder","args":[],"formats":[if index==24 {"xlsx"} else {"png"}]}), true, false, &stop).unwrap();
        }
        let access = Access {
            web: false,
            terminal: true,
            apps: false,
            desktop: false,
        };
        let result = resolve(
            root.path(),
            root.path(),
            &json!({"tool":"resource_route","operation":"create_file","path":"out.xlsx"}),
            &access,
            false,
            false,
            &stop,
        )
        .unwrap();
        assert_eq!(result["format_tools"]["total"], 1);
        assert_eq!(result["format_tools"]["items"][0]["name"], "adapter24");
        assert_eq!(result["format_tools"]["items"][0]["tested"], false);
        assert!(result.to_string().contains("tool_test"));
        assert!(crate::capabilities::execute(root.path(), root.path(), &json!({"tool":"tool_save","name":"bad","description":"bad","program":"encoder","args":[],"formats":["../xlsx"]}), true, false, &stop).is_err());
    }
}
