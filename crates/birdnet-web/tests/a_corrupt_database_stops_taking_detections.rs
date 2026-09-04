//! A database that has failed its integrity check stops taking detections —
//! and keeps everything an operator needs to find out why.
//!
//! # What was wrong (`PS-5`)
//!
//! The "never write to a corrupt database" policy existed only at startup.
//! `app.rs` runs `resilience::check_and_recover` before the state is built and,
//! failing that, quarantines the file rather than opening it. That is thorough.
//!
//! The *daily* check had none of it. `maintenance.rs` ran
//! `PRAGMA integrity_check` and, on failure, did exactly two things: wrote one
//! `error!` line, and recorded the verdict. The daemon then went on inserting
//! into the corrupt file until somebody rebooted it — which on an unattended
//! station is months. Worse, `backup_database` refuses to snapshot a corrupt
//! source, so throughout all of that the backup ring stopped producing new
//! restore points: every hour made recovery *worse*, silently.
//!
//! # Why this is not `PRAGMA query_only`
//!
//! Because login sessions are rows in this database. Making the writer
//! read-only would lock the operator out of the admin UI that exists to tell
//! them what is wrong, and would stop the notification log recording the very
//! alerts about the corruption. The line is drawn at the writes that *record a
//! detection event*; everything needed to see the problem and act on it keeps
//! working.
//!
//! # What this gate holds, and where the rest of it is
//!
//! Here: that an administrative write survives the halt (the discrimination
//! that distinguishes this from a read-only connection), and that
//! `/api/v2/health` says so and answers 503.
//!
//! The other half — that the real `event_processor` records a detection with
//! the latch clear and records nothing with it set — lives in
//! `src/daemon/processor.rs`, because the binary crate has no library target
//! and `event_processor` cannot be reached from here.

use std::sync::atomic::Ordering;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// The discrimination: this is not `PRAGMA query_only`.
///
/// An administrative write must still succeed while ingest is halted. Without
/// this, "stop writing to a corrupt database" could be implemented as a
/// read-only connection — which locks the operator out of the admin UI and
/// silences the log that records the alerts about the corruption.
#[tokio::test]
async fn an_administrative_write_still_succeeds_while_ingest_is_halted() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(tmp.path().join("birds.db")).expect("state");
    state.ingest_halt_flag().store(true, Ordering::Relaxed);

    // A settings save — the shape of every admin write, and the one an operator
    // makes while trying to fix the station.
    state
        .with_db(|conn| {
            birdnet_db::settings::ensure_settings_table(conn).ok();
            birdnet_db::settings::set(
                conn,
                "site_name",
                "still reachable",
                birdnet_db::settings::SettingsCategory::General,
            )
        })
        .expect("an administrative write must survive an ingest halt");

    // And a notification-log row, so the alerts about the corruption are still
    // recorded where an operator will look for them.
    let row = birdnet_db::notifications::NotifRecord {
        channel: "alert",
        species_com_name: None,
        species_sci_name: None,
        confidence: None,
        detection_date: None,
        detection_time: None,
        status: birdnet_db::notifications::NotifStatus::Sent,
        message: Some("Scheduled integrity check is failing"),
        error: None,
    };
    state
        .with_db(|conn| birdnet_db::notifications::log_notification(conn, &row))
        .expect("the notification log must survive an ingest halt");

    // The gate itself is closed, so the two assertions above are not passing
    // because the latch was never set.
    assert!(state.ingest_halted());
    assert!(
        state.with_ingest_db(|_| ()).is_none(),
        "the ingest gate must be closed, or this test is vacuous"
    );
}

/// The state is loud: the health endpoint says so, and answers 503.
#[tokio::test]
async fn the_health_endpoint_reports_a_halted_station() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(tmp.path().join("birds.db")).expect("state");

    let (status, body) = health(&state).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("\"detection_writes\":\"accepted\""),
        "a healthy station must say so: {body}"
    );

    state.ingest_halt_flag().store(true, Ordering::Relaxed);

    let (status, body) = health(&state).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a station recording nothing is degraded: {body}"
    );
    assert!(
        body.contains("\"detection_writes\":\"halted\""),
        "the operator must be able to see *why* from the endpoint they poll: {body}"
    );
}

async fn health(state: &AppState) -> (StatusCode, String) {
    let app = birdnet_web::server::build_router(state.clone());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}
