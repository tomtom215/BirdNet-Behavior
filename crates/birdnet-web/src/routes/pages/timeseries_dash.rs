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
use axum::response::Html;
use axum::{Router, routing::get};

use super::TIMESERIES_PAGE_HTML;
#[cfg(feature = "analytics")]
use super::escape_html;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/timeseries", get(timeseries_page))
        .route("/pages/ts-heatmap", get(ts_heatmap_partial))
        .route("/pages/ts-daily", get(ts_daily_partial))
        .route("/pages/ts-diversity", get(ts_diversity_partial))
        .route("/pages/ts-sessions", get(ts_sessions_partial))
        .route("/pages/ts-anomalies", get(ts_anomaly_partial))
        .route("/pages/ts-peak", get(ts_peak_partial))
}

async fn timeseries_page() -> Html<String> {
    super::render_page("Time Series", TIMESERIES_PAGE_HTML, "timeseries")
}

// ---------------------------------------------------------------------------
// Heatmap partial: avg detections per hour-of-day
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
async fn ts_heatmap_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("hourly heatmap");
    }
    let params = birdnet_timeseries::types::params::HourlyParams {
        lookback_days: 90,
        species: None,
    };
    let result =
        tokio::task::spawn_blocking(move || state.with_timeseries(|ts| ts.hourly_heatmap(&params)))
            .await;

    match result {
        Ok(Some(Ok(rows))) => {
            let html = render_heatmap_table(&rows);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Some(Err(e))) => ts_error(&e.to_string()),
        _ => ts_unavailable("hourly heatmap"),
    }
}

#[cfg(not(feature = "analytics"))]
async fn ts_heatmap_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("hourly heatmap")
}

// ---------------------------------------------------------------------------
// Daily trend partial: daily counts + 7-day moving average
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
async fn ts_daily_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("daily trend");
    }
    let params = birdnet_timeseries::types::params::TrendParams {
        window_days: 7,
        from_date: Some("CURRENT_DATE - INTERVAL 60 DAYS".into()),
        to_date: None,
        species: None,
    };
    let result =
        tokio::task::spawn_blocking(move || state.with_timeseries(|ts| ts.moving_average(&params)))
            .await;

    match result {
        Ok(Some(Ok(rows))) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_trend_table(&rows),
        ),
        Ok(Some(Err(e))) => ts_error(&e.to_string()),
        _ => ts_unavailable("daily trend"),
    }
}

#[cfg(not(feature = "analytics"))]
async fn ts_daily_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("daily trend")
}

// ---------------------------------------------------------------------------
// Diversity partial: Shannon H' and species richness
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
async fn ts_diversity_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("diversity");
    }
    let params = birdnet_timeseries::types::params::DiversityParams {
        lookback_days: 30,
        include_shannon: true,
    };
    let result =
        tokio::task::spawn_blocking(move || state.with_timeseries(|ts| ts.daily_richness(&params)))
            .await;

    match result {
        Ok(Some(Ok(rows))) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_diversity_table(&rows),
        ),
        Ok(Some(Err(e))) => ts_error(&e.to_string()),
        _ => ts_unavailable("diversity"),
    }
}

#[cfg(not(feature = "analytics"))]
async fn ts_diversity_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("diversity")
}

// ---------------------------------------------------------------------------
// Activity sessions partial
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
async fn ts_sessions_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("activity sessions");
    }
    let params = birdnet_timeseries::types::params::SessionParams {
        gap_minutes: 30,
        date_filter: None,
        lookback_days: 3,
        limit: 50,
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.activity_sessions(&params))
    })
    .await;

    match result {
        Ok(Some(Ok(rows))) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_sessions_table(&rows),
        ),
        Ok(Some(Err(e))) => ts_error(&e.to_string()),
        _ => ts_unavailable("activity sessions"),
    }
}

#[cfg(not(feature = "analytics"))]
async fn ts_sessions_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("activity sessions")
}

// ---------------------------------------------------------------------------
// Anomaly detection partial
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
async fn ts_anomaly_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("anomaly detection");
    }
    let params = birdnet_timeseries::types::params::AnomalyParams {
        z_threshold: 2.0,
        window_days: 30,
        lookback_days: 90,
    };
    let result =
        tokio::task::spawn_blocking(move || state.with_timeseries(|ts| ts.anomalies(&params)))
            .await;

    match result {
        Ok(Some(Ok(rows))) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_anomaly_table(&rows),
        ),
        Ok(Some(Err(e))) => ts_error(&e.to_string()),
        _ => ts_unavailable("anomaly detection"),
    }
}

#[cfg(not(feature = "analytics"))]
async fn ts_anomaly_partial(State(_): State<AppState>) -> impl axum::response::IntoResponse {
    ts_unavailable("anomaly detection")
}

// ---------------------------------------------------------------------------
// Peak windows partial
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
async fn ts_peak_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return ts_unavailable("peak windows");
    }
    let params = birdnet_timeseries::types::params::PeakParams {
        window_minutes: 15,
        hop_minutes: 5,
        lookback_days: 1,
        limit: 5,
    };
    let result =
        tokio::task::spawn_blocking(move || state.with_timeseries(|ts| ts.peak_windows(&params)))
            .await;

    match result {
        Ok(Some(Ok(rows))) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_peak_table(&rows),
        ),
        Ok(Some(Err(e))) => ts_error(&e.to_string()),
        _ => ts_unavailable("peak windows"),
    }
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
        return r#"<p style="color:var(--text-muted)">No heatmap data yet.</p>"#.to_string();
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
        let bar_pct = (row.avg_detections_per_day / max_avg * 100.0) as u32;
        let _ = write!(
            html,
            r#"<tr>
<td style="font-weight:600;">{h:02}:00</td>
<td>{avg:.1}</td>
<td>{total}</td>
<td><div style="width:{pct}%;height:8px;background:var(--accent);border-radius:4px;min-width:2px;"></div></td>
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
        return r#"<p style="color:var(--text-muted)">No trend data yet.</p>"#.to_string();
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Date</th><th>Detections</th><th>7-Day Avg</th></tr></thead><tbody>",
    );
    for row in rows.iter().rev().take(14).rev() {
        let avg = row
            .moving_avg_detections
            .map_or("—".to_string(), |v| format!("{v:.1}"));
        let _ = write!(
            html,
            r#"<tr><td>{}</td><td>{}</td><td>{}</td></tr>"#,
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
        return r#"<p style="color:var(--text-muted)">No diversity data yet.</p>"#.to_string();
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Date</th><th>Richness</th><th>Shannon H′</th><th>Evenness</th></tr></thead><tbody>",
    );
    for row in rows.iter().rev().take(14).rev() {
        let h = row.shannon_h.map_or("—".to_string(), |v| format!("{v:.3}"));
        let ev = row
            .pielou_evenness
            .map_or("—".to_string(), |v| format!("{v:.2}"));
        let _ = write!(
            html,
            r#"<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
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
        return r#"<p style="color:var(--text-muted)">No activity sessions found.</p>"#.to_string();
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Start</th><th>Duration</th><th>Detections</th><th>Species</th></tr></thead><tbody>",
    );
    for row in rows.iter().take(20) {
        let _ = write!(
            html,
            r#"<tr><td>{}</td><td>{}m</td><td>{}</td><td>{}</td></tr>"#,
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
        return r#"<p style="color:var(--success)">✓ No anomalies detected in the last 90 days.</p>"#.to_string();
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Date</th><th>Detections</th><th>Z-Score</th><th>Type</th></tr></thead><tbody>",
    );
    for row in &anomalous {
        let z = row.z_score.map_or("—".to_string(), |v| format!("{v:.2}"));
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
        return r#"<p style="color:var(--text-muted)">No peak window data today.</p>"#.to_string();
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Window Start</th><th>Window End</th><th>Detections</th><th>Species</th></tr></thead><tbody>",
    );
    for row in rows {
        let _ = write!(
            html,
            r#"<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
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
            r#"<p style="color:var(--text-muted)">{endpoint}: start with <code>--analytics-db</code> to enable.</p>"#
        )
    } else {
        format!(
            r#"<p style="color:var(--text-muted)">{endpoint}: rebuild with <code>--features analytics</code>.</p>"#
        )
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], msg)
}

#[cfg(feature = "analytics")]
fn ts_error(error: &str) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "text/html")],
        format!(
            r#"<p style="color:var(--danger)">Error: {}</p>"#,
            escape_html(error)
        ),
    )
}
