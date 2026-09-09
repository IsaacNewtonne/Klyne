//! Minimal synchronous WebSocket client for CDP, dependency-free.
//!
//! Client role only, loopback only: masked text sends, fragmented receives,
//! ping/pong, and close handshake. The `Sec-WebSocket-Accept` hash is not
//! re-verified (that needs SHA-1); the connection is local either way, and
//! every subsequent message is JSON-validated by the CDP layer.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut block = [0u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let triple = (block[0] as u32) << 16 | (block[1] as u32) << 8 | block[2] as u32;
        out.push(B64[((triple >> 18) & 63) as usize] as char);
        out.push(B64[((triple >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((triple >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(triple & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

pub(crate) fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut values = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let value = B64
            .iter()
            .position(|digit| *digit == byte)
            .ok_or_else(|| format!("invalid base64 character '{}'", byte as char))?
            as u8;
        values.push(value);
    }
    let mut out = Vec::with_capacity(values.len() * 3 / 4);
    for chunk in values.chunks(4) {
        if chunk.len() < 2 {
            return Err("truncated base64 quantum".into());
        }
        let mut triple = (chunk[0] as u32) << 18 | (chunk[1] as u32) << 12;
        triple |= (chunk.get(2).copied().unwrap_or(0) as u32) << 6;
        triple |= chunk.get(3).copied().unwrap_or(0) as u32;
        out.push((triple >> 16) as u8);
        if chunk.len() > 2 {
            out.push((triple >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(triple as u8);
        }
    }
    Ok(out)
}

fn random_key() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut bytes = [0u8; 16];
    let mut state = nanos ^ (std::process::id() as u128) << 64;
    for slot in bytes.iter_mut() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *slot = (state >> 33) as u8;
    }
    base64_encode(&bytes)
}

fn read_http_response(stream: &mut TcpStream, deadline: Instant) -> io::Result<String> {
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if Instant::now() > deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "handshake timed out",
            ));
        }
        stream.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))?;
        match stream.read(&mut byte) {
            Ok(0) => return Err(io::Error::other("handshake closed early")),
            Ok(_) => {
                raw.push(byte[0]);
                if raw.ends_with(b"\r\n\r\n") {
                    return String::from_utf8(raw).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "handshake not UTF-8")
                    });
                }
            }
            Err(e) => return Err(e),
        }
    }
}

pub(crate) struct WsClient {
    stream: TcpStream,
}

impl WsClient {
    pub(crate) fn connect(
        addr: &str,
        path: &str,
        host: &str,
        timeout: Duration,
    ) -> io::Result<Self> {
        let deadline = Instant::now() + timeout;
        let stream = TcpStream::connect_timeout(
            &addr
                .parse::<std::net::SocketAddr>()
                .map_err(|e| io::Error::other(format!("bad debugger address: {e}")))?,
            timeout,
        )?;
        stream.set_nodelay(true)?;
        let mut client = Self { stream };
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            random_key()
        );
        client.stream.write_all(request.as_bytes())?;
        let response = read_http_response(&mut client.stream, deadline)?;
        if !response.starts_with("HTTP/1.1 101") {
            return Err(io::Error::other(format!(
                "websocket upgrade refused: {}",
                response.lines().next().unwrap_or("")
            )));
        }
        Ok(client)
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        let mut header = vec![0x80 | opcode];
        // Clients must mask; mask key derived per frame.
        let mask: [u8; 4] = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u32)
            .to_be_bytes();
        if payload.len() < 126 {
            header.push(0x80 | payload.len() as u8);
        } else if payload.len() < 65536 {
            header.push(0x80 | 126);
            header.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        } else {
            header.push(0x80 | 127);
            header.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
        header.extend_from_slice(&mask);
        self.stream.write_all(&header)?;
        for (index, byte) in payload.iter().enumerate() {
            self.stream.write_all(&[byte ^ mask[index % 4]])?;
        }
        self.stream.flush()?;
        Ok(())
    }

    pub(crate) fn send_text(&mut self, text: &str) -> io::Result<()> {
        self.send_frame(0x1, text.as_bytes())
    }

    fn send_pong(&mut self, payload: &[u8]) -> io::Result<()> {
        self.send_frame(0xA, payload)
    }

    pub(crate) fn close(&mut self) -> io::Result<()> {
        let _ = self.send_frame(0x8, &[]);
        Ok(())
    }

    fn read_exact_into(&mut self, buffer: &mut [u8], deadline: Instant) -> io::Result<()> {
        let mut filled = 0;
        while filled < buffer.len() {
            if Instant::now() > deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "read timed out"));
            }
            self.stream
                .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))?;
            match self.stream.read(&mut buffer[filled..]) {
                Ok(0) => return Err(io::Error::other("connection closed")),
                Ok(count) => filled += count,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Next complete text message, assembling fragments and answering
    /// pings. `max_bytes` bounds one message; larger ones fail closed.
    pub(crate) fn recv_text(&mut self, deadline: Instant, max_bytes: usize) -> io::Result<String> {
        let mut message = Vec::new();
        let mut fragmented = false;
        loop {
            if Instant::now() > deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "read timed out"));
            }
            let mut header = [0u8; 2];
            self.read_exact_into(&mut header, deadline)?;
            let finished = header[0] & 0x80 != 0;
            let opcode = header[0] & 0x0F;
            let masked = header[1] & 0x80 != 0;
            let mut length = (header[1] & 0x7F) as u64;
            if length == 126 {
                let mut extended = [0u8; 2];
                self.read_exact_into(&mut extended, deadline)?;
                length = u16::from_be_bytes(extended) as u64;
            } else if length == 127 {
                let mut extended = [0u8; 8];
                self.read_exact_into(&mut extended, deadline)?;
                length = u64::from_be_bytes(extended);
            }
            let mask = if masked {
                let mut key = [0u8; 4];
                self.read_exact_into(&mut key, deadline)?;
                Some(key)
            } else {
                None
            };
            if length > max_bytes as u64 {
                return Err(io::Error::other("message exceeds size bound"));
            }
            let mut payload = vec![0u8; length as usize];
            self.read_exact_into(&mut payload, deadline)?;
            if let Some(key) = mask {
                for (index, byte) in payload.iter_mut().enumerate() {
                    *byte ^= key[index % 4];
                }
            }
            match opcode {
                0x0 => {
                    if !fragmented {
                        return Err(io::Error::other("stray continuation frame"));
                    }
                    message.extend_from_slice(&payload);
                }
                0x1 | 0x2 => {
                    if fragmented {
                        return Err(io::Error::other("interleaved data frame"));
                    }
                    message.extend_from_slice(&payload);
                    fragmented = !finished;
                }
                0x8 => {
                    let _ = self.close();
                    return Err(io::Error::other("peer closed the connection"));
                }
                0x9 => {
                    self.send_pong(&payload)?;
                }
                0xA => {}
                _ => return Err(io::Error::other("unknown opcode")),
            }
            if finished && (opcode == 0x0 || opcode == 0x1 || opcode == 0x2) {
                if message.len() > max_bytes {
                    return Err(io::Error::other("message exceeds size bound"));
                }
                return String::from_utf8(message)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message not UTF-8"));
            }
        }
    }
}
