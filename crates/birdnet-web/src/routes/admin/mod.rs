//! Admin panel routes.
//!
//! Provides web UI and REST endpoints for managing the system:
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET  /admin`              | Admin landing page (redirects to /admin/overview) |
//! | `GET  /admin/overview`     | Admin dashboard overview |
//! | `GET  /admin/settings`     | Settings form (all categories) |
//! | `POST /admin/settings`     | Save settings (HTMX partial) |
//! | `GET  /admin/species`      | Species exclusion / allow-list management |
//! | `GET  /admin/migrate`      | BirdNET-Pi migration page |
//! | `POST /admin/migrate/validate` | Pre-flight validation (JSON) |
//! | `POST /admin/migrate/run`  | Start import (async, progress via polling) |
//! | `GET  /admin/migrate/progress` | Poll migration progress (JSON) |
//! | `GET  /admin/system`       | System status page |
//! | `POST /admin/system/backup` | Trigger database backup |
//! | `GET  /admin/system/logs`  | SSE live log stream |
//! | `GET  /admin/system/logs/page` | Live log viewer page |
//! | `GET  /admin/notifications` | Notification history log |
//! | `GET  /admin/notifications/test` | Test notification channels |
//! | `DELETE /admin/notifications/prune` | Prune old log entries |
//! | `GET  /admin/system/backups`        | List database backups |
//! | `GET  /admin/system/backups/{name}` | Download a backup file |
//! | `DELETE /admin/system/backups/{name}` | Delete a backup file |

pub mod backup;
pub mod logs;
pub mod migration;
pub mod notification_test;
pub mod notifications;
pub mod overview;
pub mod settings;
pub mod species;
pub mod system;

use axum::{Router, routing::get};

use crate::state::AppState;

/// Build the admin router and mount all sub-routes.
pub fn router() -> Router<AppState> {
    Router::new()
        // Landing page → redirect to overview
        .route("/admin", get(landing))
        // Overview dashboard
        .merge(overview::router())
        // Settings
        .merge(settings::router())
        // Species list management
        .merge(species::router())
        // Migration
        .merge(migration::router())
        // System
        .merge(system::router())
        // Live log streaming
        .merge(logs::router())
        // Notification history
        .merge(notifications::router())
        // Notification testing
        .merge(notification_test::router())
        // Backup management
        .merge(backup::router())
}

/// Redirect `/admin` to `/admin/overview`.
async fn landing() -> axum::response::Redirect {
    axum::response::Redirect::to("/admin/overview")
}
