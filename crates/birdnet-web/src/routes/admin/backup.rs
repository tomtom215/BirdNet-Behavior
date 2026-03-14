//! Database backup management routes.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET  /admin/system/backups`          | List available backup files |
//! | `GET  /admin/system/backups/{name}`   | Download a backup file |
//! | `DELETE /admin/system/backups/{name}` | Delete a backup file |

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use tokio_util::io::ReaderStream;

use crate::state::AppState;

/// Mount backup management routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/system/backups", get(list_backups))
        .route(
            "/admin/system/backups/{name}",
            get(download_backup).delete(delete_backup),
        )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the backup directory from state: sibling `backups/` of the DB file.
fn backup_dir(state: &AppState) -> std::path::PathBuf {
    state
        .db_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups")
}

/// Validate that a filename is safe (no path traversal, `.db` extension).
fn is_safe_backup_name(name: &str) -> bool {
    !name.contains('/') && !name.contains('\\') && !name.contains("..") && name.ends_with(".db")
}

/// Basic HTML escape for untrusted strings rendered into HTML.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// GET /admin/system/backups — list backup files
// ---------------------------------------------------------------------------

async fn list_backups(State(state): State<AppState>) -> Html<String> {
    let dir = backup_dir(&state);

    let entries = tokio::task::spawn_blocking(move || -> Vec<BackupEntry> {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut entries: Vec<BackupEntry> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".db"))
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let meta = e.metadata().ok()?;
                let size = meta.len();
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                Some(BackupEntry {
                    name,
                    size,
                    modified_secs: modified,
                })
            })
            .collect();
        entries.sort_by(|a, b| b.modified_secs.cmp(&a.modified_secs));
        entries
    })
    .await
    .unwrap_or_default();

    Html(render_backup_list(&entries))
}

struct BackupEntry {
    name: String,
    size: u64,
    modified_secs: u64,
}

fn render_backup_list(entries: &[BackupEntry]) -> String {
    let rows = if entries.is_empty() {
        "<tr><td colspan=\"3\" style=\"color:#64748b;text-align:center;\">No backups found</td></tr>".to_string()
    } else {
        entries.iter().map(|e| {
            let name_esc = html_escape(&e.name);
            let size_str = format_bytes(e.size);
            let date_str = format_unix_ts(e.modified_secs);
            format!(
                r#"<tr>
                  <td style="font-family:monospace;font-size:.8rem;">{name_esc}</td>
                  <td style="color:#94a3b8;">{size_str}</td>
                  <td style="color:#64748b;">{date_str}</td>
                  <td style="display:flex;gap:.5rem;">
                    <a href="/admin/system/backups/{name_esc}"
                       download="{name_esc}"
                       style="color:#38bdf8;font-size:.8rem;text-decoration:none;">Download</a>
                    <button hx-delete="/admin/system/backups/{name_esc}"
                            hx-target="closest tr"
                            hx-swap="outerHTML"
                            hx-confirm="Delete {name_esc}?"
                            style="background:none;border:none;color:#f87171;cursor:pointer;font-size:.8rem;">
                      Delete
                    </button>
                  </td>
                </tr>"#
            )
        }).collect::<String>()
    };

    format!(
        r#"<div class="card">
          <div class="section-title">Database Backups</div>
          <table style="width:100%;border-collapse:collapse;font-size:.875rem;">
            <thead>
              <tr style="border-bottom:1px solid #334155;color:#64748b;text-align:left;">
                <th style="padding:.5rem;">Filename</th>
                <th style="padding:.5rem;">Size</th>
                <th style="padding:.5rem;">Created</th>
                <th style="padding:.5rem;">Actions</th>
              </tr>
            </thead>
            <tbody>{rows}</tbody>
          </table>
        </div>"#
    )
}

// ---------------------------------------------------------------------------
// GET /admin/system/backups/{name} — download backup file
// ---------------------------------------------------------------------------

async fn download_backup(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    if !is_safe_backup_name(&name) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let path = backup_dir(&state).join(&name);

    // Verify the canonical path is still inside the backup directory.
    let backup_dir_canon = match backup_dir(&state).canonicalize() {
        Ok(p) => p,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let file_canon = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if !file_canon.starts_with(&backup_dir_canon) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let file = match tokio::fs::File::open(&file_canon).await {
        Ok(f) => f,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let stream = ReaderStream::new(file);
    let content_disposition = format!("attachment; filename=\"{name}\"");

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .header(header::CONTENT_LENGTH, size)
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// ---------------------------------------------------------------------------
// DELETE /admin/system/backups/{name} — delete a backup
// ---------------------------------------------------------------------------

async fn delete_backup(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    if !is_safe_backup_name(&name) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let path = backup_dir(&state).join(&name);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => {
            tracing::info!(file = %name, "backup deleted");
            // Return empty (HTMX swap removes the row)
            StatusCode::OK.into_response()
        }
        Err(e) => {
            tracing::warn!(file = %name, error = %e, "failed to delete backup");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn format_bytes(bytes: u64) -> String {
    const GB: u64 = 1_073_741_824;
    const MB: u64 = 1_048_576;
    const KB: u64 = 1_024;
    #[allow(clippy::cast_precision_loss)]
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format a Unix timestamp as YYYY-MM-DD HH:MM UTC (no chrono dependency).
fn format_unix_ts(secs: u64) -> String {
    // Days since epoch → Gregorian date via algorithm by Henry S. Warren Jr.
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;

    // Convert days since Unix epoch to Gregorian date.
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_backup_name_valid() {
        assert!(is_safe_backup_name("birds-backup-2026-03-13.db"));
    }

    #[test]
    fn safe_backup_name_rejects_traversal() {
        assert!(!is_safe_backup_name("../etc/passwd"));
        assert!(!is_safe_backup_name("backups/../../passwd.db"));
    }

    #[test]
    fn safe_backup_name_rejects_non_db() {
        assert!(!is_safe_backup_name("birds.txt"));
        assert!(!is_safe_backup_name("birds.db.sh"));
    }

    #[test]
    fn format_bytes_sizes() {
        assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
        assert_eq!(format_bytes(1_024), "1.0 KB");
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn format_unix_ts_epoch() {
        // 2026-03-15 00:00:00 UTC = 1_773_532_800  (verified by algorithm output)
        let ts = format_unix_ts(1_773_532_800);
        assert!(ts.starts_with("2026-03-15"), "got: {ts}");
        // Epoch itself should be 1970-01-01
        let epoch = format_unix_ts(0);
        assert!(epoch.starts_with("1970-01-01"), "got: {epoch}");
    }

    #[test]
    fn html_escape_xss() {
        let escaped = html_escape("<script>alert(1)</script>");
        assert!(!escaped.contains('<'));
        assert!(escaped.contains("&lt;"));
    }

    #[test]
    fn render_backup_list_empty() {
        let html = render_backup_list(&[]);
        assert!(html.contains("No backups found"));
    }

    #[test]
    fn render_backup_list_with_entry() {
        let entries = vec![BackupEntry {
            name: "birds-2026-03-13.db".into(),
            size: 1_048_576,
            modified_secs: 1_773_532_800,
        }];
        let html = render_backup_list(&entries);
        assert!(html.contains("birds-2026-03-13.db"));
        assert!(html.contains("1.0 MB"));
        assert!(html.contains("Download"));
        assert!(html.contains("Delete"));
    }
}
