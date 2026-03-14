//! Detection history page with date navigation.
//!
//! Provides a date-picker-based view of hourly detection charts for any day,
//! with previous/next day navigation. Replaces BirdNET-Pi's `history.php`.

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use super::charts::render_hourly_chart;
use super::{escape_html, render_page, today_date_string};
use crate::state::AppState;

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
async fn history_page() -> Html<String> {
    render_page("Detection History", HISTORY_SHELL_HTML, "history")
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
            r##"<a href='#' hx-get='/pages/history-chart?date={next}' hx-target='#chart-content' hx-swap='innerHTML'
               style='padding:0.3rem 0.75rem;background:var(--bg-card);border:1px solid var(--border);border-radius:var(--radius);color:var(--text-muted);'>&#8594;</a>"##,
            next = next,
        )
    } else {
        r#"<span style='padding:0.3rem 0.75rem;color:var(--text-muted);opacity:0.4;'>&#8594;</span>"#.to_string()
    };
    let today_badge = if is_today {
        r#"<span style='margin-left:0.5rem;font-size:0.8rem;background:var(--accent);color:#fff;padding:0.1rem 0.4rem;border-radius:4px;'>Today</span>"#
    } else {
        ""
    };
    let _ = write!(
        html,
        r##"<div style="display:flex;align-items:center;gap:1rem;margin-bottom:1rem;">
  <a href='#' hx-get='/pages/history-chart?date={prev}' hx-target='#chart-content' hx-swap='innerHTML'
     style="padding:0.3rem 0.75rem;background:var(--bg-card);border:1px solid var(--border);border-radius:var(--radius);color:var(--text-muted);">&#8592;</a>
  <div style="flex:1;text-align:center;">
    <strong style="font-size:1.1rem;">{date}</strong>
    {today_badge}
  </div>
  {next_btn}
</div>"##,
        prev = prev,
        date = escape_html(date),
        today_badge = today_badge,
        next_btn = next_btn,
    );

    // Stats row
    let _ = write!(
        html,
        r#"<div style="display:flex;gap:1rem;margin-bottom:1rem;">
  <div style="background:var(--bg-card);padding:0.75rem 1.25rem;border-radius:var(--radius);border:1px solid var(--border);">
    <span style="font-size:1.5rem;font-weight:700;color:var(--accent);">{total}</span>
    <span style="color:var(--text-muted);margin-left:0.5rem;font-size:0.9rem;">detections</span>
  </div>
  <div style="background:var(--bg-card);padding:0.75rem 1.25rem;border-radius:var(--radius);border:1px solid var(--border);">
    <span style="font-size:1.5rem;font-weight:700;color:var(--success);">{species}</span>
    <span style="color:var(--text-muted);margin-left:0.5rem;font-size:0.9rem;">species</span>
  </div>
</div>"#,
        total = total,
        species = species_count,
    );

    // Hourly chart
    let _ = write!(
        html,
        r#"<div style="background:var(--bg-card);padding:1rem;border-radius:var(--radius);border:1px solid var(--border);">
  <h3 style="margin-bottom:0.75rem;font-size:0.95rem;color:var(--text-muted);">Detections by Hour</h3>
  {chart}
</div>"#,
        chart = render_hourly_chart(hours),
    );

    html
}

/// Render a compact list of dates with detections (newest first, for sidebar).
fn render_date_list(dates: &[String]) -> String {
    if dates.is_empty() {
        return r#"<p style="color:var(--text-muted);padding:1rem;">No detection history yet.</p>"#
            .to_string();
    }

    let mut html = String::from(
        r#"<ul style="list-style:none;padding:0;margin:0;max-height:300px;overflow-y:auto;">"#,
    );

    for date in dates.iter().rev().take(90) {
        let _ = write!(
            html,
            r##"<li><a href='#' hx-get='/pages/history-chart?date={date}' hx-target='#chart-content' hx-swap='innerHTML'
               style="display:block;padding:0.3rem 0.75rem;color:var(--text-muted);font-size:0.9rem;">{date}</a></li>"##,
            date = escape_html(date),
        );
    }

    html.push_str("</ul>");
    html
}

/// Add `delta` days to a YYYY-MM-DD date string.
fn add_days(date: &str, delta: i64) -> String {
    use super::days_to_date;

    if date.len() < 10 {
        return date.to_string();
    }

    let y: u64 = date[0..4].parse().unwrap_or(1970);
    let m: u64 = date[5..7].parse().unwrap_or(1);
    let d: u64 = date[8..10].parse().unwrap_or(1);

    // Rata Die → epoch days conversion
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2 / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let epoch_days = era * 146_097 + doe - 719_468;

    let new_days = (epoch_days as i64 + delta).max(0) as u64;
    let (ny, nm, nd) = days_to_date(new_days);
    format!("{ny}-{nm:02}-{nd:02}")
}

const HISTORY_SHELL_HTML: &str = r#"<div class="page-content" style="padding:1.5rem;">
  <h2 style="margin-bottom:1rem;">Detection History</h2>
  <div style="display:grid;grid-template-columns:200px 1fr;gap:1.5rem;align-items:start;">
    <!-- Date list sidebar -->
    <div style="background:var(--bg-card);border-radius:var(--radius);border:1px solid var(--border);">
      <div style="padding:0.75rem;border-bottom:1px solid var(--border);font-size:0.9rem;color:var(--text-muted);">
        Recent dates
      </div>
      <div hx-get="/pages/history-dates" hx-trigger="load" hx-swap="innerHTML">
        <p style="color:var(--text-muted);padding:1rem;">Loading...</p>
      </div>
    </div>
    <!-- Chart area -->
    <div id="chart-content"
         hx-get="/pages/history-chart"
         hx-trigger="load"
         hx-swap="innerHTML">
      <div style="color:var(--text-muted);text-align:center;padding:3rem;">Loading chart...</div>
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
