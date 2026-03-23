//! System API endpoints: health, version, diagnostics.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};

use crate::state::AppState;

/// System routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/stats", get(stats))
        .route("/system/disk", get(disk_info))
}

async fn root() -> Json<Value> {
    Json(json!({
        "name": "BirdNet-Behavior API",
        "version": env!("CARGO_PKG_VERSION"),
        "status": "running",
    }))
}

async fn health(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let db_ok: bool = tokio::task::spawn_blocking({
        let state = state.clone();
        move || state.with_db(|conn| birdnet_db::sqlite::quick_check(conn).unwrap_or(false))
    })
    .await
    .unwrap_or(false);

    let status = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(json!({
            "status": if db_ok { "healthy" } else { "degraded" },
            "version": env!("CARGO_PKG_VERSION"),
            "database": if db_ok { "ok" } else { "error" },
            "analytics": state.has_analytics(),
        })),
    )
}

/// `GET /api/v2/system/disk` — Disk usage for the database filesystem.
async fn disk_info(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let db_path = state.db_path().to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        let dir = db_path.parent().filter(|p| !p.as_os_str().is_empty());
        let dir = dir.unwrap_or_else(|| std::path::Path::new("."));
        birdnet_core::audio::capture::disk_usage(dir)
    })
    .await;

    match result {
        Ok(Ok(usage)) => {
            let status = if usage.is_critical() {
                "critical"
            } else if usage.is_low() {
                "low"
            } else {
                "ok"
            };

            let http_status = if usage.is_critical() {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::OK
            };

            (
                http_status,
                Json(json!({
                    "status": status,
                    "total_bytes": usage.total_bytes,
                    "used_bytes": usage.used_bytes,
                    "available_bytes": usage.available_bytes,
                    "used_percent": format!("{:.1}", usage.used_percent()),
                })),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

async fn stats(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let detections = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let latest = birdnet_db::sqlite::latest_detection(conn).ok().flatten();
            let confidence = birdnet_db::sqlite::confidence_distribution(conn).unwrap_or([0; 6]);
            (detections, species, latest, confidence)
        })
    })
    .await;

    match result {
        Ok((detections, species, latest, confidence)) => {
            let latest_json = latest.map_or(json!(null), |(date, time, name)| {
                json!({
                    "date": date,
                    "time": time,
                    "species": name,
                })
            });

            (
                StatusCode::OK,
                Json(json!({
                    "total_detections": detections,
                    "unique_species": species,
                    "latest_detection": latest_json,
                    "confidence_distribution": {
                        "0-50": confidence[0],
                        "50-60": confidence[1],
                        "60-70": confidence[2],
                        "70-80": confidence[3],
                        "80-90": confidence[4],
                        "90-100": confidence[5],
                    },
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}
