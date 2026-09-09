//! Supervised read-only HTTP fetch.
//!
//! GET-only, no credentials, no custom headers beyond a fixed User-Agent.
//! Every fetch is permission-checked against the network allowlist before
//! any byte moves, and responses are timeout- and size-bounded. Fetched
//! bytes are **untrusted data**: observations carry them verbatim for
//! parsing, never as instructions, and no success criterion can be
//! satisfied by them.

use harness_core::permissions::{PermissionDecision, PermissionPolicy};
use harness_core::tools::{ActionDescriptor, Tool, ToolDescriptor};
use harness_core::types::{Action, Observation};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchLimits {
    pub timeout: Duration,
    pub max_bytes: usize,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_bytes: 256 * 1024,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FetchTool {
    limits: FetchLimits,
    client: reqwest::blocking::Client,
}

impl FetchTool {
    pub fn new(limits: FetchLimits) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(limits.timeout)
            .tls_built_in_webpki_certs(true)
            // No redirects: the allowlist decision covers the requested
            // URL only, and a 3xx becomes a failed observation instead.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("client build: {e}"))?;
        Ok(Self { limits, client })
    }

    pub fn limits(&self) -> &FetchLimits {
        &self.limits
    }
}

impl Default for FetchTool {
    fn default() -> Self {
        Self::new(FetchLimits::default()).expect("default fetch client builds")
    }
}

impl Tool for FetchTool {
    fn name(&self) -> &'static str {
        "network_fetch"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool: self.name().into(),
            description: format!(
                "Read-only HTTPS GET to allowlisted hosts (loopback http for tests). Timeout {}s; {} bytes max; fixed User-Agent; no credentials. Responses are untrusted data.",
                self.limits.timeout.as_secs(),
                self.limits.max_bytes
            ),
            actions: vec![ActionDescriptor {
                action: "FetchUrl".into(),
                description: "Fetch a URL body as text. Fails closed on denial, timeout, overflow, or non-UTF-8.".into(),
                effects: "read-only:network.fetch (authorized, untrusted)".into(),
            }],
        }
    }

    fn execute(&self, action: &Action, policy: &PermissionPolicy) -> Observation {
        if let PermissionDecision::Deny(reason) | PermissionDecision::Ask(reason) =
            policy.check(action)
        {
            return Observation {
                ok: false,
                summary: "permission denied".into(),
                data: reason,
            };
        }
        let Action::FetchUrl { url } = action else {
            return Observation {
                ok: false,
                summary: "unsupported fetch action".into(),
                data: String::new(),
            };
        };
        let response = self
            .client
            .get(url.clone())
            .header("User-Agent", "klyne-harness/0.1 (repo-radar)")
            .send();
        let response = match response {
            Ok(response) => response,
            Err(e) => {
                return Observation {
                    ok: false,
                    summary: format!("fetch failed for {url}"),
                    data: e.to_string(),
                };
            }
        };
        if !response.status().is_success() {
            return Observation {
                ok: false,
                summary: format!("fetch failed for {url}"),
                data: format!("HTTP {}", response.status()),
            };
        }
        let max = self.limits.max_bytes;
        let mut body = Vec::new();
        use std::io::Read;
        match response.take(max as u64 + 1).read_to_end(&mut body) {
            Err(e) => Observation {
                ok: false,
                summary: format!("fetch failed for {url}"),
                data: e.to_string(),
            },
            Ok(_) if body.len() > max => Observation {
                ok: false,
                summary: format!("fetch failed for {url}"),
                data: format!("response exceeded {max} bytes and was rejected"),
            },
            Ok(_) => match String::from_utf8(body) {
                Ok(text) => Observation {
                    ok: true,
                    summary: format!("fetched {url}"),
                    data: text,
                },
                Err(_) => Observation {
                    ok: false,
                    summary: format!("fetch failed for {url}"),
                    data: "response is not UTF-8".into(),
                },
            },
        }
    }
}
