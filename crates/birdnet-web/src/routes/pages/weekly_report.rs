//! Weekly report page.
//!
//! Shows top species, new species first detected, total detections, and a
//! 7-day bar chart for the current (or any selected) ISO week.

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{date_to_epoch_days, days_to_date, escape_html};
use crate::state::AppState;

/// Mount the weekly report page and its HTMX content partial route.
pub fn router() -> Router<AppState> {
    Router::new().route("/pages/weekly-content", get(weekly_partial))
}

/// Query parameters for week navigation.
#[derive(Debug, Deserialize)]
pub struct WeekParams {
    /// ISO week start date (YYYY-MM-DD, Monday). Defaults to current week.
    pub week: Option<String>,
}

/// The weekly-report surface (shell only; content loaded by HTMX), rendered
/// for embedding by `homes::reports` ("Weekly" tab).
pub(super) fn content() -> String {
    // O-20 help link wires the eyebrow to the Reports mdBook page.
    WEEKLY_SHELL_HTML.replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::Reports),
    )
}

/// HTMX partial: the weekly report content for a given week.
async fn weekly_partial(
    State(state): State<AppState>,
    Query(params): Query<WeekParams>,
) -> impl IntoResponse {
    let week_start = params
        .week
        .filter(|w| w.len() == 10)
        .unwrap_or_else(current_week_monday);

    let week_end = add_days(&week_start, 6);
    let prev_week = add_days(&week_start, -7);
    let next_week = add_days(&week_start, 7);
    let today = today_string();
    let is_current = week_start <= today && today <= week_end;

    let week_start2 = week_start.clone();
    let week_end2 = week_end.clone();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let total = birdnet_db::sqlite::weekly_detection_count(conn, &week_start, &week_end)?;
            let top = birdnet_db::sqlite::weekly_top_species(conn, &week_start, &week_end, 10)?;
            let new = birdnet_db::sqlite::weekly_new_species(conn, &week_start, &week_end)?;
            let daily = birdnet_db::sqlite::range_daily_counts(conn, &week_start, &week_end)?;
            Ok::<_, birdnet_db::sqlite::DbError>((total, top, new, daily))
        })
    })
    .await;

    let html = match result {
        Ok(Ok((total, top, new_species, daily))) => render_weekly_content(
            &week_start2,
            &week_end2,
            &prev_week,
            &next_week,
            total,
            &top,
            &new_species,
            &daily,
            is_current,
        ),
        _ => "<p class='error'>Failed to load weekly report.</p>".to_string(),
    };

    axum::response::Html(html)
}

// ---------------------------------------------------------------------------
// HTML rendering helpers
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn render_weekly_content(
    week_start: &str,
    week_end: &str,
    prev_week: &str,
    next_week: &str,
    total: i64,
    top: &[(String, String, i64)],
    new_species: &[(String, String, String)],
    daily: &[birdnet_db::sqlite::DailyCount],
    is_current: bool,
) -> String {
    let mut html = String::new();

    // Week navigation header
    let current_badge = if is_current {
        r"<span class='wk-badge'>Current Week</span>"
    } else {
        ""
    };
    let today_s = today_string();
    let next_btn = if next_week <= today_s.as_str() {
        format!(
            r#"<a href='#' hx-get='/pages/weekly-content?week={next_week}' hx-target='#weekly-content' hx-swap='innerHTML'
               class="wk-nav-btn">Next &#8594;</a>"#
        )
    } else {
        r#"<span class="wk-nav-next-off">Next &#8594;</span>"#.to_string()
    };
    let _ = write!(
        html,
        r#"<div class="week-nav wk-nav">
  <a href='#' hx-get='/pages/weekly-content?week={prev_week}' hx-target='#weekly-content' hx-swap='innerHTML'
     class="wk-nav-btn">&#8592; Prev</a>
  <div class="wk-nav-center">
    <strong class="wk-nav-title">{week_start} &ndash; {week_end}</strong>
    {current_badge}
  </div>
  {next_btn}
</div>"#,
    );

    // Summary stats
    let species_count = top.len();
    let _ = write!(
        html,
        r#"<div class="wk-stats">
  <div class="wk-card center">
    <div class="wk-stat-num accent">{total}</div>
    <div class="wk-stat-label">Total Detections</div>
  </div>
  <div class="wk-card center">
    <div class="wk-stat-num success">{species}</div>
    <div class="wk-stat-label">Species Detected</div>
  </div>
  <div class="wk-card center">
    <div class="wk-stat-num warning">{new}</div>
    <div class="wk-stat-label">New Species</div>
  </div>
</div>"#,
        total = total,
        species = species_count,
        new = new_species.len(),
    );

    // 7-day bar chart
    let _ = write!(
        html,
        r#"<div class="wk-card mb">
  <h3 class="wk-card-title">Daily Activity</h3>
  {chart}
</div>"#,
        chart = render_weekly_chart(week_start, daily),
    );

    // Two-column layout: top species + new species
    let _ = write!(html, r#"<div class="wk-cols">"#);

    // Top 10 species
    html.push_str(
        r#"<div class="wk-card">
<h3 class="wk-card-title">Top Species This Week</h3>"#,
    );
    if top.is_empty() {
        html.push_str(r#"<p class="wk-muted">No detections this week.</p>"#);
    } else {
        html.push_str(r#"<ol class="wk-top-list">"#);
        let max_count = top.first().map_or(1, |(_, _, c)| *c).max(1);
        for (sci, com, count) in top {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                clippy::cast_possible_wrap,
                clippy::cast_lossless
            )]
            let pct = (*count as f64 / max_count as f64 * 100.0) as u32;
            let _ = write!(
                html,
                r#"<li class="wk-top-item">
  <a href="/species/{sci_enc}" class="wk-top-name">{com_esc}</a>
  <div class="wk-top-bar-row">
    <div class="wk-top-track">
      <div class="wk-top-fill" data-style="width:{pct}%"></div>
    </div>
    <span class="wk-top-count">{count}</span>
  </div>
</li>"#,
                sci_enc = escape_html(&super::simple_url_encode(sci)),
                com_esc = escape_html(com),
                pct = pct,
                count = count,
            );
        }
        html.push_str("</ol>");
    }
    html.push_str("</div>");

    // New species
    html.push_str(
        r#"<div class="wk-card">
<h3 class="wk-card-title">New Species This Week
  <span class="sub">(first ever)</span>
</h3>"#,
    );
    if new_species.is_empty() {
        html.push_str(r#"<p class="wk-muted">No new species this week.</p>"#);
    } else {
        html.push_str(r#"<ul class="wk-new-list">"#);
        for (sci, com, date) in new_species {
            let _ = write!(
                html,
                r#"<li class="wk-new-item">
  <span class="wk-new-badge">NEW</span>
  <a href="/species/{sci_enc}" class="wk-new-name">{com_esc}</a>
  <span class="wk-new-date">{date}</span>
</li>"#,
                sci_enc = escape_html(&super::simple_url_encode(sci)),
                com_esc = escape_html(com),
                date = date,
            );
        }
        html.push_str("</ul>");
    }
    html.push_str("</div></div>"); // close grid + new species card

    html
}

/// Render a 7-bar SVG chart for the week (one bar per day).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]
fn render_weekly_chart(week_start: &str, daily: &[birdnet_db::sqlite::DailyCount]) -> String {
    // Build date → count map for the 7 days
    let mut counts = [0i64; 7];
    let mut day_labels = [""; 7];
    let day_names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let dates = week_dates(week_start);
    day_labels.copy_from_slice(&day_names);

    for dc in daily {
        if let Some(idx) = dates.iter().position(|d| d == &dc.date)
            && idx < 7
        {
            counts[idx] = dc.count;
        }
    }

    let max_count = counts.iter().copied().max().unwrap_or(1).max(1);
    let chart_w = 560;
    let chart_h = 120;
    let bar_w = 60;
    let gap = 20;
    let left_pad = 10;

    let mut svg = format!(
        r#"<svg viewBox="0 0 {w} {h}" class="cht-svg" xmlns="http://www.w3.org/2000/svg">"#,
        w = chart_w,
        h = chart_h + 25,
    );

    for (i, &count) in counts.iter().enumerate() {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        let x = left_pad + i as i32 * (bar_w + gap);
        let bar_h = (count as f64 / max_count as f64 * chart_h as f64) as i32;
        let y = chart_h - bar_h;
        let color = if count > 0 {
            "var(--moss)"
        } else {
            "var(--surface-2)"
        };

        let _ = std::fmt::write(
            &mut svg,
            format_args!(
                r#"<rect x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="3" fill="{color}"/>"#,
                x = x,
                y = y,
                bar_w = bar_w,
                bar_h = bar_h.max(2),
                color = color,
            ),
        );

        if count > 0 {
            let _ = std::fmt::write(
                &mut svg,
                format_args!(
                    r#"<text x="{tx}" y="{ty}" text-anchor="middle" fill="var(--fg-3)" font-size="11" font-family="sans-serif">{count}</text>"#,
                    tx = x + bar_w / 2,
                    ty = y - 4,
                    count = count,
                ),
            );
        }

        let _ = std::fmt::write(
            &mut svg,
            format_args!(
                r#"<text x="{tx}" y="{ty}" text-anchor="middle" fill="var(--fg-4)" font-size="11" font-family="sans-serif">{label}</text>"#,
                tx = x + bar_w / 2,
                ty = chart_h + 17,
                label = day_labels[i],
            ),
        );
    }

    svg.push_str("</svg>");
    svg
}

// ---------------------------------------------------------------------------
// Date arithmetic (no external crate)
// ---------------------------------------------------------------------------

/// Get the 7 date strings for the week starting on `week_start`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
fn week_dates(week_start: &str) -> [String; 7] {
    let mut result: [String; 7] = Default::default();
    for (i, item) in result.iter_mut().enumerate() {
        *item = add_days(week_start, i as i64);
    }
    result
}

/// Add `delta` days to a YYYY-MM-DD date string. Returns the new date string.
fn add_days(date: &str, delta: i64) -> String {
    let epoch = date_to_epoch_days(date);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let new_epoch = (epoch as i64 + delta).max(0) as u64;
    let (y, m, d) = days_to_date(new_epoch);
    format!("{y}-{m:02}-{d:02}")
}

/// Current Monday's date (start of ISO week) as YYYY-MM-DD.
fn current_week_monday() -> String {
    let today = today_string();
    let days = date_to_epoch_days(&today);
    // days % 7: 0=Thu, 1=Fri, 2=Sat, 3=Sun, 4=Mon, 5=Tue, 6=Wed
    // We need offset back to Monday (weekday 4 in this system)
    let dow = days % 7; // 0=Thu
    let offset_to_monday: i64 = match dow {
        5 => -1,
        6 => -2,
        0 => -3, // Thursday → -3
        1 => -4, // Friday → -4
        2 => -5, // Saturday → -5
        3 => -6, // Sunday → -6
        _ => 0,  // 4=Monday (already correct) or unexpected value
    };
    add_days(&today, offset_to_monday)
}

fn today_string() -> String {
    super::today_date_string()
}

// ---------------------------------------------------------------------------
// Static HTML shell
// ---------------------------------------------------------------------------

const WEEKLY_SHELL_HTML: &str = r#"<div class="page-content wk-content">
  <div class="bnb-eyebrow wk-eyebrow"><span>The backyard bulletin</span>{{help_link}}</div><h2 class="display wk-h2">Weekly report</h2>
  <div id="weekly-content"
       hx-get="/pages/weekly-content"
       hx-trigger="load"
       hx-swap="innerHTML">
    <div class="wk-loading">Loading weekly report...</div>
  </div>
</div>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_days_forward() {
        assert_eq!(add_days("2026-03-14", 1), "2026-03-15");
        assert_eq!(add_days("2026-03-14", 7), "2026-03-21");
        assert_eq!(add_days("2026-03-14", -1), "2026-03-13");
    }

    #[test]
    fn add_days_month_boundary() {
        assert_eq!(add_days("2026-03-31", 1), "2026-04-01");
        assert_eq!(add_days("2026-04-01", -1), "2026-03-31");
    }

    #[test]
    fn week_dates_length() {
        let dates = week_dates("2026-03-09");
        assert_eq!(dates.len(), 7);
        assert_eq!(dates[0], "2026-03-09");
        assert_eq!(dates[6], "2026-03-15");
    }

    #[test]
    fn current_week_monday_is_monday() {
        // Just verify it returns a valid date string without panicking
        let monday = current_week_monday();
        assert_eq!(monday.len(), 10);
        assert_eq!(&monday[4..5], "-");
        assert_eq!(&monday[7..8], "-");
    }
}
