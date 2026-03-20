//! Route definitions: REST API, HTMX pages, and WebSocket.
//!
//! Organized by resource, matching the `FastAPI` router structure for API endpoints
//! and adding HTMX page routes for the web dashboard.

pub mod admin;
pub mod analytics;
pub mod detections;
pub mod export;
pub mod images;
pub mod livestream;
pub mod pages;
pub mod recordings;
pub mod species;
pub mod spectrogram;
pub mod spectrogram_ws;
pub mod static_files;
pub mod system;
pub mod timeseries;
pub mod websocket;

use axum::Router;

use crate::state::AppState;

/// Validate a date string is in YYYY-MM-DD format.
///
/// Checks structure only (10 chars, digits in right positions, dashes as separators).
/// Does not validate calendar correctness (e.g., month 13 passes).
pub(crate) fn is_valid_date(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

/// Build all routes: API under `/api/v2/`, admin routes at `/admin`, and page routes at `/`.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        .nest("/api/v2", detections::router())
        .nest("/api/v2", species::router())
        .nest("/api/v2", analytics::router())
        .nest("/api/v2", timeseries::router())
        .nest("/api/v2", system::router())
        .nest("/api/v2", export::router())
        .nest("/api/v2", websocket::router())
        .nest("/api/v2", images::router())
        .nest("/api/v2", recordings::router())
        .nest("/api/v2", spectrogram::router())
        .nest("/api/v2", spectrogram_ws::router())
        .nest("/api/v2", livestream::router())
        .merge(livestream::stream_router())
        .merge(pages::router())
        .merge(static_files::router())
        .merge(admin::router())
}

#[cfg(test)]
mod tests {
    use super::is_valid_date;

    #[test]
    fn valid_date_format() {
        assert!(is_valid_date("2026-03-12"));
        assert!(is_valid_date("2020-01-01"));
        assert!(is_valid_date("1999-12-31"));
    }

    #[test]
    fn invalid_date_format() {
        assert!(!is_valid_date(""));
        assert!(!is_valid_date("2026"));
        assert!(!is_valid_date("03-12-2026"));
        assert!(!is_valid_date("2026/03/12"));
        assert!(!is_valid_date("not-a-date"));
        assert!(!is_valid_date("20260312"));
        assert!(!is_valid_date("2026-3-12"));
    }
}
