//! Cancellation includes DNS, connecting, sending and streaming the response.
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
pub struct Response {
    pub status: u16,
    pub body: String,
}

pub fn send(
    build: impl FnOnce(reqwest::Client) -> reqwest::RequestBuilder,
    seconds: u64,
    cap: usize,
    stop: &AtomicBool,
) -> io::Result<Response> {
    if stop.load(Ordering::SeqCst) {
        return Err(io::Error::other("Request cancelled before dispatch"));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(seconds))
            .build()
            .map_err(io::Error::other)?;
        let mut operation = Box::pin(async {
            let mut response = build(client).send().await.map_err(|e| {
                if e.is_connect() {
                    io::Error::new(io::ErrorKind::ConnectionRefused,"Connection failed before request dispatch")
                } else { io::Error::other("Request failed; external outcome may be uncertain") }
            })?;
            let status = response.status().as_u16();
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                io::Error::other("Response interrupted; external outcome may be uncertain")
            })? {
                if bytes.len().saturating_add(chunk.len()) > cap {
                    return Err(io::Error::other(
                        "Response exceeded limit; external outcome may be uncertain",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(Response {
                status,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            })
        });
        loop {
            if stop.load(Ordering::SeqCst) {
                return Err(io::Error::other(
                    "Request cancelled; external outcome may be uncertain",
                ));
            }
            if let Ok(result) =
                tokio::time::timeout(Duration::from_millis(25), &mut operation).await
            {
                return result;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_interrupts_a_server_that_never_responds() {
        use std::{io::Read, net::TcpListener, sync::Arc};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 512];
            let _ = stream.read(&mut buf);
            signal.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(500));
        });
        let start = std::time::Instant::now();
        let result = send(|http| http.get(url), 20, 1024, &stop);
        assert!(result.err().unwrap().to_string().contains("cancelled"));
        assert!(start.elapsed() < Duration::from_millis(450));
        server.join().unwrap();
    }
}
