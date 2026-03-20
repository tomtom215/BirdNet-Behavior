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
        .route("/admin/system/service/restart", post(service_restart))
        .route(
            "/admin/system/service/status",
            axum::routing::get(service_status),
        )
        .route(
            "/admin/system/update/check",
            axum::routing::get(check_update),
        )
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

// ---------------------------------------------------------------------------
// POST /admin/system/service/restart — graceful restart of the binary
// ---------------------------------------------------------------------------

/// Restart the birdnet-behavior service.
///
/// Strategy (in order of preference):
/// 1. If running as a systemd service (`INVOCATION_ID` set), attempt `systemctl restart`
/// 2. Otherwise, send SIGTERM to self (systemd with `Restart=on-failure` will restart it)
///
/// BirdNET-Pi equivalent: the 9 individual `systemctl restart birdnet_*` buttons
/// in the admin web interface.
async fn service_restart() -> Html<String> {
    let result = tokio::task::spawn_blocking(|| {
        // Check if we are running under systemd.
        let under_systemd = std::env::var("INVOCATION_ID").is_ok()
            || std::env::var("JOURNAL_STREAM").is_ok();

        if under_systemd {
            // Try systemctl restart of our own unit.
            let status = std::process::Command::new("systemctl")
                .args(["restart", "birdnet-behavior"])
                .status();
            match status {
                Ok(s) if s.success() => {
                    return Ok::<String, String>("Service restart initiated via systemctl.".to_string())
                }
                Ok(s) => {
                    tracing::warn!(status = %s, "systemctl restart returned non-zero, falling back to SIGTERM");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "systemctl not available, falling back to SIGTERM");
                }
            }
        }

        // Fallback: send SIGTERM to self via the system kill command.
        // systemd (Restart=on-failure) or a process manager will restart us.
        let pid = std::process::id().to_string();
        tracing::info!(%pid, "sending SIGTERM to self for graceful restart");
        // Spawn a thread to deliver signal after response is sent.
        let pid_clone = pid.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &pid_clone])
                .status();
        });
        Ok("Restart signal sent. Service will restart momentarily.".to_string())
    })
    .await;

    match result {
        Ok(Ok(msg)) => Html(format!(
            r#"<p style="color:#4ade80;">{msg} Reconnect in a few seconds.</p>"#
        )),
        Ok(Err(e)) => Html(format!(
            r#"<p style="color:#f87171;">Restart failed: {e}</p>"#
        )),
        Err(e) => Html(format!(
            r#"<p style="color:#f87171;">Internal error: {e}</p>"#
        )),
    }
}

// ---------------------------------------------------------------------------
// GET /admin/system/service/status — service process status
// ---------------------------------------------------------------------------

/// Return HTML with current process status (PID, uptime, memory, version).
async fn service_status() -> Html<String> {
    let pid = std::process::id();
    let uptime_secs = get_process_uptime_secs(pid);
    let memory_mb = get_process_memory_mb(pid);
    let service_active = check_systemd_service_active("birdnet-behavior");
    let version = env!("CARGO_PKG_VERSION");

    let uptime_str = if uptime_secs >= 3600 {
        format!("{}h {}m", uptime_secs / 3600, (uptime_secs % 3600) / 60)
    } else if uptime_secs >= 60 {
        format!("{}m {}s", uptime_secs / 60, uptime_secs % 60)
    } else {
        format!("{uptime_secs}s")
    };

    let systemd_badge = if service_active {
        r#"<span style="color:#4ade80;font-weight:600;">● active</span>"#
    } else {
        r#"<span style="color:#94a3b8;">○ not managed by systemd</span>"#
    };

    Html(format!(
        r#"<table style="width:100%;border-collapse:collapse;font-size:.875rem;">
          <tr><td style="color:#64748b;padding:.25rem 0;">Version</td><td style="font-weight:600;">v{version}</td></tr>
          <tr><td style="color:#64748b;padding:.25rem 0;">PID</td><td>{pid}</td></tr>
          <tr><td style="color:#64748b;padding:.25rem 0;">Uptime</td><td>{uptime_str}</td></tr>
          <tr><td style="color:#64748b;padding:.25rem 0;">Memory (RSS)</td><td>{memory_mb:.1} MB</td></tr>
          <tr><td style="color:#64748b;padding:.25rem 0;">systemd service</td><td>{systemd_badge}</td></tr>
        </table>"#
    ))
}

fn get_process_uptime_secs(_pid: u32) -> u64 {
    // Read process start time from /proc/self/stat on Linux.
    // Field 22 (0-indexed: 21) is starttime in jiffies since boot.
    #[cfg(target_os = "linux")]
    {
        if let (Ok(stat), Ok(uptime_str)) = (
            std::fs::read_to_string("/proc/self/stat"),
            std::fs::read_to_string("/proc/uptime"),
        ) {
            // Get Hz via `getconf CLK_TCK` (avoids libc dependency).
            let hz: u64 = std::process::Command::new("getconf")
                .arg("CLK_TCK")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(100);

            if let (Some(start_field), Some(uptime_field)) = (
                stat.split_whitespace().nth(21),
                uptime_str.split_whitespace().next(),
            ) {
                if let (Ok(start_jiffies), Ok(sys_uptime)) =
                    (start_field.parse::<u64>(), uptime_field.parse::<f64>())
                {
                    if hz > 0 {
                        let proc_uptime = sys_uptime - (start_jiffies / hz) as f64;
                        return proc_uptime.max(0.0) as u64;
                    }
                }
            }
        }
    }
    0
}

fn get_process_memory_mb(pid: u32) -> f64 {
    #[cfg(target_os = "linux")]
    {
        let status_path = format!("/proc/{pid}/status");
        if let Ok(content) = std::fs::read_to_string(&status_path) {
            for line in content.lines() {
                if line.starts_with("VmRSS:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<f64>() {
                            return kb / 1024.0;
                        }
                    }
                }
            }
        }
    }
    let _ = pid;
    0.0
}

fn check_systemd_service_active(service: &str) -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", service])
        .status()
        .is_ok_and(|s| s.success())
}

// ---------------------------------------------------------------------------
// GET /admin/system/update/check — check GitHub for newer release
// ---------------------------------------------------------------------------

/// Check GitHub Releases API for a newer version of birdnet-behavior.
///
/// Returns JSON: `{ "current": "0.3.0", "latest": "0.4.0", "update_available": true, "release_url": "…" }`
///
/// BirdNET-Pi equivalent: `update_birdnet.sh` (git pull + pip reinstall).
async fn check_update() -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let current = env!("CARGO_PKG_VERSION");

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(format!("birdnet-behavior/{current}"))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let api_url = "https://api.github.com/repos/tomtom215/BirdNet-Behavior/releases/latest";
    let resp = client.get(api_url).send().await;

    match resp {
        Ok(r) if r.status().is_success() => {
            #[derive(serde::Deserialize)]
            struct Release {
                tag_name: String,
                html_url: String,
                published_at: Option<String>,
            }
            match r.json::<Release>().await {
                Ok(release) => {
                    let latest = release.tag_name.trim_start_matches('v').to_string();
                    let update_available = is_newer_version(&latest, current);
                    let published = release.published_at.unwrap_or_default();
                    let html = if update_available {
                        format!(
                            r#"<div style="color:#4ade80;font-weight:600;">
                              ⬆ Update available: v{latest} (published {published})<br>
                              <a href="{url}" target="_blank" rel="noopener"
                                 style="color:#38bdf8;">View release notes →</a><br>
                              <span style="color:#94a3b8;font-size:.8rem;">
                                Run: <code>curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash</code>
                              </span>
                            </div>"#,
                            url = release.html_url
                        )
                    } else {
                        format!(
                            r#"<div style="color:#94a3b8;">
                              ✓ Up to date (v{current}). Latest: v{latest} ({published}).
                            </div>"#
                        )
                    };
                    Html(html).into_response()
                }
                Err(e) => Html(format!(r#"<p style="color:#f87171;">Parse error: {e}</p>"#))
                    .into_response(),
            }
        }
        Ok(r) => Html(format!(
            r#"<p style="color:#f87171;">GitHub API returned {}</p>"#,
            r.status()
        ))
        .into_response(),
        Err(e) => Html(format!(
            r#"<p style="color:#f87171;">Network error: {e}</p>"#
        ))
        .into_response(),
    }
}

/// Compare version strings (simple semver-like: "0.4.0" > "0.3.2").
fn is_newer_version(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> [u32; 3] {
        let mut parts = v.split('.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        [major, minor, patch]
    };
    parse(latest) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer_version("0.4.0", "0.3.2"));
        assert!(!is_newer_version("0.3.2", "0.4.0"));
        assert!(!is_newer_version("0.3.2", "0.3.2"));
        assert!(is_newer_version("1.0.0", "0.99.99"));
    }
}
