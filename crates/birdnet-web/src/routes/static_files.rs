//! Embedded static file serving.
//!
//! Serves JavaScript and CSS files compiled into the binary via `include_bytes!`.
//! This enables fully air-gapped deployments with no external CDN dependencies.

use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::{Router, routing::get};

use crate::state::AppState;

/// HTMX library (minified, embedded at compile time).
const HTMX_JS: &[u8] = include_bytes!("../../static/htmx.min.js");

/// HTMX SSE extension (embedded at compile time).
const HTMX_SSE_JS: &[u8] = include_bytes!("../../static/htmx-sse.js");

/// Static file routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/static/htmx.min.js", get(htmx_js))
        .route("/static/htmx-sse.js", get(htmx_sse_js))
}

async fn htmx_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        HTMX_JS,
    )
}

async fn htmx_sse_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        HTMX_SSE_JS,
    )
}
