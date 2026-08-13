//! Full tar.gz backup download and restore upload.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};

use crate::state::AppState;

/// Set while a restore is unpacking, so a second upload cannot run concurrently.
///
/// A restore streams an archive over the live database and recordings directory.
/// Two of them interleaving would corrupt both, and the UI makes that easy to
/// trigger: the upload is multi-gigabyte and silent while it runs, which is
/// exactly the condition that gets a button clicked twice. htmx does not dedupe
/// in-flight requests unless told to (`hx-sync`), and a UI guard cannot bind a
/// client that simply POSTs twice, so the invariant is enforced here as well.
static RESTORE_IN_PROGRESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Clears [`RESTORE_IN_PROGRESS`] however the handler leaves — including an
/// early return on a malformed upload, which is most of its exit paths.
struct RestoreGuard;

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        RESTORE_IN_PROGRESS.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

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

        if db_path.exists()
            && let Some(name) = db_path.file_name()
        {
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

        let conf_path = base_dir.join("birdnet.conf");
        if conf_path.exists() {
            args.push("-C".to_string());
            args.push(base_dir.to_string_lossy().to_string());
            args.push("birdnet.conf".to_string());
        }

        if rec_dir.exists()
            && let Some(name) = rec_dir.file_name()
        {
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

            let size = file.metadata().await.map_or(0, |m| m.len());
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
    use tokio::io::AsyncWriteExt as _;

    if RESTORE_IN_PROGRESS
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        tracing::warn!("refused a second restore while one was already running");
        return Html(
            r#"<p class="ctl-err">A restore is already running. Wait for it to finish before starting another.</p>"#
                .to_string(),
        );
    }
    let _restore_guard = RestoreGuard;

    // Stream the uploaded archive straight to a temp file. A full backup
    // (database + recordings) routinely runs to many GB — far past axum's 2 MiB
    // default body limit — and the previous code buffered the whole thing in
    // memory (twice, via `field.bytes()` + `to_vec()`), which rejected real
    // backups and would OOM a Pi. Streaming keeps memory flat regardless of
    // archive size. NamedTempFile auto-removes the file on drop (even on an
    // early return), replacing the previous manual cleanup.
    let Ok(Ok(tmp)) =
        tokio::task::spawn_blocking(|| tempfile::Builder::new().suffix(".tar.gz").tempfile()).await
    else {
        return Html(
            r#"<p class="ctl-err">Internal error: could not allocate a temp file.</p>"#.to_string(),
        );
    };
    let tmp_path = tmp.path().to_path_buf();

    let mut bytes_written: u64 = 0;
    let mut found = false;
    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return Html(format!(r#"<p class="ctl-err">Upload failed: {e}</p>"#)),
        };
        if field.name() != Some("backup") {
            continue;
        }
        let mut out = match tokio::fs::File::create(&tmp_path).await {
            Ok(f) => f,
            Err(e) => return Html(format!(r#"<p class="ctl-err">Internal error: {e}</p>"#)),
        };
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if let Err(e) = out.write_all(&chunk).await {
                        return Html(format!(r#"<p class="ctl-err">Internal error: {e}</p>"#));
                    }
                    bytes_written += chunk.len() as u64;
                }
                Ok(None) => break,
                Err(e) => return Html(format!(r#"<p class="ctl-err">Upload failed: {e}</p>"#)),
            }
        }
        if let Err(e) = out.flush().await {
            return Html(format!(r#"<p class="ctl-err">Internal error: {e}</p>"#));
        }
        found = true;
        break;
    }

    if !found || bytes_written == 0 {
        return Html(r#"<p class="ctl-err">No backup file uploaded.</p>"#.to_string());
    }

    let db_path = state.db_path().to_path_buf();
    let target_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        // Keep the NamedTempFile alive for the duration of the tar operations;
        // it unlinks the archive automatically when this closure returns.
        let _archive = tmp;
        let tmp_str = tmp_path.to_string_lossy().to_string();

        let list_output = std::process::Command::new("tar")
            .args(["tzf", &tmp_str])
            .output()
            .map_err(|e| format!("failed to list archive: {e}"))?;

        if !list_output.status.success() {
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
            return Err("archive does not contain a database file".to_string());
        }

        let status = std::process::Command::new("tar")
            .args(["xzf", &tmp_str, "-C", &target_dir.to_string_lossy()])
            .status()
            .map_err(|e| format!("failed to extract: {e}"))?;

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
        Ok(Ok(msg)) => Html(format!(r#"<p class="ctl-ok">{msg}</p>"#)),
        Ok(Err(e)) => Html(format!(r#"<p class="ctl-err">Restore failed: {e}</p>"#)),
        Err(e) => Html(format!(r#"<p class="ctl-err">Internal error: {e}</p>"#)),
    }
}

#[cfg(test)]
mod tests {
    use super::{RESTORE_IN_PROGRESS, RestoreGuard};
    use std::sync::atomic::Ordering;

    fn claim() -> bool {
        RESTORE_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// The invariant the handler relies on: one restore at a time, and the slot
    /// is released however the handler leaves — including the early returns on a
    /// malformed upload, which is what a `Drop` guard buys over a manual reset.
    #[test]
    fn a_second_restore_cannot_start_while_one_is_running() {
        assert!(claim(), "the first restore claims the slot");
        let guard = RestoreGuard;
        assert!(
            !claim(),
            "a concurrent restore must be refused, not run over the live database"
        );
        drop(guard);
        assert!(claim(), "the slot is released when the handler returns");
        RESTORE_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}
