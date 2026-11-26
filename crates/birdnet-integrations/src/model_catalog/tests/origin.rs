//! A loopback HTTP origin that misbehaves on cue.
//!
//! A real server, because the properties under test are what happens to the
//! bytes on disk when a connection stalls, drops or lies — which an
//! unreachable host never exercises. Each connection takes the next step of a
//! script (the last step repeats), and every request's `Range` is logged so a
//! test can say what was asked for.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What one connection does.
#[derive(Debug, Clone)]
pub enum Step {
    /// Serve the body, honouring a `Range` request with a 206.
    Serve(Vec<u8>),
    /// Serve the whole body with a 200 whatever was asked for.
    ServeIgnoringRange(Vec<u8>),
    /// Send the headers for the body (or the asked-for range of it), then
    /// this many bytes of it, then close the connection.
    DropAfter(Vec<u8>, usize),
    /// As `DropAfter`, but hold the connection open, sending nothing, for
    /// this long instead of closing it.
    StallAfter(Vec<u8>, usize, Duration),
    /// Answer with this status and an empty body.
    Status(u16),
    /// Send a body with no `Content-Length` that never ends (64 KiB blocks
    /// of 7s, up to this many).
    Endless(usize),
}

/// One request as the origin saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// The first byte asked for, when the request carried a `Range`.
    pub range_from: Option<u64>,
}

/// A running origin.
#[derive(Debug, Clone)]
pub struct Origin {
    /// Its URL (`http://127.0.0.1:<port>/model.onnx`).
    pub url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Origin {
    /// Start one, running `script`.
    pub fn start(script: Vec<Step>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            let mut n = 0;
            for conn in listener.incoming() {
                let Ok(sock) = conn else { continue };
                let step = script
                    .get(n)
                    .or_else(|| script.last())
                    .cloned()
                    .expect("a script with at least one step");
                n += 1;
                let log = Arc::clone(&log);
                std::thread::spawn(move || handle(sock, &step, &log));
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}/model.onnx"),
            seen,
        }
    }

    /// The requests made so far.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("log").clone()
    }
}

/// A URL nothing listens on: a port that was bound and released.
pub fn closed_port_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}/model.onnx")
}

fn read_request(sock: &mut TcpStream) -> Option<Seen> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if sock.read(&mut byte).ok()? == 0 {
            return None;
        }
        buf.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&buf).to_ascii_lowercase();
    let range_from = text
        .lines()
        .find_map(|l| l.strip_prefix("range: bytes="))
        .and_then(|r| r.split('-').next())
        .and_then(|n| n.trim().parse().ok());
    Some(Seen { range_from })
}

fn handle(mut sock: TcpStream, step: &Step, log: &Mutex<Vec<Seen>>) {
    let Some(seen) = read_request(&mut sock) else {
        return;
    };
    let from = seen.range_from;
    log.lock().expect("log").push(seen);
    let partial = |body: &[u8]| -> (String, Vec<u8>) {
        let total = body.len();
        match from {
            Some(start) if usize::try_from(start).is_ok_and(|s| s < total) => {
                let s = usize::try_from(start).expect("fits");
                (
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\n\
                         Content-Range: bytes {s}-{}/{total}\r\nConnection: close\r\n\r\n",
                        total - s,
                        total - 1
                    ),
                    body[s..].to_vec(),
                )
            }
            _ => (
                format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"),
                body.to_vec(),
            ),
        }
    };
    match step {
        Step::Serve(body) => {
            let (head, rest) = partial(body);
            let _ = sock.write_all(head.as_bytes());
            let _ = sock.write_all(&rest);
        }
        Step::ServeIgnoringRange(body) => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes());
            let _ = sock.write_all(body);
        }
        Step::DropAfter(body, n) => {
            let (head, rest) = partial(body);
            let _ = sock.write_all(head.as_bytes());
            let _ = sock.write_all(&rest[..(*n).min(rest.len())]);
            let _ = sock.flush();
            let _ = sock.shutdown(std::net::Shutdown::Both);
        }
        Step::StallAfter(body, n, hold) => {
            let (head, rest) = partial(body);
            let _ = sock.write_all(head.as_bytes());
            let _ = sock.write_all(&rest[..(*n).min(rest.len())]);
            let _ = sock.flush();
            std::thread::sleep(*hold);
        }
        Step::Status(code) => {
            let head =
                format!("HTTP/1.1 {code} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = sock.write_all(head.as_bytes());
        }
        Step::Endless(blocks) => {
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
            let block = vec![7u8; 64 * 1024];
            for _ in 0..*blocks {
                if sock.write_all(&block).is_err() {
                    break;
                }
            }
        }
    }
    let _ = sock.flush();
}
