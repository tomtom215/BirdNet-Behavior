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

use crate::routes::pages::toast::{self, Toast};
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

#[allow(clippy::too_many_lines)]
async fn system_page(State(state): State<AppState>) -> Html<String> {
    let status_html = render_status_partial(&state).await;
    // Page body only — the document chrome (theme guard, app.css, htmx, the
    // admin nav, breadcrumbs, ⌘K/help/toasts) comes from `admin_shell`. The
    // page-specific CSS stays as a scoped <style> block; the old `.container`
    // and bare `nav` rules are dropped because the shell owns both now (a bare
    // `nav` selector would otherwise re-style the shell's `.admin-nav`).
    let body = format!(
        r##"<style>
      .card {{ background:var(--surface); border:1px solid var(--border); border-radius:.75rem;
               padding:1.5rem; margin-bottom:1.5rem; }}
      .section-title {{ font-size:1.1rem; font-weight:600; color:var(--moss-ink);
                        margin-bottom:.75rem; }}
      .stat-grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(200px,1fr));
                    gap:1rem; }}
      .stat-card {{ background:var(--bg); border:1px solid var(--surface); border-radius:.5rem;
                    padding:1rem; }}
      .stat-label {{ font-size:.75rem; color:var(--fg-4); text-transform:uppercase; }}
      .stat-value {{ font-size:1.4rem; font-weight:700; margin-top:.25rem; }}
      .btn {{ padding:.5rem 1.5rem; border-radius:.375rem; border:none; cursor:pointer;
               font-weight:600; font-size:.875rem; }}
      .btn-secondary {{ background:var(--surface); color:var(--fg); border:1px solid var(--border); }}
      .btn-secondary:hover {{ border-color:var(--moss-ink); color:var(--moss-ink); }}
      .btn-danger {{ background:var(--rare-soft); color:var(--rare); border:1px solid var(--rare-soft); }}
      .badge-ok {{ color:var(--moss); }} .badge-warn {{ color:var(--dawn); }}
      .badge-crit {{ color:var(--rare); }}
      /* O-25 sweep: shapes promoted out of inline style= attributes. */
      h1 {{ font-size:1.5rem; font-weight:700; margin-bottom:1.5rem; color:var(--fg); }}
      .lead {{ color:var(--fg-3); font-size:.85rem; margin-bottom:1rem; }}
      .btn-row {{ display:flex; gap:1rem; flex-wrap:wrap; }}
      .btn-row.center {{ align-items:center; }}
      .result-slot {{ margin-top:1rem; }}
      .result-slot.sm {{ font-size:.85rem; }}
      .result-slot.muted {{ color:var(--fg-3); }}
      .btn-link {{ padding:.5rem 1.5rem; border-radius:.375rem; border:1px solid var(--border);
                   color:var(--fg-3); font-size:.875rem; text-decoration:none; font-weight:600; }}
      code.inline {{ background:var(--bg); padding:.1rem .4rem; border-radius:.25rem; }}
      code.sm {{ font-size:.8rem; }}
      .card.danger {{ border-color:var(--rare-soft); }}
      .section-title.danger {{ color:var(--rare); }}
      /* Resource meters: enumerable tone classes; only the bar width stays inline. */
      .meter-row {{ display:flex; justify-content:space-between; margin-bottom:.25rem; }}
      .meter-row.tight {{ margin-bottom:.5rem; }}
      .meter-label {{ font-size:.875rem; }}
      .meter-label.sm {{ font-size:.8rem; }}
      .meter-val {{ font-weight:600; }}
      .meter-val.sm {{ font-weight:600; font-size:.8rem; }}
      .meter-val.ok {{ color:var(--moss); }}
      .meter-val.warn {{ color:var(--dawn); }}
      .meter-val.crit {{ color:var(--rare); }}
      .meter-track {{ background:var(--bg); border-radius:9999px; height:8px; overflow:hidden; }}
      .meter-track.sm {{ height:6px; margin-bottom:.75rem; }}
      .meter-track.sm.last {{ margin-bottom:.5rem; }}
      .meter-fill {{ height:100%; }}
      .meter-fill.ok {{ background:var(--moss); }}
      .meter-fill.warn {{ background:var(--dawn); }}
      .meter-fill.crit {{ background:var(--rare); }}
      .meter-note {{ color:var(--fg-4); font-size:.75rem; margin-top:.25rem; }}
      .status-grid {{ margin-bottom:1.5rem; }}
      .res-head {{ font-size:.8rem; color:var(--fg-4); margin-bottom:.5rem; }}
      .res-line {{ font-size:.875rem; }}
      .res-sub {{ font-size:.75rem; color:var(--fg-4); margin:0; }}
      .temp-line {{ font-size:.8rem; margin:.25rem 0; }}
      .temp-val {{ font-weight:600; }}
      .temp-val.ok {{ color:var(--moss); }}
      .temp-val.warn {{ color:var(--dawn); }}
      .temp-val.crit {{ color:var(--rare); }}
      .muted-note {{ color:var(--fg-4); }}
      .err-note {{ color:var(--rare); }}
      .logs-row {{ text-align:right; margin-top:.5rem; }}
      .logs-link {{ color:var(--fg-4); font-size:.8rem; text-decoration:none; }}
      .ok-note {{ color:var(--moss); }}
    </style>

  <h1>System Status</h1>

  <div id="system-status"
       hx-get="/admin/system/status"
       hx-trigger="every 30s"
       hx-swap="innerHTML">
    {status_html}
  </div>

  <!-- Service Controls -->
  <div class="card">
    <div class="section-title">Service Controls</div>
    <p class="lead">
      Control the BirdNet-Behavior detection service. The service restarts automatically
      when managed by systemd (Restart=on-failure). Reconnect after ~5 seconds.
    </p>
    <div class="btn-row center">
      <button class="btn btn-secondary"
              hx-post="/admin/system/service/restart"
              hx-target="#service-result"
              hx-swap="innerHTML"
              hx-confirm="Restart the service? Detection will pause briefly during restart."
              data-confirm-action="hx-post"
              data-confirm-url="/admin/system/service/restart"
              data-confirm-title="Restart service"
              data-confirm-body="Restart the service? Detection will pause briefly during restart."
              data-confirm-confirm-label="Restart"
              data-confirm-style="warn">
        Restart Service
      </button>
      <button class="btn btn-secondary"
              hx-get="/admin/system/service/status"
              hx-target="#service-status-box"
              hx-swap="innerHTML">
        Refresh Status
      </button>
    </div>
    <div id="service-result" class="result-slot"></div>
    <div id="service-status-box"
         hx-get="/admin/system/service/status"
         hx-trigger="load"
         class="result-slot sm muted">
      Loading service status…
    </div>
  </div>

  <!-- Update Check -->
  <div class="card">
    <div class="section-title">Software Update</div>
    <p class="lead">
      Check GitHub Releases for a newer version. If an update is available,
      download and replace the binary with: <code class="inline">
      sudo install.sh</code> or pull the latest binary from Releases.
    </p>
    <div class="btn-row center">
      <button class="btn btn-secondary"
              hx-get="/admin/system/update/check"
              hx-target="#update-result"
              hx-swap="innerHTML">
        Check for Updates
      </button>
    </div>
    <div id="update-result" class="result-slot sm"></div>
  </div>

  <!-- Database Actions -->
  <div class="card">
    <div class="section-title">Database Actions</div>
    <div class="btn-row">
      <button class="btn btn-secondary"
              hx-post="/admin/system/backup"
              hx-target="#backup-result"
              hx-swap="innerHTML">
        Create Backup Now
      </button>
      <a href="/admin/system/backups" class="btn-link">
        Manage Backups
      </a>
      <a href="/admin/system/backup/full"
         download="birdnet-backup.tar.gz"
         class="btn-link">
        Full Backup (DB + Audio + Config)
      </a>
    </div>
    <div id="backup-result" class="result-slot"></div>
  </div>

  <!-- Danger Zone -->
  <div class="card danger">
    <div class="section-title danger">Danger Zone</div>
    <p class="lead">
      These actions cannot be undone. Create a backup first.
    </p>
    <div class="btn-row">
      <button class="btn btn-danger"
              hx-post="/admin/system/clear-detections"
              hx-disabled-elt="this"
              hx-target="#clear-result"
              hx-swap="innerHTML"
              hx-confirm="Are you sure you want to delete ALL detections and notification logs? This cannot be undone."
              data-confirm-action="hx-post"
              data-confirm-url="/admin/system/clear-detections"
              data-confirm-title="Clear all detections"
              data-confirm-body="Are you sure you want to delete ALL detections and notification logs? This cannot be undone."
              data-confirm-confirm-label="Delete all"
              data-confirm-style="danger">
        Clear All Detections
      </button>
      <button class="btn btn-danger"
              hx-post="/admin/system/clear-extracted"
              hx-disabled-elt="this"
              hx-target="#clear-result"
              hx-swap="innerHTML"
              hx-confirm="Are you sure you want to delete ALL extracted audio clips? This cannot be undone."
              data-confirm-action="hx-post"
              data-confirm-url="/admin/system/clear-extracted"
              data-confirm-title="Clear extracted audio"
              data-confirm-body="Are you sure you want to delete ALL extracted audio clips? This cannot be undone."
              data-confirm-confirm-label="Delete clips"
              data-confirm-style="danger">
        Clear Extracted Audio
      </button>
    </div>
    <div id="clear-result" class="result-slot"></div>
  </div>"##
    );
    Html(super::admin_shell("System", "system", &body))
}

// ---------------------------------------------------------------------------
// GET /admin/system/status — HTMX partial
// ---------------------------------------------------------------------------

async fn system_status_partial(State(state): State<AppState>) -> Html<String> {
    Html(render_status_partial(&state).await)
}

#[allow(clippy::too_many_lines)]
async fn render_status_partial(state: &AppState) -> String {
    let db_path = state.db_path().to_path_buf();

    let (disk_html, rec_html) = tokio::task::spawn_blocking(move || {
        // Disk usage for DB directory
        let disk = db_path.parent().and_then(|p| disk_usage(p).ok());

        let disk_html = disk.map_or_else(
            || r#"<p class="muted-note">Disk info unavailable</p>"#.to_string(),
            |d| {
                let pct = d.used_percent();
                // Disk status is an enumerable triple → tone class; only the
                // computed bar width stays inline (the P3-3 endgame exception).
                let (badge, tone) = if d.is_critical() {
                    ("badge-crit", "crit")
                } else if d.is_low() {
                    ("badge-warn", "warn")
                } else {
                    ("badge-ok", "ok")
                };

                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let pct_u = pct as u64;

                format!(
                    r#"<div>
                      <div class="meter-row tight">
                        <span class="meter-label">Disk Usage</span>
                        <span class="{badge} meter-val">{pct_u}%</span>
                      </div>
                      <div class="meter-track">
                        <div class="meter-fill {tone}" data-style="width:{pct_u}%"></div>
                      </div>
                      <p class="meter-note">
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
                || r#"<p class="muted-note">Recording stats unavailable</p>"#.to_string(),
                |(count, size)| {
                    format!(
                        r#"<p class="res-line">
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
        let err = r#"<p class="err-note">Error querying system info</p>"#.to_string();
        (err.clone(), err)
    });

    // System CPU/memory snapshot (run in parallel with disk query)
    let sys_snap = tokio::task::spawn_blocking(system_info::sample).await.ok();

    let sys_html = sys_snap.map_or_else(
        || r#"<p class="muted-note">System info unavailable</p>"#.to_string(),
        |snap| {
            // CPU/memory status are enumerable → tone classes; only the bar
            // widths stay inline (computed per request — the endgame exception).
            let cpu_tone = if snap.is_cpu_high() { "crit" } else { "ok" };
            let mem_tone = if snap.is_memory_critical() { "crit" } else { "ok" };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let cpu_pct = snap.cpu_usage_pct as u32;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let mem_pct = snap.memory_usage_pct as u32;
            let uptime = system_info::format_uptime(snap.uptime_secs);
            let temp_html = snap
                .cpu_temp_celsius
                .map(|t| {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss, clippy::cast_possible_wrap, clippy::cast_lossless)]
                    let tc = t as u32;
                    let temp_tone = if tc > 80 { "crit" } else if tc > 65 { "warn" } else { "ok" };
                    format!(r#"<p class="temp-line">CPU Temp: <span class="temp-val {temp_tone}">{tc}°C</span></p>"#)
                })
                .unwrap_or_default();

            format!(
                r#"<p class="res-head">
                  {cores} cores · uptime {uptime}
                </p>
                <div class="meter-row">
                  <span class="meter-label sm">CPU</span>
                  <span class="meter-val sm {cpu_tone}">{cpu_pct}%</span>
                </div>
                <div class="meter-track sm">
                  <div class="meter-fill {cpu_tone}" data-style="width:{cpu_pct}%"></div>
                </div>
                <div class="meter-row">
                  <span class="meter-label sm">Memory</span>
                  <span class="meter-val sm {mem_tone}">{mem_pct}%</span>
                </div>
                <div class="meter-track sm last">
                  <div class="meter-fill {mem_tone}" data-style="width:{mem_pct}%"></div>
                </div>
                <p class="res-sub">{mem_summary}</p>
                {temp_html}"#,
                cores = snap.cpu_count,
                mem_summary = snap.memory_summary(),
            )
        },
    );

    format!(
        r#"<div class="stat-grid status-grid">
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
        <div class="logs-row">
          <a href="/admin/system/logs/page" class="logs-link">
            📋 Live Logs →
          </a>
        </div>"#
    )
}

// ---------------------------------------------------------------------------
// POST /admin/system/backup
// ---------------------------------------------------------------------------

async fn trigger_backup(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
) -> Result<Html<String>, StatusCode> {
    crate::audit::audit(&state, Some(&request_user), "data.backup.run", None, None);
    let db_path = state.db_path().to_path_buf();
    let backup_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups");

    let result = tokio::task::spawn_blocking(move || backup_database(&db_path, &backup_dir))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match result {
        Ok(path) => {
            let display = path.display().to_string();
            let body = Html(format!(
                r#"<p class="ok-note">
              Backup created: <code class="sm">{display}</code>
            </p>"#
            ));
            // O-18: toast confirmation of the backup outcome.
            Ok(toast::with(
                body,
                Toast::success(format!("Backup created — {display}.")),
            ))
        }
        Err(e) => {
            let body = Html(format!(r#"<p class="err-note">Backup failed: {e}</p>"#));
            Ok(toast::with(
                body,
                Toast::error(format!("Backup failed: {e}")),
            ))
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
