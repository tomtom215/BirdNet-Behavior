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
}

async fn root() -> Json<Value> {
    Json(json!({
        "name": "BirdNet-Behavior API",
        "version": env!("CARGO_PKG_VERSION"),
        "status": "running",
    }))
}

async fn health(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let db_ok: bool = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::quick_check(conn).unwrap_or(false))
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
            "database": if db_ok { "ok" } else { "error" },
        })),
    )
}

async fn stats(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let detections = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let latest = birdnet_db::sqlite::latest_detection(conn).ok().flatten();
            let confidence = birdnet_db::sqlite::confidence_distribution(conn)
                .unwrap_or([0; 6]);
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
