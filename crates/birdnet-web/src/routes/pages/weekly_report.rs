//! Weekly report page.
//!
//! Shows top species, new species first detected, total detections, and a
//! 7-day bar chart for the current (or any selected) ISO week.

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::atoms::avatar;
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
    let prev_week_end = add_days(&prev_week, 6);
    let next_week = add_days(&week_start, 7);
    let today = today_string();
    let is_current = week_start <= today && today <= week_end;

    let week_start2 = week_start.clone();
    let week_end2 = week_end.clone();
    let prev_week_c = prev_week.clone();
    let prev_week_end_c = prev_week_end.clone();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let total = birdnet_db::sqlite::weekly_detection_count(conn, &week_start, &week_end)?;
            // Large limit so the returned list length is the true weekly species
            // count; the leaderboard still renders only the top 10.
            let top = birdnet_db::sqlite::weekly_top_species(conn, &week_start, &week_end, 1000)?;
            let new = birdnet_db::sqlite::weekly_new_species(conn, &week_start, &week_end)?;
            let daily = birdnet_db::sqlite::range_daily_counts(conn, &week_start, &week_end)?;
            let prev_total =
                birdnet_db::sqlite::weekly_detection_count(conn, &prev_week_c, &prev_week_end_c)?;
            Ok::<_, birdnet_db::sqlite::DbError>((total, top, new, daily, prev_total))
        })
    })
    .await;

    let html = match result {
        Ok(Ok((total, top, new_species, daily, prev_total))) => render_weekly_content(
            &week_start2,
            &week_end2,
            &prev_week,
            &next_week,
            total,
            prev_total,
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

#[allow(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn render_weekly_content(
    week_start: &str,
    week_end: &str,
    prev_week: &str,
    next_week: &str,
    total: i64,
    prev_total: i64,
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

    // ── Editorial hero ────────────────────────────────────────────────────
    let species_count = top.len();
    let new_count = new_species.len();
    let delta_pct = if prev_total > 0 {
        ((total - prev_total) as f64 / prev_total as f64 * 100.0).round() as i64
    } else {
        0
    };
    let headline = if total == 0 {
        "A <em>silent</em> week."
    } else if delta_pct >= 10 {
        "A <em>loud</em> week."
    } else if delta_pct <= -10 {
        "A <em>quieter</em> week."
    } else {
        "A <em>steady</em> week."
    };
    let mut lead = format!("<b>{total} detections</b> from <b>{species_count} species</b>");
    if prev_total > 0 && delta_pct != 0 {
        let word = if delta_pct > 0 { "busier" } else { "quieter" };
        let _ = write!(lead, " — {}% {word} than the week before", delta_pct.abs());
    }
    if new_count > 0 {
        let plural = if new_count == 1 { "bird" } else { "birds" };
        let _ = write!(lead, ", and {new_count} {plural} new to your list");
    }
    lead.push('.');
    let _ = write!(
        html,
        r#"<div class="rp-hero"><div class="eyebrow">Weekly report</div><h1>{headline}</h1><p class="lead">{lead}</p></div>"#,
    );

    // ── Stat band ─────────────────────────────────────────────────────────
    let det_detail = if prev_total > 0 {
        let arrow = if delta_pct >= 0 { "↑" } else { "↓" };
        format!("{arrow} {}% vs last week", delta_pct.abs())
    } else {
        "first full week".to_string()
    };
    let species_detail = if new_count > 0 {
        format!("+{new_count} first-ever")
    } else {
        "none new".to_string()
    };
    let (busy_label, busy_count) = busiest_day(week_start, daily);
    let _ = write!(
        html,
        r#"<div class="rp-stats">
  <div class="rp-stat"><div class="v moss">{total}</div><div class="l">detections</div><div class="d">{det_detail}</div></div>
  <div class="rp-stat"><div class="v">{species_count}</div><div class="l">species</div><div class="d">{species_detail}</div></div>
  <div class="rp-stat"><div class="v rare">{new_count}</div><div class="l">new to your list</div></div>
  <div class="rp-stat"><div class="v">{busy_label}</div><div class="l">busiest day</div><div class="d">{busy_count} detections</div></div>
</div>"#,
    );

    // ── Daily chart ───────────────────────────────────────────────────────
    let _ = write!(
        html,
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">This week</div><h3>Detections per day</h3></div></div><div class="rp-viz">{chart}</div></div>"#,
        chart = render_weekly_chart(week_start, daily),
    );

    // ── Two columns: top species + first-ever ─────────────────────────────
    html.push_str(r#"<div class="rp-2col">"#);

    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">This week</div><h3>Top species</h3></div><a class="action" href="/species">All species →</a></div>"#,
    );
    if top.is_empty() {
        html.push_str(r#"<p class="wk-muted">No detections this week.</p>"#);
    } else {
        for (i, (_, com, count)) in top.iter().take(10).enumerate() {
            let _ = write!(
                html,
                r#"<div class="rp-row"><span class="rk">{rank}</span>{av}<div class="nm">{name}</div><span class="ct">{count}</span></div>"#,
                rank = i + 1,
                av = avatar(com, ""),
                name = escape_html(com),
            );
        }
    }
    html.push_str("</div>");

    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">First-ever</div><h3>New to your station</h3></div></div>"#,
    );
    if new_species.is_empty() {
        html.push_str(r#"<p class="wk-muted">No new species this week.</p>"#);
    } else {
        for (_, com, date) in new_species {
            let _ = write!(
                html,
                r#"<div class="rp-new">{av}<div class="nm">{name} <span class="bnb-pill rare badge">first ever</span></div><span class="when">{date}</span></div>"#,
                av = avatar(com, ""),
                name = escape_html(com),
                date = escape_html(date),
            );
        }
    }
    html.push_str("</div></div>");

    html
}

/// The week's busiest weekday — `(label, count)` from its daily totals.
fn busiest_day(week_start: &str, daily: &[birdnet_db::sqlite::DailyCount]) -> (String, i64) {
    let dates = week_dates(week_start);
    let day_names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut best_idx = 0usize;
    let mut best = -1i64;
    for dc in daily {
        if let Some(idx) = dates.iter().position(|d| d == &dc.date)
            && dc.count > best
        {
            best = dc.count;
            best_idx = idx;
        }
    }
    if best < 0 {
        ("—".to_string(), 0)
    } else {
        (day_names[best_idx].to_string(), best)
    }
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
