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
async fn apply_update(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
    request_user: crate::auth_middleware::RequestUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // First, check what the latest version is.
    let current = env!("CARGO_PKG_VERSION");

    // Before the download. An update that bricks a station has to be
    // attributable afterwards, and "afterwards" may be a binary that never
    // starts — the row is written by the process that is still working.
    crate::audit::audit(
        &state,
        Some(&request_user),
        "system.update.apply",
        None,
        Some(&format!("from={current}")),
    );

    // Before any network: the swap stages its files beside the running binary,
    // and under the shipped systemd unit that directory is read-only to the
    // service (ProtectSystem=strict, a non-root user). The download used to run
    // first and fail after ~100 MB with an I/O error that named no remedy.
    let current_binary = std::env::current_exe().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot determine current binary path: {e}"),
        )
    })?;
    if let Some(refusal) = cannot_replace(&current_binary) {
        return Err((StatusCode::CONFLICT, refusal));
    }

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
    // Verified against the release's published SHA256SUMS before the swap; the
    // staged binary is also smoke-tested. `None` means the release could not be
    // checked at all, which `apply_update` refuses — but it refuses with a
    // generic message, and the specific reason (no SHA256SUMS asset, a 503, a
    // missing line) is only known here. Report it rather than letting the
    // operator guess why their update will not install.
    let Some(expected_sha256) = info.sha256.clone() else {
        let reason = info
            .sha256_error
            .unwrap_or_else(|| "no sha256 was published for this asset".into());
        return Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "refusing to install {}: the download cannot be verified ({reason}). \
                 No binary was fetched and the running version is untouched.",
                info.latest_version
            ),
        ));
    };

    tokio::task::spawn_blocking(move || {
        auto_update::apply_update(&download_url, &current_binary, Some(&expected_sha256))
    })
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

/// Why this process cannot replace `binary`, or `None` when it can.
///
/// Answered by creating (and removing) a file where the update stages its
/// own, which is the question that matters: `access(2)` does not see a
/// read-only mount namespace, a real create does.
fn cannot_replace(binary: &std::path::Path) -> Option<String> {
    let dir = binary.parent()?;
    match tempfile::Builder::new()
        .prefix(".birdnet-update-probe")
        .tempfile_in(dir)
    {
        Ok(_) => None,
        Err(e) => Some(format!(
            "this station cannot replace its own binary: {} is not writable to it ({e}). \
             Nothing was downloaded. Update from a shell instead: \
             curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash -s -- update",
            dir.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    /// Under the shipped unit the binary's directory is read-only to the
    /// service, and the update learned that only after downloading the
    /// release. The refusal comes first now, and says what to run instead.
    #[test]
    fn an_update_the_station_cannot_install_is_refused_before_downloading() {
        // /proc refuses a new file even to root, which runs these tests here.
        let refusal = super::cannot_replace(std::path::Path::new("/proc/birdnet-behavior"))
            .expect("a directory the process cannot write is refused");
        assert!(refusal.contains("install.sh"), "{refusal}");
        assert!(refusal.contains("Nothing was downloaded"), "{refusal}");

        // Counterpart: a writable directory is not refused, and the probe
        // leaves nothing behind.
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            super::cannot_replace(&dir.path().join("birdnet-behavior")),
            None
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
