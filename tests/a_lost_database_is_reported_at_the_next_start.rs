//! A start that finds no database where the last start found a season says so
//! (UP-3), on the real binary.
//!
//! A data volume that fails to mount leaves the station starting on the empty
//! directory beneath, which used to look exactly like a first run: a fresh
//! database, an empty chart, and no line anywhere saying that forty thousand
//! detections are on a card that is not mounted. The boot journal is kept in
//! the configuration directory, outside the volume, so the second start can
//! compare itself to the first.
//!
//! Two boots of the shipped binary on one configuration directory: the first
//! with a database holding rows, the second after the database has been
//! removed — which is what the directory beneath an unmounted volume holds.

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

fn boot(config_path: &std::path::Path, port: u16) -> ChildGuard {
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
        if let Some((head, _)) = get(port, "/api/v2/health")
            && head.starts_with("HTTP/1.1 200")
        {
            return guard;
        }
        assert!(
            Instant::now() < deadline,
            "server did not come up within 45s"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[test]
fn a_database_that_held_detections_and_is_gone_is_reported_at_the_next_start() {
    let dir = tempfile::tempdir().expect("tempdir");
    let etc = dir.path().join("etc");
    let data = dir.path().join("data");
    std::fs::create_dir_all(&etc).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let config_path = etc.join("birdnet.conf");
    let db_path = data.join("birds.db");
    std::fs::write(
        &config_path,
        format!("SITENAME=Journal Probe\nDB_PATH={}\n", db_path.display()),
    )
    .unwrap();

    // A season's worth, or three rows: the journal records that there were some.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        for time in ["06:15:00", "07:15:00", "08:15:00"] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
                 VALUES ('2026-05-19', ?1, 'Turdus merula', 'Eurasian Blackbird', 0.85)",
                [time],
            )
            .unwrap();
        }
    }

    // `--web-only` runs no detection daemon, which is itself a strict fault, so
    // the status under `?strict=1` says nothing here; the web crate's own gate
    // covers the folding. This is about what the two starts *report*.
    let port = free_port();
    {
        let _first = boot(&config_path, port);
        let (_, body) = get(port, "/api/v2/health").expect("health");
        assert!(
            body.contains(r#""boot_anomalies":[]"#),
            "a first start has nothing to compare to: {body}"
        );
    }
    assert!(
        etc.join("boot-journal.json").exists(),
        "the journal must be in the configuration directory, outside the data volume"
    );

    // The volume did not mount: the directory beneath it holds nothing.
    for name in ["birds.db", "birds.db-wal", "birds.db-shm"] {
        let _ = std::fs::remove_file(data.join(name));
    }

    let port = free_port();
    let _second = boot(&config_path, port);
    let (head, body) = get(port, "/api/v2/health").expect("health");
    assert!(
        body.contains(r#""boot_anomalies":["db_lost"]"#),
        "the lost database must be named: {body}"
    );
    assert!(
        head.starts_with("HTTP/1.1 200"),
        "not strict: the container supervisor must not restart it: {head}"
    );
}
