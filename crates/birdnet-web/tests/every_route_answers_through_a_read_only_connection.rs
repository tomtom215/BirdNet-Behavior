//! Every page, partial and documented API read works on a **real** read-only
//! connection.
//!
//! Most of this crate's tests use an in-memory database, which has no second
//! connection to open, so `with_read_db` falls back to the writer and a write
//! left on a read path passes. `read_paths_against_real_readers.rs` closes that
//! for a hand-kept list of routes; the list stopped being a good idea when the
//! writer-lock sweep moved 117 more call sites. This builds the list instead: the
//! pages the visual-QA sweep covers (`tools/visual-qa/qa.mjs` `ROUTES`), the
//! `/pages/…` partials they load, and every parameterless `GET` in
//! `openapi.json`, on a file-backed station.
//!
//! A route fails if it answers with a server error, renders one of the error
//! states the handlers show when a query fails, or logs `readonly` at any level:
//! a write attempted on a reader is refused by SQLite with "attempt to write a
//! readonly database", which a handler may turn into a partial, a log line, or
//! (the case the other file records) nothing at all. The log is the third net.
//!
//! Its own test binary, because a tracing subscriber is process-global and the
//! callsite-interest cache makes a capture race the rest of a crate's tests.

use std::collections::BTreeSet;
use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::rate_limit::RateLimitConfig;
use birdnet_web::server::build_router_with_rate_limit;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<String>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The shapes a failed query takes on screen. Taken from the handlers, as in
/// `read_paths_against_real_readers.rs`; see there for why these and not the
/// `bnb-load-error` class.
const ERROR_MARKERS: &[&str] = &[
    "class='error'",
    "class=\"error\"",
    "Error loading",
    "Failed to load",
    "error-state",
    "We couldn't load",
    "readonly database",
];

fn qa_routes() -> BTreeSet<String> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/visual-qa/qa.mjs");
    let src = std::fs::read_to_string(path).expect("qa.mjs is readable");
    let table = src
        .split_once("export const ROUTES = [")
        .expect("qa.mjs exports a ROUTES table")
        .1
        .split_once("\n];")
        .expect("ROUTES table is closed")
        .0;
    let mut out = BTreeSet::new();
    for line in table.lines().filter(|l| !l.trim_start().starts_with("//")) {
        for quote in ['\'', '`'] {
            let mut rest = line;
            while let Some(start) = rest.find(quote) {
                rest = &rest[start + 1..];
                let Some(end) = rest.find(quote) else { break };
                let candidate = &rest[..end];
                if candidate.starts_with('/') && !candidate.contains("${") {
                    out.insert(candidate.to_owned());
                }
                rest = &rest[end + 1..];
            }
        }
    }
    out
}

fn openapi_reads() -> BTreeSet<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("openapi.json");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("openapi.json")).unwrap();
    doc["paths"]
        .as_object()
        .expect("paths")
        .iter()
        .filter(|(p, v)| v.get("get").is_some() && !p.contains('{'))
        // The document's paths are relative to its `/api/v2` server.
        .map(|(p, _)| format!("/api/v2{p}"))
        .collect()
}

fn partials(html: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = html;
    while let Some(i) = rest.find("hx-get=\"") {
        rest = &rest[i + "hx-get=\"".len()..];
        let Some(end) = rest.find('"') else { break };
        let raw = rest[..end].replace("&amp;", "&");
        rest = &rest[end..];
        if raw.starts_with("/pages/") && !raw.contains('{') {
            out.insert(raw);
        }
    }
    out
}

fn station(dir: &std::path::Path) -> AppState {
    let state = AppState::new(dir.join("birds.db")).expect("open");
    state.with_db(|conn| {
        for (day, offset) in [("2026-03-11", 0), ("2026-03-12", 40)] {
            for i in 0..40 {
                conn.execute(
                    "INSERT INTO detections
                         (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        day,
                        format!("{:02}:{:02}:00", 5 + i / 6, (i * 7) % 60),
                        if i % 3 == 0 {
                            "Turdus merula"
                        } else {
                            "Parus major"
                        },
                        if i % 3 == 0 {
                            "Eurasian Blackbird"
                        } else {
                            "Great Tit"
                        },
                        0.55 + f64::from(i % 40) / 100.0,
                        format!("clip-{}.wav", offset + i),
                    ],
                )
                .expect("seed");
            }
        }
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES (date('now','localtime'), '05:15:00', 'Sylvia atricapilla',
                     'Eurasian Blackcap', 0.83, 'today.wav')",
            [],
        )
        .expect("seed today");
    });
    state
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap_or_default();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Routes that are not reads of the database, or that need a live thing this
/// fixture does not have (a stream, a clip on disk, the network).
const SKIP: &[&str] = &[
    "/api/v2/ws/",
    "/api/v2/species/image/",
    "/api/v2/recordings/",
    "/api/v2/spectrogram/",
    "/admin/system/backup/full",
    "/admin/support-bundle",
    "/admin/logs/stream",
    "/r/",
    "/help/",
    "/stream",
];

#[tokio::test]
async fn every_route_answers_through_a_read_only_connection() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("the only subscriber in this binary");

    let dir = tempfile::tempdir().unwrap();
    let state = station(dir.path());
    assert!(
        state.reader_count() > 0,
        "this file proves nothing without real read-only connections"
    );
    let app = build_router_with_rate_limit(
        state,
        RateLimitConfig {
            requests_per_second: 1e9,
            burst_capacity: u32::MAX,
            ..RateLimitConfig::default()
        },
    );

    let mut queue: Vec<String> = qa_routes().into_iter().chain(openapi_reads()).collect();
    let seeded = queue.len();
    assert!(
        seeded > 80,
        "only {seeded} seed routes found; is the extraction broken?"
    );
    let mut seen = BTreeSet::new();
    let mut problems = Vec::new();
    let mut answered = 0_usize;
    while let Some(uri) = queue.pop() {
        if !seen.insert(uri.clone()) || SKIP.iter().any(|p| uri.starts_with(p)) {
            continue;
        }
        let (status, body) = get(&app, &uri).await;
        answered += 1;
        // 503 is a feature that is off here (analytics without DuckDB, a stream
        // that is not up); any other server error is a defect.
        if status.is_server_error() && status != StatusCode::SERVICE_UNAVAILABLE {
            problems.push(format!("{uri} answered {status}"));
        }
        for marker in ERROR_MARKERS {
            if body.contains(marker) {
                problems.push(format!("{uri} rendered an error state ({marker:?})"));
                break;
            }
        }
        if status.is_success() {
            queue.extend(partials(&body));
        }
    }
    let log = captured
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let readonly_lines: Vec<&str> = log
        .lines()
        .filter(|l| l.to_ascii_lowercase().contains("readonly"))
        .collect();

    assert!(answered > 120, "only {answered} routes were requested");
    assert!(
        problems.is_empty(),
        "{} route(s) failed on a real read-only connection:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
    assert!(
        readonly_lines.is_empty(),
        "something tried to write on a reader and logged it:\n  {}",
        readonly_lines.join("\n  ")
    );
}
