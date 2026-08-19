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

/// Does this filename have the shape of a backup this application writes?
///
/// [`birdnet_db::resilience::backup_database`] names its snapshots
/// `{db_name}.backup.{unix_secs}` — for the stock install, `birds.db.backup.
/// 1733400000`. Crucially the *extension* is the timestamp, not `db`, so the
/// obvious `ends_with(".db")` test matches **nothing** a station ever produces.
/// That single wrong assumption made `/admin/system/backups` report "No backups
/// found" on every station, and made download and delete reject every real file
/// with a 400 — the operator's whole backup surface, silently inert. (The same
/// mistake once disabled the maintenance pruner; this is the last place that
/// held it.)
///
/// A plain `.db` name is still accepted so a hand-placed or restored-from-
/// elsewhere snapshot remains downloadable.
pub(super) fn is_backup_file_name(name: &str) -> bool {
    if let Some((prefix, suffix)) = name.rsplit_once(".backup.") {
        return !prefix.is_empty()
            && !suffix.is_empty()
            && suffix.bytes().all(|b| b.is_ascii_digit());
    }
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("db"))
}

/// Validate that a filename is safe (no path traversal, a recognised backup
/// shape, and composed only of an ASCII allowlist that carries no
/// HTTP-header-significant bytes — so the name can be interpolated into a
/// `Content-Disposition` header without worrying about `"`, CR/LF, or control
/// characters).
fn is_safe_backup_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 || name.contains("..") {
        return false;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return false;
    }
    is_backup_file_name(name)
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
            .filter_map(std::result::Result::ok)
            .filter(|e| is_backup_file_name(&e.file_name().to_string_lossy()))
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
        entries.sort_by_key(|e| std::cmp::Reverse(e.modified_secs));
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
        "<tr><td colspan=\"3\" class=\"bk-empty\">No backups found</td></tr>".to_string()
    } else {
        {
            use std::fmt::Write as _;
            let mut buf = String::new();
            for e in entries {
                let name_esc = html_escape(&e.name);
                let size_str = format_bytes(e.size);
                let date_str = format_unix_ts(e.modified_secs);
                let _ = write!(
                    buf,
                    r#"<tr>
                  <td class="bk-name">{name_esc}</td>
                  <td class="bk-size">{size_str}</td>
                  <td class="bk-date">{date_str}</td>
                  <td class="bk-actions">
                    <a href="/admin/system/backups/{name_esc}"
                       download="{name_esc}"
                       class="bk-download">Download</a>
                    <button hx-delete="/admin/system/backups/{name_esc}"
                            hx-target="closest tr"
                            hx-swap="outerHTML"
                            hx-confirm="Delete {name_esc}?"
                            data-confirm-action="hx-delete"
                            data-confirm-url="/admin/system/backups/{name_esc}"
                            data-confirm-title="Delete backup"
                            data-confirm-body="Delete {name_esc}?"
                            data-confirm-confirm-label="Delete"
                            data-confirm-style="danger"
                            class="bk-delete">
                      Delete
                    </button>
                  </td>
                </tr>"#
                );
            }
            buf
        }
    };

    format!(
        r#"<div class="card">
          <div class="section-title">Database Backups</div>
          <table class="bk-table">
            <thead>
              <tr class="bk-head-row">
                <th class="bk-th">Filename</th>
                <th class="bk-th">Size</th>
                <th class="bk-th">Created</th>
                <th class="bk-th">Actions</th>
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
    let Ok(backup_dir_canon) = backup_dir(&state).canonicalize() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(file_canon) = path.canonicalize() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !file_canon.starts_with(&backup_dir_canon) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let Ok(file) = tokio::fs::File::open(&file_canon).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let size = file.metadata().await.map_or(0, |m| m.len());
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

    // Convert days since Unix epoch to a Gregorian date. One shared
    // implementation, in `birdnet-core::civil`; this was one of nine copies.
    #[allow(clippy::cast_possible_wrap)]
    let (y, m, d) = birdnet_core::civil::civil_from_days(days as i64);

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
    fn safe_backup_name_accepts_the_real_backup_shape() {
        // The regression: this is exactly what `resilience::backup_database`
        // writes, and it was rejected with a 400 by download and delete alike.
        assert!(is_safe_backup_name("birds.db.backup.1733400000"));
        assert!(is_backup_file_name("birds.db.backup.1733400000"));
        // A renamed database keeps working.
        assert!(is_safe_backup_name("BirdDB.db.backup.1700000001"));
    }

    #[test]
    fn backup_name_requires_a_numeric_timestamp() {
        // Counter-test: `.backup.` alone must not open the directory up to
        // arbitrary names.
        assert!(!is_backup_file_name("birds.db.backup.notatimestamp"));
        assert!(!is_backup_file_name("birds.db.backup."));
        assert!(!is_backup_file_name(".backup.123"));
    }

    #[test]
    fn safe_backup_name_rejects_traversal() {
        assert!(!is_safe_backup_name("../etc/passwd"));
        assert!(!is_safe_backup_name("backups/../../passwd.db"));
        // Traversal stays rejected even wearing the backup shape.
        assert!(!is_safe_backup_name("../birds.db.backup.1733400000"));
        assert!(!is_safe_backup_name("sub/birds.db.backup.1733400000"));
    }

    #[test]
    fn safe_backup_name_rejects_non_db() {
        assert!(!is_safe_backup_name("birds.txt"));
        assert!(!is_safe_backup_name("birds.db.sh"));
        // Header-significant bytes stay rejected.
        assert!(!is_safe_backup_name("birds.db.backup.1\r\nX-Evil: 1"));
        assert!(!is_safe_backup_name("bir\"ds.db"));
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
    fn a_real_backup_is_listable_downloadable_and_deletable() {
        // Couples the writer to the reader: whatever
        // `resilience::backup_database` actually names its file must satisfy
        // the admin surface's filter and its safety check. If either side's
        // naming ever drifts again, this fails instead of the operator
        // silently seeing an empty backup list.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
        }
        let backups = tmp.path().join("backups");
        let written = birdnet_db::resilience::backup_database(&db, &backups).unwrap();
        let name = written.file_name().unwrap().to_string_lossy().into_owned();

        assert!(
            is_backup_file_name(&name),
            "the listing filter must match what we actually write: {name}"
        );
        assert!(
            is_safe_backup_name(&name),
            "download/delete must accept what we actually write: {name}"
        );
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
