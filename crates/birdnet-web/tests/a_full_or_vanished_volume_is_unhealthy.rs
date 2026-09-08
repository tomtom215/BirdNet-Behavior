//! `/api/v2/health` reads the data volume (DD-19, DD-20).
//!
//! On a 100 % full volume the station answered `200 "healthy"` while
//! `/system/disk` said `critical`, DuckDB had been quarantined on `ENOSPC` and
//! the admin bootstrap had failed. After `umount -l` of the data mount it kept
//! writing into the directory underneath and reported the parent filesystem
//! as fine. The health verdict now reads the volume watch's last probe; these
//! gates publish probes the way the watch does and read the verdict back.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::data_volume::{DataVolumeStatus, DiskVerdict, MountState};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn health(state: &AppState, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

const fn healthy_volume() -> DataVolumeStatus {
    DataVolumeStatus {
        writable: true,
        write_error: None,
        mount: MountState::Intact,
        disk: DiskVerdict::Ok,
        used_percent: Some(35.0),
        checked_at: 1,
    }
}

/// A critically full disk is a strict fault and nothing else: the pager
/// hears, the container supervisor does not restart a station whose disk is
/// full.
#[tokio::test]
async fn a_critically_full_disk_is_a_strict_fault() {
    let state = station();
    state.set_data_volume(healthy_volume());
    let (status, body) = health(&state, "/api/v2/health").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data_volume"]["disk"], "ok");

    state.set_data_volume(DataVolumeStatus {
        disk: DiskVerdict::Critical,
        used_percent: Some(100.0),
        ..healthy_volume()
    });
    let (status, body) = health(&state, "/api/v2/health?strict=1").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["data_volume"]["disk"], "critical");
    let (status, _) = health(&state, "/api/v2/health").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the default probe is for the supervisor"
    );
}

/// A volume that takes no writes is degraded on every reading, like a halted
/// ingest: the station is running and keeping nothing.
#[tokio::test]
async fn an_unwritable_or_vanished_volume_is_degraded_on_every_reading() {
    let state = station();
    state.set_data_volume(DataVolumeStatus {
        writable: false,
        write_error: Some("Read-only file system (os error 30)".into()),
        ..healthy_volume()
    });
    let (status, body) = health(&state, "/api/v2/health").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["data_volume"]["writable"], false);
    assert!(
        body["data_volume"]["write_error"]
            .as_str()
            .is_some_and(|e| e.contains("os error 30")),
        "{body}"
    );

    state.set_data_volume(DataVolumeStatus {
        mount: MountState::Vanished,
        ..healthy_volume()
    });
    let (status, body) = health(&state, "/api/v2/health").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["data_volume"]["mount"], "vanished");
}

/// A `df` that cannot answer is a strict fault, and the disk endpoint says
/// "unknown" with a 503 rather than answering 500 as if the handler had a bug.
#[tokio::test]
async fn a_df_that_cannot_answer_is_unknown_not_an_internal_error() {
    let state = station();
    state.set_data_volume(DataVolumeStatus {
        disk: DiskVerdict::Unknown,
        used_percent: None,
        ..healthy_volume()
    });
    let (status, body) = health(&state, "/api/v2/health?strict=1").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["data_volume"]["disk"], "unknown");

    // The disk endpoint on a database whose directory does not exist.
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("gone").join("birds.db");
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, missing);
    let (status, body) = health(&state, "/api/v2/system/disk").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["status"], "unknown", "{body}");
}

/// A failed admin bootstrap is a strict fault: a station that could not write
/// its own credential is one whose database refused a write.
#[tokio::test]
async fn a_failed_admin_bootstrap_is_a_strict_fault() {
    let state = station();
    state.set_data_volume(healthy_volume());
    state.set_admin_bootstrap_failed(true);
    let (status, body) = health(&state, "/api/v2/health?strict=1").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["admin_bootstrap"], "failed");
    let (status, _) = health(&state, "/api/v2/health").await;
    assert_eq!(status, StatusCode::OK);
}

/// What the boot journal found (UP-3) is on the body and is a strict fault: a
/// station that lost its database over a restart is the pager's business and
/// not the container supervisor's.
#[tokio::test]
async fn a_boot_anomaly_is_a_strict_fault() {
    use birdnet_web::boot_journal::Anomaly;
    let state = station();
    state.set_data_volume(healthy_volume());
    // A stopped daemon is a strict fault of its own; mark it running so the
    // verdict below is the journal's and nothing else's.
    state
        .detection_status_flag()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let (status, body) = health(&state, "/api/v2/health?strict=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["boot_anomalies"], serde_json::json!([]));

    state.set_boot_anomalies(vec![Anomaly::DbLost { before: 40_000 }, Anomaly::MountLost]);
    let (status, body) = health(&state, "/api/v2/health?strict=1").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        body["boot_anomalies"],
        serde_json::json!(["db_lost", "mount_lost"])
    );
    let (status, _) = health(&state, "/api/v2/health").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "not strict: the supervisor must not restart it"
    );
}

/// A station whose watch has not run yet says so, and is not degraded for it.
#[tokio::test]
async fn an_unchecked_volume_is_reported_as_unchecked() {
    let state = station();
    let (status, body) = health(&state, "/api/v2/health").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data_volume"], "unchecked");
}
