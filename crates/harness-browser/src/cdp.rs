//! Chrome DevTools Protocol plumbing: plain-HTTP endpoints plus a
//! request/response session over the WebSocket client.

use crate::ws::WsClient;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(crate) fn http_request(
    addr: &str,
    method: &str,
    path: &str,
    timeout: Duration,
) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let socket: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| io::Error::other(format!("bad debugger address: {e}")))?;
    if !socket.ip().is_loopback() {
        return Err(io::Error::other("Debugger must be loopback"));
    }
    let mut stream = TcpStream::connect_timeout(&socket, timeout)?;
    stream.set_write_timeout(Some(timeout))?;
    stream.set_read_timeout(Some(timeout))?;
    // Chrome 150+ refuses HTTP/1.0 outright; speak 1.1 and frame the
    // body by Content-Length instead of waiting for close.
    let request = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut raw = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        if Instant::now() > deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "http timed out"));
        }
        stream.set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ))?;
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(io::Error::other("http closed before headers"));
            }
            Ok(count) => {
                raw.extend_from_slice(&chunk[..count]);
                if raw.len() > 16 * 1024 && !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                    return Err(io::Error::other("Debugger headers exceed 16 KiB"));
                }
                if let Some(position) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break position + 4;
                }
            }
            Err(e) => return Err(e),
        }
    };
    if header_end > 16 * 1024 {
        return Err(io::Error::other("Debugger headers exceed 16 KiB"));
    }
    let head = String::from_utf8_lossy(&raw[..header_end]).into_owned();
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return Err(io::Error::other(format!(
            "http refused: {}",
            head.lines().next().unwrap_or("")
        )));
    }
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .ok_or_else(|| io::Error::other("Missing or invalid Content-Length"))?;
    if length > 8 * 1024 * 1024 {
        return Err(io::Error::other("Debugger body exceeds 8 MiB"));
    }
    let mut body = raw[header_end..].to_vec();
    while body.len() < length {
        if Instant::now() > deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "http timed out"));
        }
        stream.set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ))?;
        match stream.read(&mut chunk) {
            Ok(0) => return Err(io::Error::other("http closed mid-body")),
            Ok(count) => body.extend_from_slice(&chunk[..count]),
            Err(e) => return Err(e),
        }
    }
    body.truncate(length);
    Ok(body)
}

#[derive(Clone, Debug)]
pub(crate) struct PageTarget {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) kind: String,
    pub(crate) ws_url: String,
}

pub(crate) fn list_targets(addr: &str, timeout: Duration) -> io::Result<Vec<PageTarget>> {
    let body = http_request(addr, "GET", "/json/list", timeout)?;
    let targets: Vec<Value> = serde_json::from_slice(&body)
        .map_err(|e| io::Error::other(format!("bad target list: {e}")))?;
    Ok(targets
        .into_iter()
        .filter_map(|target| {
            Some(PageTarget {
                id: target.get("id")?.as_str()?.into(),
                title: target.get("title")?.as_str().unwrap_or("").into(),
                url: target.get("url")?.as_str().unwrap_or("").into(),
                kind: target.get("type")?.as_str().unwrap_or("").into(),
                ws_url: target.get("webSocketDebuggerUrl")?.as_str()?.into(),
            })
        })
        .collect())
}

/// Split `ws://127.0.0.1:PORT/path` into socket address, path, and host.
pub(crate) fn split_ws_url(url: &str) -> io::Result<(String, String, String)> {
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| io::Error::other("debugger URL is not ws://"))?;
    let (addr, path) = match rest.find('/') {
        Some(index) => (rest[..index].to_string(), rest[index..].to_string()),
        None => (rest.to_string(), "/".to_string()),
    };
    let socket: std::net::SocketAddr = addr
        .parse()
        .map_err(|_| io::Error::other("Invalid debugger address"))?;
    if !socket.ip().is_loopback() || path.contains(['\r', '\n']) {
        return Err(io::Error::other("Debugger must use a loopback address"));
    }
    let host = addr.clone();
    Ok((addr, path, host))
}

pub(crate) struct CdpSession {
    ws: WsClient,
    next_id: u64,
    events: VecDeque<Value>,
    max_message_bytes: usize,
}

impl CdpSession {
    pub(crate) fn connect(
        ws_url: &str,
        timeout: Duration,
        max_message_bytes: usize,
    ) -> io::Result<Self> {
        let (addr, path, host) = split_ws_url(ws_url)?;
        Ok(Self {
            ws: WsClient::connect(&addr, &path, &host, timeout)?,
            next_id: 1,
            events: VecDeque::new(),
            max_message_bytes,
        })
    }

    /// Call a domain method and return its `result` object.
    pub(crate) fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let deadline = Instant::now() + timeout;
        let request = serde_json::json!({"id": id, "method": method, "params": params}).to_string();
        self.ws.send_text(&request).map_err(|e| e.to_string())?;
        loop {
            if Instant::now() > deadline {
                return Err(format!("{method} timed out"));
            }
            let text = self
                .ws
                .recv_text(deadline, self.max_message_bytes)
                .map_err(|e| e.to_string())?;
            let message: Value =
                serde_json::from_str(&text).map_err(|e| format!("bad CDP frame: {e}"))?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return Self::unwrap_response(id, &message);
            }
            if message.get("id").is_some() {
                return Err("Unexpected CDP response ID".into());
            }
            if message.get("method").is_some() {
                if self.events.len() >= 256 {
                    self.events.pop_front();
                }
                self.events.push_back(message);
            }
            // Domain events without interest here are dropped by the caller
            // pattern below; responses addressed to others are parked.
        }
    }

    fn unwrap_response(id: u64, message: &Value) -> Result<Value, String> {
        if let Some(error) = message.get("error") {
            return Err(format!("CDP error on #{id}: {error}"));
        }
        message
            .get("result")
            .cloned()
            .ok_or_else(|| format!("CDP response #{id} has no result"))
    }

    /// Drop queued events for `method`. Call before an operation whose
    /// completion you will wait on: a stale queued event (e.g. from a
    /// previous navigation) must never satisfy a later wait.
    pub(crate) fn drain_event(&mut self, method: &str) {
        self.events
            .retain(|e| e.get("method").and_then(Value::as_str) != Some(method));
    }

    /// Wait for `method` with identity correlation against `params[key]`.
    ///
    /// - `key = "frameId"` binds the wait to our frame (e.g.
    ///   `Page.frameStoppedLoading` after `Page.navigate`).
    /// - `key = "loaderId"` binds navigations where the event carries one.
    /// - `key = ""` accepts the first fresh event; only sound when the
    ///   caller drained stale events first (see [`drain_event`](Self::drain_event)).
    ///
    /// Non-matching events park back in the queue; the caller shares one
    /// absolute `deadline` across sequential waits so worst-case timing
    /// stays bounded. Note: some Chrome builds emit `Page.loadEventFired`
    /// with only a timestamp, so frame correlation is the primary signal
    /// and the load event is the freshness-checked secondary.
    pub(crate) fn wait_event_matching(
        &mut self,
        method: &str,
        key: &str,
        want: &str,
        deadline: Instant,
    ) -> Result<Value, String> {
        // Correlated check first; with an empty key any queued event for
        // the method satisfies (freshness is the caller's drain duty).
        if let Some(index) = self.events.iter().position(|e| {
            e.get("method").and_then(Value::as_str) == Some(method)
                && (key.is_empty()
                    || want.is_empty()
                    || e.get("params")
                        .and_then(|p| p.get(key))
                        .and_then(Value::as_str)
                        == Some(want))
        }) {
            return Ok(self.events.remove(index).unwrap()["params"].clone());
        }
        loop {
            if Instant::now() > deadline {
                return Err(format!("timed out waiting for {method}"));
            }
            let text = self
                .ws
                .recv_text(deadline, self.max_message_bytes)
                .map_err(|e| e.to_string())?;
            let message: Value =
                serde_json::from_str(&text).map_err(|e| format!("bad CDP frame: {e}"))?;
            if message.get("method").and_then(Value::as_str) == Some(method) {
                let correlated = key.is_empty()
                    || want.is_empty()
                    || message
                        .get("params")
                        .and_then(|p| p.get(key))
                        .and_then(Value::as_str)
                        == Some(want);
                if correlated {
                    return Ok(message.get("params").cloned().unwrap_or(Value::Null));
                }
            }
            if message.get("id").is_some() {
                return Err("Unexpected CDP response ID".into());
            }
            if message.get("method").is_some() {
                if self.events.len() >= 256 {
                    self.events.pop_front();
                }
                self.events.push_back(message);
            }
        }
    }

    pub(crate) fn close(&mut self) {
        let _ = self.ws.close();
    }
}
