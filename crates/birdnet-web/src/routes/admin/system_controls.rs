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
        .route("/admin/system/restore", post(restore_backup))
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

// ---------------------------------------------------------------------------
// POST /admin/system/restore — restore from tar.gz backup upload
// ---------------------------------------------------------------------------

async fn restore_backup(
    State(state): State<AppState>,
    mut multipart: axum::extract::Multipart,
) -> Html<String> {
    // Read the uploaded file.
    let mut file_data: Option<Vec<u8>> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("backup") {
            match field.bytes().await {
                Ok(bytes) => {
                    file_data = Some(bytes.to_vec());
                }
                Err(e) => {
                    return Html(format!(
                        r#"<p style="color:#f87171;">Upload failed: {e}</p>"#
                    ));
                }
            }
        }
    }

    let Some(data) = file_data else {
        return Html(r#"<p style="color:#f87171;">No backup file uploaded.</p>"#.to_string());
    };

    let db_path = state.db_path().to_path_buf();
    let target_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        // Write upload to temp file.
        let tmp = std::env::temp_dir().join(format!(
            "birdnet-restore-{}.tar.gz",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        std::fs::write(&tmp, &data).map_err(|e| format!("failed to write temp file: {e}"))?;

        // List archive contents to verify it looks like a backup.
        let list_output = std::process::Command::new("tar")
            .args(["tzf", &tmp.to_string_lossy()])
            .output()
            .map_err(|e| format!("failed to list archive: {e}"))?;

        if !list_output.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err("invalid archive (tar returned error)".to_string());
        }

        let listing = String::from_utf8_lossy(&list_output.stdout);
        let has_db = listing.lines().any(|l| l.ends_with(".db"));

        if !has_db {
            let _ = std::fs::remove_file(&tmp);
            return Err("archive does not contain a database file".to_string());
        }

        // Extract to target directory.
        let status = std::process::Command::new("tar")
            .args([
                "xzf",
                &tmp.to_string_lossy(),
                "-C",
                &target_dir.to_string_lossy(),
            ])
            .status()
            .map_err(|e| format!("failed to extract: {e}"))?;

        let _ = std::fs::remove_file(&tmp);

        if status.success() {
            Ok(
                "Backup restored successfully. Restart the server to load the restored data."
                    .to_string(),
            )
        } else {
            Err(format!("tar extract failed with status {status}"))
        }
    })
    .await;

    match result {
        Ok(Ok(msg)) => Html(format!(r#"<p style="color:#4ade80;">{msg}</p>"#)),
        Ok(Err(e)) => Html(format!(
            r#"<p style="color:#f87171;">Restore failed: {e}</p>"#
        )),
        Err(e) => Html(format!(
            r#"<p style="color:#f87171;">Internal error: {e}</p>"#
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn module_compiles() {
        // Verifies the module compiles without errors.
    }
}
