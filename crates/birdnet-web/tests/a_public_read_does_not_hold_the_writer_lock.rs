//! A public read goes through the pooled readers, never through the writer.
//!
//! The feeds, the bulk exports and the dashboard and migration insights are
//! public, and each scans the detection history: the rare-bird feed took 5.2 s
//! on a 300 000-detection station, the exports 2.7 to 4.3 s, the dashboard's KPI
//! block 2.4 s (measured against the real binary, debug build). They called
//! `with_db`, the single writer lock the detection processor and 245 other call
//! sites also take, so sixteen concurrent feed requests held all of them for 81 s
//! and `GET /api/v2/detections?limit=1` waited 38.9 s behind them (audit scenario
//! 2), and a single export stalled every detection write for its duration.
//!
//! This holds the writer lock from another thread and requests each route. A
//! route that reads through a pooled reader answers at once; one that wants the
//! writer waits for it, which is what a timeout here reports. The lock is
//! released before anything is asserted, so a failure ends the test instead of
//! hanging the runtime on the blocked task.
//!
//! The counterpart asks for the same routes with the lock free: they answer 200,
//! so "answers while the writer is held" cannot pass by a route being broken.
//! The list is the routes moved so far, not every public read: 245 call sites
//! still take the writer, and a route is added here when it is moved.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const ROUTES: [&str; 10] = [
    "/feeds/rare.rss",
    "/feeds/rare.ics",
    "/feeds/today.rss",
    "/api/v2/detections/export",
    "/api/v2/detections/export/birddb",
    "/api/v2/detections/export/raven",
    "/api/v2/detections/export/ebird",
    "/api/v2/dashboard/kpis",
    "/api/v2/insights/migration",
    "/pages/migration-stats",
];

async fn status(app: &axum::Router, path: &str, within: Duration) -> Option<StatusCode> {
    let req = Request::builder().uri(path).body(Body::empty()).unwrap();
    tokio::time::timeout(within, app.clone().oneshot(req))
        .await
        .ok()
        .map(|r| r.expect("router responds").status())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_public_read_answers_while_the_writer_lock_is_held() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    assert!(
        state.reader_count() > 0,
        "precondition: a file-backed station has pooled readers"
    );
    let app = build_router(state.clone());

    // Counterpart: the writer is free, every route answers.
    for path in ROUTES {
        assert_eq!(
            status(&app, path, Duration::from_secs(10)).await,
            Some(StatusCode::OK),
            "{path} with the writer free"
        );
    }

    // Hold the writer from another thread until told to let go.
    let (locked_tx, locked_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = {
        let state = state.clone();
        std::thread::spawn(move || {
            state.with_db(|_| {
                locked_tx.send(()).expect("signal");
                let _ = release_rx.recv();
            });
        })
    };
    locked_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the writer lock was taken");

    let mut outcomes = Vec::new();
    for path in ROUTES {
        outcomes.push((path, status(&app, path, Duration::from_secs(3)).await));
    }

    release_tx.send(()).expect("release");
    holder.join().expect("holder thread");

    for (path, outcome) in outcomes {
        assert_eq!(
            outcome,
            Some(StatusCode::OK),
            "{path} did not answer while the writer lock was held: it is waiting for the writer"
        );
    }
}
