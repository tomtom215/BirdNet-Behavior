//! Detection history page with date navigation.
//!
//! Provides a date-picker-based view of hourly detection charts for any day,
//! with previous/next day navigation. Replaces BirdNET-Pi's `history.php`.

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use super::charts::render_hourly_chart;
use super::{escape_html, render_page_for_request, today_date_string};
use crate::state::AppState;

/// Mount the detection history page and its HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/history", get(history_page))
        .route("/pages/history-chart", get(history_chart_partial))
        .route("/pages/history-dates", get(history_dates_partial))
}

/// Query parameters for date selection.
#[derive(Debug, Deserialize)]
pub struct HistoryParams {
    /// Selected date (YYYY-MM-DD). Defaults to today.
    pub date: Option<String>,
}

/// Full history page (shell with HTMX-loaded content).
async fn history_page(headers: HeaderMap) -> Html<String> {
    let body = HISTORY_SHELL_HTML.replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::Analytics),
    );
    render_page_for_request("Detection History", &body, "history", &headers)
}

/// HTMX partial: hourly detection chart + summary for a specific date.
async fn history_chart_partial(
    State(state): State<AppState>,
    Query(params): Query<HistoryParams>,
) -> impl IntoResponse {
    let date = params
        .date
        .filter(|d| d.len() == 10)
        .unwrap_or_else(today_date_string);

    let date2 = date.clone();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let hours = birdnet_db::sqlite::hourly_activity(conn, &date)?;
            let total = birdnet_db::sqlite::detection_count_for_date(conn, &date)?;
            let species = birdnet_db::sqlite::species_for_date(conn, &date)?;
            Ok::<_, birdnet_db::sqlite::DbError>((hours, total, species))
        })
    })
    .await;

    let html = match result {
        Ok(Ok((hours, total, species))) => {
            render_chart_content(&date2, total, species.len(), &hours)
        }
        _ => "<p class='error'>Failed to load chart data.</p>".to_string(),
    };

    axum::response::Html(html)
}

/// HTMX partial: list of all dates with detections (for calendar/date picker).
async fn history_dates_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(birdnet_db::sqlite::distinct_detection_dates)
    })
    .await;

    let html = match result {
        Ok(Ok(dates)) => render_date_list(&dates),
        _ => "<p class='error'>Failed to load dates.</p>".to_string(),
    };

    axum::response::Html(html)
}

// ---------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------

fn render_chart_content(
    date: &str,
    total: i64,
    species_count: usize,
    hours: &[birdnet_db::sqlite::HourlyCount],
) -> String {
    let prev = add_days(date, -1);
    let next = add_days(date, 1);
    let today = today_date_string();
    let is_today = date == today;

    let mut html = String::new();

    // Navigation row
    let next_btn = if next <= today {
        format!(
            r"<a href='#' hx-get='/pages/history-chart?date={next}' hx-target='#chart-content' hx-swap='innerHTML'
               class='hist-nav-btn'>&#8594;</a>",
        )
    } else {
        r"<span class='hist-nav-off'>&#8594;</span>".to_string()
    };
    let today_badge = if is_today {
        r"<span class='hist-badge'>Today</span>"
    } else {
        ""
    };
    let _ = write!(
        html,
        r#"<div class="hist-nav">
  <a href='#' hx-get='/pages/history-chart?date={prev}' hx-target='#chart-content' hx-swap='innerHTML'
     class="hist-nav-btn">&#8592;</a>
  <div class="hist-nav-center">
    <strong class="hist-nav-title">{date}</strong>
    {today_badge}
  </div>
  {next_btn}
</div>"#,
        prev = prev,
        date = escape_html(date),
        today_badge = today_badge,
        next_btn = next_btn,
    );

    // Stats row
    let _ = write!(
        html,
        r#"<div class="hist-stats">
  <div class="hist-stat">
    <span class="hist-stat-num accent">{total}</span>
    <span class="hist-stat-unit">detections</span>
  </div>
  <div class="hist-stat">
    <span class="hist-stat-num success">{species_count}</span>
    <span class="hist-stat-unit">species</span>
  </div>
</div>"#,
    );

    // Hourly chart
    let _ = write!(
        html,
        r#"<div class="hist-chart-card">
  <h3 class="hist-chart-title">Detections by Hour</h3>
  {chart}
</div>"#,
        chart = render_hourly_chart(hours),
    );

    html
}

/// Render a compact list of dates with detections (newest first, for sidebar).
fn render_date_list(dates: &[String]) -> String {
    if dates.is_empty() {
        return r#"<p class="hist-empty">No detection history yet.</p>"#.to_string();
    }

    let mut html = String::from(r#"<ul class="hist-date-list">"#);

    for date in dates.iter().rev().take(90) {
        let _ = write!(
            html,
            r#"<li><a href='#' hx-get='/pages/history-chart?date={date}' hx-target='#chart-content' hx-swap='innerHTML'
               class="hist-date-link">{date}</a></li>"#,
            date = escape_html(date),
        );
    }

    html.push_str("</ul>");
    html
}

/// Add `delta` days to a YYYY-MM-DD date string.
fn add_days(date: &str, delta: i64) -> String {
    use super::{date_to_epoch_days, days_to_date};

    // A malformed date has no sensible neighbour — leave the nav link inert by
    // echoing it back rather than snapping to an epoch date.
    if date.len() < 10 {
        return date.to_string();
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let new_days = (date_to_epoch_days(date) as i64 + delta).max(0) as u64;
    let (ny, nm, nd) = days_to_date(new_days);
    format!("{ny}-{nm:02}-{nd:02}")
}

const HISTORY_SHELL_HTML: &str = r#"<div class="page-content hist-content">
  <div class="bnb-eyebrow">Browse the past</div><h2 class="display hist-h2">History</h2>{{help_link}}
  <div class="hist-layout">
    <!-- Date list sidebar -->
    <div class="hist-side">
      <div class="hist-side-head">
        Recent dates
      </div>
      <div hx-get="/pages/history-dates" hx-trigger="load" hx-swap="innerHTML">
        <p class="hist-loading">Loading...</p>
      </div>
    </div>
    <!-- Chart area -->
    <div id="chart-content"
         hx-get="/pages/history-chart"
         hx-trigger="load"
         hx-swap="innerHTML">
      <div class="hist-chart-loading">Loading chart...</div>
    </div>
  </div>
</div>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_days_basic() {
        assert_eq!(add_days("2026-03-14", 1), "2026-03-15");
        assert_eq!(add_days("2026-03-14", -1), "2026-03-13");
    }

    #[test]
    fn add_days_month_wrap() {
        assert_eq!(add_days("2026-03-01", -1), "2026-02-28");
        assert_eq!(add_days("2026-12-31", 1), "2027-01-01");
    }

    #[test]
    fn render_chart_content_no_data() {
        let html = render_chart_content("2026-03-14", 0, 0, &[]);
        assert!(html.contains("2026-03-14"));
        assert!(html.contains('0'));
    }

    #[test]
    fn render_date_list_empty() {
        let html = render_date_list(&[]);
        assert!(html.contains("No detection history"));
    }
}
