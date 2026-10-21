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
        .filter(|w| is_ymd(w))
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
    let names = state.names_handle();

    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
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
            &names,
        ),
        // 200 either way, so this string is the reader's last word — it has
        // to carry the same "this is a fault, not an empty week" framing and
        // the same link as every other failed surface.
        _ => super::error_states::inline("this week's report"),
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
    names: &birdnet_core::i18n::NameLocalizer,
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
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">This week</div><h2 class="sh-h">Detections per day</h2></div></div><div class="rp-viz">{chart}</div></div>"#,
        chart = render_weekly_chart(week_start, daily, &super::today_date_string()),
    );

    // ── Two columns: top species + first-ever ─────────────────────────────
    html.push_str(r#"<div class="rp-2col">"#);

    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">This week</div><h3>Top species</h3></div><a class="action" href="/species">All species →</a></div>"#,
    );
    if top.is_empty() {
        html.push_str(r#"<p class="wk-muted">No detections this week.</p>"#);
    } else {
        for (i, (sci, com, count)) in top.iter().take(10).enumerate() {
            let shown = names.name(sci, com);
            let _ = write!(
                html,
                r#"<div class="rp-row"><span class="rk">{rank}</span>{av}<div class="nm">{name}</div><span class="ct">{count}</span></div>"#,
                rank = i + 1,
                av = avatar(com, shown, sci, ""),
                name = escape_html(shown),
            );
        }
    }
    html.push_str("</div>");

    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">First-ever</div><h2 class="sh-h">New to your station</h2></div></div>"#,
    );
    if new_species.is_empty() {
        html.push_str(r#"<p class="wk-muted">No new species this week.</p>"#);
    } else {
        for (sci, com, date) in new_species {
            let shown = names.name(sci, com);
            let _ = write!(
                html,
                r#"<div class="rp-new">{av}<div class="nm">{name} <span class="bnb-pill rare badge">first ever</span></div><span class="when">{date}</span></div>"#,
                av = avatar(com, shown, sci, ""),
                name = escape_html(shown),
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

/// The week as seven columns, Monday to Sunday, on an axis from zero: the busiest day
/// labelled, the average as a hairline, a tooltip for every day. A day of the current week
/// that has not happened yet is an empty slot that says "Not yet", not a zero.
fn render_weekly_chart(
    week_start: &str,
    daily: &[birdnet_db::sqlite::DailyCount],
    today: &str,
) -> String {
    let dates = week_dates(week_start);
    let mut counts = [0_i64; 7];
    for dc in daily {
        if let Some(idx) = dates.iter().position(|d| d == &dc.date)
            && idx < 7
        {
            counts[idx] = dc.count;
        }
    }
    let days: Vec<(String, i64)> = dates.iter().cloned().zip(counts).collect();
    let future_from = dates.iter().position(|d| d.as_str() > today);
    super::viz::bars::daily(&super::viz::bars::Daily {
        id: "weekly-days",
        name: "Detections per day this week",
        desc: "Detections for each day of the week, Monday to Sunday; the busiest day is labelled and every day has a tooltip.",
        unit: "detections",
        days: &days,
        slot: 0,
        width: 560.0,
        plot_h: 120.0,
        listened: None,
        mean_line: true,
        future_from,
        weekday_labels: true,
        table: false,
    })
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

/// Is `s` a `YYYY-MM-DD` date: digits where digits go, a month of 01–12 and a
/// day of 01–31? The week comes from the query string and is written back into
/// the page's navigation links, so "ten characters long" is not a date.
fn is_ymd(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
        && matches!(s[5..7].parse::<u8>(), Ok(1..=12))
        && matches!(s[8..10].parse::<u8>(), Ok(1..=31))
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

    fn view_box(svg: &str) -> (f64, f64, f64, f64) {
        let vb = svg
            .split_once("viewBox=\"")
            .unwrap()
            .1
            .split_once('"')
            .unwrap()
            .0;
        let n: Vec<f64> = vb
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        (n[0], n[1], n[2], n[3])
    }

    /// Every `x="…"` / `y="…"` / `height="…"` / `width="…"` number of every tag of one kind.
    fn nums(svg: &str, tag: &str, attr: &str) -> Vec<f64> {
        svg.split(&format!("<{tag} "))
            .skip(1)
            .filter_map(|frag| {
                let body = frag.split('>').next().unwrap_or("");
                let rest = body
                    .split_once(&format!(" {attr}=\""))
                    .or_else(|| body.split_once(&format!("\"{attr}=\"")))
                    .map(|x| x.1)?;
                rest.split('"').next()?.parse().ok()
            })
            .collect()
    }

    fn week(counts: [i64; 7]) -> Vec<birdnet_db::sqlite::DailyCount> {
        (0..7)
            .map(|i: usize| birdnet_db::sqlite::DailyCount {
                date: format!("2026-03-{:02}", 9 + i),
                count: counts[i],
            })
            .collect()
    }

    /// Nothing the weekly chart draws may fall outside its own viewBox.
    ///
    /// Observed failing against the pre-fix renderer on two counts, both visible in a
    /// print render of the demo fixture: the tallest bar's count label was placed above
    /// the top of the viewBox where the browser clips it (the busiest day was the one
    /// day whose number you could not read), and a zero-count day's stub hung below the
    /// baseline. Kept for the new renderer: the same arithmetic, new code.
    #[test]
    fn nothing_the_weekly_chart_draws_escapes_its_view_box() {
        for counts in [
            [34, 36, 33, 35, 34, 34, 0], // a tall bar and an empty day
            [1, 0, 0, 0, 0, 0, 0],
            [0; 7],
            [999_999, 1, 0, 5, 0, 2, 3],
        ] {
            let svg = render_weekly_chart("2026-03-09", &week(counts), "2026-03-20");
            let (vx, vy, vw, vh) = view_box(&svg);
            for (i, y) in nums(&svg, "text", "y").iter().enumerate() {
                assert!(
                    *y >= vy && *y <= vy + vh,
                    "{counts:?}: label {i} sits at y = {y} outside {vy}..{}",
                    vy + vh
                );
            }
            for (i, x) in nums(&svg, "text", "x").iter().enumerate() {
                assert!(
                    *x >= vx && *x <= vx + vw,
                    "{counts:?}: label {i} sits at x = {x} outside the width"
                );
            }
            for (i, (x, w)) in nums(&svg, "rect", "x")
                .iter()
                .zip(nums(&svg, "rect", "width"))
                .enumerate()
            {
                assert!(
                    *x >= vx && x + w <= vx + vw + 1e-6,
                    "{counts:?}: box {i} overflows the width"
                );
            }
        }
    }

    /// A week still under way ends in days that have not happened: they say so, and
    /// they are not in the average or the busiest day.
    #[test]
    fn days_that_have_not_happened_are_not_zeros() {
        let svg = render_weekly_chart("2026-03-09", &week([10, 20, 30, 0, 0, 0, 0]), "2026-03-11");
        assert!(svg.contains("Thu 12 Mar&#10;Not yet"), "{svg}");
        assert!(svg.contains("Sun 15 Mar&#10;Not yet"), "{svg}");
        assert!(
            svg.contains("avg 20"),
            "the average is over the three days that happened: {svg}"
        );
        assert!(svg.contains("Mon") && svg.contains("Sun"), "{svg}");
        // Seven wide slots: each weekday is centred under its own bar, none hung from an edge.
        assert!(
            !svg.contains(r#"text-anchor="start""#) && !svg.contains(r#"text-anchor="end">Sun"#),
            "{svg}"
        );
        // A finished week has no such days.
        assert!(
            !render_weekly_chart("2026-03-09", &week([10, 20, 30, 0, 0, 0, 0]), "2026-03-20")
                .contains("Not yet")
        );
    }

    #[test]
    fn a_week_is_a_date_not_ten_characters() {
        assert!(is_ymd("2026-03-09"));
        assert!(is_ymd("2026-12-31"));
        // Ten characters, and each a way to be wrong.
        for bad in [
            "\"><zq-p>12",
            "2026-13-01",
            "2026-00-10",
            "2026-02-32",
            "2026-02-00",
            "2026/03/09",
            "2026-3-009",
            "abcd-ef-gh",
            "２０２６-03-09",
            "",
            "2026-03-091",
        ] {
            assert!(!is_ymd(bad), "{bad:?} must not pass as a date");
        }
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
