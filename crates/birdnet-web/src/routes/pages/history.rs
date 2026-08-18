//! Detection history — a month heat-calendar with a per-day detail panel.
//!
//! The Reports "History" tab: a calendar grid where each day is coloured by its
//! detection count; selecting a day loads that day's hourly chart + top species
//! into the detail panel. Replaces BirdNET-Pi's `history.php`.
//!
//! | Path | Purpose |
//! |------|---------|
//! | (embedded)                    | Reports home, "History" tab               |
//! | `GET /reports/day`            | Full page — one past day's recap + log     |
//! | `GET /pages/history-calendar` | HTMX partial — the month heat-grid         |
//! | `GET /pages/history-chart`    | HTMX partial — a day's hourly chart + tops |
//! | `GET /pages/history-dates`    | HTMX partial — flat date list (legacy)     |

use std::collections::HashMap;
use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use super::atoms::{avatar, conf_bar};
use super::charts::render_hourly_chart;
use super::{date_to_epoch_days, escape_html, simple_url_encode, today_date_string};
use crate::state::AppState;

/// Month abbreviations for the calendar header.
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Mount the detection history partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/reports/day", get(day_page))
        .route("/pages/history-chart", get(history_chart_partial))
        .route("/pages/history-dates", get(history_dates_partial))
        .route("/pages/history-calendar", get(history_calendar_partial))
}

/// Query parameters for day selection.
#[derive(Debug, Deserialize)]
pub struct HistoryParams {
    /// Selected date (YYYY-MM-DD). Defaults to today.
    pub date: Option<String>,
}

/// Query parameters for the calendar partial.
#[derive(Debug, Deserialize)]
pub struct CalendarParams {
    /// Month to render (YYYY-MM). Defaults to the latest month with data.
    pub month: Option<String>,
    /// Day to mark selected (YYYY-MM-DD), kept highlighted across months.
    pub sel: Option<String>,
}

// ---------------------------------------------------------------------------
// Page content — embedded in the Reports home ("History" tab)
// ---------------------------------------------------------------------------

/// The history surface, rendered for embedding by `homes::reports`.
///
/// Server-computes the per-day counts once to seed the latest month and the
/// initial day-detail panel; month navigation and day selection then swap the
/// two partials below.
pub(super) async fn content(state: AppState) -> String {
    let days =
        tokio::task::spawn_blocking(move || state.with_db(birdnet_db::sqlite::detections_per_day))
            .await;
    let days = match days {
        Ok(Ok(d)) => d,
        _ => Vec::new(),
    };
    render_history(&days)
}

/// Editorial hero + the calendar / day-detail two-column layout.
fn render_history(days: &[birdnet_db::sqlite::DayCount]) -> String {
    let help_link = super::help::help_link(super::help::Topic::Reports);
    let mut html = String::new();
    let _ = write!(
        html,
        r#"<div class="rp-hero"><div class="eyebrow">History {help_link}</div><h1>Browse past days</h1><p class="lead">Every day your station has listened. Darker squares were busier — open one to see its hours and top species.</p></div>"#,
    );

    let Some(latest) = days.last() else {
        html.push_str(
            r#"<p class="hist-empty">No detection history yet. Once your station logs its first day, it'll appear here.</p>"#,
        );
        return html;
    };
    let sel = latest.date.as_str();
    let month = sel.get(0..7).unwrap_or("");

    let _ = write!(
        html,
        r#"<div class="rp-2col">
  <div class="bnb-card pad" id="rp-cal-wrap" hx-get="/pages/history-calendar?month={month}&amp;sel={sel}" hx-trigger="load" hx-swap="innerHTML"><p class="bnb-meta">Loading calendar…</p></div>
  <div class="bnb-card pad" id="rp-daydetail" hx-get="/pages/history-chart?date={sel}" hx-trigger="load" hx-swap="innerHTML"><p class="bnb-meta">Loading…</p></div>
</div>"#,
    );
    html
}

// ---------------------------------------------------------------------------
// GET /pages/history-calendar — the month heat-grid
// ---------------------------------------------------------------------------

async fn history_calendar_partial(
    State(state): State<AppState>,
    Query(params): Query<CalendarParams>,
) -> impl IntoResponse {
    let result =
        tokio::task::spawn_blocking(move || state.with_db(birdnet_db::sqlite::detections_per_day))
            .await;
    let Ok(Ok(days)) = result else {
        return axum::response::Html("<p class='error'>Failed to load calendar.</p>".to_string());
    };
    let sel = params.sel.as_deref().filter(|s| s.len() == 10);
    axum::response::Html(render_calendar(&days, params.month.as_deref(), sel))
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn render_calendar(
    days: &[birdnet_db::sqlite::DayCount],
    month_opt: Option<&str>,
    sel: Option<&str>,
) -> String {
    if days.is_empty() {
        return r#"<p class="hist-empty">No detection history yet.</p>"#.to_string();
    }
    let map: HashMap<&str, (i64, i64)> = days
        .iter()
        .map(|d| (d.date.as_str(), (d.count, d.species)))
        .collect();
    let earliest_month = days[0].date.get(0..7).unwrap_or("");
    let latest_month = days[days.len() - 1].date.get(0..7).unwrap_or("");

    // Resolve + clamp the requested month to the range that has data.
    let mut month = month_opt.filter(|m| m.len() == 7).unwrap_or(latest_month);
    if month < earliest_month {
        month = earliest_month;
    } else if month > latest_month {
        month = latest_month;
    }
    let (year, mon) = parse_month(month);

    // Month-local maximum drives the heat ramp so each month reads on its own.
    let cmax = days
        .iter()
        .filter(|d| d.date.starts_with(month))
        .map(|d| d.count)
        .max()
        .unwrap_or(1)
        .max(1);

    let sel_param = sel.unwrap_or("");
    let mut html = String::new();

    // Header with month navigation.
    let prev_btn = if month > earliest_month {
        format!(
            r#"<a class="rp-cal-nav" href='#' hx-get="/pages/history-calendar?month={m}&amp;sel={sel_param}" hx-target='#rp-cal-wrap' hx-swap="innerHTML" aria-label="Previous month">‹</a>"#,
            m = prev_month(year, mon),
        )
    } else {
        r#"<span class="rp-cal-nav" aria-hidden="true">‹</span>"#.to_string()
    };
    let next_btn = if month < latest_month {
        format!(
            r#"<a class="rp-cal-nav" href='#' hx-get="/pages/history-calendar?month={m}&amp;sel={sel_param}" hx-target='#rp-cal-wrap' hx-swap="innerHTML" aria-label="Next month">›</a>"#,
            m = next_month(year, mon),
        )
    } else {
        r#"<span class="rp-cal-nav" aria-hidden="true">›</span>"#.to_string()
    };
    let month_label = MONTHS
        .get((mon as usize).wrapping_sub(1))
        .copied()
        .unwrap_or("");
    let _ = write!(
        html,
        r#"<div class="rp-cal-head">{prev_btn}<h3>{month_label} {year}</h3>{next_btn}</div>"#,
    );

    // Grid: weekday headers, leading blanks, then the days.
    html.push_str(r#"<div class="rp-cal">"#);
    for dow in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] {
        let _ = write!(html, r#"<div class="dow">{dow}</div>"#);
    }
    let first = format!("{year:04}-{mon:02}-01");
    // Codebase epoch: days%7 == 4 is Monday → shift to a Monday-start offset.
    let lead = usize::try_from((date_to_epoch_days(&first) % 7 + 3) % 7).unwrap_or(0);
    for _ in 0..lead {
        html.push_str(r#"<div class="rp-day empty"></div>"#);
    }
    for day in 1..=days_in_month(year, mon) {
        let date = format!("{year:04}-{mon:02}-{day:02}");
        let (count, species) = map.get(date.as_str()).copied().unwrap_or((0, 0));
        let heat = heat_bg(count, cmax);
        let sel_cls = if sel == Some(date.as_str()) {
            " sel"
        } else {
            ""
        };
        let body = if count > 0 {
            format!(r#"<span class="dc">{count}</span><span class="dsp">{species} sp</span>"#)
        } else {
            String::new()
        };
        let _ = write!(
            html,
            r#"<a class="rp-day{sel_cls}" href='#' hx-get="/pages/history-chart?date={date}" hx-target='#rp-daydetail' hx-swap="innerHTML" data-style="background:{heat}"><span class="dn">{day}</span>{body}</a>"#,
        );
    }
    html.push_str("</div>");

    // Legend.
    html.push_str(r#"<div class="rp-legend">less"#);
    for n in [5_i64, 30, 55, 80, 100] {
        let _ = write!(
            html,
            r#"<span data-style="background:{}"></span>"#,
            heat_bg((cmax * n / 100).max(1), cmax),
        );
    }
    html.push_str("more</div>");
    html
}

/// Map a day's count to an on-brand heat fill (moss over the neutral surface).
///
/// The ramp stops at [`HEAT_MAX_PCT`] rather than at full `--moss`, because the
/// text sitting on these cells has a fixed colour while the fill moves with the
/// data. At full strength the cell is `#508357` and the day's label reads
/// 3.71:1 against it — the busiest days in the month were the least legible,
/// which is exactly backwards. Measured against the light palette:
///
/// | mix | cell      | `--fg` on it |
/// |-----|-----------|--------------|
/// | 0%  | `#f8f6f4` | 16.07:1      |
/// | 50% | `#b0b694` | 8.22:1       |
/// | 80% | `#779569` | 5.19:1       |
/// | 100%| `#508357` | 3.71:1 ✗     |
///
/// Capping at 80% keeps every cell above the 4.5:1 the label needs while
/// leaving the ramp visibly graded. The alternative — flipping the ink to white
/// past a threshold — was rejected because white only reaches 4.5:1 at the very
/// top of the ramp (4.67:1 at 100%), so there is no crossover where both inks
/// are legible and the switch would be visible as a hard band.
const HEAT_MAX_PCT: i64 = 80;

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn heat_bg(count: i64, cmax: i64) -> String {
    if count <= 0 {
        return "var(--surface-2)".to_string();
    }
    let span = f64::from(u32::try_from(HEAT_MAX_PCT - 8).unwrap_or(72));
    let pct = ((count as f64 / cmax as f64) * span).round() as i64 + 8;
    format!("color-mix(in oklch, var(--moss) {pct}%, var(--surface-2))")
}

// ---------------------------------------------------------------------------
// GET /pages/history-chart — a single day's detail panel
// ---------------------------------------------------------------------------

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
        Ok(Ok((hours, total, species))) => render_chart_content(&date2, total, &species, &hours),
        _ => "<p class='error'>Failed to load chart data.</p>".to_string(),
    };

    axum::response::Html(html)
}

/// Full page: the "Open day" landing — a read-only recap of one past day,
/// reached from the History day-detail panel's "Open day →" link. Shows the
/// day's hourly shape, the species heard, and the *complete* detection log
/// (the inline panel only has room for the top species).
async fn day_page(
    State(state): State<AppState>,
    Query(params): Query<HistoryParams>,
    headers: HeaderMap,
) -> Html<String> {
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
            let rows = birdnet_db::sqlite::detections_by_date(conn, &date)?;
            Ok::<_, birdnet_db::sqlite::DbError>((hours, total, species, rows))
        })
    })
    .await;

    let content = match result {
        Ok(Ok((hours, total, species, rows))) => {
            render_day_page(&date2, total, &species, &hours, &rows)
        }
        _ => r#"<a class="action" href="/reports?tab=history">‹ Back to history</a>
<div class="bnb-card pad"><p class="error">Failed to load this day.</p></div>"#
            .to_string(),
    };
    let title = format!("History · {date2}");
    super::render_page_for_request(&title, &content, "reports", &headers)
}

/// HTMX partial: a flat list of dates with detections (legacy date browser).
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

/// The day-detail panel: header, hourly bars, and that day's top species.
fn render_chart_content(
    date: &str,
    total: i64,
    species: &[(String, String, i64)],
    hours: &[birdnet_db::sqlite::HourlyCount],
) -> String {
    let mut html = String::new();
    // A day with detections gets an "Open day →" link to its full-page recap
    // (the complete log won't fit this panel); an empty day has nothing to open.
    let open_day = if total > 0 {
        format!(
            r#"<a class="action" href="/reports/day?date={d}">Open day →</a>"#,
            d = simple_url_encode(date),
        )
    } else {
        String::new()
    };
    let _ = write!(
        html,
        r#"<div class="section-header"><div><div class="bnb-eyebrow">{date} · {weekday}</div><h3>{total} detections · {n} species</h3></div>{open_day}</div>"#,
        date = escape_html(date),
        weekday = weekday_name(date),
        n = species.len(),
    );
    let _ = write!(
        html,
        r#"<div class="rp-viz">{chart}</div>"#,
        chart = render_hourly_chart(hours),
    );
    html.push_str(r#"<div class="rp-h3">Top species that day</div>"#);
    if species.is_empty() {
        html.push_str(r#"<p class="bnb-meta">No detections on this day.</p>"#);
    } else {
        for (i, (com, _sci, count)) in species.iter().take(6).enumerate() {
            let _ = write!(
                html,
                r#"<div class="rp-row"><span class="rk">{rank}</span>{av}<div class="nm">{name}</div><span class="ct">{count}</span></div>"#,
                rank = i + 1,
                av = avatar(com, ""),
                name = escape_html(com),
            );
        }
    }
    html
}

/// The "Open day" full-page recap: a back link, the day's hourly shape, every
/// species heard that day, and the complete chronological detection log.
/// Read-only — managing detections stays on Today / Recordings.
fn render_day_page(
    date: &str,
    total: i64,
    species: &[(String, String, i64)],
    hours: &[birdnet_db::sqlite::HourlyCount],
    rows: &[birdnet_db::sqlite::DetectionRow],
) -> String {
    let mut html = String::new();
    let _ = write!(
        html,
        r#"<a class="action" href="/reports?tab=history">‹ Back to history</a>
<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">{weekday}</div><h3>{date}</h3><div class="bnb-meta">{total} detections · {n} species</div></div></div>"#,
        weekday = weekday_name(date),
        date = escape_html(date),
        n = species.len(),
    );

    if total == 0 {
        html.push_str(r#"<p class="bnb-meta">No detections were recorded on this day.</p></div>"#);
        return html;
    }

    let _ = write!(
        html,
        r#"<div class="rp-viz">{chart}</div></div>"#,
        chart = render_hourly_chart(hours),
    );

    // Every species heard that day (the panel only shows the top six).
    html.push_str(r#"<div class="bnb-card pad"><div class="rp-h3">Species heard</div>"#);
    for (i, (com, _sci, count)) in species.iter().enumerate() {
        let _ = write!(
            html,
            r#"<div class="rp-row"><span class="rk">{rank}</span>{av}<a class="nm" href="/species/detail?name={enc}">{name}</a><span class="ct">{count}</span></div>"#,
            rank = i + 1,
            av = avatar(com, ""),
            enc = simple_url_encode(com),
            name = escape_html(com),
        );
    }
    html.push_str("</div>");

    // The complete log for the day, newest first.
    html.push_str(
        r#"<div class="bnb-card pad"><div class="rp-h3">Every detection · newest first</div>"#,
    );
    for d in rows {
        render_day_log_row(&mut html, d);
    }
    html.push_str("</div>");
    html
}

/// One read-only row of the Open-day log: avatar · species (→ detail) ·
/// confidence · scientific name · the time (→ that detection's detail). The
/// `tdl-card` treatment matches the Today log; the lock/delete/play affordances
/// are dropped — a past day is for reading, not managing.
fn render_day_log_row(html: &mut String, d: &birdnet_db::sqlite::DetectionRow) {
    let enc_name = simple_url_encode(&d.com_name);
    let _ = write!(
        html,
        "<div class=\"bnb-card tdl-card\">\
         {av}\
         <div class=\"tdl-card-main\">\
         <div class=\"tdl-card-head\">\
         <a href=\"/species/detail?name={enc_name}\" class=\"tdl-name\">{com}</a>\
         {conf}\
         </div>\
         <div class=\"bnb-meta mono tdl-card-sci\">{sci} · \
         <a href=\"/detections/detail?date={date_enc}&time={time_enc}&name={enc_name}\" class=\"tdl-time\">{time}</a></div>\
         </div></div>",
        av = avatar(&d.com_name, ""),
        conf = conf_bar(d.confidence),
        com = escape_html(&d.com_name),
        sci = escape_html(&d.sci_name),
        time = escape_html(&d.time),
        date_enc = simple_url_encode(&d.date),
        time_enc = simple_url_encode(&d.time),
    );
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
            r#"<li><a href='#' hx-get='/pages/history-chart?date={date}' hx-target='#rp-daydetail' hx-swap='innerHTML'
               class="hist-date-link">{date}</a></li>"#,
            date = escape_html(date),
        );
    }

    html.push_str("</ul>");
    html
}

// ---------------------------------------------------------------------------
// Date helpers (no external crate)
// ---------------------------------------------------------------------------

/// Weekday label for a YYYY-MM-DD date, using the codebase's epoch convention
/// (`days % 7 == 0` is Thursday).
fn weekday_name(date: &str) -> &'static str {
    const NAMES: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let idx = usize::try_from(date_to_epoch_days(date) % 7).unwrap_or(0);
    NAMES.get(idx).copied().unwrap_or("")
}

/// Number of days in a given month (Gregorian, leap-year aware).
const fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        // 4, 6, 9, 11 — and any out-of-range month — fall through to 30.
        _ => 30,
    }
}

/// Parse a `YYYY-MM` month string into `(year, month)`.
fn parse_month(m: &str) -> (i64, u32) {
    let year = m.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(2026);
    let mon = m.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(1);
    (year, mon)
}

/// The `YYYY-MM` month before `(year, mon)`.
fn prev_month(year: i64, mon: u32) -> String {
    if mon <= 1 {
        format!("{:04}-12", year - 1)
    } else {
        format!("{year:04}-{:02}", mon - 1)
    }
}

/// The `YYYY-MM` month after `(year, mon)`.
fn next_month(year: i64, mon: u32) -> String {
    if mon >= 12 {
        format!("{:04}-01", year + 1)
    } else {
        format!("{year:04}-{:02}", mon + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_month_splits() {
        assert_eq!(parse_month("2026-06"), (2026, 6));
        assert_eq!(parse_month("1999-12"), (1999, 12));
    }

    #[test]
    fn month_neighbours_wrap_years() {
        assert_eq!(prev_month(2026, 1), "2025-12");
        assert_eq!(next_month(2026, 12), "2027-01");
        assert_eq!(prev_month(2026, 6), "2026-05");
        assert_eq!(next_month(2026, 6), "2026-07");
    }

    #[test]
    fn days_in_month_leap() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(days_in_month(2026, 1), 31);
    }

    #[test]
    fn heat_bg_zero_is_surface() {
        assert_eq!(heat_bg(0, 100), "var(--surface-2)");
        assert!(heat_bg(50, 100).contains("color-mix"));
        assert!(heat_bg(100, 100).contains("var(--moss)"));
    }

    #[test]
    fn render_calendar_renders_grid_and_selection() {
        let days = vec![
            birdnet_db::sqlite::DayCount {
                date: "2026-06-10".into(),
                count: 100,
                species: 12,
            },
            birdnet_db::sqlite::DayCount {
                date: "2026-06-14".into(),
                count: 250,
                species: 20,
            },
        ];
        let html = render_calendar(&days, Some("2026-06"), Some("2026-06-14"));
        assert!(html.contains("June 2026"));
        assert!(html.contains("rp-cal"));
        // The selected day carries the sel class and the busiest day its count.
        assert!(html.contains("rp-day sel"));
        assert!(html.contains("250"));
    }

    #[test]
    fn render_chart_content_no_data() {
        let html = render_chart_content("2026-03-14", 0, &[], &[]);
        assert!(html.contains("2026-03-14"));
        assert!(html.contains("No detections on this day"));
    }

    #[test]
    fn render_date_list_empty() {
        let html = render_date_list(&[]);
        assert!(html.contains("No detection history"));
    }

    fn row(com: &str, sci: &str, time: &str, conf: f64) -> birdnet_db::sqlite::DetectionRow {
        birdnet_db::sqlite::DetectionRow {
            date: "2026-06-14".into(),
            time: time.into(),
            sci_name: sci.into(),
            com_name: com.into(),
            confidence: conf,
            lat: None,
            lon: None,
            cutoff: None,
            week: None,
            sens: None,
            overlap: None,
            file_name: None,
            correlation_id: None,
            source: None,
            duration_secs: None,
        }
    }

    #[test]
    fn open_day_link_present_only_with_detections() {
        let species = vec![("Robin".to_string(), "Erithacus".to_string(), 3_i64)];
        let hours = vec![birdnet_db::sqlite::HourlyCount {
            hour: "06".into(),
            count: 3,
        }];
        let with = render_chart_content("2026-06-14", 3, &species, &hours);
        assert!(with.contains("Open day"));
        assert!(with.contains("/reports/day?date="));
        // An empty day has nothing to open.
        let without = render_chart_content("2026-06-14", 0, &[], &[]);
        assert!(!without.contains("Open day"));
    }

    #[test]
    fn render_day_page_shows_recap_and_full_log() {
        let species = vec![
            (
                "European Robin".to_string(),
                "Erithacus rubecula".to_string(),
                5_i64,
            ),
            (
                "Blue Tit".to_string(),
                "Cyanistes caeruleus".to_string(),
                2_i64,
            ),
        ];
        let hours = vec![birdnet_db::sqlite::HourlyCount {
            hour: "06".into(),
            count: 5,
        }];
        let rows = vec![
            row("European Robin", "Erithacus rubecula", "06:12:00", 0.88),
            row("Blue Tit", "Cyanistes caeruleus", "05:40:00", 0.71),
        ];
        let html = render_day_page("2026-06-14", 7, &species, &hours, &rows);
        // Back link + header with the day's totals.
        assert!(html.contains(r#"href="/reports?tab=history""#));
        assert!(html.contains("2026-06-14"));
        assert!(html.contains("7 detections · 2 species"));
        // Species link out to their detail pages.
        assert!(html.contains(r#"<a class="nm" href="/species/detail?name="#));
        // The full log uses the tdl-card treatment with per-detection detail links…
        assert!(html.contains("tdl-card"));
        assert!(html.contains("/detections/detail?date="));
        assert!(html.contains("European Robin") && html.contains("Blue Tit"));
        // …and is read-only: no lock/delete actions leak through.
        assert!(!html.contains("today-delete"));
        assert!(!html.contains("today-lock"));
    }

    #[test]
    fn render_day_page_empty_day_has_no_log() {
        let html = render_day_page("2026-06-14", 0, &[], &[], &[]);
        assert!(html.contains("No detections were recorded"));
        assert!(html.contains("Back to history"));
        assert!(!html.contains("tdl-card"));
    }
}
