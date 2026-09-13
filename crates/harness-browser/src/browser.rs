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
            let _ = child.wait();
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
        let (mut child, debugger_addr) =
            match Self::spawn(&binary, profile_dir.clone(), &[], &limits) {
                Ok(v) => v,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&profile_dir);
                    return Err(e);
                }
            };
        let session = match Self::connect_page(&debugger_addr, &limits, None) {
            Ok(v) => v,
            Err(e) => {
                terminate_tree(&mut child);
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&profile_dir);
                return Err(e);
            }
        };
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
        let (mut child, debugger_addr) = Self::spawn(
            &binary,
            data_dir,
            &[format!("--profile-directory={directory}")],
            &limits,
        )?;
        let session = match Self::connect_page(&debugger_addr, &limits, None) {
            Ok(session) => session,
            Err(error) => {
                terminate_tree(&mut child);
                let _ = child.wait();
                return Err(error);
            }
        };
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

    /// Set a bounded CSS viewport for responsive UI inspection.
    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        if !(240..=3840).contains(&width) || !(240..=2160).contains(&height) {
            return Err("viewport must be 240–3840 wide and 240–2160 high".into());
        }
        self.session.call(
            "Emulation.setDeviceMetricsOverride",
            serde_json::json!({"width":width,"height":height,"deviceScaleFactor":1,"mobile":false}),
            self.limits.command_timeout,
        )?;
        Ok(())
    }

    /// Dispatch a keyboard shortcut through Chrome's input pipeline.
    /// Modifiers use CDP bits: Alt=1, Control=2, Meta=4, Shift=8.
    pub fn press_key(&mut self, key: &str, modifiers: u8) -> Result<(), String> {
        let code = match key {
            "Enter" => 13,
            "Escape" => 27,
            "Tab" => 9,
            "ArrowLeft" => 37,
            "ArrowUp" => 38,
            "ArrowRight" => 39,
            "ArrowDown" => 40,
            "Home" => 36,
            "End" => 35,
            "/" | "?" => 191,
            value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphabetic() => {
                u32::from(value.as_bytes()[0].to_ascii_uppercase())
            }
            _ => return Err("unsupported shortcut key".into()),
        };
        if modifiers > 15 {
            return Err("invalid keyboard modifiers".into());
        }
        for kind in ["rawKeyDown", "keyUp"] {
            self.session.call(
                "Input.dispatchKeyEvent",
                serde_json::json!({"type":kind,"key":key,"windowsVirtualKeyCode":code,"modifiers":modifiers}),
                self.limits.command_timeout,
            )?;
        }
        Ok(())
    }

    /// Emulate the operating system's reduced-motion preference for UI QA.
    pub fn set_reduced_motion(&mut self, reduce: bool) -> Result<(), String> {
        self.session.call(
            "Emulation.setEmulatedMedia",
            serde_json::json!({"features":[{"name":"prefers-reduced-motion","value":if reduce {"reduce"} else {"no-preference"}}]}),
            self.limits.command_timeout,
        )?;
        Ok(())
    }

    /// Allow downloads into an existing directory for controlled artifact QA.
    pub fn set_download_directory(&mut self, directory: &std::path::Path) -> Result<(), String> {
        if !directory.is_absolute() || !directory.is_dir() {
            return Err("download directory must be an existing absolute directory".into());
        }
        self.session.call(
            "Browser.setDownloadBehavior",
            serde_json::json!({"behavior":"allow","downloadPath":directory.to_string_lossy()}),
            self.limits.command_timeout,
        )?;
        Ok(())
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

    /// Navigate, correlate completion with this navigation, and return the
    /// document URL actually landed on (redirects included).
    ///
    /// Three gaps closed here (audit HIGH): the `Page.navigate` response
    /// `errorText` is evaluated instead of ignored; stale queued events are
    /// drained and completion is bound to our frame, so an old queued event
    /// can never satisfy a later wait; and the landed URL is returned so
    /// callers verify task-specific readiness instead of assuming the
    /// request URL. Both waits share one deadline, preserving the previous
    /// worst-case timing.
    pub fn navigate(&mut self, url: &str) -> Result<String, String> {
        self.session.drain_event("Page.loadEventFired");
        self.session.drain_event("Page.frameStoppedLoading");
        let result = self.session.call(
            "Page.navigate",
            serde_json::json!({"url": url}),
            self.limits.command_timeout,
        )?;
        if let Some(error) = result.get("errorText").and_then(Value::as_str) {
            return Err(format!("navigation failed: {error}"));
        }
        let frame = result
            .get("frameId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let deadline = Instant::now() + self.limits.command_timeout;
        self.session
            .wait_event_matching("Page.frameStoppedLoading", "frameId", frame, deadline)
            .map_err(|e| format!("navigation frame wait failed: {e}"))?;
        // Freshness-checked secondary: drained above, so any load event now
        // belongs to this navigation. (Some Chrome builds omit loaderId.)
        self.session
            .wait_event_matching("Page.loadEventFired", "", "", deadline)
            .map_err(|e| format!("navigation load wait failed: {e}"))?;
        let landed = self.evaluate_value("document.URL")?;
        landed
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "landed URL is not a string".to_string())
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
        let handle = std::mem::replace(&mut self.handle, Handle::Attached);
        if let Handle::Owned {
            mut child,
            mut profile_dir,
        } = handle
        {
            terminate_tree(&mut child);
            let _ = child.wait();
            if let Some(dir) = profile_dir.take() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

impl Drop for ControlledBrowser {
    fn drop(&mut self) {
        self.close();
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
