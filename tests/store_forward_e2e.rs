//! Store-and-forward delivery, end to end against the REAL binary.
//!
//! The outage half of the loop is proven elsewhere (unit tests on the queue
//! store; a live station with a bad token parking uploads). This test proves
//! the half that needs a willing receiver: a pre-seeded backlog is replayed
//! by the real drainer inside the real compiled binary, delivered to a local
//! stub `BirdWeather` server **oldest-first**, acknowledged with `200`, and
//! dequeued — leaving the queue empty. For a researcher whose observation
//! data is irreplaceable, "the backlog actually drains after an outage" is
//! the property that matters; this pins it against the production binary,
//! not a test double of the drainer.
//!
//! The binary is pointed at the stub via `BIRDNET_BIRDWEATHER_URL` — the
//! same self-hosted-ingest override a sensitive-species programme would use
//! in production, so the hook itself is exercised too.
//!
//! No fixed sleeps for synchronisation: readiness and completion are polled
//! with a generous deadline and a tight step (CONTRIBUTING.md).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
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

/// One captured request: path + body, in arrival order.
type Captured = Arc<Mutex<Vec<(String, String)>>>;

/// Minimal HTTP/1.1 stub standing in for `app.birdweather.com`: records each
/// POST and acknowledges it the way the real API does (2xx + success JSON).
/// Hand-rolled on `std::net` like the rest of this file's harness — the test
/// must not drag an HTTP framework into proving the production loop.
fn spawn_stub_birdweather() -> (u16, Captured) {
    spawn_stub_with_response(
        "HTTP/1.1 201 Created\r\n\
         Content-Type: application/json\r\n\
         Content-Length: 16\r\n\
         Connection: close\r\n\r\n\
         {\"success\":true}",
    )
}

/// The unhappy sibling: an ingest that is reachable but refusing — every
/// request is recorded and answered with a `500`, the way an overloaded or
/// misconfigured upstream behaves.
fn spawn_stub_refusing() -> (u16, Captured) {
    spawn_stub_with_response(
        "HTTP/1.1 500 Internal Server Error\r\n\
         Content-Length: 0\r\n\
         Connection: close\r\n\r\n",
    )
}

fn spawn_stub_with_response(response: &'static str) -> (u16, Captured) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let port = listener.local_addr().expect("stub addr").port();
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut reader = BufReader::new(stream);

            // Request line + headers; the only header we act on is
            // Content-Length (reqwest sends sized JSON bodies, not chunked).
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let path = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let mut content_length = 0_usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                    break;
                }
                if let Some(v) = line
                    .to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                {
                    content_length = v.parse().unwrap_or(0);
                }
            }
            let mut body = vec![0_u8; content_length];
            if reader.read_exact(&mut body).is_err() {
                continue;
            }
            if let Ok(mut log) = sink.lock() {
                log.push((path, String::from_utf8_lossy(&body).into_owned()));
            }

            let _ = reader.into_inner().write_all(response.as_bytes());
        }
    });

    (port, captured)
}

/// Poll `f` until it returns `Some` or the deadline passes — handshake-style
/// synchronisation instead of fixed sleeps.
fn wait_for<T>(deadline: Duration, step: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if start.elapsed() > deadline {
            return None;
        }
        std::thread::sleep(step);
    }
}

/// Migrate a fresh DB at `db_path` and park three payloads, as if posted
/// during an outage. The scientific names encode the enqueue order so
/// delivery order is checkable.
const SEEDED_SPECIES: [&str; 3] = ["Pica pica", "Turdus merula", "Erithacus rubecula"];

fn seed_backlog(db_path: &std::path::Path) {
    let conn = rusqlite::Connection::open(db_path).expect("open db");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for (i, species) in SEEDED_SPECIES.iter().enumerate() {
        let payload = serde_json::json!({
            "timestamp": format!("2026-06-10T0{i}:00:00Z"),
            "common_name": format!("Bird {i}"),
            "scientific_name": species,
            "confidence": 0.9,
            "lat": 51.0,
            "lon": -0.1,
        });
        birdnet_db::outbound_queue::enqueue(&conn, "birdweather", &payload.to_string(), 0)
            .expect("enqueue");
    }
    assert_eq!(
        birdnet_db::outbound_queue::depth(&conn, "birdweather").expect("depth"),
        3
    );
}

/// Boot the real binary in `--web-only` with its uploads pointed at the stub.
fn spawn_station(dir: &std::path::Path, db_path: &std::path::Path, stub_port: u16) -> ChildGuard {
    let config_path = dir.join("birdnet.conf");
    std::fs::write(
        &config_path,
        format!(
            "DB_PATH={}\nBIRDWEATHER_TOKEN=e2e-station-token\n",
            db_path.display()
        ),
    )
    .expect("write config");

    let child = Command::new(BIN)
        .args([
            "--web-only",
            "--config",
            config_path.to_str().expect("utf-8 path"),
            "--listen",
            "127.0.0.1:0",
        ])
        .env("RUST_LOG", "warn")
        .env_remove("BIRDNET_CONFIG")
        .env(
            "BIRDNET_BIRDWEATHER_URL",
            format!("http://127.0.0.1:{stub_port}/api/v1"),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {BIN}: {e}"));
    ChildGuard(child)
}

#[test]
fn queued_uploads_replay_in_order_and_dequeue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("birds.db");
    seed_backlog(&db_path);

    let (stub_port, captured) = spawn_stub_birdweather();
    let _guard = spawn_station(dir.path(), &db_path, stub_port);

    // The drainer's first cycle fires immediately on startup; three replays
    // at 200 ms spacing should land in a couple of seconds. The generous
    // deadline covers a cold, loaded CI runner.
    let drained = wait_for(Duration::from_secs(60), Duration::from_millis(200), || {
        let conn = rusqlite::Connection::open(&db_path).ok()?;
        let depth = birdnet_db::outbound_queue::depth(&conn, "birdweather").ok()?;
        (depth == 0).then_some(())
    });
    assert!(
        drained.is_some(),
        "queue should drain to zero once the endpoint accepts uploads"
    );

    let requests = captured.lock().expect("stub log").clone();
    assert_eq!(
        requests.len(),
        3,
        "every queued payload delivered exactly once"
    );

    // The station token from the config must appear in the path — the
    // override changes the HOST, never the API shape.
    for (path, _) in &requests {
        assert_eq!(path, "/api/v1/stations/e2e-station-token/detections");
    }

    // Oldest-first: upload order survives the outage, so the upstream
    // record's sequence matches what actually happened in the field. The
    // wire body uses BirdWeather's camelCase field names — the replay goes
    // through the real `post_detection`, not a raw payload forward.
    let names: Vec<String> = requests
        .iter()
        .map(|(_, body)| {
            serde_json::from_str::<serde_json::Value>(body).expect("valid JSON body")
                ["scientificName"]
                .as_str()
                .expect("scientificName present in wire body")
                .to_owned()
        })
        .collect();
    assert_eq!(names, SEEDED_SPECIES);
}

/// The other disposition, against the same real binary: an ingest that is
/// reachable but refusing (5xx) must leave the backlog INTACT — the failed
/// replay is recorded (attempt counter, backoff armed) and nothing is
/// dequeued. For irreplaceable observation data, "a broken upstream cannot
/// destroy the backlog" is as load-bearing as "a healthy upstream drains it".
#[test]
fn refusing_endpoint_keeps_backlog_and_records_attempt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("birds.db");
    seed_backlog(&db_path);

    let (stub_port, captured) = spawn_stub_refusing();
    let _guard = spawn_station(dir.path(), &db_path, stub_port);

    // The first drain cycle fires at startup, fails on the oldest entry
    // (after the client's own in-flight retries), and stops — recording one
    // replay attempt. Wait for that attempt to land in the queue row.
    let attempted = wait_for(Duration::from_secs(60), Duration::from_millis(200), || {
        let conn = rusqlite::Connection::open(&db_path).ok()?;
        let attempts: i64 = conn
            .query_row(
                "SELECT MAX(attempts) FROM outbound_queue WHERE kind = 'birdweather'",
                [],
                |row| row.get(0),
            )
            .ok()?;
        (attempts >= 1).then_some(())
    });
    assert!(
        attempted.is_some(),
        "the failed replay must be recorded on the queue row"
    );

    let conn = rusqlite::Connection::open(&db_path).expect("open db");
    assert_eq!(
        birdnet_db::outbound_queue::depth(&conn, "birdweather").expect("depth"),
        3,
        "a refusing upstream must not cost a single queued payload"
    );
    // The cycle stops at the first failure: the rest of the batch is never
    // hammered against an endpoint that is evidently down.
    let untouched: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM outbound_queue WHERE attempts = 0",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(untouched, 2, "only the oldest entry was attempted");
    assert!(
        !captured.lock().expect("stub log").is_empty(),
        "the refusal really did come from the stub, not a connect failure"
    );
}
