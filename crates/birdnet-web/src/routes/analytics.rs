//! Analytics API endpoints (`DuckDB`-powered).
//!
//! These endpoints are backed by `DuckDB` with the `duckdb-behavioral` extension
//! for advanced bird activity analytics. If the `DuckDB` database or behavioral
//! extension is not available, endpoints return a descriptive status message.
//!
//! Enable the `analytics` feature to compile the `DuckDB` connection code.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};

use crate::state::AppState;

/// Analytics routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/analytics/sessions", get(sessions))
        .route("/analytics/retention", get(retention))
        .route("/analytics/funnel", get(funnel))
        .route("/analytics/patterns", get(patterns))
        .route("/analytics/next-species", get(next_species))
}

async fn sessions(State(_state): State<AppState>) -> (StatusCode, Json<Value>) {
    unavailable("sessionize")
}

async fn retention(State(_state): State<AppState>) -> (StatusCode, Json<Value>) {
    unavailable("retention")
}

async fn funnel(State(_state): State<AppState>) -> (StatusCode, Json<Value>) {
    unavailable("window_funnel")
}

async fn patterns(State(_state): State<AppState>) -> (StatusCode, Json<Value>) {
    unavailable("sequence_match")
}

async fn next_species(State(_state): State<AppState>) -> (StatusCode, Json<Value>) {
    unavailable("sequence_next_node")
}

/// Response when `DuckDB` analytics is not configured or compiled.
fn unavailable(function: &str) -> (StatusCode, Json<Value>) {
    let message = if cfg!(feature = "analytics") {
        "DuckDB analytics not configured. Start with --analytics-db to enable."
    } else {
        "DuckDB analytics not compiled. Rebuild with --features analytics to enable."
    };

    (
        StatusCode::OK,
        Json(json!({
            "status": "unavailable",
            "message": message,
            "function": function,
        })),
    )
}
