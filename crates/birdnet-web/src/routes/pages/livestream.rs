//! `/live` → `/listen` redirect.
//!
//! `/live` was a minimal, orphaned second live-audio page — linked from no
//! navigation surface and superseded by the full-featured `/listen` page
//! (per-source audio + a live spectrogram + a live detection trickle). The path
//! is kept as a permanent redirect so any old bookmark or external link lands on
//! the maintained page instead of a dead-end duplicate implementation.

use axum::Router;
use axum::response::Redirect;
use axum::routing::get;

use crate::state::AppState;

/// Redirect the legacy `/live` page to the maintained `/listen` page.
pub fn router() -> Router<AppState> {
    Router::new().route("/live", get(|| async { Redirect::permanent("/listen") }))
}
