//! API route definitions.
//!
//! Organized by resource, matching the FastAPI router structure.

pub mod analytics;
pub mod detections;
pub mod species;
pub mod system;

use axum::Router;

use crate::state::AppState;

/// Build all API routes under `/api/v2/`.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        .nest("/api/v2", detections::router())
        .nest("/api/v2", species::router())
        .nest("/api/v2", analytics::router())
        .nest("/api/v2", system::router())
}
