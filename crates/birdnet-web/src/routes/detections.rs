//! Detection API endpoints.
//!
//! TODO(phase5): Implement detection query endpoints.

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

/// Detection routes.
pub fn router() -> Router {
    Router::new().route("/detections", get(list_detections))
}

async fn list_detections() -> Json<Value> {
    Json(json!({
        "detections": [],
        "total": 0,
        "message": "not yet implemented"
    }))
}
