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


use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::routing::get;
use serde::Deserialize;

use birdnet_db::sqlite::{HeatmapCell, hourly_totals, species_sparklines, weekly_heatmap};

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

static HEATMAP_CONTENT: crate::seal::Sealed = crate::seal::Sealed::new(
    r#"<div class="page-head">
  <div>
    <div class="bnb-eyebrow hm-eyebrow"><span>Behavioral analytics</span>{{help_link}}</div>
    <h1 class="display hm-h1">When the yard is alive</h1>
    <p class="bnb-lede hm-lede"><b>When your birds are out, and who is singing.</b> Two views of the same days: the flow of species over time, then the hours they favour.</p>
  </div>
  <!-- role/name/state: this row conveyed the selected range with a CSS
       class alone. `templates/today.html` already does the same control with
       `role="group"` + `aria-pressed`, so the pattern was in the codebase.
       `type="button"` because the default is submit. -->
  <div class="seg" id="range-controls" role="group" aria-label="Time range">
    <button type="button" class="btn active" data-days="7" aria-pressed="true">7 days</button>
    <button type="button" class="btn" data-days="14" aria-pressed="false">14 days</button>
    <button type="button" class="btn" data-days="30" aria-pressed="false">30 days</button>
    <button type="button" class="btn" data-days="90" aria-pressed="false">90 days</button>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Who's singing, over time</div><h2 class="sh-h">Activity streamgraph</h2></div></div>
  <p class="bnb-meta hm-chart-note">Each band is one species; its thickness is that day's detection count, stacked around a centre line. Dates run left to right.</p>
  <div id="activity-streamgraph" hx-get="/pages/activity-streamgraph?days=7" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading streamgraph...</p>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Hour × day-of-week</div><h3>Activity grid</h3></div></div>
  <p class="bnb-meta hm-chart-note">A stronger colour means more birds heard that hour (the key under the grid runs from none to most). Mornings light up first — the dawn chorus — with a smaller evening lift. Quiet on the left of each row is the middle of the night.</p>
  <div id="heatmap-grid" hx-get="/pages/heatmap-grid?days=7" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading heatmap...</p>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">All days combined</div><h2 class="sh-h">Detections by hour</h2></div><a class="action" href="/patterns?tab=dawn">See who sings when →</a></div>
  <p class="bnb-meta hm-hourly-note">Totals for every hour. Dawn (5–8 am) and dusk (6–9 pm) bars are green; the rest are grey.</p>
  <div id="hourly-totals" hx-get="/pages/hourly-totals?days=7" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading chart...</p>
  </div>
</div>

<script>
function loadDays(days, btn) {
  document.querySelectorAll('#range-controls .btn').forEach(function (b) {
    b.classList.remove('active');
    b.setAttribute('aria-pressed', 'false');
  });
  btn.classList.add('active');
  btn.setAttribute('aria-pressed', 'true');
  htmx.ajax('GET', '/pages/activity-streamgraph?days=' + days, '#activity-streamgraph');
  htmx.ajax('GET', '/pages/heatmap-grid?days=' + days, '#heatmap-grid');
  htmx.ajax('GET', '/pages/hourly-totals?days=' + days, '#hourly-totals');
}
document.getElementById('range-controls').addEventListener('click', function(e) {
  const btn = e.target.closest('button[data-days]');
  if (btn) loadDays(parseInt(btn.dataset.days, 10), btn);
});
</script>"#,
);

// ---------------------------------------------------------------------------
// GET /pages/heatmap-grid — SVG heatmap partial
// ---------------------------------------------------------------------------

/// Compute the hour × day-of-week heatmap SVG, or `None` on a query error.
fn compute_heatmap_grid(state: &AppState, days: u32) -> Option<String> {
    let cells = state
        .with_read_db(|conn| weekly_heatmap(conn, days))
        .ok()?;
    let today = state
        .with_read_db(|conn| {
            conn.query_row("SELECT date('now','localtime')", [], |r| r.get::<_, String>(0))
        })
        .ok();
    // How many days of each weekday the window held and the station listened on, so a
    // weekday it was off for is not drawn as a quiet one, and a 90-day window's thirteen
    // Mondays are compared with its twelve Tuesdays per day and not in total.
    let n = days as usize;
    let effort = today.as_deref().and_then(|end| {
        let from = super::listening::first_date(end, n)?;
        state
            .with_read_db(|conn| super::listening::effort_by_date(conn, &from, end))
            .ok()
    });
    let listened = match (today.as_deref(), effort.as_ref()) {
        (Some(end), Some(e)) => super::listening::listened_days(e, end, n),
        _ => None,
    };
    let mut occurrences = [0_u32; 7];
    for d in 0..n {
        let on = listened.as_ref().is_none_or(|l| l.get(d).copied().unwrap_or(true));
        let dow = today
            .as_deref()
            .and_then(|end| {
                super::viz::kit::epoch_day(end).map(|e| ((e - (n as i64 - 1 - d as i64) + 4).rem_euclid(7)) as usize)
            })
            // Without a date, spread the days over the week evenly.
            .unwrap_or(d % 7);
        if on {
            occurrences[dow] += 1;
        }
    }
    // Today's weekday and the hour it is now, so the hours still to come are not drawn as quiet ones.
    let now = today.as_deref().and_then(super::viz::kit::epoch_day).map(|e| {
        ((e + 4).rem_euclid(7) as usize, super::now_hour_local().floor() as usize)
    });
    Some(render_heatmap_svg(&cells, occurrences, listened.is_some(), days, now))
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
    let totals = state.with_read_db(|conn| hourly_totals(conn, days)).ok()?;
    // Today's sun, on a window short enough for today's sun to stand for it.
    let solar = if days <= 14 {
        state.with_read_db(super::solar_times_local)
    } else {
        None
    };
    Some(render_hourly_bars(&totals, solar, days))
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

/// Compute the per-species activity streamgraph (top 8, and the rest as one band), or
/// `None` on error.
fn compute_streamgraph(state: &AppState, days: u32) -> Option<String> {
    let map = state
        .with_read_db(|conn| species_sparklines(conn, days))
        .ok()?;
    let mut series: Vec<(String, Vec<i64>)> = map.into_iter().collect();
    // Most active species first → stable, readable stacking order.
    series.sort_by(|a, b| {
        b.1.iter()
            .sum::<i64>()
            .cmp(&a.1.iter().sum::<i64>())
            .then_with(|| a.0.cmp(&b.0))
    });
    // The eight busiest are drawn one by one; every other species is one more band,
    // so a day's thickness is still that day's detections and not "the top eight's".
    let rest = series.split_off(series.len().min(8));
    // Ordered and cut on the stored names; drawn in the station's language.
    let mut series: Vec<(String, Vec<i64>)> = series
        .into_iter()
        .map(|(name, days)| (state.names().name_of_stored(&name).to_owned(), days))
        .collect();
    if !rest.is_empty() {
        let len = rest.iter().map(|(_, d)| d.len()).max().unwrap_or(0);
        let mut other = vec![0_i64; len];
        for (_, days) in &rest {
            for (slot, n) in other.iter_mut().zip(days) {
                *slot += n;
            }
        }
        series.push((format!("{} other species", rest.len()), other));
    }
    // The sparkline series ends at today (local), which is what labels the
    // axis. Without it the chart showed no period at all.
    let today = state.with_read_db(|conn| {
        conn.query_row("SELECT date('now','localtime')", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    });
    // When the station was not listening, so a day that pinches to nothing because the
    // power was out is not read as a silent day. Unknown (no ledger) claims nothing.
    let n_days = series.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let listened = today.as_deref().and_then(|end| {
        let from = super::listening::first_date(end, n_days)?;
        let effort = state
            .with_read_db(|conn| super::listening::effort_by_date(conn, &from, end))
            .ok()?;
        super::listening::listened_days(&effort, end, n_days)
    });
    Some(super::viz::streamgraph(
        &series,
        today.as_deref(),
        listened.as_deref(),
    ))
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
}

// ---------------------------------------------------------------------------
// SVG heatmap renderer
// ---------------------------------------------------------------------------

/// The activity grid: see [`super::viz::heat::hour_by_weekday`].
fn render_heatmap_svg(
    cells: &[HeatmapCell],
    occurrences: [u32; 7],
    coverage_known: bool,
    window_days: u32,
    now: Option<(usize, usize)>,
) -> String {
    if cells.is_empty() {
        return r#"<p class="hm-empty">
            No data available for the selected period.
        </p>"#
            .to_string();
    }
    let mut counts = [[0_i64; 24]; 7];
    for cell in cells {
        // A malformed row (a bad `Time`, imported) is skipped, not indexed: `panic = "abort"`.
        if let (Some(row), Some(slot)) = (
            usize::try_from(cell.dow).ok().and_then(|d| counts.get_mut(d)),
            usize::try_from(cell.hour).ok(),
        ) && let Some(c) = row.get_mut(slot)
        {
            *c = cell.count;
        }
    }
    super::viz::heat::hour_by_weekday(&super::viz::heat::HourByWeekday {
        id: "heat-hour-weekday",
        counts,
        occurrences,
        coverage_known,
        window_days,
        now,
    })
}

/// Map a 0.0–1.0 intensity to the sequential ramp: kept for the dashboard's species
/// widget (an HTML grid, not an SVG), which paints one `--seq-N` step per cell.
pub(crate) fn heat_color(t: f64) -> String {
    let t = t.clamp(0.0, 1.0);
    if t < 0.001 {
        return "var(--surface-2)".to_string(); // empty cell
    }
    format!("var(--seq-{})", super::viz::kit::seq_step(t))
}

// ---------------------------------------------------------------------------
// Hourly bar chart renderer
// ---------------------------------------------------------------------------

fn render_hourly_bars(
    totals: &[birdnet_db::sqlite::HourTotal],
    solar: Option<(f64, f64)>,
    window_days: u32,
) -> String {
    if totals.is_empty() {
        return r#"<p class="hm-empty">
            No data available for the selected period.
        </p>"#
            .to_string();
    }
    // An hour outside 0–23 comes from a malformed `Time` (the BirdNET-Pi importer
    // copies those through), and is skipped: indexing with it panicked, and
    // `panic = "abort"` took the station down with the page.
    let mut by_hour = [0_i64; 24];
    for h in totals {
        if let Some(slot) = by_hour.get_mut(usize::from(h.hour)) {
            *slot = h.count;
        }
    }
    // One hue for the hours the page is about (dawn 5-8, dusk 18-21), a neutral for
    // the rest. It was amber against green, which is the pair a reader who cannot
    // tell red from green cannot tell apart.
    super::viz::bars::hourly(&super::viz::bars::Hourly {
        id: "hourly-totals",
        name: "Detections by hour of the day",
        desc: &format!(
            "Detections for each hour of the day over the last {window_days} days; dawn and dusk hours are in colour, the rest grey."
        ),
        unit: "detections",
        counts: by_hour,
        fill: super::viz::bars::Fill::DawnDusk,
        width: 760.0,
        plot_h: 150.0,
        solar,
        now_h: None,
        top: None,
        temps: &[],
        table: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_color_empty() {
        assert_eq!(heat_color(0.0), "var(--surface-2)");
    }

    /// The widget's cells take a step of the one sequential ramp (`--seq-1` … `--seq-7`),
    /// never a literal and never a retired token, rising with the share of the busiest cell.
    #[test]
    fn heat_color_is_a_step_of_the_sequential_ramp() {
        assert_eq!(heat_color(1.0), "var(--seq-7)");
        assert_eq!(heat_color(0.001), "var(--seq-1)");
        let mut last = 0;
        for i in 1..=100 {
            let c = heat_color(f64::from(i) / 100.0);
            let n: usize = c.trim_start_matches("var(--seq-").trim_end_matches(')').parse().expect("a step");
            assert!((1..=7).contains(&n) && n >= last, "{c}");
            last = n;
            assert!(!c.contains("oklch") && !c.contains("--heat") && !c.contains("--dawn"), "{c}");
        }
    }

    #[test]
    fn render_heatmap_svg_empty() {
        let html = render_heatmap_svg(&[], [1; 7], true, 7, None);
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
        let svg = render_heatmap_svg(&cells, [1; 7], true, 7, None);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("Mon"));
        assert!(svg.contains("Mon 07:00\u{2013}08:00&#10;Detections&#9;10"), "{svg}");
    }

    /// A row from an imported database with a weekday or hour outside the grid is
    /// skipped; indexing with it aborted the process.
    #[test]
    fn a_malformed_cell_is_skipped_not_indexed() {
        let cells = vec![
            HeatmapCell { dow: 9, hour: 7, count: 10 },
            HeatmapCell { dow: 1, hour: 99, count: 10 },
            HeatmapCell { dow: 1, hour: 7, count: 3 },
        ];
        let svg = render_heatmap_svg(&cells, [1; 7], false, 7, None);
        assert!(svg.contains("Mon 07:00\u{2013}08:00&#10;Detections&#9;3"), "{svg}");
    }

    #[test]
    fn render_hourly_bars_empty() {
        let html = render_hourly_bars(&[], None, 7);
        assert!(html.contains("No data"));
    }

    #[test]
    fn render_hourly_bars_with_data() {
        use birdnet_db::sqlite::HourTotal;
        let totals = vec![
            HourTotal { hour: 7, count: 20 },
            HourTotal { hour: 8, count: 15 },
            // Imported, malformed: skipped rather than indexed.
            HourTotal { hour: 99, count: 5 },
        ];
        let svg = render_hourly_bars(&totals, Some((6.5, 18.5)), 7);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("Detections&#9;20"), "{svg}");
        assert!(svg.contains("sunrise 6:30"), "{svg}");
    }
}
