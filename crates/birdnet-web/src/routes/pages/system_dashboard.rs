//! System health dashboard: live CPU, memory, temperature, disk, and database metrics.
//!
//! | Path                          | Purpose                                   |
//! |-------------------------------|-------------------------------------------|
//! | (embedded)                    | Station home, public "Health" tab        |
//! | `GET /pages/sys-vitals`       | CPU/memory/temp vitals partial (HTMX)     |
//! | `GET /pages/sys-disk`         | Disk usage partial (HTMX)                 |
//! | `GET /pages/sys-database`     | Database stats partial (HTMX)             |
//! | `GET /pages/sys-uptime`       | Process uptime and version partial (HTMX) |
//! | `GET /pages/sys-audio`        | Audio pipeline status partial (HTMX)      |

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::{Router, routing::get};

use super::escape_html;
use crate::state::AppState;

/// Mount the system health dashboard and all HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/sys-vitals", get(sys_vitals_partial))
        .route("/pages/sys-disk", get(sys_disk_partial))
        .route("/pages/sys-database", get(sys_database_partial))
        .route("/pages/sys-uptime", get(sys_uptime_partial))
        .route("/pages/sys-audio", get(sys_audio_partial))
}

/// The system-health surface, rendered for embedding by `homes::station`
/// (the public "Health" tab).
pub(super) fn content() -> String {
    format!("{SYSTEM_DASHBOARD_HTML}{DISPLAY_PREFS_HTML}").replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::AdminSystem),
    )
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

            #[allow(clippy::cast_lossless)]
            let cpu_f64 = snap.cpu_usage_pct as f64;
            // Temperature is shown on a 0–90 °C scale; absent sensor → empty gauge.
            #[allow(clippy::cast_lossless)]
            let temp_pct = snap
                .cpu_temp_celsius
                .map_or(0.0, |t| (f64::from(t) / 90.0 * 100.0).clamp(0.0, 100.0));

            let mut html = String::with_capacity(2048);
            html.push_str(&arc_gauge(
                cpu_f64,
                &format!("{cpu_f64:.0}%"),
                "CPU",
                &format!("{} cores", snap.cpu_count),
                cpu_color,
            ));
            html.push_str(&arc_gauge(
                mem_pct,
                &format!("{mem_pct:.0}%"),
                "Memory",
                &escape_html(&mem_summary),
                mem_color,
            ));
            html.push_str(&arc_gauge(
                temp_pct,
                &temp_str,
                "Temperature",
                if snap.cpu_temp_celsius.is_some() {
                    "core"
                } else {
                    "no sensor"
                },
                temp_color,
            ));
            // Uptime is a duration, not a ratio — keep it as a clean value tile.
            let _ = write!(
                html,
                "<div class=\"stat-card sys-uptime-card\">\
                  <div class=\"display sys-uptime-val\">{uptime}</div>\
                  <div class=\"label sys-uptime-label\">System uptime</div>\
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

/// Render a 3/4-arc gauge (270° sweep, gap at the bottom) with a big display
/// value in the centre — the design's system-vitals gauge. `center` is the
/// value text (e.g. "42%"), `label` the metric and `sub` a small detail line;
/// both `center` and `sub` must already be escaped.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn arc_gauge(pct: f64, center: &str, label: &str, sub: &str, color: &str) -> String {
    let pct = pct.clamp(0.0, 100.0);
    let (cx, cy, r) = (60.0_f64, 58.0_f64, 44.0_f64);
    let start = 135.0_f64;
    let total = 270.0_f64;
    let xy = |deg: f64| {
        let a: f64 = deg.to_radians();
        (r.mul_add(a.cos(), cx), r.mul_add(a.sin(), cy))
    };
    let (sx, sy) = xy(start);
    let (ex, ey) = xy(start + total);
    let v_sweep = total * pct / 100.0;
    let (vx, vy) = xy(start + v_sweep);
    let large_v = i32::from(v_sweep > 180.0);
    format!(
        "<div class=\"stat-card sys-gauge-card\">\
          <svg viewBox=\"0 0 120 100\" width=\"128\" class=\"sys-gauge-svg\" aria-hidden=\"true\">\
            <path d=\"M{sx:.1},{sy:.1} A{r},{r} 0 1 1 {ex:.1},{ey:.1}\" fill=\"none\" stroke=\"var(--surface-2)\" stroke-width=\"9\" stroke-linecap=\"round\"/>\
            <path d=\"M{sx:.1},{sy:.1} A{r},{r} 0 {large_v} 1 {vx:.1},{vy:.1}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"9\" stroke-linecap=\"round\"/>\
            <text x=\"60\" y=\"62\" text-anchor=\"middle\" class=\"display sys-gauge-text\">{center}</text>\
          </svg>\
          <div class=\"label sys-gauge-label\">{label}</div>\
          <div class=\"bnb-meta sys-gauge-sub\">{sub}</div>\
        </div>",
    )
}

/// HTMX partial: real filesystem disk usage for the data directory.
async fn sys_disk_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let db_path = state.db_path().to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        let dir = db_path.parent().unwrap_or(&db_path).to_path_buf();
        let dir_str = dir.to_string_lossy().to_string();
        // Real statvfs-backed usage for the filesystem the data lives on — the
        // metric that actually matters for "will recording run out of space".
        let usage = birdnet_core::audio::capture::disk_usage(&dir).ok();
        let db_size = std::fs::metadata(&db_path).map_or(0, |m| m.len());
        (dir_str, usage, db_size)
    })
    .await;

    match result {
        Ok((dir, usage, db_size)) => {
            #[allow(clippy::cast_precision_loss)]
            let db_mb = db_size as f64 / 1_048_576.0;
            let disk_row = usage.map_or_else(
                || {
                    String::from(
                        "<tr><td class=\"sys-th\">Disk Usage</td><td>unavailable</td></tr>",
                    )
                },
                |d| {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    let pct = d.used_percent() as u64;
                    let status = if d.is_critical() {
                        " — critically low"
                    } else if d.is_low() {
                        " — running low"
                    } else {
                        ""
                    };
                    format!(
                        "<tr><td class=\"sys-th\">Disk Usage</td>\
                         <td>{pct}% used · {avail} free of {total}{status}</td></tr>",
                        avail = crate::system_info::format_bytes(d.available_bytes),
                        total = crate::system_info::format_bytes(d.total_bytes),
                    )
                },
            );
            let html = format!(
                "<table class=\"sys-table\">\
                 <tr><td class=\"sys-th\">Database Path</td><td><code>{dir}</code></td></tr>\
                 {disk_row}\
                 <tr><td class=\"sys-th\">Database Size</td><td>{db_mb:.1} MB</td></tr>\
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
            let silence = birdnet_db::sqlite::seconds_since_last_detection(conn)
                .ok()
                .flatten();
            let queued = birdnet_db::outbound_queue::depth(
                conn,
                birdnet_integrations::birdweather::QUEUE_KIND,
            )
            .unwrap_or(0);
            (total, species, dates.len(), integrity, silence, queued)
        })
    })
    .await;

    match result {
        Ok((total, species, days, integrity, silence, queued)) => {
            let status_badge = if integrity {
                r#"<span class="sys-ok">OK</span>"#
            } else {
                r#"<span class="sys-bad">CORRUPT</span>"#
            };
            // The non-technical answer to "is it working right now?": how
            // long ago the last detection landed, end to end.
            let last_seen =
                silence.map_or_else(|| "no detections yet".to_owned(), format_silence_duration);
            // Only worth a row when uploads are actually backed up — a
            // permanent "0" line is noise on a healthy station.
            let queued_row = if queued > 0 {
                format!(
                    "<tr><td class=\"sys-th\">Queued Uploads</td>\
                     <td>{queued} BirdWeather upload(s) awaiting network</td></tr>"
                )
            } else {
                String::new()
            };
            let html = format!(
                "<table class=\"sys-table\">\
                 <tr><td class=\"sys-th\">Total Detections</td>\
                 <td>{total}</td></tr>\
                 <tr><td class=\"sys-th\">Unique Species</td>\
                 <td>{species}</td></tr>\
                 <tr><td class=\"sys-th\">Days with Data</td>\
                 <td>{days}</td></tr>\
                 <tr><td class=\"sys-th\">Last Detection</td>\
                 <td>{last_seen}</td></tr>{queued_row}\
                 <tr><td class=\"sys-th\">Integrity Check</td>\
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

/// Human duration for the "Last Detection" row: seconds-level freshness is
/// noise, so round to the unit an operator thinks in.
fn format_silence_duration(secs: u64) -> String {
    match secs {
        0..=119 => "just now".to_owned(),
        120..=7_199 => format!("{} min ago", secs / 60),
        7_200..=172_799 => format!("{} h ago", secs / 3_600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// HTMX partial: uptime and version info.
async fn sys_uptime_partial(State(_state): State<AppState>) -> impl axum::response::IntoResponse {
    let version = env!("CARGO_PKG_VERSION");
    let rust_version = env!("CARGO_PKG_RUST_VERSION");

    let html = format!(
        "<table class=\"sys-table\">\
         <tr><td class=\"sys-th\">Version</td>\
         <td>v{version}</td></tr>\
         <tr><td class=\"sys-th\">MSRV</td>\
         <td>Rust {rust_version}</td></tr>\
         <tr><td class=\"sys-th\">Analytics</td>\
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
        let count = std::fs::read_dir(&rec_dir).map_or(0, |rd| {
            rd.filter_map(Result::ok)
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "wav" || ext == "flac" || ext == "mp3")
                })
                .count()
        });
        let dir_str = rec_dir.to_string_lossy().to_string();
        (dir_str, count)
    })
    .await;

    match result {
        Ok((dir, count)) => {
            let html = format!(
                "<table class=\"sys-table\">\
                 <tr><td class=\"sys-th\">Recording Directory</td>\
                 <td><code>{dir}</code></td></tr>\
                 <tr><td class=\"sys-th\">Audio Files</td>\
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

const DISPLAY_PREFS_HTML: &str = include_str!("../../../templates/_partial_display_prefs.html");

const SYSTEM_DASHBOARD_HTML: &str = r#"<div class="page-head sys-head">
    <div>
        <div class="bnb-eyebrow">Operations</div>
        <h1 class="display sys-h1">System health</h1>
        {{help_link}}
        <p class="bnb-meta sys-lede">Live vitals for this station — CPU, memory, temperature, storage, and the audio pipeline.</p>
    </div>
</div>

<div class="stats-grid" hx-get="/pages/sys-vitals" hx-trigger="load, every 10s" hx-swap="innerHTML">
    <div class="stat-card"><div class="value">--</div><div class="label">CPU Usage</div></div>
    <div class="stat-card"><div class="value">--</div><div class="label">Memory</div></div>
    <div class="stat-card"><div class="value">--</div><div class="label">Temperature</div></div>
    <div class="stat-card"><div class="value">--</div><div class="label">System uptime</div></div>
</div>

<div class="grid-2">
    <div>
        <div class="card">
            <h2>Database</h2>
            <div hx-get="/pages/sys-database" hx-trigger="load, every 60s" hx-swap="innerHTML">
                <p class="sys-loading">Loading...</p>
            </div>
        </div>

        <div class="card">
            <h2>Disk</h2>
            <div hx-get="/pages/sys-disk" hx-trigger="load, every 60s" hx-swap="innerHTML">
                <p class="sys-loading">Loading...</p>
            </div>
        </div>
    </div>

    <div>
        <div class="card">
            <h2>Version &amp; Runtime</h2>
            <div hx-get="/pages/sys-uptime" hx-trigger="load" hx-swap="innerHTML">
                <p class="sys-loading">Loading...</p>
            </div>
        </div>

        <div class="card">
            <h2>Audio Pipeline</h2>
            <div hx-get="/pages/sys-audio" hx-trigger="load, every 30s" hx-swap="innerHTML">
                <p class="sys-loading">Loading...</p>
            </div>
        </div>
    </div>
</div>"#;
