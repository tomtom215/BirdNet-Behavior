//! Admin overview dashboard.
//!
//! Provides a summary page with:
//! - System health (CPU, memory, temperature, disk)
//! - Quick links to all admin sections
//! - Database stats (total detections, species, today's count)
//! - Migration status hint

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Router, routing::get};

use crate::routes::pages::{escape_html, today_count, today_date_string};
use crate::state::AppState;
use crate::system_info::sample as sample_system;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/overview", get(overview_page))
        .route("/admin/overview/stats", get(stats_partial))
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

async fn overview_page(State(state): State<AppState>) -> Html<String> {
    let stats_html = blocking_stats(&state);
    Html(render_overview_page(&stats_html))
}

async fn stats_partial(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    let html = blocking_stats(&state);
    Ok(Html(html))
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn blocking_stats(state: &AppState) -> String {
    let (total, species, today) = state.with_db(|conn| {
        let t = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
        let s = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
        let d = today_count(conn);
        (t, s, d)
    });

    let sys = sample_system();

    let mut out = String::with_capacity(2048);

    // DB stats row
    let stat_card_total = stat_card("Total Detections", &total.to_string(), "#38bdf8");
    let stat_card_species = stat_card("Unique Species", &species.to_string(), "#34d399");
    let stat_card_today = stat_card(
        &format!("Today ({})", today_date_string()),
        &today.to_string(),
        "#a78bfa",
    );
    out.push_str(&format!(
        r#"<div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:1rem;margin-bottom:1.5rem;">{stat_card_total}{stat_card_species}{stat_card_today}</div>"#,
        stat_card_total = stat_card_total,
        stat_card_species = stat_card_species,
        stat_card_today = stat_card_today,
    ));

    // System health row
    let cpu = format!("{:.0}%", sys.cpu_usage_pct);
    let mem = format!(
        "{} / {}",
        crate::system_info::format_bytes(sys.used_memory_bytes),
        crate::system_info::format_bytes(sys.total_memory_bytes),
    );
    let temp = sys
        .cpu_temp_celsius
        .map(|t| format!("{t:.1}\u{b0}C"))
        .unwrap_or_else(|| "N/A".to_string());

    let cpu_card = stat_card("CPU", &cpu, "#fb923c");
    let mem_card = stat_card("Memory", &mem, "#60a5fa");
    let temp_card = stat_card("Temperature", &temp, "#f472b6");
    out.push_str(&format!(
        r#"<div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:1rem;">{cpu_card}{mem_card}{temp_card}</div>"#,
        cpu_card = cpu_card,
        mem_card = mem_card,
        temp_card = temp_card,
    ));

    out
}

fn stat_card(label: &str, value: &str, color: &str) -> String {
    format!(
        r#"<div style="background:#1e293b;border:1px solid #334155;border-radius:0.75rem;padding:1rem;text-align:center;">
  <div style="font-size:1.5rem;font-weight:700;color:{color};">{value}</div>
  <div style="font-size:0.8rem;color:#94a3b8;margin-top:0.25rem;">{label}</div>
</div>"#,
        value = escape_html(value),
        label = escape_html(label),
    )
}

fn render_overview_page(stats_html: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Admin Overview — BirdNet-Behavior</title>
    <script src="/static/htmx.min.js"></script>
    <link rel="stylesheet" href="/static/style.css">
    <style>
      body {{ background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; }}
      .container {{ max-width:1000px; margin:0 auto; padding:2rem 1rem; }}
      nav a {{ color:#94a3b8; text-decoration:none; margin-right:1.5rem; }}
      nav a.active, nav a:hover {{ color:#38bdf8; }}
      .card {{ background:#1e293b; border:1px solid #334155; border-radius:0.75rem; padding:1.5rem; margin-bottom:1.5rem; }}
      .section-title {{ font-size:1.1rem; font-weight:600; color:#38bdf8; margin-bottom:1rem; border-bottom:1px solid #334155; padding-bottom:0.5rem; }}
      .quick-links {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(200px,1fr)); gap:1rem; }}
      .quick-link {{ background:#0f172a; border:1px solid #334155; border-radius:0.5rem; padding:1rem; text-decoration:none; color:#e2e8f0; transition:border-color 0.2s; }}
      .quick-link:hover {{ border-color:#38bdf8; color:#38bdf8; }}
      .quick-link-title {{ font-weight:600; font-size:0.95rem; }}
      .quick-link-desc {{ font-size:0.75rem; color:#64748b; margin-top:0.25rem; }}
    </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem; padding:1rem 0; border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/admin/overview" class="active">Admin</a>
    <a href="/admin/settings">Settings</a>
  </nav>

  <h1 style="font-size:1.5rem;font-weight:700;margin-bottom:1.5rem;color:#f1f5f9;">Admin Overview</h1>

  <!-- Live stats (auto-refresh every 30s) -->
  <div class="card">
    <div class="section-title">System Status</div>
    <div id="overview-stats"
         hx-get="/admin/overview/stats"
         hx-trigger="load, every 30s"
         hx-swap="innerHTML">
      {stats_html}
    </div>
  </div>

  <!-- Quick navigation -->
  <div class="card">
    <div class="section-title">Administration</div>
    <div class="quick-links">
      <a href="/admin/settings" class="quick-link">
        <div class="quick-link-title">⚙️ Settings</div>
        <div class="quick-link-desc">Audio, detection, location, notifications</div>
      </a>
      <a href="/admin/species" class="quick-link">
        <div class="quick-link-title">🐦 Species Lists</div>
        <div class="quick-link-desc">Exclusion and allow-lists</div>
      </a>
      <a href="/admin/migrate" class="quick-link">
        <div class="quick-link-title">📥 Migration</div>
        <div class="quick-link-desc">Import from BirdNET-Pi (SQLite or CSV)</div>
      </a>
      <a href="/admin/system" class="quick-link">
        <div class="quick-link-title">🖥 System</div>
        <div class="quick-link-desc">Health, backups, database info</div>
      </a>
      <a href="/admin/notifications/test" class="quick-link">
        <div class="quick-link-title">🔔 Test Notifications</div>
        <div class="quick-link-desc">Send test messages to all channels</div>
      </a>
      <a href="/admin/notifications" class="quick-link">
        <div class="quick-link-title">📋 Notification Log</div>
        <div class="quick-link-desc">Recent notification history</div>
      </a>
      <a href="/admin/system/logs/page" class="quick-link">
        <div class="quick-link-title">📄 System Logs</div>
        <div class="quick-link-desc">Live log stream</div>
      </a>
      <a href="/admin/system/backups" class="quick-link">
        <div class="quick-link-title">💾 Backups</div>
        <div class="quick-link-desc">Database backup management</div>
      </a>
    </div>
  </div>

  <!-- Analytics quick links -->
  <div class="card">
    <div class="section-title">Analytics</div>
    <div class="quick-links">
      <a href="/heatmap" class="quick-link">
        <div class="quick-link-title">🗓 Activity Heatmap</div>
        <div class="quick-link-desc">24h × 7-day activity map</div>
      </a>
      <a href="/correlation" class="quick-link">
        <div class="quick-link-title">🔗 Species Correlation</div>
        <div class="quick-link-desc">Co-occurring species pairs</div>
      </a>
      <a href="/timeseries" class="quick-link">
        <div class="quick-link-title">📈 Time Series</div>
        <div class="quick-link-desc">Trends, diversity, peak activity</div>
      </a>
      <a href="/api/v2/export/csv" class="quick-link">
        <div class="quick-link-title">⬇️ Export CSV</div>
        <div class="quick-link-desc">Download all detections as CSV</div>
      </a>
    </div>
  </div>
</div>
</body>
</html>"#
    )
}
