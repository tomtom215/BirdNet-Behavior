//! 24-hour × 7-day activity heatmap page and partials.
//!
//! Shows a grid of detection counts by (hour-of-day × day-of-week) so users
//! can quickly see when birds are most active throughout the week.
//!
//! | Path | Purpose |
//! |------|---------|
//! | (embedded)                   | Patterns home, "When active" tab    |
//! | `GET /pages/heatmap-grid`    | HTMX partial — SVG heatmap grid      |
//! | `GET /pages/hourly-totals`   | HTMX partial — bar chart by hour     |

use std::fmt::Write as _;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::routing::get;
use serde::Deserialize;

use birdnet_db::sqlite::{
    HeatmapCell, hourly_totals, species_hourly_activity_batch, species_sparklines, top_species,
    weekly_heatmap,
};

use crate::analytics_cache::cached_fragment;
use crate::state::AppState;

/// Fallback body served (uncached) when an analytics fragment query errors, so a
/// transient failure never pins an error message in the cache for the TTL.
const FRAGMENT_ERR: &str = r#"<p class="bnb-meta">Analytics temporarily unavailable.</p>"#;

/// Mount heatmap routes.
pub fn router() -> Router<AppState> {
    Router::new()
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
// Page content — embedded in the Patterns home ("When active" tab)
// ---------------------------------------------------------------------------

/// The heatmap surface, rendered for embedding by `homes::patterns`.
pub(super) fn content() -> String {
    // O-20 help link drops a methodology shortcut next to the top eyebrow.
    HEATMAP_CONTENT.replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::Analytics),
    )
}

const HEATMAP_CONTENT: &str = r#"<div class="page-head">
  <div>
    <div class="bnb-eyebrow hm-eyebrow"><span>Behavioral analytics</span>{{help_link}}</div>
    <h1 class="display hm-h1">When the yard is alive</h1>
    <p class="bnb-lede hm-lede"><b>Darker cells mean more birds heard that hour.</b> Mornings light up first — the dawn chorus — with a smaller evening lift. Quiet on the left of each row is the middle of the night.</p>
  </div>
  <div class="seg" id="range-controls">
    <button class="btn active" data-days="7">7 days</button>
    <button class="btn" data-days="14">14 days</button>
    <button class="btn" data-days="30">30 days</button>
    <button class="btn" data-days="90">90 days</button>
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

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">All days combined</div><h3>Detections by hour</h3></div><a class="action" href="/patterns?tab=dawn">See who sings when →</a></div>
  <p class="bnb-meta hm-hourly-note">Totals for every hour. Dawn (5–8 am) and dusk (6–9 pm) bars are amber; the rest green.</p>
  <div id="hourly-totals" hx-get="/pages/hourly-totals?days=7" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading chart...</p>
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
document.getElementById('range-controls').addEventListener('click', function(e) {
  const btn = e.target.closest('button[data-days]');
  if (btn) loadDays(parseInt(btn.dataset.days, 10), btn);
});
</script>"#;

// ---------------------------------------------------------------------------
// GET /pages/heatmap-grid — SVG heatmap partial
// ---------------------------------------------------------------------------

/// Compute the hour × day-of-week heatmap SVG, or `None` on a query error.
fn compute_heatmap_grid(state: &AppState, days: u32) -> Option<String> {
    state
        .with_db(|conn| weekly_heatmap(conn, days))
        .ok()
        .map(|cells| render_heatmap_svg(&cells))
}

async fn heatmap_grid_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(7).min(365);
    let html = cached_fragment(
        &state,
        format!("heatmap-grid:{days}"),
        FRAGMENT_ERR,
        move |s| compute_heatmap_grid(s, days),
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// GET /pages/hourly-totals — bar chart partial
// ---------------------------------------------------------------------------

/// Compute the by-hour bar chart, or `None` on a query error.
fn compute_hourly_totals(state: &AppState, days: u32) -> Option<String> {
    state
        .with_db(|conn| hourly_totals(conn, days))
        .ok()
        .map(|totals| render_hourly_bars(&totals))
}

async fn hourly_totals_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(7).min(365);
    let html = cached_fragment(
        &state,
        format!("hourly-totals:{days}"),
        FRAGMENT_ERR,
        move |s| compute_hourly_totals(s, days),
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// GET /pages/activity-streamgraph — per-species stacked activity
// ---------------------------------------------------------------------------

/// Compute the per-species activity streamgraph (top 8), or `None` on error.
fn compute_streamgraph(state: &AppState, days: u32) -> Option<String> {
    let map = state.with_db(|conn| species_sparklines(conn, days)).ok()?;
    let mut series: Vec<(String, Vec<i64>)> = map.into_iter().collect();
    // Most active species first → stable, readable stacking order.
    series.sort_by(|a, b| {
        b.1.iter()
            .sum::<i64>()
            .cmp(&a.1.iter().sum::<i64>())
            .then_with(|| a.0.cmp(&b.0))
    });
    series.truncate(8);
    Some(super::viz::streamgraph(&series))
}

async fn streamgraph_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(7).clamp(2, 365);
    let html = cached_fragment(
        &state,
        format!("streamgraph:{days}"),
        FRAGMENT_ERR,
        move |s| compute_streamgraph(s, days),
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// GET /pages/dawn-chorus — circadian polar of the top species
// ---------------------------------------------------------------------------

/// Compute the dawn-chorus circadian polar for the top 5 species.
///
/// Uses one batched hourly-activity query rather than a scan per species (the
/// previous N+1). Always returns `Some`: an empty yard renders an empty polar,
/// not an error.
#[allow(clippy::cast_precision_loss)]
fn compute_dawn_chorus(state: &AppState) -> String {
    let series = state.with_db(|conn| {
        let top = top_species(conn, 5).unwrap_or_default();
        let names: Vec<String> = top.iter().map(|s| s.com_name.clone()).collect();
        let hourly = species_hourly_activity_batch(conn, &names).unwrap_or_default();
        top.into_iter()
            .map(|s| {
                let mut arr = [0.0_f64; 24];
                if let Some(counts) = hourly.get(&s.com_name) {
                    for (i, &c) in counts.iter().enumerate() {
                        arr[i] = c as f64;
                    }
                }
                (s.com_name, arr)
            })
            .collect::<Vec<_>>()
    });
    // Current hour-of-day (UTC) for the "now" hand on the polar.
    let now_h = {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        (secs % 86_400) as f64 / 3600.0
    };
    super::viz::circadian_polar(&series, now_h)
}

async fn dawn_chorus_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let html = cached_fragment(&state, "dawn-chorus".to_string(), FRAGMENT_ERR, |s| {
        Some(compute_dawn_chorus(s))
    })
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// GET /pages/seasonal-phenology — per-species seasonal joyplot
// (the dedicated /migration page owns the canonical /pages/migration-ridgeline)
// ---------------------------------------------------------------------------

/// Compute the seasonal-phenology ridgeline, or `None` on a query error.
///
/// Top 7 species, weekly buckets over the year. This is the heaviest single
/// analytics query (a full year of per-species daily counts), so caching it
/// matters most.
fn compute_seasonal_phenology(state: &AppState) -> Option<String> {
    // One dense query for the year, then bucket each species into ~52 weeks.
    let map = state.with_db(|conn| species_sparklines(conn, 364)).ok()?;
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
    Some(super::viz::ridgeline(&series))
}

async fn migration_ridgeline_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let html = cached_fragment(
        &state,
        "seasonal-phenology".to_string(),
        FRAGMENT_ERR,
        compute_seasonal_phenology,
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// Pre-warm
// ---------------------------------------------------------------------------

/// Pre-compute and cache this page's default-range fragments.
///
/// Runs the same `compute_*` functions the handlers use, under the keys they
/// read, so the first visit (and each background refresh) is instant.
pub fn prewarm(state: &AppState) {
    let cache = state.analytics_cache();
    if let Some(h) = compute_streamgraph(state, 7) {
        cache.put("streamgraph:7".to_string(), h);
    }
    if let Some(h) = compute_heatmap_grid(state, 7) {
        cache.put("heatmap-grid:7".to_string(), h);
    }
    if let Some(h) = compute_hourly_totals(state, 7) {
        cache.put("hourly-totals:7".to_string(), h);
    }
    cache.put("dawn-chorus".to_string(), compute_dawn_chorus(state));
    if let Some(h) = compute_seasonal_phenology(state) {
        cache.put("seasonal-phenology".to_string(), h);
    }
}

// ---------------------------------------------------------------------------
// SVG heatmap renderer
// ---------------------------------------------------------------------------

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

#[allow(clippy::too_many_lines)]
fn render_heatmap_svg(cells: &[HeatmapCell]) -> String {
    if cells.is_empty() {
        return r#"<p class="hm-empty">
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
        r#"<div class="hm-scroll">
<svg xmlns="http://www.w3.org/2000/svg" width="{svg_w}" height="{svg_h}"
     class="hm-svg">
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
        return r#"<p class="hm-empty">
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
             class="hm-svg-block">
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
