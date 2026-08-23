//! Every read path moved onto the pool, exercised against a **real** read-only
//! connection.
//!
//! # Why this file exists separately from the rest of the suite
//!
//! Almost every test in this crate builds its state with
//! `AppState::from_connection` on an in-memory database. An in-memory database
//! has no path to open a second connection to, so there is no reader pool and
//! `with_read_db` falls straight back to the writer. Those tests therefore say
//! nothing at all about whether a handler routed to `with_read_db` still works —
//! they would pass identically if the pool did not exist.
//!
//! That is the exact trap this repository keeps paying for: a gate satisfied for
//! a reason unrelated to what it claims. So this file builds a **file-backed**
//! state, which does get a pool, and drives each migrated route through the real
//! router. A write accidentally left on a read path fails here with `attempt to
//! write a readonly database` — which is the whole reason the pooled connections
//! are opened read-only rather than read-write.
//!
//! Add a route here whenever a handler moves from `with_db` to `with_read_db`.
//!
//! # What this does not catch
//!
//! A write whose error is deliberately discarded — `let _ = conn.execute(...)`
//! — leaves no trace in the response, so nothing here notices it. Checked
//! rather than assumed: planting exactly that in the `/api/v2/stats` handler
//! left all four tests green. What is caught is a write whose failure reaches
//! the response, which is every one of the migrated handlers as they are
//! written today, because each propagates its `DbError` into an error partial.
//! A future handler that swallows a write error is outside this gate's reach
//! and inside `db_pool`'s: the connection is read-only either way, so the write
//! does not happen — it just happens silently.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// A file-backed station with enough history that every partial has something
/// to render, and rows on more than one day so the date-scoped queries are not
/// answering trivially.
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
        // And today, so the Today home and its partials are not an empty state.
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

/// Every route whose handler now reads through the pool.
///
/// Grouped by the module the migration touched, so a future reader can tell at a
/// glance which file a failure belongs to.
const POOLED_ROUTES: &[(&str, &str)] = &[
    // pages/today.rs
    ("today", "/"),
    ("today", "/pages/today-list"),
    ("today", "/pages/today-daystrip"),
    ("today", "/pages/today-count"),
    ("today", "/pages/today-pills"),
    ("today", "/pages/today-nudge"),
    // pages/history.rs
    ("history", "/pages/history-calendar"),
    ("history", "/pages/history-chart"),
    ("history", "/pages/history-dates"),
    ("history", "/reports/day"),
    // pages/life_list.rs
    ("life_list", "/pages/life-accumulation"),
    // pages/dashboard/partials.rs
    ("dashboard", "/pages/detections"),
    ("dashboard", "/pages/best-detections"),
    ("dashboard", "/pages/top-species"),
    ("dashboard", "/pages/species-list"),
    ("dashboard", "/pages/hourly-chart"),
    ("dashboard", "/pages/daily-chart"),
    ("dashboard", "/pages/confidence-chart"),
    ("dashboard", "/pages/most-recent"),
    // pages/health.rs
    ("health", "/pages/health-badge"),
    // routes/system.rs
    ("system", "/api/v2/health"),
    ("system", "/api/v2/stats"),
];

/// The error partials the migrated handlers render when their query fails.
///
/// A read path that tries to write does not produce a 500 and does not echo
/// SQLite's `attempt to write a readonly database` — it produces one of these,
/// with a 200. Taken verbatim from the handlers, so a renamed message shows up
/// as a gate that stops discriminating rather than one that silently passes:
/// if you change a message below, change it here too.
const ERROR_MARKERS: &[&str] = &[
    "class='error'",
    "class=\"error\"",
    "Error loading",
    "Failed to load",
];

async fn get(state: &AppState, path: &str) -> (StatusCode, String) {
    let response = build_router(state.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .expect("response");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// The precondition. Without a pool this whole file degrades into a duplicate of
/// the in-memory tests, so say so loudly rather than passing quietly.
#[test]
fn the_fixture_actually_has_a_reader_pool() {
    let tmp = tempfile::tempdir().unwrap();
    let state = station(tmp.path());
    assert!(
        state.reader_count() > 0,
        "this file must run against real read-only connections, or it proves \
         nothing that the in-memory tests do not already prove"
    );
}

#[tokio::test]
async fn every_pooled_route_answers_through_a_read_only_connection() {
    let tmp = tempfile::tempdir().unwrap();
    let state = station(tmp.path());
    assert!(state.reader_count() > 0, "fixture must have a pool");

    for (module, path) in POOLED_ROUTES {
        let (status, body) = get(&state, path).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{module}: {path} did not answer ({status}): {}",
            body.chars().take(400).collect::<String>()
        );
        // Status alone is not enough, and this was established the hard way:
        // a write planted in the history-chart handler produced a **200** whose
        // body was `<p class='error'>Failed to load chart data.</p>`, and an
        // earlier version of this assertion — "the body must not mention
        // `readonly`" — passed. These handlers swallow their `DbError` into an
        // error partial rather than surfacing the SQLite message, so the thing
        // to look for is the partial.
        for marker in ERROR_MARKERS {
            assert!(
                !body.contains(marker),
                "{module}: {path} rendered an error partial ({marker:?}) — most \
                 likely a write left on the read path: {}",
                body.chars().take(400).collect::<String>()
            );
        }
    }
}

/// The counterpart. A pooled read must return the same bytes as the same handler
/// reading through the writer — otherwise "it answered" would be compatible with
/// "it answered wrongly", which is the failure mode a read-only connection does
/// *not* protect against.
#[tokio::test]
async fn a_pooled_route_renders_what_the_writer_would_have_rendered() {
    let tmp = tempfile::tempdir().unwrap();
    let state = station(tmp.path());

    // Same database, opened without a pool: `from_connection` is handed an
    // already-open writer, and its `:memory:` path means no readers.
    let unpooled = AppState::from_connection(
        birdnet_db::sqlite::open_or_create(&tmp.path().join("birds.db")).expect("open"),
        std::path::PathBuf::from(":memory:"),
    );
    assert_eq!(unpooled.reader_count(), 0, "control must have no pool");
    assert!(state.reader_count() > 0, "subject must have a pool");

    // Date-scoped and history-wide, so both a seek and a full aggregate are
    // compared.
    for path in [
        "/pages/history-calendar",
        "/pages/life-accumulation",
        "/pages/top-species",
        "/pages/daily-chart",
    ] {
        let (pooled_status, pooled_body) = get(&state, path).await;
        let (writer_status, writer_body) = get(&unpooled, path).await;
        assert_eq!(pooled_status, writer_status, "{path}: status differs");
        assert_eq!(
            pooled_body, writer_body,
            "{path}: the pooled reader rendered something different from the writer"
        );
    }
}

/// Writes still work, on the same state, after the reads have been moved. The
/// migration split call sites by hand across six files; this is the cheapest
/// possible check that none of the *write* paths went with them.
#[tokio::test]
async fn writes_still_reach_the_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let state = station(tmp.path());

    let before: i64 = state.with_read_db(|conn| {
        conn.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .expect("count")
    });

    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-03-13', '06:00:00', 'Erithacus rubecula', 'European Robin', 0.91)",
            [],
        )
        .expect("the writer must still write");
    });

    let after: i64 = state.with_read_db(|conn| {
        conn.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .expect("count")
    });
    assert_eq!(after, before + 1);

    // And the new row is visible through the router, not just through a raw
    // pooled read — which is what a stale reader would break.
    let (status, body) = get(&state, "/pages/history-dates").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("2026-03-13"),
        "a row written after the readers opened must be visible to them: {}",
        body.chars().take(400).collect::<String>()
    );
}
