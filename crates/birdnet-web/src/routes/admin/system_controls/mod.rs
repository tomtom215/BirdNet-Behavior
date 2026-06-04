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
            // A full backup (database + recordings) is far larger than axum's
            // 2 MiB default body limit; without lifting it, restoring a real
            // backup is rejected before it starts. The handler streams the
            // upload to disk (constant memory), and the route is admin-only
            // (RBAC), so an operator restoring their own archive of any size is
            // safe — disk free space, not a body cap, is the real bound.
            routing::post(backup::restore_backup).layer(axum::extract::DefaultBodyLimit::disable()),
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
