//! Controlled Chrome sessions: isolated by default, personal on request.
//!
//! Two launch modes and one attach mode:
//!
//! - [`ControlledBrowser::launch_isolated`] starts headless Chrome in a
//!   disposable profile. Nothing personal is reachable. This is the default.
//! - [`ControlledBrowser::launch_with_profile`] starts Chrome pointed at
//!   one of the user's real profiles. Fails closed when Chrome already
//!   runs (the new process would just forward to it) — close Chrome first
//!   or use [`ControlledBrowser::attach`] instead.
//! - [`ControlledBrowser::attach`] drives an already-debuggable instance
//!   (started with `--remote-debugging-port`), e.g. the user's live
//!   browser. Attached sessions **never** terminate the browser process:
//!   closing only drops the debugger connection.

use crate::cdp::{self, CdpSession};
use crate::profile;
use serde_json::Value;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct BrowserLimits {
    pub launch_timeout: Duration,
    pub command_timeout: Duration,
    pub max_message_bytes: usize,
    pub max_screenshot_bytes: usize,
}

impl Default for BrowserLimits {
    fn default() -> Self {
        Self {
            launch_timeout: Duration::from_secs(30),
            command_timeout: Duration::from_secs(30),
            max_message_bytes: 8 * 1024 * 1024,
            max_screenshot_bytes: 4 * 1024 * 1024,
        }
    }
}

enum Handle {
    /// Browser we spawned: closing terminates the whole process tree and
    /// removes the disposable profile.
    Owned {
        child: std::process::Child,
        profile_dir: Option<PathBuf>,
    },
    /// Somebody else's browser: closing only drops our connection.
    Attached,
}

pub struct ControlledBrowser {
    handle: Handle,
    debugger_addr: String,
    session: CdpSession,
    limits: BrowserLimits,
}

fn free_port() -> io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn wait_debugger(addr: &str, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if cdp::http_request(addr, "GET", "/json/version", Duration::from_secs(2)).is_ok() {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(io::Error::other(format!(
                "debugger at {addr} never came up; if Chrome is already running, close it or use attach()"
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn terminate_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .output();
        let _ = child.kill();
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
}

impl ControlledBrowser {
    fn connect_page(
        debugger_addr: &str,
        limits: &BrowserLimits,
        tab_filter: Option<&str>,
    ) -> io::Result<CdpSession> {
        let targets = cdp::list_targets(debugger_addr, limits.launch_timeout)
            .map_err(|e| io::Error::other(format!("cannot list targets: {e}")))?;
        let page = targets
            .iter()
            .filter(|target| target.kind == "page")
            .find(|target| {
                tab_filter.is_none_or(|filter| {
                    target.title.contains(filter) || target.url.contains(filter)
                })
            })
            .ok_or_else(|| io::Error::other("no matching page target"))?;
        let mut session = CdpSession::connect(
            &page.ws_url,
            limits.launch_timeout,
            limits.max_message_bytes,
        )
        .map_err(|e| io::Error::other(format!("cannot attach to page: {e}")))?;
        session
            .call("Page.enable", Value::Null, limits.command_timeout)
            .map_err(io::Error::other)?;
        Ok(session)
    }

    fn spawn(
        binary: &PathBuf,
        profile_dir: PathBuf,
        extra_args: &[String],
        limits: &BrowserLimits,
    ) -> io::Result<(std::process::Child, String)> {
        let port = free_port()?;
        let debugger_addr = format!("127.0.0.1:{port}");
        let mut command = std::process::Command::new(binary);
        command
            .arg(format!("--user-data-dir={}", profile_dir.to_string_lossy()))
            .arg(format!("--remote-debugging-port={port}"))
            .args([
                "--headless=new",
                "--disable-gpu",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
            ])
            .args(extra_args)
            .arg("about:blank")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = command
            .spawn()
            .map_err(|e| io::Error::other(format!("cannot launch Chrome: {e}")))?;
        if let Err(e) = wait_debugger(&debugger_addr, limits.launch_timeout) {
            let mut child = child;
            terminate_tree(&mut child);
            return Err(e);
        }
        Ok((child, debugger_addr))
    }

    /// Disposable headless Chrome. Nothing outside the temp profile is
    /// reachable, and nothing persists afterwards.
    pub fn launch_isolated(limits: BrowserLimits) -> io::Result<Self> {
        let binary =
            profile::chrome_binary().ok_or_else(|| io::Error::other("Chrome not found"))?;
        let profile_dir = std::env::temp_dir().join(format!(
            "harness-browser-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&profile_dir)?;
        let (child, debugger_addr) = Self::spawn(&binary, profile_dir.clone(), &[], &limits)?;
        let session = Self::connect_page(&debugger_addr, &limits, None)?;
        Ok(Self {
            handle: Handle::Owned {
                child,
                profile_dir: Some(profile_dir),
            },
            debugger_addr,
            session,
            limits,
        })
    }

    /// Chrome pointed at one of the user's real profiles (`Local State`
    /// name or directory, e.g. `"Work"` or `"Profile 1"`). Browses as the
    /// user: sessions, logins, and cookies all apply, and every action
    /// runs with the user's identity — treat page content as untrusted
    /// data with full ambient authority behind it. Fails closed while
    /// another Chrome instance holds the profile.
    pub fn launch_with_profile(profile: &str, limits: BrowserLimits) -> io::Result<Self> {
        let binary =
            profile::chrome_binary().ok_or_else(|| io::Error::other("Chrome not found"))?;
        let data_dir = profile::user_data_dir()
            .ok_or_else(|| io::Error::other("Chrome user-data directory unknown"))?;
        let directory = profile::list_profiles()?
            .into_iter()
            .find(|info| info.name == profile || info.directory == profile)
            .map(|info| info.directory)
            .ok_or_else(|| io::Error::other(format!("unknown Chrome profile '{profile}'")))?;
        let (child, debugger_addr) = Self::spawn(
            &binary,
            data_dir,
            &[format!("--profile-directory={directory}")],
            &limits,
        )?;
        let session = Self::connect_page(&debugger_addr, &limits, None)?;
        Ok(Self {
            handle: Handle::Owned {
                child,
                profile_dir: None,
            },
            debugger_addr,
            session,
            limits,
        })
    }

    /// Drive an already-running debuggable instance without owning it.
    /// Start Chrome with `--remote-debugging-port=<port>` first (an
    /// explicit user action); `tab_filter` selects by title or URL
    /// substring, or the first page when `None`. Attached sessions never
    /// kill the browser: [`ControlledBrowser::close`] only disconnects.
    pub fn attach(
        debugger_addr: &str,
        tab_filter: Option<&str>,
        limits: BrowserLimits,
    ) -> io::Result<Self> {
        let session = Self::connect_page(debugger_addr, &limits, tab_filter)?;
        Ok(Self {
            handle: Handle::Attached,
            debugger_addr: debugger_addr.into(),
            session,
            limits,
        })
    }

    /// List page targets (id, title, URL) visible on a debuggable instance
    /// without attaching. Read-only; safe to run against a live browser.
    /// The id is the stable tab handle for future select/close calls.
    pub fn list_tabs(
        debugger_addr: &str,
        timeout: Duration,
    ) -> io::Result<Vec<(String, String, String)>> {
        Ok(cdp::list_targets(debugger_addr, timeout)
            .map_err(|e| io::Error::other(format!("cannot list targets: {e}")))?
            .into_iter()
            .filter(|target| target.kind == "page")
            .map(|target| (target.id, target.title, target.url))
            .collect())
    }

    pub fn debugger_addr(&self) -> &str {
        &self.debugger_addr
    }

    fn evaluate_value(&mut self, expression: &str) -> Result<Value, String> {
        let result = self.session.call(
            "Runtime.evaluate",
            serde_json::json!({"expression": expression, "returnByValue": true}),
            self.limits.command_timeout,
        )?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(format!("page threw: {exception}"));
        }
        let result = result
            .get("result")
            .ok_or_else(|| "expression returned no result".to_string())?;
        // `undefined` (and unserializable values) carry no `value` key;
        // surface them as null rather than failing the call.
        Ok(result.get("value").cloned().unwrap_or(Value::Null))
    }

    /// Navigate and wait for the load event.
    pub fn navigate(&mut self, url: &str) -> Result<(), String> {
        self.session.call(
            "Page.navigate",
            serde_json::json!({"url": url}),
            self.limits.command_timeout,
        )?;
        self.session
            .wait_event("Page.loadEventFired", self.limits.command_timeout)?;
        Ok(())
    }

    pub fn title(&mut self) -> Result<String, String> {
        self.evaluate_value("document.title").and_then(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "title is not a string".to_string())
        })
    }

    pub fn text(&mut self) -> Result<String, String> {
        self.evaluate_value("document.body ? document.body.innerText : ''")
            .and_then(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "text is not a string".to_string())
            })
    }

    /// Raw evaluated value, for assertions and structured reads.
    pub fn eval(&mut self, expression: &str) -> Result<Value, String> {
        self.evaluate_value(expression)
    }

    /// Click the first element matching a CSS selector. Returns an error
    /// (never clicks blindly) when nothing matches.
    pub fn click(&mut self, selector: &str) -> Result<(), String> {
        let selector_json = serde_json::to_string(selector).map_err(|e| e.to_string())?;
        let outcome = self.evaluate_value(&format!(
            "(() => {{ const el = document.querySelector({selector_json}); if (!el) return 'missing'; el.click(); return 'clicked'; }})()"
        ))?;
        match outcome.as_str() {
            Some("clicked") => Ok(()),
            _ => Err(format!("selector matched nothing: {selector}")),
        }
    }

    /// Type into the first element matching a CSS selector, dispatching
    /// input/change events so framework bindings observe it.
    pub fn fill(&mut self, selector: &str, text: &str) -> Result<(), String> {
        let selector_json = serde_json::to_string(selector).map_err(|e| e.to_string())?;
        let text_json = serde_json::to_string(text).map_err(|e| e.to_string())?;
        let outcome = self.evaluate_value(&format!(
            "(() => {{ const el = document.querySelector({selector_json}); if (!el) return 'missing'; el.focus(); el.value = {text_json}; el.dispatchEvent(new Event('input', {{bubbles: true}})); el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'filled'; }})()"
        ))?;
        match outcome.as_str() {
            Some("filled") => Ok(()),
            _ => Err(format!("selector matched nothing: {selector}")),
        }
    }

    /// PNG screenshot bytes, bounded. Large pages fail closed instead of
    /// growing memory without limit.
    pub fn screenshot(&mut self) -> Result<Vec<u8>, String> {
        let result = self.session.call(
            "Page.captureScreenshot",
            serde_json::json!({}),
            self.limits.command_timeout,
        )?;
        let data = result
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| "screenshot returned no data".to_string())?;
        let bytes = crate::ws::base64_decode(data)?;
        if bytes.len() > self.limits.max_screenshot_bytes {
            return Err(format!(
                "screenshot exceeds {} bytes",
                self.limits.max_screenshot_bytes
            ));
        }
        Ok(bytes)
    }

    /// Close deterministically. Owned browsers are terminated with their
    /// whole process tree and their disposable profile removed; attached
    /// sessions only disconnect — the user's browser keeps running.
    pub fn close(&mut self) {
        self.session.close();
        if let Handle::Owned { child, profile_dir } = &mut self.handle {
            terminate_tree(child);
            let _ = child.wait();
            if let Some(dir) = profile_dir.take() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

impl Drop for ControlledBrowser {
    fn drop(&mut self) {
        self.session.close();
        if let Handle::Owned { child, .. } = &mut self.handle {
            terminate_tree(child);
        }
    }
}

/// `file://` URL for a local path, with minimal percent-encoding.
pub fn file_url(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}
