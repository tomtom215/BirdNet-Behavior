//! Admin settings routes — GET / POST /admin/settings.
//!
//! Split into sub-modules for single responsibility:
//!
//! | Module    | Responsibility                        |
//! |-----------|---------------------------------------|
//! | `form`    | Form deserialization types            |
//! | `handler` | Route handler functions               |
//! | `render`  | HTML page and form rendering          |

pub mod form;
pub mod handler;
pub mod render;

use axum::{Router, routing::get};

use crate::state::AppState;

/// Mount settings routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/settings",
            get(handler::settings_page).post(handler::save_settings),
        )
        .route("/admin/settings/partial", get(handler::settings_partial))
        .route(
            "/admin/settings/detect-location",
            get(handler::detect_location),
        )
}
