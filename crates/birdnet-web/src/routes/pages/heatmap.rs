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
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Html;
use axum::routing::get;
use serde::Deserialize;

use birdnet_db::sqlite::{
    HeatmapCell, hourly_totals, species_hourly_activity, species_sparklines, top_species,
    weekly_heatmap,
};

use crate::state::AppState;

/// Mount heatmap routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/heatmap", get(heatmap_page))
        .route("/pages/heatmap-grid", get(heatmap_grid_partial))
        .route("/pages/hourly-totals", get(hourly_totals_partial))
        .route("/pages/activity-streamgraph", get(streamgraph_partial))
        .route("/pages/dawn-chorus", get(dawn_chorus_partial))
        .route(
            "/pages/seasonal-phenology",
            get(migration_ridgeline_partial),
        )
}

#[derive(Deserialize)]
struct HeatmapQuery {
    days: Option<u32>,
}

// ---------------------------------------------------------------------------
// GET /heatmap — full page
// ---------------------------------------------------------------------------

async fn heatmap_page(headers: HeaderMap) -> Html<String> {
    // O-20 help link drops a methodology shortcut next to the top eyebrow.
    let body = HEATMAP_CONTENT.replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::Analytics),
    );
    super::render_page_for_request("Activity Heatmap", &body, "heatmap", &headers)
}

const HEATMAP_CONTENT: &str = r#"<div class="page-head">
  <div>
    <div class="bnb-eyebrow" style="display:flex;align-items:center;gap:10px;flex-wrap:wrap;"><span>Behavioral analytics</span>{{help_link}}</div>
    <h1 class="display" style="font-size:34px;">When the yard is alive</h1>
    <p class="bnb-meta" style="margin-top:4px;">Detection frequency by hour of day and day of week.</p>
  </div>
  <div class="seg" id="range-controls">
    <button class="btn active" onclick="loadDays(7, this)">7 days</button>
    <button class="btn" onclick="loadDays(14, this)">14 days</button>
    <button class="btn" onclick="loadDays(30, this)">30 days</button>
    <button class="btn" onclick="loadDays(90, this)">90 days</button>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Who's singing, over time</div><h3>Activity streamgraph</h3></div></div>
  <div id="activity-streamgraph" hx-get="/pages/activity-streamgraph?days=7" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading streamgraph...</p>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Hour × day-of-week</div><h3>Activity grid</h3></div></div>
  <div id="heatmap-grid" hx-get="/pages/heatmap-grid?days=7" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading heatmap...</p>
  </div>
</div>

<div class="grid-2">
  <div class="bnb-card pad">
    <div class="section-header"><div><div class="bnb-eyebrow">The dawn chorus</div><h3>Circadian rhythm</h3></div></div>
    <div id="dawn-chorus" hx-get="/pages/dawn-chorus" hx-trigger="load" hx-swap="innerHTML">
      <p class="bnb-meta">Loading dawn chorus...</p>
    </div>
  </div>
  <div class="bnb-card pad">
    <div class="section-header"><div><div class="bnb-eyebrow">All days</div><h3>Detections by hour</h3></div></div>
    <div id="hourly-totals" hx-get="/pages/hourly-totals?days=7" hx-trigger="load" hx-swap="innerHTML">
      <p class="bnb-meta">Loading chart...</p>
    </div>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Arrivals & departures</div><h3>Seasonal phenology</h3></div></div>
  <div id="seasonal-phenology" hx-get="/pages/seasonal-phenology" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading phenology...</p>
  </div>
</div>

<script>
function loadDays(days, btn) {
  document.querySelectorAll('#range-controls .btn').forEach(b => b.classList.remove('active'));
  btn.classList.add('active');
  htmx.ajax('GET', '/pages/activity-streamgraph?days=' + days, '#activity-streamgraph');
  htmx.ajax('GET', '/pages/heatmap-grid?days=' + days, '#heatmap-grid');
  htmx.ajax('GET', '/pages/hourly-totals?days=' + days, '#hourly-totals');
}
</script>"#;

// ---------------------------------------------------------------------------
// GET /pages/heatmap-grid — SVG heatmap partial
// ---------------------------------------------------------------------------

async fn heatmap_grid_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(7).min(365);
    let result =
        tokio::task::spawn_blocking(move || state.with_db(|conn| weekly_heatmap(conn, days))).await;

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
    let result =
        tokio::task::spawn_blocking(move || state.with_db(|conn| hourly_totals(conn, days))).await;

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
// GET /pages/activity-streamgraph — per-species stacked activity
// ---------------------------------------------------------------------------

async fn streamgraph_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(7).clamp(2, 365);
    let result =
        tokio::task::spawn_blocking(move || state.with_db(|conn| species_sparklines(conn, days)))
            .await;

    match result {
        Ok(Ok(map)) => {
            let mut series: Vec<(String, Vec<i64>)> = map.into_iter().collect();
            // Most active species first → stable, readable stacking order.
            series.sort_by(|a, b| {
                b.1.iter()
                    .sum::<i64>()
                    .cmp(&a.1.iter().sum::<i64>())
                    .then_with(|| a.0.cmp(&b.0))
            });
            series.truncate(8);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                super::viz::streamgraph(&series),
            )
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading streamgraph</p>".to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// GET /pages/dawn-chorus — circadian polar of the top species
// ---------------------------------------------------------------------------

#[allow(clippy::cast_precision_loss)]
async fn dawn_chorus_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let series = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let top = top_species(conn, 5).unwrap_or_default();
            top.iter()
                .map(|s| {
                    let mut arr = [0.0_f64; 24];
                    if let Ok(hours) = species_hourly_activity(conn, &s.com_name) {
                        for hc in hours {
                            if let Ok(h) = hc.hour.parse::<usize>()
                                && h < 24
                            {
                                arr[h] = hc.count as f64;
                            }
                        }
                    }
                    (s.com_name.clone(), arr)
                })
                .collect::<Vec<_>>()
        })
    })
    .await;

    let Ok(series) = series else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading dawn chorus</p>".to_string(),
        );
    };
    // Current hour-of-day (UTC) for the "now" hand on the polar.
    #[allow(clippy::cast_precision_loss)]
    let now_h = {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        (secs % 86_400) as f64 / 3600.0
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        super::viz::circadian_polar(&series, now_h),
    )
}

// ---------------------------------------------------------------------------
// GET /pages/seasonal-phenology — per-species seasonal joyplot
// (the dedicated /migration page owns the canonical /pages/migration-ridgeline)
// ---------------------------------------------------------------------------

async fn migration_ridgeline_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    // One dense query for the year, then bucket each species into ~52 weeks.
    let result =
        tokio::task::spawn_blocking(move || state.with_db(|conn| species_sparklines(conn, 364)))
            .await;

    match result {
        Ok(Ok(map)) => {
            let mut ranked: Vec<(String, Vec<i64>)> = map.into_iter().collect();
            ranked.sort_by(|a, b| {
                b.1.iter()
                    .sum::<i64>()
                    .cmp(&a.1.iter().sum::<i64>())
                    .then_with(|| a.0.cmp(&b.0))
            });
            ranked.truncate(7);
            let series: Vec<(String, Vec<i64>)> = ranked
                .into_iter()
                .map(|(name, daily)| {
                    let weekly: Vec<i64> = daily.chunks(7).map(|c| c.iter().sum()).collect();
                    (name, weekly)
                })
                .collect();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                super::viz::ridgeline(&series),
            )
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading ridgeline</p>".to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// SVG heatmap renderer
// ---------------------------------------------------------------------------

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

#[allow(clippy::too_many_lines)]
fn render_heatmap_svg(cells: &[HeatmapCell]) -> String {
    if cells.is_empty() {
        return r#"<p style="color:var(--fg-4);text-align:center;padding:2rem;">
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
        r#"<div style="overflow-x:auto;">
<svg xmlns="http://www.w3.org/2000/svg" width="{svg_w}" height="{svg_h}"
     style="font-family:system-ui,sans-serif;">
  <!-- Background -->
  <rect width="{svg_w}" height="{svg_h}" fill="var(--surface)" rx="8"/>
"#
    );

    // Hour labels (0..23)
    for h in 0..24_usize {
        let x = label_w + h * cell_w + cell_w / 2;
        let _ = write!(
            svg,
            r#"  <text x="{x}" y="{y}" text-anchor="middle" font-size="9"
                fill="var(--fg-4)">{h:02}</text>
"#,
            y = label_h - 4,
        );
    }

    // Day-of-week labels and cells
    for dow in 0..7_usize {
        let y_label = label_h + dow * cell_h + cell_h / 2 + 4;
        let _ = write!(
            svg,
            r#"  <text x="{x}" y="{y_label}" text-anchor="end" font-size="10"
                fill="var(--fg-3)">{day}</text>
"#,
            x = label_w - 4,
            day = DAYS[dow],
        );

        for (hour, &count) in grid[dow].iter().enumerate() {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                clippy::cast_possible_wrap,
                clippy::cast_lossless
            )]
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
    let _ = writeln!(
        svg,
        r#"  <text x="{label_w}" y="{legend_y}" font-size="9" fill="var(--fg-4)">Low</text>"#,
    );
    for i in 0..20_usize {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        let color = heat_color(i as f64 / 19.0);
        let lx = label_w + 30 + i * 12;
        let _ = writeln!(
            svg,
            r#"  <rect x="{lx}" y="{ly}" width="12" height="10" fill="{color}"/>"#,
            ly = legend_y - 8,
        );
    }
    let _ = writeln!(
        svg,
        r#"  <text x="{lx}" y="{legend_y}" font-size="9" fill="var(--fg-4)">High</text>"#,
        lx = label_w + 30 + 20 * 12 + 4,
    );

    svg.push_str("</svg></div>");
    svg
}

/// Map a 0.0–1.0 intensity to an on-brand, theme-aware heat colour.
///
/// Mirrors the documented `.bnb-heat-*` ramp: the warm **dawn** hue deepens
/// over the neutral surface as activity climbs, then tips toward the **rare**
/// hue for the busiest cells. Uses `color-mix(in oklch, …)` so the ramp tracks
/// the active light/dark theme instead of baking in fixed sRGB values (the old
/// blue→red rainbow ignored the palette and looked identical in both themes).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn heat_color(t: f64) -> String {
    let t = t.clamp(0.0, 1.0);
    if t < 0.001 {
        return "var(--surface-2)".to_string(); // empty cell
    }
    if t < 0.6 {
        // surface-2 → dawn (12% … 100% mix)
        let pct = (t / 0.6).mul_add(88.0, 12.0).round() as i32;
        format!("color-mix(in oklch, var(--dawn) {pct}%, var(--surface-2))")
    } else {
        // dawn → rare for the hottest cells
        let s = ((t - 0.6) / 0.4 * 100.0).round() as i32;
        format!("color-mix(in oklch, var(--rare) {s}%, var(--dawn))")
    }
}

// ---------------------------------------------------------------------------
// Hourly bar chart renderer
// ---------------------------------------------------------------------------

fn render_hourly_bars(totals: &[birdnet_db::sqlite::HourTotal]) -> String {
    if totals.is_empty() {
        return r#"<p style="color:var(--fg-4);text-align:center;padding:2rem;">
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
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{svg_w}" height="{svg_h}"
             style="font-family:system-ui,sans-serif;display:block;">
  <rect width="{svg_w}" height="{svg_h}" fill="var(--surface)" rx="8"/>
"#
    );

    // Build a lookup by hour
    let mut by_hour = [0i64; 24];
    for h in totals {
        by_hour[h.hour as usize] = h.count;
    }

    for (hour, &count) in by_hour.iter().enumerate() {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        let bar_h = if max > 0 {
            (count as f64 / max as f64 * chart_h as f64).round() as u32
        } else {
            0
        };
        let x = 20 + hour * bar_w;
        let y = chart_h - bar_h as usize;
        // Dawn/dusk hours: 5-8 and 18-21 get a lighter color
        let color = if (5..=8).contains(&hour) || (18..=21).contains(&hour) {
            "var(--dawn)"
        } else {
            "var(--moss)"
        };
        let _ = write!(
            svg,
            r#"  <rect x="{x}" y="{y}" width="{bw}" height="{bar_h}"
                  fill="{color}" rx="2">
                <title>{hour:02}:00 — {count} detections</title></rect>
  <text x="{lx}" y="{ly}" text-anchor="middle" font-size="8" fill="var(--fg-4)">
    {hour:02}</text>
"#,
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
        assert_eq!(heat_color(0.0), "var(--surface-2)");
    }

    #[test]
    fn heat_color_full() {
        // Hottest cells tip toward the rare hue, theme-aware via color-mix.
        let c = heat_color(1.0);
        assert!(c.contains("color-mix"));
        assert!(c.contains("var(--rare)"));
    }

    #[test]
    fn heat_color_mid() {
        // Mid intensity is a dawn-over-surface mix (not the old sRGB rainbow).
        let c = heat_color(0.5);
        assert!(c.contains("color-mix"));
        assert!(c.contains("var(--dawn)"));
    }

    #[test]
    fn render_heatmap_svg_empty() {
        let html = render_heatmap_svg(&[]);
        assert!(html.contains("No data"));
    }

    #[test]
    fn render_heatmap_svg_with_cells() {
        let cells = vec![
            HeatmapCell {
                dow: 1,
                hour: 7,
                count: 10,
            },
            HeatmapCell {
                dow: 2,
                hour: 8,
                count: 5,
            },
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
