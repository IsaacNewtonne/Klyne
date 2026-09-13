//! Host-owned authority broker (audit Phase 2).
//!
//! The model proposes; only this module — plus grants the user wrote into
//! their own request body — disposes. Every decision here is a pure
//! function over exact bindings, so dispatchers enforce without discretion
//! and the recovery guardian cannot widen authority by editing call sites:
//! this file is on its protected list.
//!
//! Three bindings, all exact:
//!
//! - [`ShellGrant`]: one program plus its exact argv. Changing the
//!   executable or any argument invalidates the grant.
//! - [`SecretGrant`]: one environment-secret name plus the origins it may
//!   leave the machine for. Empty origins means process-environment use
//!   only (shell `env`), never network exfiltration.
//! - [`DeleteGrant`]: one (connection, method, path) triple for
//!   destructive API calls.
//!
//! Approvals cover retries of the *identical* action only. Anything else
//! needs a new approval. There is deliberately no wildcard, prefix, or
//! "approve all" grant: those reintroduce the broad ambient authority this
//! broker removes.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellGrant {
    pub program: String,
    pub args: Vec<String>,
    pub granted_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretGrant {
    pub name: String,
    pub origins: Vec<String>,
    pub granted_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteGrant {
    pub connection: String,
    pub method: String,
    pub path: String,
    pub granted_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    NeedsApproval,
}

/// Exact-argv shell authorization. The model-chosen program is never
/// allowlisted by the act of proposing it.
pub fn shell_allowed(grants: &[ShellGrant], program: &str, args: &[String]) -> Decision {
    if grants
        .iter()
        .any(|grant| grant.program == program && grant.args == args)
    {
        Decision::Allow
    } else {
        Decision::NeedsApproval
    }
}

/// Secret authorization. `origin` is the normalized destination the secret
/// would be sent to; pass an empty origin for process-environment use,
/// which additionally requires the grant to carry no origins (a network
/// grant never authorizes local exfiltration into child processes, and an
/// env-only grant never authorizes transmission).
pub fn secret_allowed(grants: &[SecretGrant], name: &str, origin: &str) -> bool {
    grants.iter().any(|grant| {
        grant.name == name
            && if origin.is_empty() {
                grant.origins.is_empty()
            } else {
                grant.origins.iter().any(|allowed| allowed == origin)
            }
    })
}

/// Destructive API authorization: exact (connection, method, path).
pub fn delete_allowed(grants: &[DeleteGrant], connection: &str, method: &str, path: &str) -> bool {
    grants
        .iter()
        .any(|grant| grant.connection == connection && grant.method == method && grant.path == path)
}

/// Validate a user-supplied shell grant from a request body. The request
/// body is user-controlled (the model never writes it), which is what
/// makes these grants host-owned rather than model-proposed.
pub fn valid_shell_grant(program: &str, args: &[String]) -> bool {
    !program.is_empty()
        && program.len() <= 256
        && !program.bytes().any(|b| b.is_ascii_control())
        && args.len() <= 64
        && args
            .iter()
            .all(|a| a.len() <= 1024 && !a.bytes().any(|b| b.is_ascii_control()))
}

/// Validate a user-supplied secret grant. Names follow environment-variable
/// rules; origins are checked for shape here and normalized against the
/// saved connection origin at enforcement time.
pub fn valid_secret_grant(name: &str, origins: &[String]) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && origins.len() <= 16
        && origins.iter().all(|o| {
            !o.is_empty() && o.len() <= 512 && !o.bytes().any(|b| b.is_ascii_control() || b == b' ')
        })
}

/// Merge user-supplied grants into conversation grants, deduplicating exact
/// bindings. Merge-only (never replace): a later turn without grants must
/// not silently drop earlier approvals.
pub fn merge_shell_grants(into: &mut Vec<ShellGrant>, fresh: Vec<ShellGrant>) {
    for grant in fresh {
        if !into
            .iter()
            .any(|g| g.program == grant.program && g.args == grant.args)
        {
            into.push(grant);
        }
    }
}

pub fn merge_secret_grants(into: &mut Vec<SecretGrant>, fresh: Vec<SecretGrant>) {
    for grant in fresh {
        if let Some(existing) = into.iter_mut().find(|g| g.name == grant.name) {
            for origin in grant.origins {
                if !existing.origins.contains(&origin) {
                    existing.origins.push(origin);
                }
            }
        } else {
            into.push(grant);
        }
    }
}

/// Caller context for app-route enforcement. User-driven management calls
/// (`by_user`) carry the user's own typed secrets and skip grant checks;
/// model-driven chat calls enforce them.
#[derive(Clone, Default)]
pub struct AppAccess {
    pub secrets: Vec<SecretGrant>,
    pub deletes: Vec<DeleteGrant>,
    pub by_user: bool,
}

/// Collect live secret values for granted names. Short or absent values
/// are skipped: scrubbing a 3-character value would corrupt unrelated
/// text, and a missing variable has nothing to leak.
pub fn secret_values(grants: &[SecretGrant]) -> Vec<String> {
    let mut seen = Vec::new();
    for grant in grants {
        if let Ok(value) = std::env::var(&grant.name)
            && value.len() >= 8
            && !seen.contains(&value)
        {
            seen.push(value);
        }
    }
    seen
}

/// Redact known secret values inside a JSON document, in place. Applied at
/// the two chokepoints the audit names: before model dispatch (prompts)
/// and before persistence (logs). Process memory between those points is
/// out of scope; at-rest and in-prompt data stays clean.
pub fn scrub_value(value: &mut serde_json::Value, secrets: &[String]) {
    match value {
        serde_json::Value::String(text) => {
            for secret in secrets {
                if text.contains(secret) {
                    *text = text.replace(secret, "[redacted]");
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                scrub_value(item, secrets);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                scrub_value(item, secrets);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(program: &str, args: &[&str]) -> (String, Vec<String>) {
        (program.into(), args.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn shell_grants_bind_exact_argv() {
        let (program, args) = shell("pytest", &["tests/", "-q"]);
        let grants = vec![ShellGrant {
            program: program.clone(),
            args: args.clone(),
            granted_at_ms: 1,
        }];
        assert_eq!(shell_allowed(&grants, &program, &args), Decision::Allow);
        // Any argument change invalidates the approval.
        for other in [
            vec!["tests/".to_string()],
            vec![
                "tests/".to_string(),
                "-q".to_string(),
                "--extra".to_string(),
            ],
            vec!["other/".to_string(), "-q".to_string()],
        ] {
            assert_eq!(
                shell_allowed(&grants, &program, &other),
                Decision::NeedsApproval
            );
        }
        // Another program never inherits.
        assert_eq!(
            shell_allowed(&grants, "python", &args),
            Decision::NeedsApproval
        );
        // Empty grants deny everything.
        assert_eq!(shell_allowed(&[], &program, &args), Decision::NeedsApproval);
    }

    #[test]
    fn secrets_bind_name_to_origin() {
        let grants = vec![SecretGrant {
            name: "GITHUB_TOKEN".into(),
            origins: vec!["https://api.github.com".into()],
            granted_at_ms: 1,
        }];
        assert!(secret_allowed(
            &grants,
            "GITHUB_TOKEN",
            "https://api.github.com"
        ));
        // Lookalike and fresh origins are refused.
        assert!(!secret_allowed(&grants, "GITHUB_TOKEN", "https://evil.com"));
        assert!(!secret_allowed(
            &grants,
            "GITHUB_TOKEN",
            "https://api.github.com.evil.com"
        ));
        // Unlisted names are refused, including for env use.
        assert!(!secret_allowed(&grants, "OTHER_KEY", ""));
        // A network grant never authorizes process-environment exfiltration.
        assert!(!secret_allowed(&grants, "GITHUB_TOKEN", ""));
        // An env-only grant authorizes shell env use but no transmission.
        let env_only = vec![SecretGrant {
            name: "BUILD_KEY".into(),
            origins: vec![],
            granted_at_ms: 2,
        }];
        assert!(secret_allowed(&env_only, "BUILD_KEY", ""));
        assert!(!secret_allowed(
            &env_only,
            "BUILD_KEY",
            "https://api.github.com"
        ));
    }

    #[test]
    fn deletes_bind_connection_method_path() {
        let grants = vec![DeleteGrant {
            connection: "api".into(),
            method: "DELETE".into(),
            path: "/items/7".into(),
            granted_at_ms: 1,
        }];
        assert!(delete_allowed(&grants, "api", "DELETE", "/items/7"));
        assert!(!delete_allowed(&grants, "api", "DELETE", "/items/8"));
        assert!(!delete_allowed(&grants, "api", "POST", "/items/7"));
        assert!(!delete_allowed(&grants, "other", "DELETE", "/items/7"));
    }

    #[test]
    fn user_grant_shapes_are_validated() {
        assert!(valid_shell_grant("pytest", &["a".to_string()]));
        assert!(!valid_shell_grant("", &[]));
        assert!(!valid_shell_grant("a\nb", &[]));
        assert!(valid_secret_grant(
            "GITHUB_TOKEN",
            &["https://api.github.com".into()]
        ));
        assert!(!valid_secret_grant("evil name", &[]));
        assert!(!valid_secret_grant("K", &["has space".into()]));
    }

    #[test]
    fn merges_never_drop_existing_approvals() {
        let mut grants = vec![ShellGrant {
            program: "a".into(),
            args: vec![],
            granted_at_ms: 1,
        }];
        merge_shell_grants(
            &mut grants,
            vec![ShellGrant {
                program: "a".into(),
                args: vec![],
                granted_at_ms: 2,
            }],
        );
        assert_eq!(grants.len(), 1);
        let mut secrets = vec![SecretGrant {
            name: "K".into(),
            origins: vec!["https://a.example".into()],
            granted_at_ms: 1,
        }];
        merge_secret_grants(
            &mut secrets,
            vec![SecretGrant {
                name: "K".into(),
                origins: vec!["https://b.example".into()],
                granted_at_ms: 2,
            }],
        );
        assert_eq!(secrets[0].origins.len(), 2);
    }

    #[test]
    fn scrubbing_removes_live_values_everywhere_they_hide() {
        let secrets = vec!["hunter2-secret-value".to_string()];
        let mut document = serde_json::json!({
            "summary": "called with hunter2-secret-value",
            "nested": {"body": "Bearer hunter2-secret-value here", "other": "untouched"},
            "list": ["hunter2-secret-value", 42, true, null],
        });
        scrub_value(&mut document, &secrets);
        let text = document.to_string();
        assert!(!text.contains("hunter2-secret-value"), "{text}");
        assert!(text.contains("[redacted]"));
        assert!(text.contains("untouched"));
    }

    #[test]
    fn short_values_never_reach_the_scrubber() {
        unsafe { std::env::set_var("HARNESS_BROKER_SHORT_XYZ", "abc") };
        unsafe { std::env::set_var("HARNESS_BROKER_LONG_XYZ", "long-enough-secret") };
        let values = secret_values(&[
            SecretGrant {
                name: "HARNESS_BROKER_SHORT_XYZ".into(),
                origins: vec![],
                granted_at_ms: 1,
            },
            SecretGrant {
                name: "HARNESS_BROKER_LONG_XYZ".into(),
                origins: vec![],
                granted_at_ms: 1,
            },
            SecretGrant {
                name: "HARNESS_BROKER_MISSING_XYZ".into(),
                origins: vec![],
                granted_at_ms: 1,
            },
        ]);
        // A 3-character "secret" would redact half the logs; missing
        // variables have nothing to leak.
        assert_eq!(values, vec!["long-enough-secret".to_string()]);
        unsafe { std::env::remove_var("HARNESS_BROKER_SHORT_XYZ") };
        unsafe { std::env::remove_var("HARNESS_BROKER_LONG_XYZ") };
    }
}
