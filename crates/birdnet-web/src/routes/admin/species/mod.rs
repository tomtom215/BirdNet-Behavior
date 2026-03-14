//! Admin species list management.
//!
//! Provides interactive management of the species exclusion and allow-lists.
//! Changes are stored as comma-separated values in the SQLite settings table
//! under `species_exclude` and `species_include`.
//!
//! Routes:
//!
//! | Method | Path | Action |
//! |--------|------|--------|
//! | GET    | /admin/species | Species list management page |
//! | POST   | /admin/species/exclude/add | Add species to exclusion list |
//! | POST   | /admin/species/exclude/remove | Remove from exclusion list |
//! | POST   | /admin/species/include/add | Add to allow-list |
//! | POST   | /admin/species/include/remove | Remove from allow-list |
//! | GET    | /admin/species/partial | HTMX partial re-render |

pub mod handler;
pub mod render;

use axum::{Router, routing::get};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/species", get(handler::species_page))
        .route("/admin/species/test", get(handler::filter_test_page))
        .route("/admin/species/partial", get(handler::species_partial))
        .route(
            "/admin/species/exclude/add",
            axum::routing::post(handler::add_exclude),
        )
        .route(
            "/admin/species/exclude/remove",
            axum::routing::post(handler::remove_exclude),
        )
        .route(
            "/admin/species/include/add",
            axum::routing::post(handler::add_include),
        )
        .route(
            "/admin/species/include/remove",
            axum::routing::post(handler::remove_include),
        )
        .route(
            "/admin/species/thresholds",
            get(handler::thresholds_partial),
        )
        .route(
            "/admin/species/thresholds/set",
            axum::routing::post(handler::set_threshold),
        )
        .route(
            "/admin/species/thresholds/delete",
            axum::routing::post(handler::delete_threshold),
        )
}
