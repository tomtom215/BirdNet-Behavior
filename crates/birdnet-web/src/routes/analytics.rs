//! Analytics API endpoints (DuckDB-powered).
//!
//! These endpoints will be backed by `DuckDB` with the behavioral extension
//! once the birdnet-behavioral crate is fully integrated. For now, they
//! provide SQLite-based approximations.

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

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

async fn sessions() -> Json<Value> {
    Json(json!({
        "message": "Activity sessionization (duckdb-behavioral: sessionize) - coming soon",
        "status": "planned",
        "extension": "duckdb-behavioral",
        "function": "sessionize",
    }))
}

async fn retention() -> Json<Value> {
    Json(json!({
        "message": "Species retention analysis (duckdb-behavioral: retention) - coming soon",
        "status": "planned",
        "extension": "duckdb-behavioral",
        "function": "retention",
    }))
}

async fn funnel() -> Json<Value> {
    Json(json!({
        "message": "Dawn chorus funnel analysis (duckdb-behavioral: window_funnel) - coming soon",
        "status": "planned",
        "extension": "duckdb-behavioral",
        "function": "window_funnel",
    }))
}

async fn patterns() -> Json<Value> {
    Json(json!({
        "message": "Sequence pattern matching (duckdb-behavioral: sequence_match) - coming soon",
        "status": "planned",
        "extension": "duckdb-behavioral",
        "function": "sequence_match",
    }))
}

async fn next_species() -> Json<Value> {
    Json(json!({
        "message": "Next species prediction (duckdb-behavioral: sequence_next_node) - coming soon",
        "status": "planned",
        "extension": "duckdb-behavioral",
        "function": "sequence_next_node",
    }))
}
