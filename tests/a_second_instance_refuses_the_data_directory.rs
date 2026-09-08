//! One process per data directory (DD-25).
//!
//! A second instance started on a data directory another process is using
//! opened the same files, quarantined the first process's live analytics
//! store, and only then died on `AddrInUse`. It now takes an advisory lock on
//! `birdnet.lock` beside the database before it opens anything, waits the
//! shutdown grace for the first to go, and refuses with a message that says
//! why. The first keeps serving throughout.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_birdnet-behavior");

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn health(port: u16) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .ok()?;
    stream
        .write_all(b"GET /api/v2/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
    buf.lines().next().map(str::to_owned)
}

fn spawn(config_path: &std::path::Path, port: u16, grace_secs: &str) -> Child {
    Command::new(BIN)
        .args([
            "--web-only",
            "--config",
            config_path.to_str().unwrap(),
            "--listen",
            &format!("127.0.0.1:{port}"),
        ])
        .env("RUST_LOG", "warn")
        .env_remove("BIRDNET_CONFIG")
        .env("BNB_INSTANCE_LOCK_GRACE_SECS", grace_secs)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {BIN}: {e}"))
}

#[test]
fn a_second_instance_waits_the_grace_then_refuses_and_the_first_keeps_serving() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("birdnet.conf");
    std::fs::write(
        &config_path,
        format!(
            "DB_PATH={}\nCADDY_PWD=\n",
            dir.path().join("birds.db").display()
        ),
    )
    .unwrap();

    let first_port = free_port();
    let mut first = ChildGuard(spawn(&config_path, first_port, "2"));
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = first.0.try_wait().unwrap() {
            panic!("the first instance exited during startup with {status}");
        }
        if health(first_port).is_some_and(|l| l.contains("200")) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the first instance did not come up"
        );
        std::thread::sleep(Duration::from_millis(300));
    }

    // A second instance on the same directory, on another port so the bind
    // cannot be what stops it.
    let second_port = free_port();
    let started = Instant::now();
    let mut second = spawn(&config_path, second_port, "2");
    let status = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(40),
            "the second instance neither exited nor was refused"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    let mut stderr = String::new();
    second
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(
        !status.success(),
        "the second instance must refuse to start: {stderr}"
    );
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "it waited the grace before refusing ({:?})",
        started.elapsed()
    );
    assert!(
        stderr.contains("another instance is running"),
        "the refusal says why: {stderr}"
    );
    assert!(
        health(second_port).is_none(),
        "the second instance never bound its port"
    );

    // The first is untouched: still serving, nothing quarantined beside it.
    assert!(health(first_port).is_some_and(|l| l.contains("200")));
    let quarantined: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".corrupt."))
        .collect();
    assert!(quarantined.is_empty(), "{quarantined:?}");
    drop(first);
}
