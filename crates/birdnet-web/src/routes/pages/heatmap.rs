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

/// The ranges the page offers, in days. One control sets every figure on the tab; the terrain
/// shows the nearest range it can draw that is not shorter than the choice (a month at least).
const RANGES: [(u32, &str); 5] = [
    (7, "7 days"),
    (14, "14 days"),
    (30, "30 days"),
    (90, "90 days"),
    (365, "1 year"),
];

/// The range the page opens on. A week is too short to tell weekdays apart or to draw a
/// landscape, so the page opens on a month.
const DEFAULT_DAYS: u32 = 30;

/// The range control. `role`/`aria-pressed` carry the selection (a CSS class alone said nothing
/// to a screen reader); `type="button"` because the default is submit.
fn range_control() -> String {
    let mut out = String::from(
        r#"<div class="seg" id="range-controls" role="group" aria-label="Time range for every chart on this page">"#,
    );
    for (days, label) in RANGES {
        let on = days == DEFAULT_DAYS;
        let _ = write!(
            out,
            r#"<button type="button" class="btn{}" data-days="{days}" aria-pressed="{on}">{label}</button>"#,
            if on { " active" } else { "" }
        );
    }
    out.push_str("</div>");
    out
}

/// A figure whose body is an HTMX slot filled by `url` at the default range.
fn slot_figure(f: &super::frame::Figure<'_>, id: &str, url: &str, loading: &str) -> String {
    let body = format!(
        r#"<div id="{id}" hx-get="{url}?days={DEFAULT_DAYS}" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">{loading}</p>
  </div>"#
    );
    super::frame::figure(&super::frame::Figure { body: &body, ..*f })
}

/// The heatmap surface, rendered for embedding by `homes::patterns`: the head, then the
/// soundscape as the lead figure, then the three that zoom in (who, which hours, which
/// weekdays). `terrain` is the terrain card's markup.
pub(super) fn content(terrain: &str) -> String {
    let head = super::frame::page_head(&super::frame::Head {
        eyebrow: "Patterns",
        help: &super::help::help_link(super::help::Topic::Analytics),
        title: "When the yard is <em>alive</em>",
        lead: "When your birds are out, and who is singing.",
        lede: "Four views of the same days, from the whole soundscape down to a single hour. The range on the right sets all of them.",
        aside: &range_control(),
    });
    let stream = slot_figure(
        &super::frame::Figure {
            topic: "Who's singing, over time",
            title: "Activity streamgraph",
            action: None,
            note: "Each band is one species; its thickness is that day's detection count, stacked around a centre line. Dates run left to right.",
            body: "",
        },
        "activity-streamgraph",
        "/pages/activity-streamgraph",
        "Loading streamgraph...",
    );
    let hours = slot_figure(
        &super::frame::Figure {
            topic: "Hour by hour",
            title: "Detections by hour",
            action: Some(("/patterns?tab=dawn", "See who sings when →")),
            note: "Totals for every hour of the day over the chosen range. Dawn (05:00 to 09:00) and dusk (18:00 to 22:00) bars are green; the rest are grey.",
            body: "",
        },
        "hourly-totals",
        "/pages/hourly-totals",
        "Loading chart...",
    );
    let grid = slot_figure(
        &super::frame::Figure {
            topic: "Hour × weekday",
            title: "Activity grid",
            action: None,
            note: "A stronger colour means more birds heard in that hour (the key under the grid runs from none to most). The bars at the right and along the bottom are the totals for each weekday and each hour.",
            body: "",
        },
        "heatmap-grid",
        "/pages/heatmap-grid",
        "Loading heatmap...",
    );
    format!(
        "{head}\n{terrain}\n{stream}\n{hours}\n{grid}\n{}",
        RANGE_SCRIPT.as_str()
    )
}

/// Sets every figure from the one range control: the three partials by `htmx.ajax`, the terrain
/// by an event it listens for (`bnb:range`, on its card).
static RANGE_SCRIPT: crate::seal::Sealed = crate::seal::Sealed::new(
    r"<script>
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
  var t = document.getElementById('terrain');
  if (t) t.dispatchEvent(new CustomEvent('bnb:range', { detail: { days: days } }));
}
document.getElementById('range-controls').addEventListener('click', function(e) {
  const btn = e.target.closest('button[data-days]');
  if (btn) loadDays(parseInt(btn.dataset.days, 10), btn);
});
</script>",
);

// ---------------------------------------------------------------------------
// GET /pages/heatmap-grid — SVG heatmap partial
// ---------------------------------------------------------------------------

/// Compute the hour × day-of-week heatmap SVG, or `None` on a query error.
fn compute_heatmap_grid(state: &AppState, days: u32) -> Option<String> {
    let cells = state.with_read_db(|conn| weekly_heatmap(conn, days)).ok()?;
    let today = state
        .with_read_db(|conn| {
            conn.query_row("SELECT date('now','localtime')", [], |r| {
                r.get::<_, String>(0)
            })
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
        let on = listened
            .as_ref()
            .is_none_or(|l| l.get(d).copied().unwrap_or(true));
        let dow = today
            .as_deref()
            .and_then(|end| {
                super::viz::kit::epoch_day(end).map(|e| super::viz::kit::weekday_back(e, n - 1 - d))
            })
            // Without a date, spread the days over the week evenly.
            .unwrap_or(d % 7);
        if on {
            occurrences[dow] += 1;
        }
    }
    // Today's weekday and the hour it is now, so the hours still to come are not drawn as quiet ones.
    let now = today
        .as_deref()
        .and_then(super::viz::kit::epoch_day)
        .map(|e| {
            (
                super::viz::kit::weekday_back(e, 0),
                super::viz::kit::hour_floor(super::now_hour_local()),
            )
        });
    let take = super::insight::weekday_sentence(&weekday_counts(&cells), occurrences, days)
        .map(|s| super::frame::takeaway(&s))
        .unwrap_or_default();
    Some(format!(
        "{take}{}",
        render_heatmap_svg(&cells, occurrences, listened.is_some(), days, now)
    ))
}

async fn heatmap_grid_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(DEFAULT_DAYS).min(365);
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
    let take = super::insight::hours_sentence(&hour_array(&totals))
        .map(|s| super::frame::takeaway(&s))
        .unwrap_or_default();
    Some(format!(
        "{take}{}",
        render_hourly_bars(&totals, solar, days)
    ))
}

async fn hourly_totals_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(DEFAULT_DAYS).min(365);
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
    // What the window held, for the sentence above the chart: every species' total, and the
    // busiest first by their stored names (shown in the station's language below).
    let total: i64 = series.iter().map(|(_, d)| d.iter().sum::<i64>()).sum();
    let named: Vec<(String, i64)> = series
        .iter()
        .take(3)
        .map(|(name, d)| {
            (
                state.names().name_of_stored(name).to_owned(),
                d.iter().sum::<i64>(),
            )
        })
        .collect();
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
    let take = super::insight::leader_sentence(&named, total)
        .map(|s| super::frame::takeaway(&s))
        .unwrap_or_default();
    Some(format!(
        "{take}{}",
        super::viz::streamgraph(&series, today.as_deref(), listened.as_deref())
    ))
}

async fn streamgraph_partial(
    State(state): State<AppState>,
    Query(query): Query<HeatmapQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(DEFAULT_DAYS).clamp(2, 365);
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
    if let Some(h) = compute_streamgraph(state, DEFAULT_DAYS) {
        cache.put(format!("streamgraph:{DEFAULT_DAYS}"), h);
    }
    if let Some(h) = compute_heatmap_grid(state, DEFAULT_DAYS) {
        cache.put(format!("heatmap-grid:{DEFAULT_DAYS}"), h);
    }
    if let Some(h) = compute_hourly_totals(state, DEFAULT_DAYS) {
        cache.put(format!("hourly-totals:{DEFAULT_DAYS}"), h);
    }
}

// ---------------------------------------------------------------------------
// SVG heatmap renderer
// ---------------------------------------------------------------------------

/// Detections by `[weekday][hour]` (Sunday first) from the query's cells.
fn weekday_counts(cells: &[HeatmapCell]) -> [[i64; 24]; 7] {
    let mut counts = [[0_i64; 24]; 7];
    for cell in cells {
        // A malformed row (a bad `Time`, imported) is skipped, not indexed: `panic = "abort"`.
        if let Some(c) = counts
            .get_mut(usize::from(cell.dow))
            .and_then(|row| row.get_mut(usize::from(cell.hour)))
        {
            *c = cell.count;
        }
    }
    counts
}

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
    super::viz::heat::hour_by_weekday(&super::viz::heat::HourByWeekday {
        id: "heat-hour-weekday",
        counts: weekday_counts(cells),
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

/// Detections by hour of the day, 0 to 23.
///
/// An hour outside that comes from a malformed `Time` (the BirdNET-Pi importer copies those
/// through), and is skipped: indexing with it panicked, and `panic = "abort"` took the station
/// down with the page.
fn hour_array(totals: &[birdnet_db::sqlite::HourTotal]) -> [i64; 24] {
    let mut by_hour = [0_i64; 24];
    for h in totals {
        if let Some(slot) = by_hour.get_mut(usize::from(h.hour)) {
            *slot = h.count;
        }
    }
    by_hour
}

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
    let by_hour = hour_array(totals);
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
        stack: None,
        temps: &[],
        table: true,
        class: "",
        compact: false,
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
            let n: usize = c
                .trim_start_matches("var(--seq-")
                .trim_end_matches(')')
                .parse()
                .expect("a step");
            assert!((1..=7).contains(&n) && n >= last, "{c}");
            last = n;
            assert!(
                !c.contains("oklch") && !c.contains("--heat") && !c.contains("--dawn"),
                "{c}"
            );
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
        assert!(
            svg.contains("Mon 07:00\u{2013}08:00&#10;Detections&#9;10"),
            "{svg}"
        );
    }

    /// A row from an imported database with a weekday or hour outside the grid is
    /// skipped; indexing with it aborted the process.
    #[test]
    fn a_malformed_cell_is_skipped_not_indexed() {
        let cells = vec![
            HeatmapCell {
                dow: 9,
                hour: 7,
                count: 10,
            },
            HeatmapCell {
                dow: 1,
                hour: 99,
                count: 10,
            },
            HeatmapCell {
                dow: 1,
                hour: 7,
                count: 3,
            },
        ];
        let svg = render_heatmap_svg(&cells, [1; 7], false, 7, None);
        assert!(
            svg.contains("Mon 07:00\u{2013}08:00&#10;Detections&#9;3"),
            "{svg}"
        );
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
