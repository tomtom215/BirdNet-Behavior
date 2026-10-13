//! The request log names a share link's route, never its token.
//!
//! A share link is `/r/{token}`, and the token is the whole of the access
//! control. `TraceLayer`'s default span recorded the full URI at `debug`, so
//! the token went to the journal and from there into the support bundle.

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::Request;
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;
use tracing_subscriber::fmt::format::FmtSpan;

/// A writer that keeps everything the subscriber prints.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Everything logged at `debug` while `uri` is served.
async fn log_of(uri: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    let captured = Captured::default();
    let sink = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_span_events(FmtSpan::NEW)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    // `#[tokio::test]` is single-threaded, so a thread-local default covers
    // every poll of the request.
    let _guard = tracing::subscriber::set_default(subscriber);
    let _ = build_router(state)
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let bytes = captured.0.lock().expect("lock").clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Observed failing with `TraceLayer::new_for_http()` restored: the span
/// line read `request{method=GET uri=/r/S3CRETshareTOKEN?x=1 …}`.
#[tokio::test]
async fn a_share_link_never_reaches_the_request_log() {
    let log = log_of("/r/S3CRETshareTOKEN?x=1").await;
    assert!(
        log.contains("route=/r/{token}"),
        "the route is named: {log}"
    );
    assert!(!log.contains("S3CRET"), "the token was logged: {log}");
}

/// The counterpart: the capture works, and a request that matched no route
/// is still recorded by its path — without its query string.
#[tokio::test]
async fn an_unmatched_request_is_logged_by_its_path() {
    let log = log_of("/no/such/page?q=private").await;
    assert!(log.contains("route=/no/such/page"), "{log}");
    assert!(!log.contains("private"), "{log}");
}
