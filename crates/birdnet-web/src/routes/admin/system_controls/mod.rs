//! System control routes for data management, backup/restore, service control.
//!
//! | Module    | Responsibility                                      |
//! |-----------|-----------------------------------------------------|
//! | `data`    | Clear detections + extracted recordings              |
//! | `backup`  | Full tar.gz backup download + restore upload         |
//! | `service` | Service restart, status, systemd integration         |
//! | `update`  | GitHub Releases update check                        |

mod backup;
mod data;
mod service;
mod update;

use axum::{Router, routing};

use crate::state::AppState;

/// Mount system control routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/system/clear-detections",
            routing::post(data::clear_detections),
        )
        .route(
            "/admin/system/clear-extracted",
            routing::post(data::clear_extracted),
        )
        .route(
            "/admin/system/backup/full",
            routing::get(backup::full_backup),
        )
        .route(
            "/admin/system/restore",
            routing::post(backup::restore_backup),
        )
        .route(
            "/admin/system/service/restart",
            routing::post(service::service_restart),
        )
        .route(
            "/admin/system/service/status",
            routing::get(service::service_status),
        )
        .route(
            "/admin/system/update/check",
            routing::get(update::check_update),
        )
}
