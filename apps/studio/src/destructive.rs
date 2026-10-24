//! Confirm-before-destructive gate (added 2026-10-24).
//!
//! Klyne stays autonomous for ordinary work (reads, listings, writes, web
//! fetches, desktop observation). A small set of consequential actions pauses
//! in the existing approval queue first:
//!   * `delete_path`
//!   * `move_file` / `copy_file` onto an existing destination (overwrite)
//!   * `run_shell` commands matching destructive patterns (delete, format,
//!     registry edits, shutdown/restart, process kill, force push / hard
//!     reset, package installs, setx)
//!   * desktop clicks/invokes/typing on send, buy, post, submit-style controls
//!   * runtime activation (`runtime_stage`)
//!
//! The user can approve once (exact action) or for this conversation
//! (same category, e.g. every delete in this conversation).
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Stable category for "approve for this conversation" grants.
    pub category: String,
    /// Human-readable reason shown on the approval card.
    pub reason: String,
}

fn finding(category: &str, reason: String) -> Option<Finding> {
    Some(Finding {
        category: category.into(),
        reason,
    })
}

/// Lowercased words of a command line, split on whitespace and shell
/// punctuation, keeping `-`, `.`, `:`, `/`, `\` inside words.
fn words(line: &str) -> Vec<String> {
    line.to_lowercase()
        .split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    ';' | '|' | '&' | '(' | ')' | '{' | '}' | '"' | '\'' | '`' | ','
                )
        })
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

fn base(word: &str) -> &str {
    let name = word.rsplit(['\\', '/']).next().unwrap_or(word);
    name.strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".cmd"))
        .or_else(|| name.strip_suffix(".bat"))
        .or_else(|| name.strip_suffix(".com"))
        .unwrap_or(name)
}

/// Classify a shell command. Returns the matched pattern label.
pub fn shell(program: &str, args: &[String]) -> Option<Finding> {
    let mut line = program.to_owned();
    for arg in args {
        line.push(' ');
        line.push_str(arg);
    }
    let w = words(&line);
    let commands: Vec<&str> = w.iter().map(|x| base(x)).collect();
    let has = |x: &str| commands.contains(&x);
    let after = |first: &str, seconds: &[&str]| {
        commands
            .windows(2)
            .any(|pair| pair[0] == first && seconds.contains(&pair[1]))
    };
    let label = if has("remove-item")
        || has("ri")
        || has("rm")
        || has("rmdir")
        || has("rd")
        || has("del")
        || has("erase")
        || has("remove-itemproperty")
    {
        Some("delete files")
    } else if has("format")
        || has("format-volume")
        || has("clear-disk")
        || has("initialize-disk")
        || has("diskpart")
    {
        Some("format or wipe a disk")
    } else if after("reg", &["delete", "add", "import"]) || has("remove-itemproperty") {
        Some("edit the registry")
    } else if has("shutdown") || has("restart-computer") || has("stop-computer") {
        Some("shut down or restart the computer")
    } else if has("stop-process") || has("taskkill") || has("kill") || has("spps") {
        Some("stop processes")
    } else if (after(
        "push",
        &[
            "--force",
            "-f",
            "--force-with-lease",
            "--mirror",
            "--delete",
        ],
    ) || w
        .iter()
        .any(|x| x == "--force" || x.starts_with("--force-with-lease"))
        && has("push")
        && has("git"))
        || (has("git") && after("reset", &["--hard"]))
        || (has("git") && after("clean", &["-f", "-fd", "-fdx", "-xdf", "-df"]))
    {
        Some("rewrite git history or discard changes")
    } else if has("setx") || line.to_lowercase().contains("setenvironmentvariable") {
        Some("change persistent environment variables")
    } else if after("npm", &["install", "i", "add", "ci", "uninstall"])
        || after("pnpm", &["add", "install", "i", "remove"])
        || after("yarn", &["add", "install", "remove"])
        || after("pip", &["install", "uninstall"])
        || after("pip3", &["install", "uninstall"])
        || after("winget", &["install", "uninstall", "upgrade"])
        || after("choco", &["install", "uninstall", "upgrade"])
        || after("scoop", &["install", "uninstall"])
        || after("cargo", &["install", "uninstall"])
        || after("apt", &["install", "remove"])
        || after("apt-get", &["install", "remove"])
        || after("msiexec", &["/i", "/x"])
        || has("install-package")
        || has("install-module")
        || has("uninstall-package")
    {
        Some("install or remove software")
    } else {
        None
    };
    label.and_then(|label| {
        finding(
            &format!("shell:{label}"),
            format!(
                "Runs a command that may {label}: {}",
                line.chars().take(300).collect::<String>()
            ),
        )
    })
}

const CONSEQUENTIAL: &[&str] = &[
    "send", "buy", "purchase", "pay", "post", "publish", "submit", "checkout", "order", "tweet",
    "reply", "share", "transfer", "confirm", "delete", "remove",
];

fn consequential_label(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    // Short control labels only: a long text block that happens to contain
    // "post" is content, not a button.
    if tokens.is_empty() || tokens.len() > 6 {
        return None;
    }
    tokens
        .iter()
        .find(|t| CONSEQUENTIAL.contains(t))
        .map(|t| (*t).to_owned())
}

fn control_at(observation: &Value, x: i64, y: i64) -> Option<&Value> {
    observation["controls"]
        .as_array()?
        .iter()
        .filter(|c| {
            let (cx, cy, cw, ch) = (
                c["x"].as_i64().unwrap_or(i64::MIN),
                c["y"].as_i64().unwrap_or(i64::MIN),
                c["width"].as_i64().unwrap_or(0),
                c["height"].as_i64().unwrap_or(0),
            );
            cw > 0 && ch > 0 && x >= cx && y >= cy && x < cx + cw && y < cy + ch
        })
        .min_by_key(|c| c["width"].as_i64().unwrap_or(0) * c["height"].as_i64().unwrap_or(0))
}

fn control_by_id<'a>(observation: &'a Value, element: &Value) -> Option<&'a Value> {
    observation["controls"]
        .as_array()?
        .iter()
        .find(|c| c["element"] == *element)
}

const SEND_APPS: &[&str] = &[
    "outlook",
    "mail",
    "thunderbird",
    "teams",
    "slack",
    "discord",
    "whatsapp",
    "telegram",
    "signal",
    "zalo",
    "messenger",
    "gmail",
    "amazon",
    "checkout",
    "x.com",
    "twitter",
];

/// Classify a desktop action against the most recent observation.
pub fn desktop(action: &Value, observation: Option<&Value>) -> Option<Finding> {
    let tool = action["tool"].as_str()?;
    let observation = observation?;
    let control = match tool {
        "desktop_click" => control_at(
            observation,
            action["x"].as_i64().unwrap_or(-1),
            action["y"].as_i64().unwrap_or(-1),
        ),
        "desktop_invoke" | "desktop_fill" => control_by_id(observation, &action["element"]),
        "desktop_key" => {
            let key = action["key"].as_str().unwrap_or("").to_ascii_uppercase();
            if key == "ENTER" || key.ends_with("+ENTER") {
                let window = observation["windows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|w| w["window"] == action["window"]);
                let title = window
                    .map(|w| {
                        format!(
                            "{} {}",
                            w["title"].as_str().unwrap_or(""),
                            w["process_name"].as_str().unwrap_or("")
                        )
                        .to_lowercase()
                    })
                    .unwrap_or_default();
                if let Some(app) = SEND_APPS.iter().find(|a| title.contains(*a)) {
                    return finding(
                        "desktop:send",
                        format!(
                            "Presses {key} in a messaging/shopping window ({app}); this may send or submit."
                        ),
                    );
                }
            }
            return None;
        }
        _ => return None,
    }?;
    let name = control["name"].as_str().unwrap_or("");
    let label = consequential_label(name)?;
    finding(
        &format!("desktop:{label}"),
        format!(
            "{tool} on the \"{}\" control, which may {label}.",
            name.chars().take(80).collect::<String>()
        ),
    )
}

/// Classify a canonical action (paths already normalized against the
/// workspace). `workspace` resolves relative destinations for overwrite checks.
pub fn classify(action: &Value, workspace: &Path, observation: Option<&Value>) -> Option<Finding> {
    let tool = action["tool"].as_str()?;
    match tool {
        "delete_path" => finding(
            "delete_path",
            format!("Deletes {}", action["path"].as_str().unwrap_or("?")),
        ),
        "move_file" | "copy_file" => {
            let to = action["to"].as_str()?;
            let target = if Path::new(to).is_absolute() {
                Path::new(to).to_path_buf()
            } else {
                workspace.join(to)
            };
            target.exists().then(|| Finding {
                category: format!("{tool}:overwrite"),
                reason: format!(
                    "{} {} onto existing {} (overwrites it)",
                    if tool == "move_file" {
                        "Moves"
                    } else {
                        "Copies"
                    },
                    action["from"].as_str().unwrap_or("?"),
                    to
                ),
            })
        }
        "run_shell" => {
            let program = action["program"].as_str()?;
            let args: Vec<String> =
                serde_json::from_value(action["args"].clone()).unwrap_or_default();
            shell(program, &args)
        }
        "runtime_stage" => finding(
            "runtime_stage",
            format!(
                "Activates a new Klyne runtime binary {}",
                action["binary"].as_str().unwrap_or("?")
            ),
        ),
        t if t.starts_with("desktop_") => desktop(action, observation),
        _ => None,
    }
}

/// Exact binding for an "approve once" grant.
pub fn binding(action: &Value) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(action.to_string().as_bytes()))
}

/// The approval proposal stored in the existing queue.
pub fn proposal(action: &Value, finding: &Finding) -> Value {
    json!({"kind":"destructive","tool":action["tool"],"action":action,
        "category":finding.category,"reason":finding.reason,"binding":binding(action)})
}

/// Whether a grant list allows this action. Returns the index of a
/// once-grant to consume, or None for a conversation grant.
pub fn allowed(grants: &[Value], action: &Value, finding: &Finding) -> Option<Option<usize>> {
    let exact = binding(action);
    if grants
        .iter()
        .any(|g| g["scope"] == "conversation" && g["category"] == finding.category.as_str())
    {
        return Some(None);
    }
    grants
        .iter()
        .position(|g| g["scope"] == "once" && g["binding"] == exact.as_str())
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sh(line: &str) -> Option<Finding> {
        let mut parts = line.split(' ').map(str::to_owned);
        let program = parts.next().unwrap();
        shell(&program, &parts.collect::<Vec<_>>())
    }
    #[test]
    fn destructive_shell_patterns_are_detected() {
        for line in [
            "powershell -NoProfile -Command Remove-Item C:\\x -Recurse",
            "cmd /c del C:\\x\\a.txt",
            "rm -rf build",
            "cmd /c rmdir /s /q out",
            "format D:",
            "reg delete HKCU\\Software\\X /f",
            "reg add HKCU\\Software\\X /v a /d b",
            "shutdown /r /t 0",
            "powershell -Command Restart-Computer",
            "powershell -Command Stop-Process -Name chrome",
            "taskkill /F /IM chrome.exe",
            "git push --force origin main",
            "git push -f",
            "git reset --hard HEAD~1",
            "npm.cmd install left-pad",
            "pip install requests",
            "winget install Git.Git",
            "setx PATH C:\\x",
            "C:\\Windows\\System32\\cmd.exe /c del x",
        ] {
            assert!(sh(line).is_some(), "{line}");
        }
        for line in [
            "git status",
            "git push origin main",
            "cargo build --locked",
            "powershell -Command Get-ChildItem C:\\temp",
            "npm.cmd test",
            "cmd /c dir",
            "powershell -Command Get-Date -Format o",
            "python script.py --format json",
        ] {
            assert!(sh(line).is_none(), "{line}");
        }
    }
    #[test]
    fn deletes_and_overwrites_need_approval() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("exists.txt"), "x").unwrap();
        assert!(
            classify(
                &json!({"tool":"delete_path","path":"a.txt"}),
                dir.path(),
                None
            )
            .is_some()
        );
        assert!(
            classify(
                &json!({"tool":"move_file","from":"a.txt","to":"exists.txt"}),
                dir.path(),
                None
            )
            .is_some()
        );
        assert!(
            classify(
                &json!({"tool":"move_file","from":"a.txt","to":"new.txt"}),
                dir.path(),
                None
            )
            .is_none()
        );
        assert!(
            classify(
                &json!({"tool":"write_file","path":"exists.txt","contents":"y"}),
                dir.path(),
                None
            )
            .is_none()
        );
        assert!(classify(&json!({"tool":"list_dir","path":"."}), dir.path(), None).is_none());
        assert!(
            classify(
                &json!({"tool":"runtime_stage","binary":"x.exe","sha256":"0"}),
                dir.path(),
                None
            )
            .is_some()
        );
    }
    #[test]
    fn desktop_send_controls_need_approval() {
        let observation = json!({"windows":[{"window":"7","title":"Inbox - Outlook","process_name":"olk"}],
            "controls":[{"element":"1","name":"Send","x":10,"y":10,"width":50,"height":20},
                        {"element":"2","name":"Message body","x":0,"y":40,"width":300,"height":200}]});
        assert!(
            desktop(
                &json!({"tool":"desktop_click","window":"7","x":20,"y":15,"button":"left"}),
                Some(&observation)
            )
            .is_some()
        );
        assert!(
            desktop(
                &json!({"tool":"desktop_click","window":"7","x":20,"y":100,"button":"left"}),
                Some(&observation)
            )
            .is_none()
        );
        assert!(
            desktop(
                &json!({"tool":"desktop_invoke","window":"7","element":"1"}),
                Some(&observation)
            )
            .is_some()
        );
        assert!(
            desktop(
                &json!({"tool":"desktop_key","window":"7","key":"CTRL+ENTER"}),
                Some(&observation)
            )
            .is_some()
        );
        assert!(desktop(&json!({"tool":"desktop_observe"}), Some(&observation)).is_none());
    }
    #[test]
    fn once_and_conversation_grants_match_correctly() {
        let action = json!({"tool":"delete_path","path":"a.txt"});
        let f = classify(&action, Path::new("."), None).unwrap();
        assert_eq!(allowed(&[], &action, &f), None);
        let once =
            vec![json!({"scope":"once","binding":binding(&action),"category":"delete_path"})];
        assert_eq!(allowed(&once, &action, &f), Some(Some(0)));
        let other = json!({"tool":"delete_path","path":"b.txt"});
        assert_eq!(
            allowed(
                &once,
                &other,
                &classify(&other, Path::new("."), None).unwrap()
            ),
            None
        );
        let conv = vec![json!({"scope":"conversation","category":"delete_path"})];
        assert_eq!(allowed(&conv, &other, &f), Some(None));
    }
}
