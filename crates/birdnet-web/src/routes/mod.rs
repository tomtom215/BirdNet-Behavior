//! Route definitions: REST API, HTMX pages, and WebSocket.
//!
//! Organized by resource, matching the `FastAPI` router structure for API endpoints
//! and adding HTMX page routes for the web dashboard.

pub mod analytics;
pub mod detections;
pub mod export;
pub mod images;
pub mod pages;
pub mod species;
pub mod static_files;
pub mod system;
pub mod websocket;

use axum::Router;

use crate::state::AppState;

/// Build all routes: API under `/api/v2/` and page routes at the root.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        .nest("/api/v2", detections::router())
        .nest("/api/v2", species::router())
        .nest("/api/v2", analytics::router())
        .nest("/api/v2", system::router())
        .nest("/api/v2", export::router())
        .nest("/api/v2", websocket::router())
        .nest("/api/v2", images::router())
        .merge(pages::router())
        .merge(static_files::router())
}
