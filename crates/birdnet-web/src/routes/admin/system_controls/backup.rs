//! Full tar.gz backup download and restore upload.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};

use crate::state::AppState;

pub(super) async fn full_backup(State(state): State<AppState>) -> axum::response::Response {
    let db_path = state.db_path().to_path_buf();
    let rec_dir = state.recording_dir();
    let base_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        let tmp = std::env::temp_dir().join(format!(
            "birdnet-backup-{}.tar.gz",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));

        let mut args = vec!["czf".to_string(), tmp.to_string_lossy().to_string()];

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

        let conf_path = base_dir.join("birdnet.conf");
        if conf_path.exists() {
            args.push("-C".to_string());
            args.push(base_dir.to_string_lossy().to_string());
            args.push("birdnet.conf".to_string());
        }

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

pub(super) async fn restore_backup(
    State(state): State<AppState>,
    mut multipart: axum::extract::Multipart,
) -> Html<String> {
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
        let tmp = std::env::temp_dir().join(format!(
            "birdnet-restore-{}.tar.gz",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        std::fs::write(&tmp, &data).map_err(|e| format!("failed to write temp file: {e}"))?;

        let list_output = std::process::Command::new("tar")
            .args(["tzf", &tmp.to_string_lossy()])
            .output()
            .map_err(|e| format!("failed to list archive: {e}"))?;

        if !list_output.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err("invalid archive (tar returned error)".to_string());
        }

        let listing = String::from_utf8_lossy(&list_output.stdout);
        let has_db = listing.lines().any(|l| {
            std::path::Path::new(l)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("db"))
        });

        if !has_db {
            let _ = std::fs::remove_file(&tmp);
            return Err("archive does not contain a database file".to_string());
        }

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
