//! Boot smoke test: spawn the real compiled binary in `--web-only` mode and
//! confirm it serves `GET /` with HTTP 200.
//!
//! The in-process router tests (`web_api_*.rs`) build an `AppState` and call
//! the router via `tower::ServiceExt::oneshot`, so they never exercise
//! `main()` / `app::run()` startup: clap wiring, config loading, the
//! migration-on-startup path, `AppState::new`, and the actual `axum::serve`
//! bind. A panic on that real startup path therefore slipped past CI once.
//! This test closes that gap by booting the binary as a subprocess.
//!
//! Cargo populates `CARGO_BIN_EXE_birdnet-behavior` with the binary path for
//! integration tests, so no extra dependencies are needed.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_birdnet-behavior");

/// Kill the child on drop so a failed assertion never leaks a server process.
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Grab an ephemeral free port by binding to :0 and releasing it. There is a
/// small TOCTOU window before the server re-binds, but it is good enough for a
/// single-process smoke test.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Issue `GET <path>` over a fresh connection and return the first response
/// line (e.g. `HTTP/1.1 200 OK`), or `None` if the server is not up yet.
fn http_status_line(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    text.lines().next().map(str::to_owned)
}

#[test]
fn web_only_boots_and_serves_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("birds.db");
    let config_path = dir.path().join("birdnet.conf");
    std::fs::write(
        &config_path,
        format!(
            "SITENAME=Smoke Test\nLATITUDE=51.48\nLONGITUDE=-0.13\nDB_PATH={}\n",
            db_path.display()
        ),
    )
    .expect("write config");

    let port = free_port();
    let child = Command::new(BIN)
        .args([
            "--web-only",
            "--config",
            config_path.to_str().unwrap(),
            "--listen",
            &format!("127.0.0.1:{port}"),
        ])
        .env("RUST_LOG", "warn")
        .env_remove("BIRDNET_CONFIG")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {BIN}: {e}"));
    let mut guard = ChildGuard(child);

    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last = String::from("(no response)");
    loop {
        // Fail fast if the process died (a boot panic) instead of waiting out
        // the full deadline.
        if let Some(status) = guard.0.try_wait().expect("try_wait") {
            panic!("server exited during startup with {status}");
        }
        if let Some(line) = http_status_line(port, "/") {
            last = line.clone();
            if line.contains("200") {
                return; // booted and serving — success
            }
        }
        if Instant::now() >= deadline {
            panic!("server did not serve GET / with 200 within 45s (last: {last})");
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}
