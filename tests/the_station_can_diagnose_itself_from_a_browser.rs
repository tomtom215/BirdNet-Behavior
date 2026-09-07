//! `OP-1`, end to end: boot the real binary and fetch its own diagnostic and
//! support bundle over HTTP.
//!
//! `crates/birdnet-web/tests/the_diagnostics_are_reachable_from_the_browser.rs`
//! drives the routes with stand-in hooks. This file is the half that cannot
//! be faked: that `app::run` actually installs the hooks, that the doctor the
//! page runs is the doctor (`checks` non-empty, the summary present), and that
//! the download is a real gzip'd tar with the members `--support-bundle`
//! writes. Same launch shape as `boot_smoke.rs`.
//!
//! Observed failing against the shipped binary: both requests answered
//! `HTTP/1.1 404 Not Found`.

use std::io::{Read, Write};
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

/// `GET path` over a fresh connection: the status line, the headers, and the
/// body bytes. `None` while the server is not up yet.
fn http_get(port: u16, path: &str) -> Option<(String, String, Vec<u8>)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .ok()?;
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let status = head.lines().next()?.to_owned();
    Some((status, head, buf[split + 4..].to_vec()))
}

/// Chunked transfer encoding is what axum uses for a body of unknown length;
/// undo it so the bytes can be handed to `tar`.
fn dechunk(head: &str, body: &[u8]) -> Vec<u8> {
    if !head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return body.to_vec();
    }
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(nl) = rest.windows(2).position(|w| w == b"\r\n") {
        let size =
            usize::from_str_radix(std::str::from_utf8(&rest[..nl]).unwrap().trim(), 16).unwrap();
        if size == 0 {
            break;
        }
        out.extend_from_slice(&rest[nl + 2..nl + 2 + size]);
        rest = &rest[nl + 2 + size + 2..];
    }
    out
}

fn boot(config_path: &std::path::Path) -> (ChildGuard, u16) {
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
    loop {
        if let Some(status) = guard.0.try_wait().expect("try_wait") {
            panic!("server exited during startup with {status}");
        }
        if let Some((line, _, _)) = http_get(port, "/api/v2/health")
            && line.contains("200")
        {
            return (guard, port);
        }
        assert!(
            Instant::now() < deadline,
            "server did not come up within 45s"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[test]
fn the_real_binary_serves_its_doctor_and_its_support_bundle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("birdnet.conf");
    std::fs::write(
        &config_path,
        format!(
            "SITENAME=Browser Doctor\nLATITUDE=51.48\nLONGITUDE=-0.13\nDB_PATH={}\nCADDY_PWD=\n",
            dir.path().join("birds.db").display()
        ),
    )
    .unwrap();
    let (_guard, port) = boot(&config_path);

    // The doctor: the same document `--doctor-json` prints, with real checks.
    let (status, head, body) = http_get(port, "/admin/doctor.json").expect("doctor.json");
    assert!(status.contains("200"), "GET /admin/doctor.json: {status}");
    let json: serde_json::Value =
        serde_json::from_slice(&dechunk(&head, &body)).expect("doctor.json parses");
    let checks = json["checks"].as_array().expect("checks array");
    assert!(!checks.is_empty(), "no checks in {json}");
    assert!(
        checks
            .iter()
            .any(|c| c["name"].as_str() == Some("Station location")),
        "the station-location check the CLI runs is missing: {json}"
    );
    assert!(json["summary"]["exit_code"].is_number(), "{json}");

    // The bundle: a gzip'd tar carrying the members `--support-bundle` writes.
    let (status, head, body) = http_get(port, "/admin/support-bundle").expect("bundle");
    assert!(
        status.contains("200"),
        "GET /admin/support-bundle: {status}"
    );
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: application/gzip"),
        "{head}"
    );
    let archive = dechunk(&head, &body);
    assert_eq!(
        &archive[..2],
        b"\x1f\x8b",
        "not gzip: {:?}",
        &archive[..8.min(archive.len())]
    );
    let path = dir.path().join("downloaded.tar.gz");
    std::fs::write(&path, &archive).unwrap();
    let listing = Command::new("tar")
        .args(["tzf", path.to_str().unwrap()])
        .output()
        .expect("tar");
    let names = String::from_utf8_lossy(&listing.stdout);
    for member in [
        "doctor.json",
        "doctor.txt",
        "config.redacted",
        "version.txt",
    ] {
        assert!(
            names.contains(&format!("birdnet-support/{member}")),
            "{member} missing from the archive: {names}"
        );
    }

    // Nothing is left behind on the data partition.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".birdnet-support"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "scratch files left behind: {leftovers:?}"
    );
}
