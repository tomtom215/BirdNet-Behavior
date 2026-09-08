//! A configuration file with an error no longer stops the station (LC-6), on
//! the real binary.
//!
//! Validation runs inside the new process after systemd has stopped the old
//! one, so a bad edit used to be a restart loop with no web UI and no way
//! back. Three starts on one file: a good one, which keeps a last-good copy;
//! a broken one, which must come up on the copy and say `config_reverted`;
//! and a broken one with the copy removed, which must still come up, web-only,
//! and say `config_rejected`.

use std::io::{Read, Write};
use std::net::TcpStream;
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
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

fn get(port: u16, path: &str) -> Option<(String, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let (head, body) = raw.split_once("\r\n\r\n")?;
    Some((head.to_owned(), body.to_owned()))
}

/// Boot the binary and wait for it to answer, or report how it died.
fn boot(config_path: &std::path::Path, port: u16) -> Result<ChildGuard, String> {
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
    loop {
        if let Some(status) = guard.0.try_wait().expect("try_wait") {
            return Err(format!("server exited during startup with {status}"));
        }
        if let Some((head, _)) = get(port, "/api/v2/health")
            && head.starts_with("HTTP/1.1 200")
        {
            return Ok(guard);
        }
        assert!(
            Instant::now() < deadline,
            "server did not come up within 45s"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[test]
fn a_file_with_an_error_starts_on_the_last_good_copy_or_web_only_and_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("birdnet.conf");
    let db_path = dir.path().join("birds.db");
    let good = format!(
        "SITENAME=Config Probe\nDB_PATH={}\nLATITUDE=42.36\nLONGITUDE=-71.06\n",
        db_path.display()
    );
    std::fs::write(&config_path, &good).unwrap();

    let port = free_port();
    {
        let _first = boot(&config_path, port).expect("a good file starts");
        let (_, body) = get(port, "/api/v2/health").expect("health");
        assert!(body.contains(r#""boot_anomalies":[]"#), "{body}");
    }
    let last_good = dir.path().join("birdnet.conf.last-good");
    assert!(
        last_good.exists(),
        "a successful start must keep a last-good copy beside the file"
    );

    // The edit over SSH: a latitude that is not a number.
    std::fs::write(&config_path, good.replace("LATITUDE=42.36", "LATITUDE=abc")).unwrap();

    let port = free_port();
    {
        let _second = boot(&config_path, port)
            .expect("a file with an error must not stop the station: it has a last-good copy");
        let (_, body) = get(port, "/api/v2/health").expect("health");
        assert!(
            body.contains(r#""boot_anomalies":["config_reverted"]"#),
            "the station must say it is running on the last good file: {body}"
        );
    }

    std::fs::remove_file(&last_good).unwrap();
    let port = free_port();
    let _third = boot(&config_path, port)
        .expect("a file with an error and no copy must still come up, web-only");
    let (_, body) = get(port, "/api/v2/health").expect("health");
    assert!(
        body.contains(r#""boot_anomalies":["config_rejected"]"#),
        "the station must say it rejected the file: {body}"
    );
    assert!(
        !last_good.exists(),
        "a rejected start must not make its file the last good one"
    );
}
