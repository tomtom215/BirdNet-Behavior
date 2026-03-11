//! System API endpoints: health, version, diagnostics.

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

/// System routes.
pub fn router() -> Router {
    Router::new()
        .route("/", get(root))
        .route("/health", get(health))
}

async fn root() -> Json<Value> {
    Json(json!({
        "name": "BirdNET-Pi API",
        "version": env!("CARGO_PKG_VERSION"),
        "status": "running",
    }))
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "healthy",
        "database": "ok",
    }))
}
