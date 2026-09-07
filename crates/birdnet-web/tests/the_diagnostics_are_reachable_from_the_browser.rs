//! `OP-1`: the station's `--doctor` and `--support-bundle` must be reachable
//! by an operator who has only a browser.
//!
//! The binary hands the web layer two hooks; this file drives the routes with
//! stand-in hooks and holds three things:
//!
//! 1. what the hooks produce is what the routes serve, unchanged;
//! 2. the diagnostics page renders the full report when it has one, and still
//!    renders (the configuration checks) when it does not — the counterpart
//!    that stops "always 503" or "always 500" from passing;
//! 3. a process with no hooks says so on the machine routes, with a status a
//!    monitor can tell apart from a typo in the URL.
//!
//! Observed failing against the shipped tree: every request to
//! `/admin/doctor.json` and `/admin/support-bundle` answered `404`, and the
//! page contained no check named by the hook.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

use birdnet_web::diagnostics::Diagnostics;
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

const REPORT: &str = r#"{"summary":{"passed":1,"warnings":0,"errors":1,"skipped":0,"exit_code":2},"checks":[{"status":"pass","name":"Audio source","message":"ALSA source configured","remediation":null},{"status":"fail","name":"System clock","message":"reads 1970-01-01","remediation":"check the NTP uplink"}]}"#;

fn bare_state() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// Hooks that record how often they ran and write a recognisable archive.
fn hooks(runs: &Arc<AtomicUsize>) -> Diagnostics {
    let doctor_runs = Arc::clone(runs);
    Diagnostics::new(
        move || {
            doctor_runs.fetch_add(1, Ordering::SeqCst);
            REPORT.to_owned()
        },
        |dest: &Path| {
            std::fs::write(dest, b"\x1f\x8b-not-really-gzip-but-the-hooks-bytes")
                .map_err(|e| e.to_string())
        },
    )
}

async fn get(app: axum::Router, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = app
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, bytes.to_vec())
}

#[tokio::test]
async fn doctor_json_serves_exactly_what_the_binary_produces() {
    let runs = Arc::new(AtomicUsize::new(0));
    let app = build_router(bare_state().with_diagnostics(hooks(&runs)));

    let (status, headers, body) = get(app, "/admin/doctor.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE].to_str().unwrap(),
        "application/json"
    );
    assert_eq!(String::from_utf8(body).unwrap(), REPORT);
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "the doctor ran once, on request"
    );
}

#[tokio::test]
async fn the_support_bundle_is_the_binarys_archive_offered_as_a_download() {
    let runs = Arc::new(AtomicUsize::new(0));
    let app = build_router(bare_state().with_diagnostics(hooks(&runs)));

    let (status, headers, body) = get(app, "/admin/support-bundle").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE].to_str().unwrap(),
        "application/gzip"
    );
    let disposition = headers[header::CONTENT_DISPOSITION].to_str().unwrap();
    assert!(
        disposition.starts_with("attachment; filename=\"birdnet-support-")
            && disposition.ends_with(".tar.gz\""),
        "{disposition}"
    );
    assert_eq!(body, b"\x1f\x8b-not-really-gzip-but-the-hooks-bytes");
}

#[tokio::test]
async fn the_page_renders_the_full_report_when_the_process_has_one() {
    let runs = Arc::new(AtomicUsize::new(0));
    let app = build_router(bare_state().with_diagnostics(hooks(&runs)));

    let (status, _, body) = get(app, "/admin/doctor").await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("System clock"), "{html}");
    assert!(html.contains("reads 1970-01-01"), "{html}");
    assert!(html.contains("check the NTP uplink"), "{html}");
    assert!(html.contains("/admin/support-bundle"), "{html}");
    assert!(
        !html.contains("run <code>birdnet-behavior --doctor</code> on the host"),
        "a page that has the report must not send the operator to SSH: {html}"
    );
}

/// The counterpart. Tooling and tests build a state with no hooks; the page
/// must still render its configuration checks, and the machine routes must say
/// what is missing rather than 404 like a typo or 500 like a bug.
#[tokio::test]
async fn a_process_without_the_hooks_says_so_instead_of_pretending() {
    let (status, _, body) = get(build_router(bare_state()), "/admin/doctor").await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("birdnet-behavior --doctor"), "{html}");

    for path in ["/admin/doctor.json", "/admin/support-bundle"] {
        let (status, headers, body) = get(build_router(bare_state()), path).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(
            headers[header::CONTENT_TYPE].to_str().unwrap(),
            "application/json"
        );
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["error"].as_str().unwrap().contains("not wired"),
            "{path}: {json}"
        );
    }
}
