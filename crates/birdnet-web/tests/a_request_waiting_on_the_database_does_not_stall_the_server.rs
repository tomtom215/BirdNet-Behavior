//! A request waiting on the database does not stall the server.
//!
//! `AppState::with_db` blocks the thread it is called on until the writer lock
//! is free. Called from a handler, that thread is one of the runtime's
//! workers — four on a Raspberry Pi 4 — and every other request scheduled on
//! it waits too: a page, a script, a live socket's next frame. The writer
//! lock is the one the detection processor holds while it writes, so this is
//! the ordinary case on a busy morning, not a contrived one.
//!
//! The static half of this is `tests/database_work_stays_off_the_async_runtime.rs`
//! in the root crate, which finds every such call in the source. This is the
//! dynamic half: one worker, the writer lock held, and a request for a
//! static script that must still be answered.

use std::sync::mpsc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Long enough that a loaded runner is not mistaken for a stall, short enough
/// that a real one fails the suite rather than hanging it.
const PATIENCE: Duration = Duration::from_secs(5);

#[test]
fn a_static_file_is_served_while_another_request_waits_for_the_database() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(tmp.path().join("birds.db")).expect("open state");
    let router = birdnet_web::routes::public_routes().with_state(state.clone());

    // One worker: whatever blocks it blocks the server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");

    // The detection processor's part, played by a thread holding the writer.
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        state.with_db(|_| {
            held_tx.send(()).expect("report held");
            release_rx.recv().ok();
        });
    });
    held_rx.recv_timeout(PATIENCE).expect("writer lock held");

    // Every wait below is on an OS thread, never on the runtime: with the one
    // worker blocked nothing drives tokio's timer, and a `tokio::time::timeout`
    // would hang rather than fail.
    //
    // `/stream` resolves its audio source from the database first.
    let (stream_tx, stream_rx) = mpsc::channel();
    let r = router.clone();
    rt.spawn(async move {
        let response = r
            .oneshot(Request::get("/stream").body(Body::empty()).unwrap())
            .await
            .expect("infallible");
        stream_tx.send(response.status()).ok();
    });
    // Give the single worker time to take that request to the lock.
    std::thread::sleep(Duration::from_millis(300));
    let early = stream_rx.try_recv();

    let (script_tx, script_rx) = mpsc::channel();
    rt.spawn(async move {
        let response = router
            .oneshot(
                Request::get("/static/htmx.min.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("infallible");
        script_tx.send(response.status()).ok();
    });
    let served = script_rx.recv_timeout(PATIENCE);

    // Let go before asserting, so a failure reports instead of leaving the
    // runtime unable to shut down.
    release_tx.send(()).expect("release");
    let answered = stream_rx.recv_timeout(PATIENCE);
    holder.join().expect("holder");
    drop(rt);

    assert!(
        early.is_err(),
        "precondition: /stream answered without waiting for the database, so \
         this test held nothing up"
    );
    assert_eq!(
        served,
        Ok(StatusCode::OK),
        "a static file was not served while another request waited for the database"
    );
    // The waiting request finishes once the writer lets go: a fresh station
    // has no audio source, which `/stream` answers with 503.
    assert_eq!(answered, Ok(StatusCode::SERVICE_UNAVAILABLE));
}
