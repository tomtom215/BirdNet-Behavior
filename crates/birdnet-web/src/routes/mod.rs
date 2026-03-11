//! API route definitions.
//!
//! Organized by resource, matching the FastAPI router structure.

pub mod detections;
pub mod system;

use axum::Router;

/// Build all API routes under `/api/v2/`.
pub fn api_routes() -> Router {
    Router::new()
        .nest("/api/v2", detections::router())
        .nest("/api/v2", system::router())
}
