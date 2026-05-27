//! Route definitions: REST API, HTMX pages, and WebSocket.
//!
//! Organized by resource, matching the `FastAPI` router structure for API endpoints
//! and adding HTMX page routes for the web dashboard.

pub mod admin;
pub mod analytics;
pub mod detections;
pub mod export;
pub mod feeds;
pub mod health;
pub mod images;
pub mod livestream;
pub mod pages;
pub mod recordings;
pub mod share;
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

/// Log an internal (5xx) error server-side and return a generic, safe message
/// for the client.
///
/// 5xx responses must never echo internal error strings: a `DbError`'s Display
/// leaks SQL/schema detail (e.g. "sqlite error: no such column …") and a
/// `JoinError` leaks panic text. Handlers keep their own response shape and use
/// this for the `"error"` field, so the real detail is logged for the operator
/// (with the module as the tracing target) but never disclosed over the network.
pub(crate) fn log_internal<E: std::fmt::Display>(context: &str, err: &E) -> &'static str {
    tracing::error!(error = %err, "{context}");
    "internal server error"
}

/// Public routes: everything except the `/admin` panel — the dashboard, API,
/// live stream, feeds, spectrograms, static assets.
///
/// These are safe to serve without a login so a station is viewable on the LAN
/// out of the box. State-changing admin actions live in [`admin_routes`], which
/// is gated separately (see `server::build_router_with_auth`).
pub fn public_routes() -> Router<AppState> {
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
        .nest("/api/v2", health::router())
        .merge(livestream::stream_router())
        .merge(pages::router())
        .merge(feeds::router())
        .merge(share::router())
        .merge(static_files::router())
        // Friendly branded 404 for any unmatched path (e.g. a mistyped page URL).
        .fallback(pages::not_found)
}

/// The `/admin` panel: settings, software update, system controls, backups,
/// migrations. These can change configuration and update the software, so they
/// are gated behind HTTP Basic Auth when a password is configured.
pub fn admin_routes() -> Router<AppState> {
    admin::router()
}

/// Build all routes: API under `/api/v2/`, admin routes at `/admin`, and page
/// routes at `/`. The admin panel is open here; callers that want it gated use
/// [`public_routes`] + [`admin_routes`] with auth applied to the latter.
pub fn api_routes() -> Router<AppState> {
    public_routes().merge(admin_routes())
}

#[cfg(test)]
mod tests {
    use super::is_valid_date;

    /// Constructing the router must not panic on overlapping routes.
    ///
    /// `Router::merge`/`route` panic when two handlers register the same
    /// method+path. The main CI test job runs `cargo test --workspace --lib
    /// --bins`, which excludes the integration tests under `tests/` (they link
    /// libonnxruntime), and CI never boots the server — so without this
    /// lib-level guard a duplicate route only surfaces as a startup crash in
    /// production. Building the full router here makes any collision fail
    /// `cargo test --lib`.
    #[test]
    fn api_routes_build_without_collision() {
        let _router = super::api_routes();
    }

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
