//! System API endpoints: health, version, diagnostics.

use axum::extract::State;
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

async fn health(State(state): State<AppState>) -> Json<Value> {
    let db_ok: bool = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::quick_check(conn).unwrap_or(false))
    })
    .await
    .unwrap_or(false);

    Json(json!({
        "status": if db_ok { "healthy" } else { "degraded" },
        "database": if db_ok { "ok" } else { "error" },
    }))
}

async fn stats(State(state): State<AppState>) -> Json<Value> {
    let result: Result<(i64, i64), _> = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let detections = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            (detections, species)
        })
    })
    .await;

    match result {
        Ok((detections, species)) => Json(json!({
            "total_detections": detections,
            "unique_species": species,
        })),
        Err(e) => Json(json!({ "error": format!("internal error: {e}") })),
    }
}
