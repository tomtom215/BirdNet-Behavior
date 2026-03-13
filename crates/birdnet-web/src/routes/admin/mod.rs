//! Admin panel routes.
//!
//! Provides web UI and REST endpoints for managing the system:
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET  /admin`              | Admin landing page (redirects to /admin/settings) |
//! | `GET  /admin/settings`     | Settings form (all categories) |
//! | `POST /admin/settings`     | Save settings (HTMX partial) |
//! | `GET  /admin/migrate`      | BirdNET-Pi migration page |
//! | `POST /admin/migrate/validate` | Pre-flight validation (JSON) |
//! | `POST /admin/migrate/run`  | Start import (async, progress via polling) |
//! | `GET  /admin/migrate/progress` | Poll migration progress (JSON) |
//! | `GET  /admin/system`       | System status page |
//! | `POST /admin/system/backup` | Trigger database backup |
//! | `GET  /admin/system/logs`  | SSE live log stream |
//! | `GET  /admin/system/logs/page` | Live log viewer page |
//! | `GET  /admin/notifications` | Notification history log |
//! | `DELETE /admin/notifications/prune` | Prune old log entries |

pub mod logs;
pub mod migration;
pub mod notifications;
pub mod settings;
pub mod system;

use axum::{Router, routing::get};

use crate::state::AppState;

/// Build the admin router and mount all sub-routes.
pub fn router() -> Router<AppState> {
    Router::new()
        // Landing page → redirect to settings
        .route("/admin", get(landing))
        // Settings
        .merge(settings::router())
        // Migration
        .merge(migration::router())
        // System
        .merge(system::router())
        // Live log streaming
        .merge(logs::router())
        // Notification history
        .merge(notifications::router())
}

/// Redirect `/admin` to `/admin/settings`.
async fn landing() -> axum::response::Redirect {
    axum::response::Redirect::to("/admin/settings")
}
