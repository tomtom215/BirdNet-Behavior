//! The analytics store is a `DuckDB` database, and for a long time it was one
//! connection behind one `Mutex`: every analytics request, cheap or six seconds
//! long, queued behind whichever was running. A visitor to the retention page
//! held the status badge, the time-series charts and every analytics fragment.
//!
//! Reads now run on two further connections to the same database
//! (`AppState::with_analytics_read`), and the single connection is left to what
//! writes: mirroring a detection, a delete, the import-exclusion setting.
//!
//! This holds the writer connection from another thread and asks for analytics
//! reads. A route that reads on a reader answers; one that wants the writer
//! connection waits, which is what the timeout reports. The connection is
//! released before anything is asserted, so a failure ends the test instead of
//! hanging it. The counterpart asks for the same routes with nothing held, so
//! "answers while it is held" cannot pass by a route being broken, and a write
//! made through the writer connection must be visible to the reads afterwards.

// Needs the DuckDB analytics store: without the feature `AppState` has no
// analytics methods to call, and the file would not compile.
#![cfg(feature = "analytics")]

use std::sync::mpsc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const ROUTES: [&str; 3] = [
    "/api/v2/analytics/status",
    "/api/v2/timeseries/hourly",
    "/api/v2/timeseries/daily",
];

fn station(dir: &std::path::Path) -> AppState {
    let state = AppState::new_with_analytics(dir.join("birds.db"), &dir.join("analytics.duckdb"))
        .expect("state with analytics");
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-03-11', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'a.wav')",
            [],
        )
        .expect("seed");
    });
    state
}

async fn status(app: &axum::Router, path: &str, within: Duration) -> Option<StatusCode> {
    let req = Request::builder().uri(path).body(Body::empty()).unwrap();
    tokio::time::timeout(within, app.clone().oneshot(req))
        .await
        .ok()
        .map(|r| r.expect("router responds").status())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn analytics_reads_answer_while_the_writer_connection_is_held() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());
    assert!(
        state.with_analytics(|_| ()).is_some(),
        "precondition: the station has an analytics store"
    );
    let app = build_router(state.clone());

    // The counterpart first: with nothing held every route answers.
    for path in ROUTES {
        let got = status(&app, path, Duration::from_secs(10)).await;
        assert!(
            got.is_some_and(|s| s.as_u16() < 500),
            "{path} did not answer cleanly with nothing held: {got:?}"
        );
    }

    let (held_tx, held_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = {
        let state = state.clone();
        std::thread::spawn(move || {
            state.with_analytics(|_| {
                held_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(30));
            });
        })
    };
    held_rx.recv().unwrap();

    let mut results = Vec::new();
    for path in ROUTES {
        results.push((path, status(&app, path, Duration::from_secs(3)).await));
    }
    release_tx.send(()).ok();
    holder.join().unwrap();

    for (path, got) in results {
        assert!(
            got.is_some_and(|s| s.as_u16() < 500),
            "{path} waited for the analytics writer connection (None = no answer in 3 s): {got:?}"
        );
    }
}

#[tokio::test]
async fn a_read_sees_what_the_writer_connection_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());
    state
        .with_analytics(|adb| {
            adb.conn()
                .execute_batch("CREATE TABLE IF NOT EXISTS read_your_write (a INTEGER); INSERT INTO read_your_write VALUES (7);")
                .expect("write on the writer connection");
        })
        .expect("analytics store");
    let seen: i64 = state
        .with_analytics_read(|adb| {
            adb.conn()
                .query_row("SELECT a FROM read_your_write", [], |r| r.get(0))
                .expect("read on a reader")
        })
        .expect("analytics store");
    assert_eq!(
        seen, 7,
        "a reader must see what the writer connection committed"
    );
}
