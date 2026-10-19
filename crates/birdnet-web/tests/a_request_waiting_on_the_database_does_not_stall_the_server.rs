//! A request waiting on the database does not stall the server.
//!
//! `AppState::with_db` blocks the thread it is called on until the writer lock
//! is free, and `with_read_db` until a pooled reader is. Called from a handler,
//! that thread is one of the runtime's workers — four on a Raspberry Pi 4 — and
//! every other request scheduled on it waits too: a page, a script, a live
//! socket's next frame. The writer lock is the one the detection processor holds
//! while it writes, and the readers are what every page shares, so this is the
//! ordinary case on a busy morning, not a contrived one.
//!
//! The static half of this is `tests/database_work_stays_off_the_async_runtime.rs`
//! in the root crate, which finds every such call in the source. This is the
//! dynamic half: one worker, every pooled reader held (`/stream` reads its audio
//! source through the pool since the writer-lock sweep; it waited for the writer
//! lock before), and a request for a static script that must still be answered.

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

    // Every pooled reader held by a thread of its own, the way a morning's page
    // views would hold them. (The detection processor's writer lock is not what
    // `/stream` waits for any more: it only reads.)
    let readers = state.reader_count();
    assert!(
        readers > 0,
        "precondition: a file-backed station has readers"
    );
    let (held_tx, held_rx) = mpsc::channel();
    let release = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let holders: Vec<_> = (0..readers)
        .map(|_| {
            let state = state.clone();
            let held_tx = held_tx.clone();
            let release = std::sync::Arc::clone(&release);
            std::thread::spawn(move || {
                state.with_read_db(|_| {
                    held_tx.send(()).expect("report held");
                    let (flag, cv) = &*release;
                    let done = cv
                        .wait_while(flag.lock().unwrap(), |released| !*released)
                        .unwrap();
                    drop(done);
                });
            })
        })
        .collect();
    for _ in 0..readers {
        held_rx.recv_timeout(PATIENCE).expect("reader held");
    }

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
    {
        let (flag, cv) = &*release;
        *flag.lock().unwrap() = true;
        cv.notify_all();
    }
    let answered = stream_rx.recv_timeout(PATIENCE);
    for holder in holders {
        holder.join().expect("holder");
    }
    drop(rt);

    assert!(
        early.is_err(),
        "precondition: /stream answered without waiting for the database, so \
         this test held nothing up"
    );
    assert_eq!(
        served,
        Ok(StatusCode::OK),
        "a static file was not served while another request waited for a reader"
    );
    // The waiting request finishes once the readers are let go: a fresh station
    // has no audio source, which `/stream` answers with 503.
    assert_eq!(answered, Ok(StatusCode::SERVICE_UNAVAILABLE));
}
