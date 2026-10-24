//! Native tool calling for local models (added 2026-10-24).
//!
//! Small local models (for example qwen3.5:9b through Ollama) follow the
//! provider's native `tools` / `tool_calls` protocol far more reliably than a
//! long prose description of a JSON decision format. This module converts the
//! host's tool inventory into tool schemas, and converts the model's reply
//! (native tool calls, a strict `<tool_call>` text fallback, or a JSON
//! decision) back into Klyne's ordinary decision JSON. Nothing here executes
//! anything: every decision still goes through the same validation, access
//! policy, approval broker and receipts in `chat.rs`.
use serde_json::{Map, Value, json};

/// Tools always exposed in compact (small-model) mode, before `enable_tools`.
const CORE_FILES: &[&str] = &[
    "list_dir",
    "read_file",
    "write_file",
    "search_file",
    "stat_path",
    "make_dir",
    "copy_file",
    "move_file",
    "delete_path",
];
const CORE_WEB: &[&str] = &["fetch_url", "browser_open", "browser_read", "browser_close"];
/// Cheap read-only/meta tools kept visible in compact mode (2026-10-24):
/// without them small models ran e.g. "runtime_status" as a shell program.
const CORE_META: &[&str] = &[
    "runtime_status",
    "capability_list",
    "skill_read",
    "memory_read",
    "memory_save",
    "memory_forget",
    "tool_save",
    "tool_test",
    "tool_run",
    "skill_forget",
    "tool_forget",
    "app_list",
];
const CORE_TERMINAL: &[&str] = &["run_shell", "ask_model"];
const CORE_DESKTOP: &[&str] = &[
    "desktop_observe",
    "desktop_apps",
    "desktop_launch",
    "desktop_focus",
    "desktop_click",
    "desktop_type",
    "desktop_key",
];
/// Host-side meta tools that map to non-action decisions.
pub const FINISH: &str = "finish";
pub const ASK_USER: &str = "ask_user";
pub const REPORT_BLOCKER: &str = "report_blocker";
pub const ENABLE_TOOLS: &str = "enable_tools";
pub const REQUEST_REPAIR: &str = "request_repair";

fn s(description: &str) -> Value {
    json!({"type":"string","description":description})
}
fn i(description: &str) -> Value {
    json!({"type":"integer","description":description})
}
fn object(description: &str, properties: Value, required: &[&str]) -> (String, Value) {
    (
        description.to_owned(),
        json!({"type":"object","properties":properties,"required":required}),
    )
}

/// Description and JSON schema for a built-in tool. Unknown tools get a
/// permissive schema; host dispatch still validates their arguments.
fn schema(tool: &str) -> (String, Value) {
    let path = || {
        s(
            "File or folder path. Relative paths are inside the workspace; an absolute path inside the workspace also works.",
        )
    };
    let window = || s("Window handle string from the current desktop observation");
    match tool {
        "list_dir" => object(
            "List the entries of a folder. Use path \".\" for the workspace root.",
            json!({"path":path()}),
            &["path"],
        ),
        "read_file" => object("Read a UTF-8 text file.", json!({"path":path()}), &["path"]),
        "read_range" => object(
            "Read part of a file by byte offset.",
            json!({"path":path(),"offset":i("Byte offset"),"length":i("Bytes to read")}),
            &["path", "offset", "length"],
        ),
        "hash_file" => object("SHA-256 of a file.", json!({"path":path()}), &["path"]),
        "search_file" => object(
            "Find text inside a file.",
            json!({"path":path(),"needle":s("Text to find"),"max_matches":i("Maximum matches, e.g. 20")}),
            &["path", "needle", "max_matches"],
        ),
        "stat_path" => object(
            "Report whether a path exists, its type and size.",
            json!({"path":path()}),
            &["path"],
        ),
        "write_file" => object(
            "Create or replace a text file with the exact contents. The host reads it back to verify.",
            json!({"path":path(),"contents":s("Complete file contents")}),
            &["path", "contents"],
        ),
        "patch_file" => object(
            "Replace an exact byte range in a file.",
            json!({"path":path(),"offset":i("Byte offset"),"expected":s("Exact current text"),"replacement":s("New text"),"expected_sha256":s("SHA-256 of the current file")}),
            &[
                "path",
                "offset",
                "expected",
                "replacement",
                "expected_sha256",
            ],
        ),
        "make_dir" => object(
            "Create a folder (use this, not shell mkdir/New-Item).",
            json!({"path":path()}),
            &["path"],
        ),
        "copy_file" => object(
            "Copy a file.",
            json!({"from":path(),"to":path()}),
            &["from", "to"],
        ),
        "move_file" => object(
            "Move or rename a file. Overwriting an existing destination needs user approval.",
            json!({"from":path(),"to":path()}),
            &["from", "to"],
        ),
        "delete_path" => object(
            "Delete a file or folder. The host asks the user to approve before deleting.",
            json!({"path":path()}),
            &["path"],
        ),
        "fetch_url" => object(
            "Download an HTTPS page as text.",
            json!({"url":s("https:// URL")}),
            &["url"],
        ),
        "run_shell" => object(
            "Run a program with arguments in the workspace (Windows). For PowerShell use program \"powershell\" with args [\"-NoProfile\",\"-Command\",\"...\"]. Destructive commands (delete, install, registry, shutdown, force push, setx) pause for user approval. Not for file work: use make_dir, write_file, read_file, list_dir instead of mkdir, New-Item, Set-Content or cat.",
            json!({"program":s("Executable name or path"),"args":{"type":"array","items":{"type":"string"},"description":"Arguments"},"timeout_seconds":i("Optional timeout; 0 waits until completion")}),
            &["program", "args"],
        ),
        "browser_open" => object(
            "Open a URL in Klyne's isolated browser.",
            json!({"url":s("URL")}),
            &["url"],
        ),
        "browser_read" => object(
            "Read the current browser page title and text.",
            json!({}),
            &[],
        ),
        "browser_click" => object(
            "Click a CSS selector in the browser page.",
            json!({"selector":s("CSS selector")}),
            &["selector"],
        ),
        "browser_fill" => object(
            "Fill a browser input.",
            json!({"selector":s("CSS selector"),"text":s("Text")}),
            &["selector", "text"],
        ),
        "browser_screenshot" => object("Save a screenshot of the browser page.", json!({}), &[]),
        "browser_close" => object("Close the browser session.", json!({}), &[]),
        "browser_attach" => object(
            "Attach to a loopback Chrome DevTools endpoint (for example the user's real Chrome profile when that setting is on).",
            json!({"address":s("e.g. 127.0.0.1:9222"),"tab":s("Optional tab id")}),
            &["address"],
        ),
        "browser_eval" => object(
            "Evaluate JavaScript in the attached page.",
            json!({"expression":s("JavaScript expression")}),
            &["expression"],
        ),
        "desktop_observe" => object(
            "Take a screenshot and list windows and accessible controls.",
            json!({}),
            &[],
        ),
        "desktop_apps" => object("List installed Start-menu app IDs.", json!({}), &[]),
        "desktop_launch" => object(
            "Launch an app by ID or name (notepad, calc, or a desktop_apps ID). The result's launched.window is the new, focused window: pass it as window to desktop_type/key/click right away; do not launch again.",
            json!({"app_id":s("App ID from desktop_apps")}),
            &["app_id"],
        ),
        "desktop_focus" => object(
            "Bring an observed window to the foreground.",
            json!({"window":window()}),
            &["window"],
        ),
        "desktop_click" => object(
            "Click at screenshot pixel coordinates. Clicking send/buy/post/submit controls needs user approval.",
            json!({"window":window(),"x":i("X pixel"),"y":i("Y pixel"),"button":s("left, right or double")}),
            &["window", "x", "y", "button"],
        ),
        "desktop_type" => object(
            "Type text into the focused control.",
            json!({"window":window(),"text":s("Text to type")}),
            &["window", "text"],
        ),
        "desktop_key" => object(
            "Press a key or combination such as ENTER, TAB, CTRL+S.",
            json!({"window":window(),"key":s("Key name")}),
            &["window", "key"],
        ),
        "desktop_scroll" => object(
            "Scroll; negative ticks scroll down.",
            json!({"window":window(),"ticks":i("-10 to 10")}),
            &["window", "ticks"],
        ),
        "desktop_invoke" => object(
            "Invoke an observed accessible control.",
            json!({"window":window(),"element":s("Element id from the observation")}),
            &["window", "element"],
        ),
        "desktop_fill" => object(
            "Replace an observed control's value.",
            json!({"window":window(),"element":s("Element id"),"text":s("Text")}),
            &["window", "element", "text"],
        ),
        "desktop_drag" => object(
            "Drag between screenshot pixels.",
            json!({"window":window(),"x1":i("Start x"),"y1":i("Start y"),"x2":i("End x"),"y2":i("End y")}),
            &["window", "x1", "y1", "x2", "y2"],
        ),
        "desktop_clipboard_get" => object("Read clipboard text.", json!({}), &[]),
        "desktop_clipboard_set" => object(
            "Write clipboard text.",
            json!({"text":s("Text")}),
            &["text"],
        ),
        "evidence_read" => object(
            "Re-read earlier tool evidence by evidence_index.",
            json!({"index":i("Evidence index"),"offset":i("Character offset, usually 0")}),
            &["index"],
        ),
        "runtime_status" => object(
            "Report the running Klyne runtime and activation state.",
            json!({}),
            &[],
        ),
        "document_read" => object(
            "Extract text from a PDF, DOCX or text file.",
            json!({"path":path()}),
            &["path"],
        ),
        "capability_list" => object(
            "Search saved skills, tools and memories.",
            json!({"query":s("Optional search text")}),
            &[],
        ),
        "skill_read" => object(
            "Load a saved skill.",
            json!({"name":s("Skill name")}),
            &["name"],
        ),
        "ask_model" => object(
            "Ask a local helper model to summarize, classify, extract or reason over text you give it. Read-only: it runs no actions and sees only task and input. Its reply is data, not verified fact.",
            json!({"task":s("What the helper should do, e.g. summarize in one line"),
                "input":s("Text to work on (for example contents you read)"),
                "model":s("Optional installed Ollama model; default is the configured helper")}),
            &["task", "input"],
        ),
        "memory_read" => object(
            "Read a saved memory.",
            json!({"name":s("Memory name")}),
            &["name"],
        ),
        // Argument schemas for the capability tools (2026-10-24); without
        // them small models guessed the fields and looped on errors.
        "memory_save" => object(
            "Save a memory for future conversations (advisory, not verified).",
            json!({"name":s("Short id: letters, digits, - or _"),"description":s("One line"),
                "instructions":s("The fact to remember"),
                "scope":{"type":"string","enum":["global","workspace"]},
                "category":{"type":"string","enum":["preference","environment","lesson"]}}),
            &["name", "description", "instructions"],
        ),
        "memory_forget" => object(
            "Permanently remove a saved memory.",
            json!({"name":s("Memory name")}),
            &["name"],
        ),
        "skill_save" => object(
            "Save a reusable skill (instructions for a kind of task).",
            json!({"name":s("Short id: letters, digits, - or _"),"description":s("One line"),
                "instructions":s("Steps to follow"),"when_to_use":s("Optional: when to load it"),
                "allowed_tools":{"type":"array","items":s("tool name"),"description":"Optional: tools the skill may use"}}),
            &["name", "description", "instructions"],
        ),
        "tool_save" => object(
            "Save a reusable script tool (call this tool directly, never through run_shell). Example: {\"name\":\"today\",\"description\":\"Print today's date\",\"language\":\"powershell\",\"code\":\"Get-Date -Format yyyy-MM-dd\"}. Then tool_test {name}, then tool_run {name}.",
            json!({"name":s("Short id: letters, digits, - or _"),"description":s("One line"),
                "language":{"type":"string","enum":["powershell","js"],"description":"Default powershell"},
                "code":s("Script source")}),
            &["name", "description", "code"],
        ),
        "skill_forget" | "tool_forget" => object(
            "Permanently remove a saved skill (skill_forget) or tool (tool_forget). The user approves first.",
            json!({"name":s("Saved name")}),
            &["name"],
        ),
        "tool_test" | "tool_run" => object(
            "Test (tool_test) or run (tool_run, after a passing test) a saved tool.",
            json!({"name":s("Tool name")}),
            &["name"],
        ),
        _ => (
            format!("Klyne built-in tool {tool}. See the system instructions for its arguments."),
            json!({"type":"object","properties":{},"additionalProperties":true}),
        ),
    }
}

fn function(name: &str, description: &str, parameters: Value) -> Value {
    json!({"type":"function","function":{"name":name,"description":description,"parameters":parameters}})
}

/// Tool names grouped as the host advertises them in `available_tools`.
fn groups(available: &Value) -> Vec<(String, Vec<String>)> {
    available
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(group, tools)| {
            tools.as_array().map(|items| {
                (
                    group.clone(),
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect(),
                )
            })
        })
        .collect()
}

/// Build the native `tools` array. In compact mode only core tools plus
/// `enable_tools` are exposed until the model enables more groups.
pub fn tools(
    available: &Value,
    enabled_groups: &[String],
    compact: bool,
    read_only: bool,
) -> Vec<Value> {
    let all = groups(available);
    let mut names: Vec<String> = Vec::new();
    let mut hidden_groups: Vec<String> = Vec::new();
    for (group, items) in &all {
        let core: Vec<&String> = items
            .iter()
            .filter(|t| {
                !compact
                    || enabled_groups.contains(group)
                    || CORE_FILES.contains(&t.as_str())
                    || CORE_WEB.contains(&t.as_str())
                    || CORE_TERMINAL.contains(&t.as_str())
                    || CORE_DESKTOP.contains(&t.as_str())
                    || CORE_META.contains(&t.as_str())
            })
            .collect();
        if core.len() < items.len() {
            hidden_groups.push(group.clone());
        }
        for tool in core {
            if !names.contains(tool) {
                names.push(tool.clone());
            }
        }
    }
    if read_only {
        names.retain(|t| {
            matches!(
                t.as_str(),
                "list_dir"
                    | "read_file"
                    | "read_range"
                    | "hash_file"
                    | "search_file"
                    | "stat_path"
                    | "browser_read"
                    | "browser_screenshot"
                    | "desktop_observe"
                    | "desktop_apps"
                    | "evidence_read"
                    | "document_read"
                    | "runtime_status"
                    | "fetch_url"
            )
        });
    }
    let mut out: Vec<Value> = names
        .iter()
        .map(|name| {
            let (description, parameters) = schema(name);
            function(name, &description, parameters)
        })
        .collect();
    if compact && !read_only && !hidden_groups.is_empty() {
        out.push(function(
            ENABLE_TOOLS,
            &format!("Expose more tools for the rest of this task. Groups: {}.", hidden_groups.join(", ")),
            json!({"type":"object","properties":{"group":{"type":"string","enum":hidden_groups}},"required":["group"]}),
        ));
    }
    if read_only {
        out.push(function(
            FINISH,
            "The original request is fulfilled. summary is the final answer shown to the user with the actual result (names, paths, contents). For app/desktop actions set outcome to achieved and cite observed_results.",
            json!({"type":"object","properties":{"summary":s("Final answer for the user"),
                "outcome":{"type":"string","enum":["achieved"]},
                "observed_results":{"type":"array","items":{"type":"object","properties":{
                    "effect":s("send|open|click|delete|install|upload"),"target":s("app and destination"),
                    "observation":s("exact visible result"),"evidence_index":i("review_observations index")}}},
                "findings":{"type":"array","description":"One finding per requirement or checklist item","items":{"type":"object","properties":{
                    "item":s("requirement checked"),"verdict":{"type":"string","enum":["pass","fail"]},
                    "evidence":s("what you observed"),"severity":{"type":"string","enum":["low","medium","high"]}},
                    "required":["item","verdict","evidence","severity"]}},
                "examined":{"type":"array","items":s("thing you inspected")},
                "not_examined":{"type":"array","items":s("thing you did not check")}},
                "required":["summary","findings","examined","not_examined"]}),
        ));
        out.push(function(
            REQUEST_REPAIR,
            "The original request is NOT yet fulfilled; send the worker back with one concrete instruction.",
            json!({"type":"object","properties":{"summary":s("What is missing"),"instruction":s("Concrete next task for the worker")},"required":["summary","instruction"]}),
        ));
    } else {
        out.push(function(
            FINISH,
            "Finish the assigned step. summary is the actual answer shown to the user (include listed file names, paths, results). Only call after the work has really been done and observed.",
            json!({"type":"object","properties":{"summary":s("Final answer for the user")},"required":["summary"]}),
        ));
    }
    out.push(function(
        ASK_USER,
        "Ask the user for essential information you cannot discover with tools.",
        json!({"type":"object","properties":{"question":s("Specific question")},"required":["question"]}),
    ));
    out.push(function(
        REPORT_BLOCKER,
        "Report that the step is blocked (access off, tool failed repeatedly).",
        json!({"type":"object","properties":{"reason":s("What is blocked")},"required":["reason"]}),
    ));
    out
}

/// Remove `<think>...</think>` (and an unterminated leading think block).
/// Remove the reviewer findings fields from the finish tool (used when the
/// findings report is off for small models).
pub fn without_findings(tools: &mut [Value]) {
    for tool in tools.iter_mut() {
        if tool["function"]["name"] == FINISH {
            let parameters = &mut tool["function"]["parameters"];
            if let Some(properties) = parameters["properties"].as_object_mut() {
                for key in ["findings", "examined", "not_examined"] {
                    properties.remove(key);
                }
            }
            parameters["required"] = json!(["summary"]);
        }
    }
}

pub fn strip_think(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let lower = rest.to_ascii_lowercase();
        match lower.find("<think>") {
            Some(start) => {
                out.push_str(&rest[..start]);
                match lower[start..].find("</think>") {
                    Some(end) => rest = &rest[start + end + "</think>".len()..],
                    None => {
                        rest = "";
                    }
                }
            }
            None => {
                // A stray closing tag means the opening was in the template.
                if let Some(end) = lower.find("</think>")
                    && out.is_empty()
                {
                    rest = &rest[end + "</think>".len()..];
                    continue;
                }
                out.push_str(rest);
                break;
            }
        }
    }
    out.trim().to_owned()
}

fn arguments(raw: &Value) -> Option<Map<String, Value>> {
    match raw {
        Value::Object(map) => Some(map.clone()),
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => Some(map),
            _ => None,
        },
        Value::Null => Some(Map::new()),
        _ => None,
    }
}

/// Convert one native tool call into a Klyne decision.
pub fn call_to_decision(name: &str, raw_arguments: &Value) -> Option<Value> {
    let args = arguments(raw_arguments)?;
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    match name {
        FINISH => {
            let mut decision = json!({"decision":"complete","summary":text("summary")});
            // Reviewer completion details for app/desktop outcomes.
            if let Some(outcome) = args.get("outcome").filter(|v| v.is_string()) {
                decision["outcome"] = outcome.clone();
            }
            if let Some(observed) = args.get("observed_results").filter(|v| v.is_array()) {
                decision["observed_results"] = observed.clone();
            }
            // Reviewer findings schema (2026-10-24); validated by chat.rs.
            for key in ["findings", "examined", "not_examined"] {
                if let Some(value) = args.get(key) {
                    decision[key] = value.clone();
                }
            }
            decision["via_tool"] = json!(true);
            Some(decision)
        }
        REQUEST_REPAIR => Some(json!({"decision":"repair","summary":text("summary"),
            "tasks":[{"agent":"Assistant","instruction":text("instruction")}]})),
        ASK_USER => Some(json!({"decision":"needs_input","question":text("question")})),
        REPORT_BLOCKER => Some(json!({"decision":"fail","reason":text("reason")})),
        "" => None,
        tool => {
            let mut action = Map::new();
            action.insert("tool".into(), json!(tool));
            for (key, value) in args {
                if key != "tool" {
                    action.insert(key, value);
                }
            }
            // Models sometimes send run_shell args as one string.
            if tool == "run_shell"
                && let Some(Value::String(line)) = action.get("args").cloned()
            {
                action.insert(
                    "args".into(),
                    json!(line.split_whitespace().collect::<Vec<_>>()),
                );
            }
            if tool == "run_shell" && !action.contains_key("args") {
                action.insert("args".into(), json!([]));
            }
            // Small models sometimes "run" an argument-free Klyne tool as a
            // shell program; call the tool itself (2026-10-24).
            if tool == "run_shell"
                && action
                    .get("args")
                    .is_some_and(|a| a.as_array().is_some_and(Vec::is_empty))
                && let Some(program) = action.get("program").and_then(Value::as_str)
                && matches!(program, "runtime_status" | "app_list" | "capability_list")
            {
                return Some(json!({"decision":"act","action":{"tool":program}}));
            }
            // ...and capability tools run as `tool_save name=x --program y`:
            // turn `key=value` / `--key value` arguments into fields.
            if tool == "run_shell"
                && let Some(program) = action.get("program").and_then(Value::as_str)
                && matches!(
                    program,
                    "skill_save"
                        | "tool_save"
                        | "tool_test"
                        | "tool_run"
                        | "memory_save"
                        | "memory_forget"
                        | "skill_forget"
                        | "tool_forget"
                )
                && let Some(fields) = action
                    .get("args")
                    .and_then(Value::as_array)
                    .and_then(|a| keyed_fields(a))
            {
                let mut call = fields;
                call.insert("tool".into(), json!(program));
                return Some(json!({"decision":"act","action":Value::Object(call)}));
            }
            // `tool_test today`, `tool_forget today`: one bare name.
            if tool == "run_shell"
                && let Some(program) = action.get("program").and_then(Value::as_str)
                && matches!(
                    program,
                    "tool_test" | "tool_run" | "skill_forget" | "tool_forget"
                )
                && let Some([name]) = action
                    .get("args")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                && let Some(name) = name.as_str().filter(|n| !n.contains('='))
            {
                return Some(json!({"decision":"act","action":{"tool":program,"name":name}}));
            }
            // Same for memory: `memory_save <name> <fact...>` and
            // `memory_forget <name>` as shell programs.
            if tool == "run_shell"
                && let Some(program) = action.get("program").and_then(Value::as_str)
                && matches!(program, "memory_save" | "memory_forget")
                && let Some(args) = action.get("args").and_then(Value::as_array)
                && let Some(name) = args.first().and_then(Value::as_str)
            {
                let rest: Vec<&str> = args[1..].iter().filter_map(Value::as_str).collect();
                if program == "memory_forget" && rest.is_empty() {
                    return Some(json!({"decision":"act","action":{"tool":program,"name":name}}));
                }
                if program == "memory_save" && !rest.is_empty() {
                    let fact = rest.join(" ");
                    return Some(
                        json!({"decision":"act","action":{"tool":program,"name":name,
                        "description":name.replace(['_', '-'], " "),"instructions":fact}}),
                    );
                }
            }
            Some(json!({"decision":"act","action":Value::Object(action)}))
        }
    }
}

/// `["name=x","--program","y"]` -> `{name:"x",program:"y"}`; `None` unless
/// every argument is part of a key/value pair.
fn keyed_fields(args: &[Value]) -> Option<serde_json::Map<String, Value>> {
    let mut out = serde_json::Map::new();
    let mut items = args.iter().map(Value::as_str);
    while let Some(item) = items.next() {
        let item = item?;
        if let Some(key) = item.strip_prefix("--") {
            let (key, value) = match key.split_once('=') {
                Some((k, v)) => (k, v.to_string()),
                None => (key, items.next()??.to_string()),
            };
            out.insert(key.replace('-', "_"), json!(value));
        } else {
            let (key, value) = item.split_once('=')?;
            out.insert(key.to_string(), json!(value));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Strict text fallback: the whole reply (after removing reasoning) must be
/// `<tool_call>{"name":...,"arguments":{...}}</tool_call>`. Partial matches
/// inside prose are ignored so narrated examples never execute.
pub fn text_tool_call(text: &str) -> Option<Value> {
    let clean = strip_think(text);
    let inner = clean
        .strip_prefix("<tool_call>")?
        .strip_suffix("</tool_call>")?
        .trim();
    let value: Value = serde_json::from_str(inner).ok()?;
    let name = value["name"].as_str()?;
    call_to_decision(name, value.get("arguments").unwrap_or(&Value::Null))
}

const REFUSALS: &[&str] = &[
    "i can't access",
    "i cannot access",
    "i can't directly",
    "i cannot directly",
    "i don't have access",
    "i do not have access",
    "i don't have the ability",
    "i do not have the ability",
    "i'm unable to",
    "i am unable to",
    "i can't browse",
    "i cannot browse",
    "i can't open",
    "i cannot open",
    "i can't create files",
    "i cannot create files",
    "as an ai",
    "as a language model",
    "as a text-based",
    "no access to a file system",
    "no access to your file",
    "don't have file system access",
    "do not have file system access",
    "i can't interact",
    "i cannot interact",
    "i can't execute",
    "i cannot execute",
    "i can't run",
    "i cannot run",
];
const PROMISES: &[&str] = &[
    "i'll ",
    "i will ",
    "let me ",
    "i'm going to",
    "i am going to",
    "give me a moment",
    "one moment",
    "working on it",
    "i'll get started",
    "getting started",
    "starting now",
    "on it!",
    "on it.",
];

/// A reply that claims Klyne lacks abilities it has.
pub fn refusal(text: &str) -> bool {
    let lower = text.to_lowercase().replace('\u{2019}', "'");
    REFUSALS.iter().any(|p| lower.contains(p))
}

/// A reply that promises future work instead of doing or answering it.
/// Narrower refusal for worker completions: claims about lacking access or
/// abilities (not ordinary "I can't open X because it is locked" results).
pub fn capability_refusal(text: &str) -> bool {
    let lower = strip_think(text).to_lowercase();
    refusal(text)
        && [
            "access",
            "ability",
            "as an ai",
            "language model",
            "text-based",
            "file system",
            "browse",
            "interact",
        ]
        .iter()
        .any(|w| lower.contains(w))
}
pub fn empty_promise(text: &str) -> bool {
    let lower = text.to_lowercase().replace('\u{2019}', "'");
    let lower = lower.trim();
    // Only short replies: a long answer that merely contains "I'll" is fine.
    lower.chars().count() <= 240
        && PROMISES.iter().any(|p| {
            lower.starts_with(p)
                || lower.contains(&format!(". {p}"))
                || lower.contains(&format!("! {p}"))
                || lower.contains(&format!(", {p}"))
        })
}

/// Normalize a model reply in native-tool mode into decision JSON text.
/// Returns the text unchanged when it is already a decision (or not
/// recoverable) so the ordinary parser and validator report the problem.
pub fn normalize_reply(content: &str, tool_calls: &[Value]) -> String {
    for call in tool_calls {
        let function = &call["function"];
        if let Some(name) = function["name"].as_str()
            && let Some(decision) =
                call_to_decision(name, function.get("arguments").unwrap_or(&Value::Null))
        {
            return decision.to_string();
        }
    }
    let clean = strip_think(content);
    if let Some(decision) = text_tool_call(&clean) {
        return decision.to_string();
    }
    let trimmed = clean.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    if unfenced.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<Value>(unfenced)
            && value.get("decision").is_none()
            && let Some(name) = value["name"].as_str()
            && let Some(decision) =
                call_to_decision(name, value.get("arguments").unwrap_or(&Value::Null))
        {
            return decision.to_string();
        }
        return unfenced.to_owned();
    }
    if trimmed.is_empty() || refusal(trimmed) || empty_promise(trimmed) {
        // Leave it invalid: the caller retries with correction feedback.
        return trimmed.to_owned();
    }
    json!({"decision":"complete","summary":trimmed}).to_string()
}

/// Parse a size like `9b`, `14b`, `0.5b`, `3.8b-instruct` from a model tag.
pub fn parameter_billions(model: &str) -> Option<f64> {
    let tag = model.rsplit_once(':').map(|(_, t)| t).unwrap_or(model);
    for part in tag.split(['-', '_']) {
        let lower = part.to_ascii_lowercase();
        if let Some(number) = lower.strip_suffix('b')
            && let Ok(value) = number.parse::<f64>()
        {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klyne_tools_run_as_shell_programs_are_mapped_to_the_tool() {
        assert_eq!(
            call_to_decision("run_shell", &json!({"program":"runtime_status","args":[]})),
            Some(json!({"decision":"act","action":{"tool":"runtime_status"}}))
        );
        assert_eq!(
            call_to_decision("run_shell", &json!({"program":"cmd","args":[]})).unwrap()["action"]["tool"],
            "run_shell"
        );
        let saved = call_to_decision(
            "run_shell",
            &json!({"program":"memory_save","args":["favorite_color_test","teal"]}),
        )
        .unwrap();
        assert_eq!(saved["action"]["tool"], "memory_save");
        assert_eq!(saved["action"]["name"], "favorite_color_test");
        assert_eq!(saved["action"]["instructions"], "teal");
        let tool = call_to_decision(
            "run_shell",
            &json!({"program":"tool_save","args":["name=t","--program","hostname","args=[]"]}),
        )
        .unwrap();
        assert_eq!(
            tool["action"],
            json!({"tool":"tool_save","name":"t","program":"hostname","args":"[]"})
        );
        assert_eq!(
            call_to_decision(
                "run_shell",
                &json!({"program":"memory_forget","args":["x"]})
            ),
            Some(json!({"decision":"act","action":{"tool":"memory_forget","name":"x"}}))
        );
    }

    #[test]
    fn findings_fields_can_be_removed_from_the_reviewer_finish() {
        let mut tools = tools(&json!({"files":["read_file"]}), &[], true, true);
        let finish = |t: &[Value]| {
            t.iter()
                .find(|t| t["function"]["name"] == FINISH)
                .unwrap()
                .clone()
        };
        assert!(finish(&tools)["function"]["parameters"]["properties"]["findings"].is_object());
        without_findings(&mut tools);
        let f = finish(&tools);
        assert!(f["function"]["parameters"]["properties"]["findings"].is_null());
        assert_eq!(f["function"]["parameters"]["required"], json!(["summary"]));
    }

    #[test]
    fn native_tool_calls_become_act_decisions() {
        let calls = vec![json!({"function":{"name":"list_dir","arguments":{"path":"C:\\temp"}}})];
        let decision: Value = serde_json::from_str(&normalize_reply("", &calls)).unwrap();
        assert_eq!(decision["decision"], "act");
        assert_eq!(decision["action"]["tool"], "list_dir");
        assert_eq!(decision["action"]["path"], "C:\\temp");
        // String-encoded arguments (OpenAI style) are accepted too.
        let calls = vec![
            json!({"function":{"name":"write_file","arguments":"{\"path\":\"a.txt\",\"contents\":\"hello\"}"}}),
        ];
        let decision: Value = serde_json::from_str(&normalize_reply("", &calls)).unwrap();
        assert_eq!(decision["action"]["contents"], "hello");
        let finish =
            vec![json!({"function":{"name":"finish","arguments":{"summary":"Done: a.txt"}}})];
        let decision: Value = serde_json::from_str(&normalize_reply("", &finish)).unwrap();
        assert_eq!(
            decision,
            json!({"decision":"complete","summary":"Done: a.txt","via_tool":true})
        );
    }

    #[test]
    fn strict_text_fallback_requires_a_full_match() {
        let reply = "<think>plan</think>\n<tool_call>{\"name\":\"read_file\",\"arguments\":{\"path\":\"a.txt\"}}</tool_call>";
        let decision: Value = serde_json::from_str(&normalize_reply(reply, &[])).unwrap();
        assert_eq!(decision["action"]["tool"], "read_file");
        // Embedded in prose: never executed; becomes a plain answer instead.
        let prose = "Example: <tool_call>{\"name\":\"delete_path\",\"arguments\":{\"path\":\"x\"}}</tool_call> is how";
        assert!(text_tool_call(prose).is_none());
        let decision: Value = serde_json::from_str(&normalize_reply(prose, &[])).unwrap();
        assert_eq!(decision["decision"], "complete");
    }

    #[test]
    fn think_blocks_are_removed() {
        assert_eq!(strip_think("<think>a\nb</think>{\"x\":1}"), "{\"x\":1}");
        assert_eq!(strip_think("reasoning</think> answer"), "answer");
        assert_eq!(strip_think("<THINK>x</THINK>ok"), "ok");
        assert_eq!(strip_think("<think>never closed"), "");
    }

    #[test]
    fn refusals_and_promises_stay_invalid() {
        assert_eq!(
            normalize_reply("I don't have access to your file system.", &[]),
            "I don't have access to your file system."
        );
        assert!(empty_promise("Sure! I'll open the folder for you now."));
        assert!(!empty_promise("The folder contains a.txt and b.txt."));
        assert!(refusal("As an AI, I can't open browsers"));
        let decision = normalize_reply("{\"decision\":\"complete\",\"summary\":\"hi\"}", &[]);
        assert_eq!(decision, "{\"decision\":\"complete\",\"summary\":\"hi\"}");
    }

    #[test]
    fn compact_mode_exposes_core_tools_and_enable_tools() {
        let available = crate::autonomy::available_tools(true, true, false, true);
        let compact = tools(&available, &[], true, false);
        let names: Vec<&str> = compact
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert!(names.contains(&"list_dir") && names.contains(&"run_shell"));
        assert!(names.contains(&ENABLE_TOOLS) && names.contains(&FINISH));
        assert!(!names.contains(&"desktop_drag"));
        let enabled = tools(&available, &["desktop".into()], true, false);
        assert!(
            enabled
                .iter()
                .any(|t| t["function"]["name"] == "desktop_drag")
        );
        let full = tools(&available, &[], false, false);
        assert!(full.len() > compact.len());
        let review = tools(&available, &[], false, true);
        assert!(review.iter().all(|t| t["function"]["name"] != "write_file"));
    }

    #[test]
    fn model_sizes_are_parsed_from_tags() {
        assert_eq!(parameter_billions("qwen3.5:9b"), Some(9.0));
        assert_eq!(parameter_billions("llama3.2:3b-instruct-q4"), Some(3.0));
        assert_eq!(parameter_billions("qwen2.5:0.5b"), Some(0.5));
        assert_eq!(parameter_billions("fixture"), None);
        assert_eq!(parameter_billions("gpt-oss:latest"), None);
    }
}
