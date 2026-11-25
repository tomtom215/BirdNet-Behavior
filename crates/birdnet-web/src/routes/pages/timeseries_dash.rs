//! Time-series analytics dashboard page and HTMX partials.
//!
//! Provides the `/timeseries` full page and a set of HTMX partials that
//! populate it with live data from the `birdnet-timeseries` crate:
//!
//! | Partial path                    | Content                                  |
//! |---------------------------------|------------------------------------------|
//! `/pages/ts-heatmap`              | Hour-of-day detection heatmap            |
//! `/pages/ts-daily`                | Daily trend with 7-day moving average    |
//! `/pages/ts-diversity`            | Shannon diversity + species richness     |
//! `/pages/ts-anomalies`            | Unusual days (anomaly) table             |
//! `/pages/ts-peak`                 | Top busiest 15-minute windows            |

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::{Router, routing::get};
#[cfg(feature = "analytics")]
use std::fmt::Write as _;

use super::TIMESERIES_PAGE_HTML;
#[cfg(feature = "analytics")]
use super::escape_html;
#[cfg(feature = "analytics")]
use crate::analytics_cache::cached_fragment;
use crate::state::AppState;

/// Fallback served (uncached) when a DuckDB time-series query errors after the
/// analytics database is otherwise available.
#[cfg(feature = "analytics")]
const TS_FALLBACK: &str = r#"<p class="tsd-muted">Analytics temporarily unavailable.</p>"#;

/// Mount the time-series analytics dashboard and all HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/ts-heatmap", get(ts_heatmap_partial))
        .route("/pages/ts-daily", get(ts_daily_partial))
        .route("/pages/ts-diversity", get(ts_diversity_partial))
        .route("/pages/ts-anomalies", get(ts_anomaly_partial))
        .route("/pages/ts-peak", get(ts_peak_partial))
}

/// The time-series surface, rendered for embedding by `homes::patterns`
/// ("Trends" tab).
pub(super) fn content() -> String {
    let head = super::frame::page_head(&super::frame::Head {
        eyebrow: "Patterns",
        help: &super::help::help_link(super::help::Topic::Analytics),
        title: "Is your yard getting <em>busier</em>?",
        lead: "Busier, or quieter.",
        lede: "Detections per day, and how many different species you've heard, over the last 30 days against the 30 before. A rising species count often matters more than raw detections: it means more kinds of birds are finding your patch.",
        aside: "",
    });
    TIMESERIES_PAGE_HTML.replace("{{page_head}}", &head)
}

// ---------------------------------------------------------------------------
// Heatmap partial: avg detections per hour-of-day
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
fn compute_ts_heatmap(state: &AppState) -> Option<String> {
    let params = birdnet_timeseries::types::params::HourlyParams {
        lookback_days: 90,
        species: None,
    };
    match state.with_timeseries_read(|ts| ts.hourly_heatmap(&params)) {
        Some(Ok(rows)) => Some(render_heatmap_table(&rows)),
        _ => None,
    }
}

#[cfg(feature = "analytics")]
async fn ts_heatmap_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("hourly heatmap");
    }
    let html = cached_fragment(
        &state,
        "ts-heatmap".to_string(),
        TS_FALLBACK,
        compute_ts_heatmap,
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

#[cfg(not(feature = "analytics"))]
async fn ts_heatmap_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("hourly heatmap")
}

// ---------------------------------------------------------------------------
// Daily trend partial: daily counts + 7-day moving average
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
fn compute_ts_daily(state: &AppState) -> Option<String> {
    let params = birdnet_timeseries::types::params::TrendParams {
        window_days: 7,
        from_date: Some("CURRENT_DATE - INTERVAL 60 DAYS".into()),
        to_date: None,
        species: None,
    };
    match state.with_timeseries_read(|ts| ts.moving_average(&params)) {
        Some(Ok(rows)) => {
            // Which of the days shown the station was recording on, from the ledger.
            let dates: Vec<String> = rows.iter().map(|r| r.date.clone()).collect();
            let listened = listened_for(state, &dates);
            Some(render_trend_table(&rows, listened.as_deref()))
        }
        _ => None,
    }
}

#[cfg(feature = "analytics")]
async fn ts_daily_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("daily trend");
    }
    let html = cached_fragment(
        &state,
        "ts-daily".to_string(),
        TS_FALLBACK,
        compute_ts_daily,
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

#[cfg(not(feature = "analytics"))]
async fn ts_daily_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("daily trend")
}

// ---------------------------------------------------------------------------
// Diversity partial: Shannon H' and species richness
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
fn compute_ts_diversity(state: &AppState) -> Option<String> {
    // Sixty days: the last thirty are the chart, the thirty before are its ghost line.
    let params = birdnet_timeseries::types::params::DiversityParams {
        lookback_days: 60,
        include_shannon: true,
    };
    match state.with_timeseries_read(|ts| ts.daily_richness(&params)) {
        Some(Ok(rows)) => {
            let today = state
                .with_read_db(|conn| {
                    conn.query_row("SELECT date('now','localtime')", [], |r| {
                        r.get::<_, String>(0)
                    })
                })
                .ok();
            Some(render_diversity_table(&rows, today.as_deref(), &|dates| {
                listened_for(state, dates)
            }))
        }
        _ => None,
    }
}

#[cfg(feature = "analytics")]
async fn ts_diversity_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("diversity");
    }
    let html = cached_fragment(
        &state,
        "ts-diversity".to_string(),
        TS_FALLBACK,
        compute_ts_diversity,
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

#[cfg(not(feature = "analytics"))]
async fn ts_diversity_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("diversity")
}

// ---------------------------------------------------------------------------
// Anomaly detection partial
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
fn compute_ts_anomaly(state: &AppState) -> Option<String> {
    let params = birdnet_timeseries::types::params::AnomalyParams {
        z_threshold: 2.0,
        window_days: 30,
        lookback_days: 90,
    };
    match state.with_timeseries_read(|ts| ts.anomalies(&params)) {
        Some(Ok(rows)) => Some(render_anomaly_table(&rows)),
        _ => None,
    }
}

#[cfg(feature = "analytics")]
async fn ts_anomaly_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("anomaly detection");
    }
    let html = cached_fragment(
        &state,
        "ts-anomalies".to_string(),
        TS_FALLBACK,
        compute_ts_anomaly,
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

#[cfg(not(feature = "analytics"))]
async fn ts_anomaly_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("anomaly detection")
}

// ---------------------------------------------------------------------------
// Peak windows partial
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
fn compute_ts_peak(state: &AppState) -> Option<String> {
    let params = birdnet_timeseries::types::params::PeakParams {
        window_minutes: 15,
        hop_minutes: 5,
        lookback_days: 1,
        limit: 5,
    };
    match state.with_timeseries_read(|ts| ts.peak_windows(&params)) {
        Some(Ok(rows)) => Some(render_peak_table(&rows)),
        _ => None,
    }
}

#[cfg(feature = "analytics")]
async fn ts_peak_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("peak windows");
    }
    let html = cached_fragment(&state, "ts-peak".to_string(), TS_FALLBACK, compute_ts_peak).await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

#[cfg(not(feature = "analytics"))]
async fn ts_peak_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("peak windows")
}

// ---------------------------------------------------------------------------
// HTML renderers (only used with analytics feature)
// ---------------------------------------------------------------------------

/// For each of `dates`, whether the station was recording, from the recording-effort
/// ledger; `None` when the ledger has nothing for them (no claim is made). Reads the
/// database, so it is for the synchronous compute functions the cache runs on the blocking pool.
#[cfg(feature = "analytics")]
fn listened_for(state: &AppState, dates: &[String]) -> Option<Vec<bool>> {
    let (first, last) = (dates.iter().min()?, dates.iter().max()?);
    let effort = state
        .with_read_db(|conn| super::listening::effort_by_date(conn, first, last))
        .ok()?;
    super::listening::listened_on(&effort, dates, last)
}

/// Fill the days with no row with zero, so every calendar day has its place on the axis.
/// Whether a filled day is a real zero or a day nobody measured is the caller's to say. `(date, value)` in, one per calendar
/// day from the first to `last` out.
#[cfg(feature = "analytics")]
fn zero_fill(rows: &[(String, f64)], last: &str) -> Vec<(String, f64)> {
    use super::viz::kit::{epoch_day, shift_date};
    let Some(first) = rows.first().and_then(|r| epoch_day(&r.0)) else {
        return rows.to_vec();
    };
    let Some(end) = epoch_day(last) else {
        return rows.to_vec();
    };
    let by_date: std::collections::HashMap<&str, f64> =
        rows.iter().map(|(d, v)| (d.as_str(), *v)).collect();
    (first..=end)
        .filter_map(|day| {
            let (y, m, d) = shift_date("1970-01-01", day)?;
            let iso = format!("{y:04}-{m:02}-{d:02}");
            let v = by_date.get(iso.as_str()).copied().unwrap_or(0.0);
            Some((iso, v))
        })
        .collect()
}

#[cfg(feature = "analytics")]
fn render_heatmap_table(rows: &[birdnet_timeseries::types::results::HourlyHeatmapRow]) -> String {
    if rows.is_empty() {
        return r#"<p class="tsd-muted">No heatmap data yet.</p>"#.to_string();
    }
    let max_avg = rows
        .iter()
        .map(|r| r.avg_detections_per_day)
        .fold(0.0_f64, f64::max);
    // Scale to the busiest hour, whatever its size: averages below one a day
    // are ordinary once quiet days count (ANA11).
    let max_avg = if max_avg > 0.0 { max_avg } else { 1.0 };

    let mut html = String::from(
        r"<table><thead><tr><th>Hour</th><th>Avg/Day</th><th>Total</th><th>Trend</th></tr></thead><tbody>",
    );
    for row in rows {
        let bar_pct = (row.avg_detections_per_day / max_avg * 100.0).clamp(0.0, 100.0);
        let _ = write!(
            html,
            r#"<tr>
<td class="tsd-key">{hour}</td>
<td>{avg:.2}</td>
<td>{total}</td>
<td><div data-style="width:{pct:.0}%;height:8px;background:var(--accent);border-radius:4px;min-width:2px;"></div></td>
</tr>"#,
            hour = crate::display::clock_hour(row.hour_of_day),
            avg = row.avg_detections_per_day,
            total = row.total_detections,
            pct = bar_pct,
        );
    }
    html.push_str("</tbody></table>");
    html
}

/// The table behind a trend chart: the same numbers, one disclosure away, for a
/// keyboard, a screen reader, or a person who wants to copy a figure.
#[cfg(feature = "analytics")]
const TABLE_OPEN: &str = r#"<details class="viz-table"><summary>Show as a table</summary><div class="viz-table-scroll">"#;

#[cfg(feature = "analytics")]
// Daily counts and richness are tens to thousands: an `f64` holds them exactly.
#[allow(clippy::cast_precision_loss)]
fn render_trend_table(
    rows: &[birdnet_timeseries::types::results::TrendRow],
    listened: Option<&[bool]>,
) -> String {
    if rows.is_empty() {
        return r#"<p class="tsd-muted">No trend data yet.</p>"#.to_string();
    }
    // The last thirty complete days are the chart; the thirty before them are its ghost.
    let shown: Vec<_> = rows.iter().rev().take(30).rev().collect();
    let before: Vec<_> = rows.iter().rev().skip(30).take(30).rev().collect();
    let point = |r: &&birdnet_timeseries::types::results::TrendRow| super::viz::TrendPoint {
        date: r.date.clone(),
        value: r.daily_detections as f64,
        smooth: r.moving_avg_detections,
    };
    let pts: Vec<_> = shown.iter().map(point).collect();
    let prior: Vec<_> = before.iter().map(point).collect();
    let listened_shown = listened.map(|l| l[l.len().saturating_sub(pts.len())..].to_vec());
    let mut html = super::viz::trend_chart(
        &pts,
        &super::viz::TrendOpts {
            id: "ts-daily-trend",
            unit: "detections",
            smooth_name: "7-day average",
            bars: true,
            prior: &prior,
            listened: listened_shown.as_deref(),
        },
    );
    html.push_str(TABLE_OPEN);
    html.push_str(
        r"<table><thead><tr><th>Date</th><th>Detections</th><th>7-Day Avg</th></tr></thead><tbody>",
    );
    for row in shown {
        let avg = row
            .moving_avg_detections
            .map_or_else(|| "—".to_string(), |v| format!("{v:.1}"));
        let _ = write!(
            html,
            r"<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(&row.date),
            row.daily_detections,
            avg
        );
    }
    html.push_str("</tbody></table></div></details>");
    html
}

/// Shannon H′ for a table cell. A day with one species has H′ = −(1 · ln 1),
/// which DuckDB returns as −0.0 and `{:.3}` prints as "-0.000" (`ANA14d`).
/// H′ is never negative, so anything at or below zero is shown as 0.000.
#[cfg(feature = "analytics")]
fn shannon_cell(h: Option<f64>) -> String {
    h.map_or_else(
        || "—".to_string(),
        |v| format!("{:.3}", if v <= 0.0 { 0.0 } else { v }),
    )
}

#[cfg(feature = "analytics")]
// Daily counts and richness are tens to thousands: an `f64` holds them exactly.
#[allow(clippy::cast_precision_loss)]
fn render_diversity_table(
    rows: &[birdnet_timeseries::types::results::DiversityRow],
    today: Option<&str>,
    listened_for: &dyn Fn(&[String]) -> Option<Vec<bool>>,
) -> String {
    if rows.is_empty() {
        return r#"<p class="tsd-muted">No diversity data yet.</p>"#.to_string();
    }
    // One point a calendar day to today. The query reports a listened day that heard
    // nothing as a row of zero species and leaves out a day the station did not listen to
    // (RES-14), so a day with no row is unknown: it is filled only to keep its place on
    // the axis and is marked not listened below, never drawn as a zero.
    let last = today.unwrap_or_else(|| rows.last().map_or("", |r| r.date.as_str()));
    let dated: Vec<(String, f64)> = rows
        .iter()
        .map(|r| (r.date.clone(), r.species_richness as f64))
        .collect();
    let filled = zero_fill(&dated, last);
    let day_keys: Vec<String> = filled.iter().map(|d| d.0.clone()).collect();
    let reported: std::collections::HashSet<&str> = rows.iter().map(|r| r.date.as_str()).collect();
    let ledger = listened_for(&day_keys);
    let any_left_out = day_keys.iter().any(|d| !reported.contains(d.as_str()));
    let listened = (ledger.is_some() || any_left_out).then(|| {
        day_keys
            .iter()
            .enumerate()
            .map(|(i, d)| {
                reported.contains(d.as_str())
                    && ledger
                        .as_ref()
                        .is_none_or(|l| l.get(i).copied().unwrap_or(true))
            })
            .collect::<Vec<bool>>()
    });
    let split = filled.len().saturating_sub(30);
    let point = |d: &(String, f64)| super::viz::TrendPoint {
        date: d.0.clone(),
        value: d.1,
        smooth: None,
    };
    let pts: Vec<_> = filled[split..].iter().map(point).collect();
    let prior: Vec<_> = filled[..split].iter().map(point).collect();
    let listened_shown = listened.map(|l| l[split..].to_vec());
    let shown: Vec<_> = rows.iter().rev().take(30).rev().collect();
    let mut html = super::viz::trend_chart(
        &pts,
        &super::viz::TrendOpts {
            id: "ts-richness-trend",
            unit: "species",
            smooth_name: "",
            bars: false,
            prior: &prior,
            listened: listened_shown.as_deref(),
        },
    );
    html.push_str(TABLE_OPEN);
    html.push_str(
        r"<table><thead><tr><th>Date</th><th>Kinds of birds</th><th>Diversity (Shannon H′)</th><th>Balance (0–1)</th></tr></thead><tbody>",
    );
    for row in shown {
        let h = shannon_cell(row.shannon_h);
        let ev = row
            .pielou_evenness
            .map_or_else(|| "—".to_string(), |v| format!("{v:.2}"));
        let _ = write!(
            html,
            r"<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(&row.date),
            row.species_richness,
            h,
            ev
        );
    }
    html.push_str("</tbody></table></div></details>");
    html
}

#[cfg(feature = "analytics")]
fn render_anomaly_table(rows: &[birdnet_timeseries::types::results::AnomalyRow]) -> String {
    // Only a judged day can be an anomaly. A day the station recorded less
    // than half its usual day of (`unlistened`) was not judged; every flag other than `high`
    // and `low` used to be drawn as a row, and an outage was drawn as "low".
    let anomalous: Vec<_> = rows
        .iter()
        .filter(|r| matches!(r.anomaly_flag.as_str(), "high" | "low"))
        .collect();
    let unlistened = rows.iter().filter(|r| !r.listened).count();
    let left_out = match unlistened {
        0 => String::new(),
        1 => r#"<p class="tsd-muted">1 day the station recorded less than half its usual day of is left out, and is not part of any baseline.</p>"#.to_string(),
        n => format!(
            r#"<p class="tsd-muted">{n} days the station recorded less than half its usual day of are left out, and are not part of any baseline.</p>"#
        ),
    };
    if anomalous.is_empty() {
        return format!(
            r#"<p class="tsd-ok">✓ No unusual days in the last 90 days.</p>{left_out}"#
        );
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Date</th><th>Detections</th><th>How unusual (z)</th><th>Busier or quieter</th></tr></thead><tbody>",
    );
    for row in &anomalous {
        let z = row
            .z_score
            .map_or_else(|| "—".to_string(), |v| format!("{v:.2}"));
        let (cls, said) = if row.anomaly_flag == "high" {
            ("high", "busier")
        } else {
            ("low", "quieter")
        };
        // A judged day that was not recorded in full: its count is real, and
        // so is the gap in it.
        let partial = match (row.partial_effort, row.effort_hours) {
            (true, Some(h)) => {
                format!(r#" <span class="tsd-muted">partial day, {h:.0} h recorded</span>"#)
            }
            _ => String::new(),
        };
        let _ = write!(
            html,
            r#"<tr><td>{d}</td><td>{c}</td><td>{z}</td><td><span class="conf {cls}">{f}</span>{partial}</td></tr>"#,
            d = escape_html(&row.date),
            c = row.detections,
            f = said,
        );
    }
    html.push_str("</tbody></table>");
    html.push_str(&left_out);
    html
}

#[cfg(feature = "analytics")]
fn render_peak_table(rows: &[birdnet_timeseries::types::results::PeakWindowRow]) -> String {
    // The range is the 24 hours up to the newest detection, not "today"; the
    // card said today, which at 07:00 is mostly yesterday and on a station
    // that stopped last week is last week.
    //
    // Anchored to the newest detection, the range is empty only when there
    // are no detections at all.
    if rows.is_empty() {
        return r#"<p class="tsd-muted">No detections to rank yet.</p>"#.to_string();
    }
    let mut html = String::from(
        r#"<p class="tsd-muted">Busiest separate windows in the 24 hours up to the latest detection.</p>"#,
    );
    html.push_str(
        r"<table><thead><tr><th>Window Start</th><th>Window End</th><th>Detections</th><th>Species</th></tr></thead><tbody>",
    );
    for row in rows {
        let _ = write!(
            html,
            r"<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(&row.window_start),
            escape_html(&row.window_end),
            row.detection_count,
            row.species_count
        );
    }
    html.push_str("</tbody></table>");
    html
}

// ---------------------------------------------------------------------------
// Error/unavailable helpers
// ---------------------------------------------------------------------------

fn ts_unavailable(endpoint: &str) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    // These render into cards on the Patterns page. The JSON siblings in
    // `routes::timeseries::helpers` keep their flags — an API consumer is
    // exactly who can act on them — but nobody reading a chart is.
    let msg = format!(
        r#"<p class="tsd-muted">{endpoint} isn't available on this station — the extra analytics aren't switched on. Everything else keeps recording as normal.</p>"#
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], msg)
}

/// Pre-compute and cache the time-series dashboard fragments the page asks for, so the first
/// visit (and each background refresh) is instant. No-op without an analytics DB.
///
/// `ts-heatmap` is not among them: the page no longer shows it (the hour-of-day average is
/// "When active"'s), so warming it every refresh cost a 90-day query for nobody. The route
/// still answers.
#[cfg(feature = "analytics")]
pub fn prewarm(state: &AppState) {
    if !state.has_analytics() {
        return;
    }
    let cache = state.analytics_cache();
    let fragments = [
        ("ts-daily", compute_ts_daily(state)),
        ("ts-diversity", compute_ts_diversity(state)),
        ("ts-anomalies", compute_ts_anomaly(state)),
        ("ts-peak", compute_ts_peak(state)),
    ];
    for (key, html) in fragments {
        if let Some(html) = html {
            cache.put(key.to_string(), html);
        }
    }
}

/// No-op pre-warm when analytics is not compiled in.
#[cfg(not(feature = "analytics"))]
pub const fn prewarm(_state: &AppState) {}

#[cfg(all(test, feature = "analytics"))]
mod tests {
    /// The peak card's range is the 24 hours up to the newest detection
    /// (`peak_windows` anchors to the data, not the clock), which is not
    /// "today": at 07:00 it is mostly yesterday, and on a station that stopped
    /// last week it is last week. Its text must not say "today".
    #[test]
    fn the_peak_card_does_not_claim_today() {
        let row = birdnet_timeseries::types::results::PeakWindowRow {
            window_start: "2026-05-01 05:00:00".into(),
            window_end: "2026-05-01 05:15:00".into(),
            detection_count: 3,
            species_count: 1,
            peak_confidence: None,
        };
        let empty = super::render_peak_table(&[]);
        assert!(!empty.contains("today"), "{empty}");
        let html = super::render_peak_table(&[row]);
        assert!(!html.contains("today"), "{html}");
        assert!(
            html.contains("24 hours up to the latest detection"),
            "{html}"
        );
    }

    /// `ANA14d`: a one-species day reads 0.000, not -0.000.
    #[test]
    fn a_one_species_day_has_zero_diversity_not_negative() {
        assert_eq!(super::shannon_cell(Some(-0.0)), "0.000");
        // Counterparts: an ordinary value and a missing one are unchanged.
        assert_eq!(super::shannon_cell(Some(std::f64::consts::LN_2)), "0.693");
        assert_eq!(super::shannon_cell(None), "—");
    }

    fn diversity(date: &str, species: i64) -> birdnet_timeseries::types::results::DiversityRow {
        birdnet_timeseries::types::results::DiversityRow {
            date: date.into(),
            species_richness: species,
            total_detections: species * 3,
            shannon_h: None,
            pielou_evenness: None,
        }
    }

    /// RES-14: the diversity query leaves out a day the station did not
    /// listen to (it reports a listened day with nothing heard as zero
    /// species). The chart filled every missing day with zero, so the day
    /// left out as unknown was drawn as "0 species" whenever the 60-second
    /// ledger rule still called it listened.
    #[test]
    fn a_day_the_diversity_query_left_out_is_not_drawn_as_zero_species() {
        let rows = [
            diversity("2026-10-05", 6),
            diversity("2026-10-06", 5),
            // 2026-10-07: no row, the station recorded under half its usual day.
            diversity("2026-10-08", 7),
        ];
        let all_listened = |dates: &[String]| Some(vec![true; dates.len()]);
        let html = super::render_diversity_table(&rows, Some("2026-10-08"), &all_listened);
        assert!(html.contains("The station was not listening"), "{html}");
        assert!(html.contains("&quot;Wed 7 Oct&quot;,[null]"), "{html}");

        // Counterpart: a listened day that heard nothing is a row with zero
        // species, and is drawn as the zero it is.
        let rows = [
            diversity("2026-10-05", 6),
            diversity("2026-10-06", 5),
            diversity("2026-10-07", 0),
            diversity("2026-10-08", 7),
        ];
        let html = super::render_diversity_table(&rows, Some("2026-10-08"), &all_listened);
        assert!(!html.contains("The station was not listening"), "{html}");
        assert!(html.contains("&quot;Wed 7 Oct&quot;,[0.0]"), "{html}");

        // And with no ledger at all the query returns every day, so nothing
        // is marked.
        let html = super::render_diversity_table(&rows, Some("2026-10-08"), &|_| None);
        assert!(!html.contains("The station was not listening"), "{html}");
    }

    fn anomaly(
        date: &str,
        flag: &str,
        hours: Option<f64>,
    ) -> birdnet_timeseries::types::results::AnomalyRow {
        let listened = flag != "unlistened";
        birdnet_timeseries::types::results::AnomalyRow {
            date: date.into(),
            detections: if listened { 40 } else { 0 },
            rolling_mean: Some(10.0),
            rolling_stddev: Some(1.0),
            z_score: listened.then_some(30.0),
            anomaly_flag: flag.into(),
            effort_hours: hours,
            listened,
            partial_effort: listened && hours.is_some_and(|h| h < 23.0),
        }
    }

    /// UIC-03: a day left out for want of recording is not drawn as an
    /// anomaly row, and the card says how many were left out.
    #[test]
    fn an_unlistened_day_is_not_an_anomaly_row() {
        let html = super::render_anomaly_table(&[
            anomaly("2026-05-01", "unlistened", Some(0.0)),
            anomaly("2026-05-02", "unlistened", Some(2.0)),
            anomaly("2026-05-03", "normal", Some(24.0)),
        ]);
        assert!(!html.contains("2026-05-01"), "{html}");
        assert!(!html.contains("<table>"), "{html}");
        assert!(html.contains("No unusual days"), "{html}");
        assert!(
            html.contains("2 days the station recorded less than half its usual day of"),
            "{html}"
        );
        // Counterpart: a judged outlier beside them is still a row.
        let html = super::render_anomaly_table(&[
            anomaly("2026-05-01", "unlistened", Some(0.0)),
            anomaly("2026-05-04", "high", Some(24.0)),
        ]);
        assert!(html.contains("<td>2026-05-04</td>"), "{html}");
        assert!(!html.contains("2026-05-01"), "{html}");
        assert!(!html.contains("partial day"), "{html}");
    }

    /// A judged day the station recorded only part of carries the label.
    #[test]
    fn a_partial_day_anomaly_is_labelled() {
        let html = super::render_anomaly_table(&[anomaly("2026-05-04", "high", Some(14.0))]);
        assert!(html.contains("partial day, 14 h recorded"), "{html}");
    }
}

/// UIC-07: one figure per name across the Patterns tabs.
#[cfg(test)]
mod one_name_per_figure {
    const TRENDS: &str = include_str!("../../../templates/timeseries.html");
    const VISITS: &str = include_str!("../../../templates/analytics.html");

    /// Trends and Visits each had a card titled "Activity sessions", drawn by
    /// two engines (`birdnet-timeseries` and `birdnet-behavioral`'s
    /// `sessionize`), which can count the same days differently under the one
    /// name. Visits owns sessions; Trends' card is gone.
    #[test]
    fn only_visits_has_an_activity_sessions_card() {
        assert!(
            !TRENDS.contains(">Activity sessions</h2>"),
            "Trends still draws its own Activity sessions card"
        );
        assert!(
            !TRENDS.contains("/pages/ts-sessions"),
            "Trends still loads the sessions partial"
        );
        // The counterpart: Visits keeps the card.
        assert!(VISITS.contains("Activity sessions"));
    }
}
