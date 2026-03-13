//! 24-hour × 7-day activity heatmap page and partials.
//!
//! Shows a grid of detection counts by (hour-of-day × day-of-week) so users
//! can quickly see when birds are most active throughout the week.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET /heatmap`               | Full heatmap page                    |
//! | `GET /pages/heatmap-grid`    | HTMX partial — SVG heatmap grid      |
//! | `GET /pages/hourly-totals`   | HTMX partial — bar chart by hour     |

use std::fmt::Write as _;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::Html;
use axum::routing::get;
use serde::Deserialize;

use birdnet_db::sqlite::{HeatmapCell, weekly_heatmap, hourly_totals};

use crate::state::AppState;

/// Mount heatmap routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/heatmap", get(heatmap_page))
        .route("/pages/heatmap-grid", get(heatmap_grid_partial))
        .route("/pages/hourly-totals", get(hourly_totals_partial))
}

#[derive(Deserialize)]
struct HeatmapQuery {
    days: Option<u32>,
}

// ---------------------------------------------------------------------------
// GET /heatmap — full page
// ---------------------------------------------------------------------------

async fn heatmap_page() -> Html<String> {
    Html(HEATMAP_PAGE.to_string())
}

const HEATMAP_PAGE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width,initial-scale=1.0">
  <title>Activity Heatmap — BirdNet-Behavior</title>
  <script src="/static/htmx.min.js"></script>
  <style>
    body { background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; margin:0; }
    .container { max-width:1100px; margin:0 auto; padding:2rem 1rem; }
    nav a { color:#94a3b8; text-decoration:none; margin-right:1.5rem; font-size:.9rem; }
    nav a:hover, nav a.active { color:#38bdf8; }
    h1 { font-size:1.5rem; font-weight:700; color:#f1f5f9; margin-bottom:.5rem; }
    .subtitle { color:#64748b; font-size:.875rem; margin-bottom:2rem; }
    .card { background:#1e293b; border:1px solid #334155; border-radius:.75rem;
            padding:1.5rem; margin-bottom:1.5rem; }
    .section-title { font-size:1rem; font-weight:600; color:#38bdf8;
                     margin-bottom:1rem; }
    .controls { display:flex; gap:.75rem; margin-bottom:1.5rem; flex-wrap:wrap; }
    .btn { padding:.4rem 1rem; border-radius:.375rem; border:1px solid #334155;
           background:#1e293b; color:#e2e8f0; cursor:pointer; font-size:.875rem; }
    .btn.active, .btn:hover { background:#0ea5e9; border-color:#0ea5e9; color:#fff; }
  </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem;padding:1rem 0;border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/heatmap" class="active">Heatmap</a>
    <a href="/analytics">Analytics</a>
    <a href="/correlation">Correlation</a>
    <a href="/admin">Admin</a>
  </nav>

  <h1>Activity Heatmap</h1>
  <p class="subtitle">Detection frequency by hour of day and day of week</p>

  <div class="controls">
    <button class="btn active" onclick="loadDays(7, this)">7 days</button>
    <button class="btn" onclick="loadDays(14, this)">14 days</button>
    <button class="btn" onclick="loadDays(30, this)">30 days</button>
    <button class="btn" onclick="loadDays(90, this)">90 days</button>
  </div>

  <div class="card">
    <div class="section-title">Hour × Day-of-Week Grid</div>
    <div id="heatmap-grid"
         hx-get="/pages/heatmap-grid?days=7"
         hx-trigger="load"
         hx-swap="innerHTML">
      <p style="color:#64748b;">Loading heatmap…</p>
    </div>
  </div>

  <div class="card">
    <div class="section-title">Detections by Hour (all days)</div>
    <div id="hourly-totals"
         hx-get="/pages/hourly-totals?days=7"
         hx-trigger="load"
         hx-swap="innerHTML">
      <p style="color:#64748b;">Loading chart…</p>
    </div>
  </div>
</div>

<script>
function loadDays(days, btn) {
  document.querySelectorAll('.btn').forEach(b => b.classList.remove('active'));
  btn.classList.add('active');
  htmx.ajax('GET', '/pages/heatmap-grid?days=' + days, '#heatmap-grid');
  htmx.ajax('GET', '/pages/hourly-totals?days=' + days, '#hourly-totals');
}
</script>
</body>
</html>"##;

// ---------------------------------------------------------------------------
// GET /pages/heatmap-grid — SVG heatmap partial
// ---------------------------------------------------------------------------

async fn heatmap_grid_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(7).min(365);
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| weekly_heatmap(conn, days))
    })
    .await;

    match result {
        Ok(Ok(cells)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_heatmap_svg(&cells),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading heatmap</p>".to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// GET /pages/hourly-totals — bar chart partial
// ---------------------------------------------------------------------------

async fn hourly_totals_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(7).min(365);
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| hourly_totals(conn, days))
    })
    .await;

    match result {
        Ok(Ok(totals)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_hourly_bars(&totals),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading hourly totals</p>".to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// SVG heatmap renderer
// ---------------------------------------------------------------------------

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn render_heatmap_svg(cells: &[HeatmapCell]) -> String {
    if cells.is_empty() {
        return r#"<p style="color:#64748b;text-align:center;padding:2rem;">
            No data available for the selected period.
        </p>"#
        .to_string();
    }

    // Build lookup: (dow, hour) → count
    let mut grid = [[0i64; 24]; 7];
    let mut max_count = 0i64;
    for cell in cells {
        let dow = (cell.dow as usize).min(6);
        let hour = (cell.hour as usize).min(23);
        grid[dow][hour] = cell.count;
        if cell.count > max_count {
            max_count = cell.count;
        }
    }

    let cell_w = 32;
    let cell_h = 22;
    let label_w = 36;
    let label_h = 20;
    let svg_w = label_w + 24 * cell_w + 20;
    let svg_h = label_h + 7 * cell_h + 40;

    let mut svg = format!(
        r##"<div style="overflow-x:auto;">
<svg xmlns="http://www.w3.org/2000/svg" width="{svg_w}" height="{svg_h}"
     style="font-family:system-ui,sans-serif;">
  <!-- Background -->
  <rect width="{svg_w}" height="{svg_h}" fill="#0f172a" rx="8"/>
"##
    );

    // Hour labels (0..23)
    for h in 0..24_usize {
        let x = label_w + h * cell_w + cell_w / 2;
        let _ = write!(
            svg,
            r##"  <text x="{x}" y="{y}" text-anchor="middle" font-size="9"
                fill="#64748b">{h:02}</text>
"##,
            y = label_h - 4,
        );
    }

    // Day-of-week labels and cells
    for dow in 0..7_usize {
        let y_label = label_h + dow * cell_h + cell_h / 2 + 4;
        let _ = write!(
            svg,
            r##"  <text x="{x}" y="{y_label}" text-anchor="end" font-size="10"
                fill="#94a3b8">{day}</text>
"##,
            x = label_w - 4,
            day = DAYS[dow],
        );

        for hour in 0..24_usize {
            let count = grid[dow][hour];
            let intensity = if max_count > 0 {
                count as f64 / max_count as f64
            } else {
                0.0
            };
            let color = heat_color(intensity);
            let x = label_w + hour * cell_w;
            let y = label_h + dow * cell_h;
            let title = format!("{} {}:00 — {} detections", DAYS[dow], hour, count);
            let _ = write!(
                svg,
                r#"  <rect x="{x}" y="{y}" width="{cw}" height="{ch}" fill="{color}"
                      rx="2" ry="2">
                    <title>{title}</title></rect>
"#,
                cw = cell_w - 2,
                ch = cell_h - 2,
            );
        }
    }

    // Legend
    let legend_y = label_h + 7 * cell_h + 10;
    let _ = write!(
        svg,
        r##"  <text x="{lx}" y="{legend_y}" font-size="9" fill="#64748b">Low</text>
"##,
        lx = label_w,
    );
    for i in 0..20_usize {
        let color = heat_color(i as f64 / 19.0);
        let lx = label_w + 30 + i * 12;
        let _ = write!(
            svg,
            r#"  <rect x="{lx}" y="{ly}" width="12" height="10" fill="{color}"/>
"#,
            ly = legend_y - 8,
        );
    }
    let _ = write!(
        svg,
        r##"  <text x="{lx}" y="{legend_y}" font-size="9" fill="#64748b">High</text>
"##,
        lx = label_w + 30 + 20 * 12 + 4,
    );

    svg.push_str("</svg></div>");
    svg
}

/// Map a 0.0–1.0 intensity to a sky-blue → green → amber heat color.
fn heat_color(t: f64) -> String {
    let t = t.clamp(0.0, 1.0);
    if t < 0.001 {
        return "#1e293b".to_string(); // empty cell — dark slate
    }
    // Interpolate: dark-blue → cyan → green → yellow → orange
    let (r, g, b) = if t < 0.25 {
        let s = t / 0.25;
        lerp_rgb((14, 165, 233), (6, 182, 212), s) // sky → cyan
    } else if t < 0.5 {
        let s = (t - 0.25) / 0.25;
        lerp_rgb((6, 182, 212), (74, 222, 128), s) // cyan → green
    } else if t < 0.75 {
        let s = (t - 0.5) / 0.25;
        lerp_rgb((74, 222, 128), (251, 191, 36), s) // green → yellow
    } else {
        let s = (t - 0.75) / 0.25;
        lerp_rgb((251, 191, 36), (239, 68, 68), s) // yellow → red
    };
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let lerp = |a: u8, b: u8| -> u8 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = a as f64 + (b as f64 - a as f64) * t;
        v.round() as u8
    };
    (lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
}

// ---------------------------------------------------------------------------
// Hourly bar chart renderer
// ---------------------------------------------------------------------------

fn render_hourly_bars(totals: &[birdnet_db::sqlite::HourTotal]) -> String {
    if totals.is_empty() {
        return r#"<p style="color:#64748b;text-align:center;padding:2rem;">
            No data available for the selected period.
        </p>"#
        .to_string();
    }

    let max = totals.iter().map(|h| h.count).max().unwrap_or(1);
    let bar_w = 24;
    let chart_h = 120;
    let label_h = 20;
    let svg_w = 24 * bar_w + 40;
    let svg_h = chart_h + label_h + 10;

    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{svg_w}" height="{svg_h}"
             style="font-family:system-ui,sans-serif;display:block;">
  <rect width="{svg_w}" height="{svg_h}" fill="#0f172a" rx="8"/>
"##
    );

    // Build a lookup by hour
    let mut by_hour = [0i64; 24];
    for h in totals {
        by_hour[h.hour as usize] = h.count;
    }

    for hour in 0..24_usize {
        let count = by_hour[hour];
        #[allow(clippy::cast_precision_loss)]
        let bar_h = if max > 0 {
            (count as f64 / max as f64 * chart_h as f64).round() as u32
        } else {
            0
        };
        let x = 20 + hour * bar_w;
        let y = chart_h - bar_h as usize;
        // Dawn/dusk hours: 5-8 and 18-21 get a lighter color
        let color = if (5..=8).contains(&hour) || (18..=21).contains(&hour) {
            "#fbbf24"
        } else {
            "#0ea5e9"
        };
        let _ = write!(
            svg,
            r##"  <rect x="{x}" y="{y}" width="{bw}" height="{bar_h}"
                  fill="{color}" rx="2">
                <title>{hour:02}:00 — {count} detections</title></rect>
  <text x="{lx}" y="{ly}" text-anchor="middle" font-size="8" fill="#64748b">
    {hour:02}</text>
"##,
            bw = bar_w - 2,
            lx = x + bar_w / 2,
            ly = chart_h + label_h,
        );
    }

    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_color_empty() {
        assert_eq!(heat_color(0.0), "#1e293b");
    }

    #[test]
    fn heat_color_full() {
        let c = heat_color(1.0);
        assert!(c.starts_with('#'));
        assert_eq!(c.len(), 7);
    }

    #[test]
    fn heat_color_mid() {
        let c = heat_color(0.5);
        assert!(c.starts_with('#'));
    }

    #[test]
    fn lerp_rgb_endpoints() {
        let (r, g, b) = lerp_rgb((0, 0, 0), (255, 255, 255), 0.0);
        assert_eq!((r, g, b), (0, 0, 0));
        let (r, g, b) = lerp_rgb((0, 0, 0), (255, 255, 255), 1.0);
        assert_eq!((r, g, b), (255, 255, 255));
    }

    #[test]
    fn render_heatmap_svg_empty() {
        let html = render_heatmap_svg(&[]);
        assert!(html.contains("No data"));
    }

    #[test]
    fn render_heatmap_svg_with_cells() {
        let cells = vec![
            HeatmapCell { dow: 1, hour: 7, count: 10 },
            HeatmapCell { dow: 2, hour: 8, count: 5 },
        ];
        let svg = render_heatmap_svg(&cells);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("Mon"));
    }

    #[test]
    fn render_hourly_bars_empty() {
        let html = render_hourly_bars(&[]);
        assert!(html.contains("No data"));
    }

    #[test]
    fn render_hourly_bars_with_data() {
        use birdnet_db::sqlite::HourTotal;
        let totals = vec![
            HourTotal { hour: 7, count: 20 },
            HourTotal { hour: 8, count: 15 },
        ];
        let svg = render_hourly_bars(&totals);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("20 detections"));
    }
}
