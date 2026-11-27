//! The station's own record: who it is, where each source was deployed, which
//! device each source was, and what it knew of its clock (RES-12, RES-22,
//! MSC-13, OOB-17).
//!
//! Both endpoints are behind the bearer gate, beside the other private reads:
//! a deployment holds coordinates, and a device record holds camera URLs.
//!
//! * `GET /api/v2/station/record` — the station id, every deployment, every
//!   device record (URL credentials and secret query values redacted), and the
//!   most recent clock observations, newest first.
//! * `POST /api/v2/deployments` — state a deployment of one source from a
//!   start; the source's open deployment is closed there. Answers `400` for a
//!   value the record refuses and `409` when it would overlap a stated one.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::state::AppState;

/// How many clock observations the record shows.
const CLOCK_ROWS: u32 = 50;

/// `GET /api/v2/station/record`.
pub(crate) async fn record(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let result = state
        .run_blocking(|s| {
            s.with_read_db(|conn| {
                let station = birdnet_db::station::station_uuid(conn).map_err(|e| e.to_string())?;
                let deployments =
                    birdnet_db::station::list_deployments(conn).map_err(|e| e.to_string())?;
                let devices =
                    birdnet_db::station::list_source_devices(conn).map_err(|e| e.to_string())?;
                let clock = birdnet_db::capture_ledger::recent_observations(conn, CLOCK_ROWS)
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>((station, deployments, devices, clock))
            })
        })
        .await;
    match result {
        Ok((station, deployments, devices, clock)) => {
            let devices: Vec<Value> = devices
                .into_iter()
                .map(|d| {
                    json!({
                        "source_id": d.source_id,
                        "device_seq": d.device_seq,
                        "kind": d.kind,
                        "device_id": birdnet_core::config::redact::redact_url_credentials(&d.device_id),
                        "from_utc": d.from_utc,
                        "to_utc": d.to_utc,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(json!({
                    "station_uuid": station,
                    "deployments": deployments,
                    "source_devices": devices,
                    "clock_observations": clock,
                    "notes": "Deployments are what an operator stated; nothing here measured \
                              them. device_seq numbers a source's distinct devices: the same \
                              device string keeps its number, a changed one gets a new one. \
                              A clock state between two observations is the earlier one's; \
                              an instant before the first is unknown.",
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": crate::routes::log_internal("internal error", &e)})),
        ),
    }
}

/// `POST /api/v2/deployments`.
pub(crate) async fn open_deployment(
    State(state): State<AppState>,
    Json(new): Json<birdnet_db::station::NewDeployment>,
) -> (StatusCode, Json<Value>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| birdnet_db::station::open_deployment(conn, &new, now))
        })
        .await;
    match result {
        Ok(d) => (StatusCode::CREATED, Json(json!({ "deployment": d }))),
        Err(birdnet_db::station::StationError::Invalid(m)) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": m })))
        }
        Err(birdnet_db::station::StationError::Sqlite(e))
            if e.to_string().contains("may not overlap") =>
        {
            (
                StatusCode::CONFLICT,
                Json(
                    json!({ "error": "it would overlap a deployment already stated for this source" }),
                ),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": crate::routes::log_internal("internal error", &e)})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};

    fn state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        conn.insert(&NewAudioSource::defaults(
            "cam",
            SourceKind::Rtsp,
            "rtsp://admin:hunter2@pond.local/audio",
        ))
        .unwrap();
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    fn new(start: i64) -> birdnet_db::station::NewDeployment {
        serde_json::from_value(json!({
            "source_id": "cam", "start_utc": start, "height_m": 3.0, "microphone": "EM272"
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn a_deployment_is_stated_then_read_back_with_the_station_and_devices() {
        let s = state();
        let (code, _) = open_deployment(State(s.clone()), Json(new(1_000))).await;
        assert_eq!(code, StatusCode::CREATED);
        let (code, Json(body)) = record(State(s.clone())).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["station_uuid"].as_str().map(str::len), Some(36));
        assert_eq!(body["deployments"][0]["height_m"], json!(3.0));
        assert_eq!(body["deployments"][0]["microphone"], json!("EM272"));
        let device = body["source_devices"][0]["device_id"].as_str().unwrap();
        assert!(
            !device.contains("hunter2"),
            "a camera's password reached the record: {device}"
        );
        assert!(device.contains("pond.local"), "{device}");
    }

    #[tokio::test]
    async fn a_start_before_the_open_deployment_and_a_bad_value_are_refused() {
        let s = state();
        let (code, _) = open_deployment(State(s.clone()), Json(new(5_000))).await;
        assert_eq!(code, StatusCode::CREATED);
        let (code, _) = open_deployment(State(s.clone()), Json(new(4_000))).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let mut bad = new(6_000);
        bad.stated.orientation_deg = Some(400.0);
        let (code, _) = open_deployment(State(s.clone()), Json(bad)).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        // Counterpart: a later start is accepted and closes the first.
        let (code, _) = open_deployment(State(s), Json(new(6_000))).await;
        assert_eq!(code, StatusCode::CREATED);
    }
}
