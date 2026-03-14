//! System control routes for data management.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `POST /admin/system/clear-detections` | Delete all detections + notification log |
//! | `POST /admin/system/clear-extracted`  | Remove all extracted audio clips |

use axum::extract::State;
use axum::response::Html;
use axum::{Router, routing::post};

use crate::state::AppState;

/// Mount system control routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/system/clear-detections", post(clear_detections))
        .route("/admin/system/clear-extracted", post(clear_extracted))
        .route("/admin/system/backup/full", axum::routing::get(full_backup))
}

// ---------------------------------------------------------------------------
// POST /admin/system/clear-detections
// ---------------------------------------------------------------------------

async fn clear_detections(State(state): State<AppState>) -> Html<String> {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let det = conn.execute("DELETE FROM detections", []);
            let notif = conn.execute("DELETE FROM notification_log", []);
            match (det, notif) {
                (Ok(d), Ok(n)) => Ok(format!(
                    "Cleared {d} detections and {n} notification log entries."
                )),
                (Err(e), _) | (_, Err(e)) => Err(e.to_string()),
            }
        })
    })
    .await;

    match result {
        Ok(Ok(msg)) => Html(format!(r#"<p style="color:#4ade80;">{msg}</p>"#)),
        Ok(Err(e)) => Html(format!(
            r#"<p style="color:#f87171;">Failed to clear data: {e}</p>"#
        )),
        Err(e) => Html(format!(
            r#"<p style="color:#f87171;">Internal error: {e}</p>"#
        )),
    }
}

// ---------------------------------------------------------------------------
// POST /admin/system/clear-extracted
// ---------------------------------------------------------------------------

async fn clear_extracted(State(state): State<AppState>) -> Html<String> {
    let rec_dir = state.recording_dir();

    let result = tokio::task::spawn_blocking(move || {
        if !rec_dir.exists() {
            return Ok::<String, String>("No extracted recordings directory found.".to_string());
        }
        let mut removed = 0u64;
        let mut errors = 0u64;
        if let Ok(entries) = std::fs::read_dir(&rec_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    match std::fs::remove_file(&path) {
                        Ok(()) => removed += 1,
                        Err(_) => errors += 1,
                    }
                } else if path.is_dir() {
                    match std::fs::remove_dir_all(&path) {
                        Ok(()) => removed += 1,
                        Err(_) => errors += 1,
                    }
                }
            }
        }
        if errors > 0 {
            Ok(format!("Removed {removed} items ({errors} errors)."))
        } else {
            Ok(format!(
                "Removed {removed} items from recordings directory."
            ))
        }
    })
    .await;

    match result {
        Ok(Ok(msg)) => Html(format!(r#"<p style="color:#4ade80;">{msg}</p>"#)),
        Ok(Err(e)) => Html(format!(r#"<p style="color:#f87171;">Failed: {e}</p>"#)),
        Err(e) => Html(format!(
            r#"<p style="color:#f87171;">Internal error: {e}</p>"#
        )),
    }
}

// ---------------------------------------------------------------------------
// GET /admin/system/backup/full — download full tar.gz backup
// ---------------------------------------------------------------------------

async fn full_backup(State(state): State<AppState>) -> axum::response::Response {
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;

    let db_path = state.db_path().to_path_buf();
    let rec_dir = state.recording_dir();
    let base_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    // Build the tar.gz in a blocking task, then stream the file.
    let result = tokio::task::spawn_blocking(move || {
        let tmp = std::env::temp_dir().join(format!(
            "birdnet-backup-{}.tar.gz",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));

        // Use tar command to create the archive
        let mut args = vec!["czf".to_string(), tmp.to_string_lossy().to_string()];

        // Add db file if it exists
        if db_path.exists() {
            if let Some(name) = db_path.file_name() {
                args.push("-C".to_string());
                args.push(
                    db_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .to_string_lossy()
                        .to_string(),
                );
                args.push(name.to_string_lossy().to_string());
            }
        }

        // Add birdnet.conf if it exists
        let conf_path = base_dir.join("birdnet.conf");
        if conf_path.exists() {
            args.push("-C".to_string());
            args.push(base_dir.to_string_lossy().to_string());
            args.push("birdnet.conf".to_string());
        }

        // Add recordings dir if it exists
        if rec_dir.exists() {
            if let Some(name) = rec_dir.file_name() {
                args.push("-C".to_string());
                args.push(
                    rec_dir
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .to_string_lossy()
                        .to_string(),
                );
                args.push(name.to_string_lossy().to_string());
            }
        }

        let status = std::process::Command::new("tar").args(&args).status();

        match status {
            Ok(s) if s.success() => Ok(tmp),
            Ok(s) => Err(format!("tar exited with status {s}")),
            Err(e) => Err(format!("failed to run tar: {e}")),
        }
    })
    .await;

    match result {
        Ok(Ok(tmp_path)) => {
            let file = match tokio::fs::File::open(&tmp_path).await {
                Ok(f) => f,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("failed to open backup: {e}"),
                    )
                        .into_response();
                }
            };

            let size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
            let stream = tokio_util::io::ReaderStream::new(file);

            // Clean up temp file after streaming (best-effort)
            let tmp_clone = tmp_path.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                let _ = tokio::fs::remove_file(&tmp_clone).await;
            });

            axum::response::Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/gzip")
                .header(
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"birdnet-backup.tar.gz\"",
                )
                .header(header::CONTENT_LENGTH, size)
                .body(axum::body::Body::from_stream(stream))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("backup failed: {e}"),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("internal error: {e}"),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn module_compiles() {
        // Verifies the module compiles without errors.
    }
}
