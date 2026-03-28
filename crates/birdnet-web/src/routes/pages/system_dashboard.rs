//! System health dashboard: live CPU, memory, temperature, disk, and database metrics.
//!
//! | Path                          | Purpose                                   |
//! |-------------------------------|-------------------------------------------|
//! | `GET /system`                 | Full system dashboard page                |
//! | `GET /pages/sys-vitals`       | CPU/memory/temp vitals partial (HTMX)     |
//! | `GET /pages/sys-disk`         | Disk usage partial (HTMX)                 |
//! | `GET /pages/sys-database`     | Database stats partial (HTMX)             |
//! | `GET /pages/sys-uptime`       | Process uptime and version partial (HTMX) |
//! | `GET /pages/sys-audio`        | Audio pipeline status partial (HTMX)      |

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Html;
use axum::{Router, routing::get};

use super::{escape_html, render_page};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/system", get(system_page))
        .route("/pages/sys-vitals", get(sys_vitals_partial))
        .route("/pages/sys-disk", get(sys_disk_partial))
        .route("/pages/sys-database", get(sys_database_partial))
        .route("/pages/sys-uptime", get(sys_uptime_partial))
        .route("/pages/sys-audio", get(sys_audio_partial))
}

async fn system_page() -> Html<String> {
    render_page("System Health", SYSTEM_DASHBOARD_HTML, "system")
}

/// HTMX partial: CPU, memory, temperature.
async fn sys_vitals_partial(State(_state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(crate::system_info::sample).await;

    match result {
        Ok(snap) => {
            let cpu_color = if snap.cpu_usage_pct > 80.0 {
                "var(--danger)"
            } else if snap.cpu_usage_pct > 50.0 {
                "var(--warning)"
            } else {
                "var(--success)"
            };
            #[allow(clippy::cast_lossless)]
            let mem_pct = snap.memory_usage_pct as f64;
            let mem_color = if mem_pct > 85.0 {
                "var(--danger)"
            } else if mem_pct > 60.0 {
                "var(--warning)"
            } else {
                "var(--success)"
            };
            let temp_str = snap
                .cpu_temp_celsius
                .map_or_else(|| "\u{2014}".to_string(), |t| format!("{t:.1}\u{00b0}C"));
            let temp_color = snap.cpu_temp_celsius.map_or("var(--text-muted)", |t| {
                if t > 75.0 {
                    "var(--danger)"
                } else if t > 60.0 {
                    "var(--warning)"
                } else {
                    "var(--success)"
                }
            });

            let mem_summary = snap.memory_summary();
            let uptime = crate::system_info::format_uptime(snap.uptime_secs);

            let mut html = String::with_capacity(2048);
            // CPU gauge
            #[allow(clippy::cast_lossless)]
            let cpu_f64 = snap.cpu_usage_pct as f64;
            let _ = write!(
                html,
                "<div class=\"stat-card\">\
                  <div class=\"value\" style=\"color:{cpu_color};\">{cpu:.0}%</div>\
                  <div class=\"label\">CPU ({cores} cores)</div>\
                  {bar}\
                </div>",
                cpu = snap.cpu_usage_pct,
                cores = snap.cpu_count,
                bar = progress_bar(cpu_f64, cpu_color),
            );
            // Memory gauge
            let _ = write!(
                html,
                "<div class=\"stat-card\">\
                  <div class=\"value\" style=\"color:{mem_color};\">{mem_pct:.0}%</div>\
                  <div class=\"label\">Memory ({mem_summary})</div>\
                  {bar}\
                </div>",
                bar = progress_bar(mem_pct, mem_color),
                mem_summary = escape_html(&mem_summary),
            );
            // Temperature
            let _ = write!(
                html,
                "<div class=\"stat-card\">\
                  <div class=\"value\" style=\"color:{temp_color};\">{temp_str}</div>\
                  <div class=\"label\">Temperature</div>\
                </div>",
            );
            // Uptime
            let _ = write!(
                html,
                "<div class=\"stat-card\">\
                  <div class=\"value\" style=\"font-size:1.2rem;\">{uptime}</div>\
                  <div class=\"label\">System Uptime</div>\
                </div>",
                uptime = escape_html(&uptime),
            );

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error gathering system info</p>".to_string(),
        ),
    }
}

/// Render an inline CSS progress bar.
fn progress_bar(pct: f64, color: &str) -> String {
    let clamped = pct.clamp(0.0, 100.0);
    format!(
        "<div style=\"margin-top:0.5rem;height:6px;background:var(--bg-hover);border-radius:3px;overflow:hidden;\">\
         <div style=\"width:{clamped:.0}%;height:100%;background:{color};border-radius:3px;transition:width 0.3s;\"></div>\
         </div>",
    )
}

/// HTMX partial: disk usage.
async fn sys_disk_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let db_path = state.db_path().to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        // Get filesystem stats for the DB directory
        let dir = db_path.parent().unwrap_or(&db_path);
        let dir_str = dir.to_string_lossy().to_string();

        // Use statvfs via std::fs metadata as a proxy — count directory size
        let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        (dir_str, db_size)
    })
    .await;

    match result {
        Ok((dir, db_size)) => {
            #[allow(clippy::cast_precision_loss)]
            let db_mb = db_size as f64 / 1_048_576.0;
            let html = format!(
                "<table style=\"font-size:0.9rem;\">\
                 <tr><td style=\"font-weight:600;padding-right:1rem;\">Database Path</td>\
                 <td><code>{dir}</code></td></tr>\
                 <tr><td style=\"font-weight:600;padding-right:1rem;\">Database Size</td>\
                 <td>{db_mb:.1} MB</td></tr>\
                 </table>",
                dir = escape_html(&dir),
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading disk info</p>".to_string(),
        ),
    }
}

/// HTMX partial: database statistics.
async fn sys_database_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let dates = birdnet_db::sqlite::distinct_detection_dates(conn).unwrap_or_default();
            let integrity = birdnet_db::sqlite::quick_check(conn).unwrap_or(false);
            (total, species, dates.len(), integrity)
        })
    })
    .await;

    match result {
        Ok((total, species, days, integrity)) => {
            let status_badge = if integrity {
                r#"<span style="color:var(--success);font-weight:600;">OK</span>"#
            } else {
                r#"<span style="color:var(--danger);font-weight:600;">CORRUPT</span>"#
            };
            let html = format!(
                "<table style=\"font-size:0.9rem;\">\
                 <tr><td style=\"font-weight:600;padding-right:1rem;\">Total Detections</td>\
                 <td>{total}</td></tr>\
                 <tr><td style=\"font-weight:600;padding-right:1rem;\">Unique Species</td>\
                 <td>{species}</td></tr>\
                 <tr><td style=\"font-weight:600;padding-right:1rem;\">Days with Data</td>\
                 <td>{days}</td></tr>\
                 <tr><td style=\"font-weight:600;padding-right:1rem;\">Integrity Check</td>\
                 <td>{status_badge}</td></tr>\
                 </table>",
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading database info</p>".to_string(),
        ),
    }
}

/// HTMX partial: uptime and version info.
async fn sys_uptime_partial(State(_state): State<AppState>) -> impl axum::response::IntoResponse {
    let version = env!("CARGO_PKG_VERSION");
    let rust_version = env!("CARGO_PKG_RUST_VERSION");

    let html = format!(
        "<table style=\"font-size:0.9rem;\">\
         <tr><td style=\"font-weight:600;padding-right:1rem;\">Version</td>\
         <td>v{version}</td></tr>\
         <tr><td style=\"font-weight:600;padding-right:1rem;\">MSRV</td>\
         <td>Rust {rust_version}</td></tr>\
         <tr><td style=\"font-weight:600;padding-right:1rem;\">Analytics</td>\
         <td>{analytics}</td></tr>\
         </table>",
        analytics = if cfg!(feature = "analytics") {
            "Enabled"
        } else {
            "Disabled"
        },
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: audio pipeline status.
async fn sys_audio_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let rec_dir = state.recording_dir();
    let result = tokio::task::spawn_blocking(move || {
        let count = std::fs::read_dir(&rec_dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| {
                        e.path()
                            .extension()
                            .is_some_and(|ext| ext == "wav" || ext == "flac" || ext == "mp3")
                    })
                    .count()
            })
            .unwrap_or(0);
        let dir_str = rec_dir.to_string_lossy().to_string();
        (dir_str, count)
    })
    .await;

    match result {
        Ok((dir, count)) => {
            let html = format!(
                "<table style=\"font-size:0.9rem;\">\
                 <tr><td style=\"font-weight:600;padding-right:1rem;\">Recording Directory</td>\
                 <td><code>{dir}</code></td></tr>\
                 <tr><td style=\"font-weight:600;padding-right:1rem;\">Audio Files</td>\
                 <td>{count}</td></tr>\
                 </table>",
                dir = escape_html(&dir),
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading audio info</p>".to_string(),
        ),
    }
}

const SYSTEM_DASHBOARD_HTML: &str = r#"<h1 style="margin-bottom:1.5rem;">System Health</h1>

<div class="stats-grid" hx-get="/pages/sys-vitals" hx-trigger="load, every 10s" hx-swap="innerHTML">
    <div class="stat-card"><div class="value">--</div><div class="label">CPU Usage</div></div>
    <div class="stat-card"><div class="value">--</div><div class="label">Memory</div></div>
    <div class="stat-card"><div class="value">--</div><div class="label">Temperature</div></div>
    <div class="stat-card"><div class="value">--</div><div class="label">Load Average</div></div>
</div>

<div class="grid-2">
    <div>
        <div class="card">
            <h2>Database</h2>
            <div hx-get="/pages/sys-database" hx-trigger="load, every 60s" hx-swap="innerHTML">
                <p style="color:var(--text-muted);">Loading...</p>
            </div>
        </div>

        <div class="card">
            <h2>Disk</h2>
            <div hx-get="/pages/sys-disk" hx-trigger="load, every 60s" hx-swap="innerHTML">
                <p style="color:var(--text-muted);">Loading...</p>
            </div>
        </div>
    </div>

    <div>
        <div class="card">
            <h2>Version &amp; Runtime</h2>
            <div hx-get="/pages/sys-uptime" hx-trigger="load" hx-swap="innerHTML">
                <p style="color:var(--text-muted);">Loading...</p>
            </div>
        </div>

        <div class="card">
            <h2>Audio Pipeline</h2>
            <div hx-get="/pages/sys-audio" hx-trigger="load, every 30s" hx-swap="innerHTML">
                <p style="color:var(--text-muted);">Loading...</p>
            </div>
        </div>
    </div>
</div>"#;
