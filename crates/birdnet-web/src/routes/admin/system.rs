//! Admin system-management routes.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET  /admin/system`        | System status page (disk, DB, processes) |
//! | `POST /admin/system/backup` | Trigger an immediate database backup |
//! | `GET  /admin/system/status` | HTMX partial — live system status |

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Router, routing::get};

use birdnet_core::audio::capture::{disk_usage, recording_stats};
use birdnet_db::resilience::backup_database;

use crate::state::AppState;
use crate::system_info;

/// Mount system routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/system", get(system_page))
        .route("/admin/system/backup", axum::routing::post(trigger_backup))
        .route("/admin/system/status", get(system_status_partial))
}

// ---------------------------------------------------------------------------
// GET /admin/system
// ---------------------------------------------------------------------------

async fn system_page(State(state): State<AppState>) -> Html<String> {
    let status_html = render_status_partial(&state).await;
    Html(format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width,initial-scale=1.0">
    <title>System — BirdNet-Behavior Admin</title>
    <script src="/static/htmx.min.js"></script>
    <style>
      body {{ background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; }}
      .container {{ max-width:900px; margin:0 auto; padding:2rem 1rem; }}
      nav a {{ color:#94a3b8; text-decoration:none; margin-right:1.5rem; }}
      nav a:hover, nav a.active {{ color:#38bdf8; }}
      .card {{ background:#1e293b; border:1px solid #334155; border-radius:.75rem;
               padding:1.5rem; margin-bottom:1.5rem; }}
      .section-title {{ font-size:1.1rem; font-weight:600; color:#38bdf8;
                        margin-bottom:.75rem; }}
      .stat-grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(200px,1fr));
                    gap:1rem; }}
      .stat-card {{ background:#0f172a; border:1px solid #1e293b; border-radius:.5rem;
                    padding:1rem; }}
      .stat-label {{ font-size:.75rem; color:#64748b; text-transform:uppercase; }}
      .stat-value {{ font-size:1.4rem; font-weight:700; margin-top:.25rem; }}
      .btn {{ padding:.5rem 1.5rem; border-radius:.375rem; border:none; cursor:pointer;
               font-weight:600; font-size:.875rem; }}
      .btn-secondary {{ background:#1e293b; color:#e2e8f0; border:1px solid #334155; }}
      .btn-secondary:hover {{ border-color:#38bdf8; color:#38bdf8; }}
      .btn-danger {{ background:#7f1d1d; color:#fca5a5; border:1px solid #991b1b; }}
      .badge-ok {{ color:#4ade80; }} .badge-warn {{ color:#fbbf24; }}
      .badge-crit {{ color:#f87171; }}
    </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem;padding:1rem 0;border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/admin/settings">Settings</a>
    <a href="/admin/migrate">Migration</a>
    <a href="/admin/system" class="active">System</a>
  </nav>

  <h1 style="font-size:1.5rem;font-weight:700;margin-bottom:1.5rem;color:#f1f5f9;">
    System Status
  </h1>

  <div id="system-status"
       hx-get="/admin/system/status"
       hx-trigger="every 30s"
       hx-swap="innerHTML">
    {status_html}
  </div>

  <!-- Actions -->
  <div class="card">
    <div class="section-title">Database Actions</div>
    <div style="display:flex;gap:1rem;flex-wrap:wrap;">
      <button class="btn btn-secondary"
              hx-post="/admin/system/backup"
              hx-target="#backup-result"
              hx-swap="innerHTML">
        Create Backup Now
      </button>
      <a href="/admin/system/backups"
         style="padding:.5rem 1.5rem;border-radius:.375rem;border:1px solid #334155;
                color:#94a3b8;font-size:.875rem;text-decoration:none;font-weight:600;">
        Manage Backups
      </a>
      <a href="/admin/system/backup/full"
         download="birdnet-backup.tar.gz"
         style="padding:.5rem 1.5rem;border-radius:.375rem;border:1px solid #334155;
                color:#94a3b8;font-size:.875rem;text-decoration:none;font-weight:600;">
        Full Backup (DB + Audio + Config)
      </a>
    </div>
    <div id="backup-result" style="margin-top:1rem;"></div>
  </div>

  <!-- Danger Zone -->
  <div class="card" style="border-color:#7f1d1d;">
    <div class="section-title" style="color:#f87171;">Danger Zone</div>
    <p style="color:#94a3b8;font-size:.85rem;margin-bottom:1rem;">
      These actions cannot be undone. Create a backup first.
    </p>
    <div style="display:flex;gap:1rem;flex-wrap:wrap;">
      <button class="btn btn-danger"
              hx-post="/admin/system/clear-detections"
              hx-target="#clear-result"
              hx-swap="innerHTML"
              hx-confirm="Are you sure you want to delete ALL detections and notification logs? This cannot be undone.">
        Clear All Detections
      </button>
      <button class="btn btn-danger"
              hx-post="/admin/system/clear-extracted"
              hx-target="#clear-result"
              hx-swap="innerHTML"
              hx-confirm="Are you sure you want to delete ALL extracted audio clips? This cannot be undone.">
        Clear Extracted Audio
      </button>
    </div>
    <div id="clear-result" style="margin-top:1rem;"></div>
  </div>
</div>
</body>
</html>"##
    ))
}

// ---------------------------------------------------------------------------
// GET /admin/system/status — HTMX partial
// ---------------------------------------------------------------------------

async fn system_status_partial(State(state): State<AppState>) -> Html<String> {
    Html(render_status_partial(&state).await)
}

async fn render_status_partial(state: &AppState) -> String {
    let db_path = state.db_path().to_path_buf();

    let (disk_html, rec_html) = tokio::task::spawn_blocking(move || {
        // Disk usage for DB directory
        let disk = db_path
            .parent()
            .and_then(|p| disk_usage(p).ok());

        let disk_html = disk.map_or_else(
            || r#"<p style="color:#64748b">Disk info unavailable</p>"#.to_string(),
            |d| {
                let pct = d.used_percent();
                let (badge, bar_color) = if d.is_critical() {
                    ("badge-crit", "#f87171")
                } else if d.is_low() {
                    ("badge-warn", "#fbbf24")
                } else {
                    ("badge-ok", "#4ade80")
                };

                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let pct_u = pct as u64;

                format!(
                    r#"<div>
                      <div style="display:flex;justify-content:space-between;margin-bottom:.5rem;">
                        <span style="font-size:.875rem;">Disk Usage</span>
                        <span class="{badge}" style="font-weight:600;">{pct_u}%</span>
                      </div>
                      <div style="background:#0f172a;border-radius:9999px;height:8px;overflow:hidden;">
                        <div style="background:{bar_color};height:100%;width:{pct_u}%;"></div>
                      </div>
                      <p style="color:#64748b;font-size:.75rem;margin-top:.25rem;">
                        {avail} free of {total}
                      </p>
                    </div>"#,
                    avail = format_bytes(d.available_bytes),
                    total = format_bytes(d.total_bytes),
                )
            },
        );

        // Recording stats (use parent directory of db as proxy)
        let rec_html = db_path
            .parent()
            .and_then(|p| recording_stats(p).ok())
            .map_or_else(
                || r#"<p style="color:#64748b">Recording stats unavailable</p>"#.to_string(),
                |(count, size)| {
                    format!(
                        r#"<p style="font-size:.875rem;">
                          {count} audio files · {size} total
                        </p>"#,
                        size = format_bytes(size),
                    )
                },
            );

        (disk_html, rec_html)
    })
    .await
    .unwrap_or_else(|_| {
        let err = r#"<p style="color:#f87171">Error querying system info</p>"#.to_string();
        (err.clone(), err)
    });

    // System CPU/memory snapshot (run in parallel with disk query)
    let sys_snap = tokio::task::spawn_blocking(system_info::sample).await.ok();

    let sys_html = sys_snap.map_or_else(
        || r#"<p style="color:#64748b">System info unavailable</p>"#.to_string(),
        |snap| {
            let cpu_color = if snap.is_cpu_high() { "#f87171" } else { "#4ade80" };
            let mem_color = if snap.is_memory_critical() { "#f87171" } else { "#4ade80" };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let cpu_pct = snap.cpu_usage_pct as u32;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let mem_pct = snap.memory_usage_pct as u32;
            let uptime = system_info::format_uptime(snap.uptime_secs);
            let temp_html = snap
                .cpu_temp_celsius
                .map(|t| {
                    let tc = t as u32;
                    let c = if tc > 80 { "#f87171" } else if tc > 65 { "#fbbf24" } else { "#4ade80" };
                    format!(r#"<p style="font-size:.8rem;margin:.25rem 0;">CPU Temp: <span style="color:{c};font-weight:600;">{tc}°C</span></p>"#)
                })
                .unwrap_or_default();

            format!(
                r#"<p style="font-size:.8rem;color:#64748b;margin-bottom:.5rem;">
                  {cores} cores · uptime {uptime}
                </p>
                <div style="display:flex;justify-content:space-between;margin-bottom:.25rem;">
                  <span style="font-size:.8rem;">CPU</span>
                  <span style="color:{cpu_color};font-weight:600;font-size:.8rem;">{cpu_pct}%</span>
                </div>
                <div style="background:#0f172a;border-radius:9999px;height:6px;margin-bottom:.75rem;overflow:hidden;">
                  <div style="background:{cpu_color};height:100%;width:{cpu_pct}%;"></div>
                </div>
                <div style="display:flex;justify-content:space-between;margin-bottom:.25rem;">
                  <span style="font-size:.8rem;">Memory</span>
                  <span style="color:{mem_color};font-weight:600;font-size:.8rem;">{mem_pct}%</span>
                </div>
                <div style="background:#0f172a;border-radius:9999px;height:6px;margin-bottom:.5rem;overflow:hidden;">
                  <div style="background:{mem_color};height:100%;width:{mem_pct}%;"></div>
                </div>
                <p style="font-size:.75rem;color:#64748b;margin:0;">{mem_summary}</p>
                {temp_html}"#,
                cores = snap.cpu_count,
                mem_summary = snap.memory_summary(),
            )
        },
    );

    format!(
        r#"<div class="stat-grid" style="margin-bottom:1.5rem;">
          <div class="card">{disk_html}</div>
          <div class="card">
            <div class="stat-label">Recordings</div>
            {rec_html}
          </div>
          <div class="card">
            <div class="stat-label">System Resources</div>
            {sys_html}
          </div>
        </div>
        <div style="text-align:right;margin-top:.5rem;">
          <a href="/admin/system/logs/page" style="color:#64748b;font-size:.8rem;text-decoration:none;">
            📋 Live Logs →
          </a>
        </div>"#
    )
}

// ---------------------------------------------------------------------------
// POST /admin/system/backup
// ---------------------------------------------------------------------------

async fn trigger_backup(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    let db_path = state.db_path().to_path_buf();
    let backup_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups");

    let result = tokio::task::spawn_blocking(move || backup_database(&db_path, &backup_dir))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match result {
        Ok(path) => Ok(Html(format!(
            r#"<p style="color:#4ade80;">
              Backup created: <code style="font-size:.8rem;">{}</code>
            </p>"#,
            path.display()
        ))),
        Err(e) => Ok(Html(format!(
            r#"<p style="color:#f87171;">Backup failed: {e}</p>"#
        ))),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn format_bytes(bytes: u64) -> String {
    const GB: u64 = 1_073_741_824;
    const MB: u64 = 1_048_576;
    const KB: u64 = 1_024;

    if bytes >= GB {
        #[allow(clippy::cast_precision_loss)]
        return format!("{:.1} GB", bytes as f64 / GB as f64);
    }
    if bytes >= MB {
        #[allow(clippy::cast_precision_loss)]
        return format!("{:.1} MB", bytes as f64 / MB as f64);
    }
    if bytes >= KB {
        #[allow(clippy::cast_precision_loss)]
        return format!("{:.1} KB", bytes as f64 / KB as f64);
    }
    format!("{bytes} B")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_gb() {
        assert_eq!(format_bytes(2_147_483_648), "2.0 GB");
    }

    #[test]
    fn format_bytes_mb() {
        assert_eq!(format_bytes(10_485_760), "10.0 MB");
    }

    #[test]
    fn format_bytes_kb() {
        assert_eq!(format_bytes(2_048), "2.0 KB");
    }

    #[test]
    fn format_bytes_small() {
        assert_eq!(format_bytes(512), "512 B");
    }
}
