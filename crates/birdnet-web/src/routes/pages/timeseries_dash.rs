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
//! `/pages/ts-sessions`             | Today's activity sessions                |
//! `/pages/ts-anomalies`            | Anomaly detection table                  |
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
        .route("/pages/ts-sessions", get(ts_sessions_partial))
        .route("/pages/ts-anomalies", get(ts_anomaly_partial))
        .route("/pages/ts-peak", get(ts_peak_partial))
}

/// The time-series surface, rendered for embedding by `homes::patterns`
/// ("Trends" tab).
pub(super) fn content() -> String {
    TIMESERIES_PAGE_HTML.replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::Analytics),
    )
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
    match state.with_timeseries(|ts| ts.hourly_heatmap(&params)) {
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
    match state.with_timeseries(|ts| ts.moving_average(&params)) {
        Some(Ok(rows)) => Some(render_trend_table(&rows)),
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
    let params = birdnet_timeseries::types::params::DiversityParams {
        lookback_days: 30,
        include_shannon: true,
    };
    match state.with_timeseries(|ts| ts.daily_richness(&params)) {
        Some(Ok(rows)) => Some(render_diversity_table(&rows)),
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
// Activity sessions partial
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
fn compute_ts_sessions(state: &AppState) -> Option<String> {
    let params = birdnet_timeseries::types::params::SessionParams {
        gap_minutes: 30,
        date_filter: None,
        lookback_days: 3,
        limit: 50,
    };
    match state.with_timeseries(|ts| ts.activity_sessions(&params)) {
        Some(Ok(rows)) => Some(render_sessions_table(&rows)),
        _ => None,
    }
}

#[cfg(feature = "analytics")]
async fn ts_sessions_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("activity sessions");
    }
    let html = cached_fragment(
        &state,
        "ts-sessions".to_string(),
        TS_FALLBACK,
        compute_ts_sessions,
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

#[cfg(not(feature = "analytics"))]
async fn ts_sessions_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("activity sessions")
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
    match state.with_timeseries(|ts| ts.anomalies(&params)) {
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
    match state.with_timeseries(|ts| ts.peak_windows(&params)) {
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

#[cfg(feature = "analytics")]
fn render_heatmap_table(rows: &[birdnet_timeseries::types::results::HourlyHeatmapRow]) -> String {
    if rows.is_empty() {
        return r#"<p class="tsd-muted">No heatmap data yet.</p>"#.to_string();
    }
    let max_avg = rows
        .iter()
        .map(|r| r.avg_detections_per_day)
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let mut html = String::from(
        r"<table><thead><tr><th>Hour</th><th>Avg/Day</th><th>Total</th><th>Trend</th></tr></thead><tbody>",
    );
    for row in rows {
        let bar_pct = (row.avg_detections_per_day / max_avg * 100.0).clamp(0.0, 100.0);
        let _ = write!(
            html,
            r#"<tr>
<td class="tsd-key">{h:02}:00</td>
<td>{avg:.1}</td>
<td>{total}</td>
<td><div data-style="width:{pct:.0}%;height:8px;background:var(--accent);border-radius:4px;min-width:2px;"></div></td>
</tr>"#,
            h = row.hour_of_day,
            avg = row.avg_detections_per_day,
            total = row.total_detections,
            pct = bar_pct,
        );
    }
    html.push_str("</tbody></table>");
    html
}

#[cfg(feature = "analytics")]
fn render_trend_table(rows: &[birdnet_timeseries::types::results::TrendRow]) -> String {
    if rows.is_empty() {
        return r#"<p class="tsd-muted">No trend data yet.</p>"#.to_string();
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Date</th><th>Detections</th><th>7-Day Avg</th></tr></thead><tbody>",
    );
    for row in rows.iter().rev().take(14).rev() {
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
    html.push_str("</tbody></table>");
    html
}

#[cfg(feature = "analytics")]
fn render_diversity_table(rows: &[birdnet_timeseries::types::results::DiversityRow]) -> String {
    if rows.is_empty() {
        return r#"<p class="tsd-muted">No diversity data yet.</p>"#.to_string();
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Date</th><th>Richness</th><th>Shannon H′</th><th>Evenness</th></tr></thead><tbody>",
    );
    for row in rows.iter().rev().take(14).rev() {
        let h = row
            .shannon_h
            .map_or_else(|| "—".to_string(), |v| format!("{v:.3}"));
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
    html.push_str("</tbody></table>");
    html
}

#[cfg(feature = "analytics")]
fn render_sessions_table(rows: &[birdnet_timeseries::types::results::SessionRow]) -> String {
    if rows.is_empty() {
        return r#"<p class="tsd-muted">No activity sessions found.</p>"#.to_string();
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Start</th><th>Duration</th><th>Detections</th><th>Species</th></tr></thead><tbody>",
    );
    for row in rows.iter().take(20) {
        let _ = write!(
            html,
            r"<tr><td>{}</td><td>{}m</td><td>{}</td><td>{}</td></tr>",
            escape_html(&row.session_start),
            row.duration_minutes,
            row.detection_count,
            row.species_count
        );
    }
    html.push_str("</tbody></table>");
    html
}

#[cfg(feature = "analytics")]
fn render_anomaly_table(rows: &[birdnet_timeseries::types::results::AnomalyRow]) -> String {
    let anomalous: Vec<_> = rows.iter().filter(|r| r.anomaly_flag != "normal").collect();
    if anomalous.is_empty() {
        return r#"<p class="tsd-ok">✓ No anomalies detected in the last 90 days.</p>"#.to_string();
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Date</th><th>Detections</th><th>Z-Score</th><th>Type</th></tr></thead><tbody>",
    );
    for row in &anomalous {
        let z = row
            .z_score
            .map_or_else(|| "—".to_string(), |v| format!("{v:.2}"));
        let cls = if row.anomaly_flag == "high" {
            "high"
        } else {
            "low"
        };
        let _ = write!(
            html,
            r#"<tr><td>{d}</td><td>{c}</td><td>{z}</td><td><span class="conf {cls}">{f}</span></td></tr>"#,
            d = escape_html(&row.date),
            c = row.detections,
            f = escape_html(&row.anomaly_flag),
        );
    }
    html.push_str("</tbody></table>");
    html
}

#[cfg(feature = "analytics")]
fn render_peak_table(rows: &[birdnet_timeseries::types::results::PeakWindowRow]) -> String {
    if rows.is_empty() {
        return r#"<p class="tsd-muted">No peak window data today.</p>"#.to_string();
    }
    let mut html = String::from(
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
    let msg = if cfg!(feature = "analytics") {
        format!(
            r#"<p class="tsd-muted">{endpoint}: start with <code>--analytics-db</code> to enable.</p>"#
        )
    } else {
        format!(
            r#"<p class="tsd-muted">{endpoint}: rebuild with <code>--features analytics</code>.</p>"#
        )
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], msg)
}

/// Pre-compute and cache all six time-series dashboard fragments so the first
/// visit (and each background refresh) is instant. No-op without an analytics DB.
#[cfg(feature = "analytics")]
pub fn prewarm(state: &AppState) {
    if !state.has_analytics() {
        return;
    }
    let cache = state.analytics_cache();
    let fragments = [
        ("ts-heatmap", compute_ts_heatmap(state)),
        ("ts-daily", compute_ts_daily(state)),
        ("ts-diversity", compute_ts_diversity(state)),
        ("ts-sessions", compute_ts_sessions(state)),
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
