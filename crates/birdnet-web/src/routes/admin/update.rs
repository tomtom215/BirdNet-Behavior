//! Admin update routes.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET  /admin/update/check` | Check GitHub for a newer release (JSON) |
//! | `POST /admin/update/apply` | Download and install the latest release |

use axum::http::StatusCode;
use axum::response::Json;
use axum::{Router, routing::get};

use birdnet_integrations::auto_update;

use crate::state::AppState;

/// Mount update routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/update/check", get(check_update))
        .route("/admin/update/apply", axum::routing::post(apply_update))
}

// ---------------------------------------------------------------------------
// GET /admin/update/check
// ---------------------------------------------------------------------------

/// Check GitHub Releases for a newer version and return JSON.
async fn check_update() -> Result<Json<auto_update::UpdateInfo>, (StatusCode, String)> {
    let current = env!("CARGO_PKG_VERSION");

    let info = tokio::task::spawn_blocking(move || auto_update::check_for_update(current))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("task join error: {e}"),
            )
        })?
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("{e}")))?;

    Ok(Json(info))
}

// ---------------------------------------------------------------------------
// POST /admin/update/apply
// ---------------------------------------------------------------------------

/// Download the latest release binary and replace the running binary.
///
/// Reads the current executable path via `std::env::current_exe()` and
/// delegates to `auto_update::apply_update`.
async fn apply_update() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // First, check what the latest version is.
    let current = env!("CARGO_PKG_VERSION");

    let info = tokio::task::spawn_blocking(move || auto_update::check_for_update(current))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("task join error: {e}"),
            )
        })?
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("{e}")))?;

    if !info.update_available {
        return Ok(Json(serde_json::json!({
            "status": "up_to_date",
            "version": info.current_version,
        })));
    }

    let download_url = info.download_url.clone();
    let latest_version = info.latest_version.clone();

    let current_binary = std::env::current_exe().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot determine current binary path: {e}"),
        )
    })?;

    tokio::task::spawn_blocking(move || auto_update::apply_update(&download_url, &current_binary))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("task join error: {e}"),
            )
        })?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    Ok(Json(serde_json::json!({
        "status": "updated",
        "version": latest_version,
        "message": "Binary updated. Restart the service to use the new version.",
    })))
}
