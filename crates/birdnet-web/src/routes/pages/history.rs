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
    /// The day page's page of its log, from 1 (newest first). Anything else is page 1.
    pub page: Option<String>,
}

/// Rows of a day's log on one page of the day page. A busy dawn on a station with several
/// sources is thousands of rows, which the page drew in one response (UIC-21).
const DAY_PAGE_ROWS: u32 = 200;

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
    // Only the last day is needed here (it seeds the calendar's month and the day panel), and
    // the per-day counts of every day ever recorded were read to learn it (UIC-21).
    let days = tokio::task::spawn_blocking(move || {
        state.with_read_db(birdnet_db::sqlite::detection_date_bounds)
    })
    .await;
    // A failed read is not an empty history. Collapsing both into `Vec::new()`
    // made `render_history` tell a station with three years of data that it had
    // never recorded anything — the same file already gets this right for the
    // calendar partial below ("Failed to load calendar.").
    let days = match days {
        Ok(Ok(d)) => d,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "history: detection_date_bounds failed");
            return history_error();
        }
        Err(e) => {
            tracing::warn!(error = %e, "history: detection_date_bounds task failed");
            return history_error();
        }
    };
    render_history(days.as_ref().map(|(_, last)| last.as_str()))
}

/// The hero, then an honest failure card in place of the calendar.
///
/// Keeps the page's own heading so the operator still knows where they are.
fn history_error() -> String {
    let help_link = super::help::help_link(super::help::Topic::Reports);
    format!(
        "{}{}",
        super::frame::page_head(&super::frame::Head {
            eyebrow: "Reports · history",
            help: &help_link,
            title: "Browse past <em>days</em>",
            lead: "",
            lede: "",
            aside: "",
        }),
        super::error_states::could_not_load("your detection history"),
    )
}

/// Editorial hero + the calendar / day-detail two-column layout.
fn render_history(latest: Option<&str>) -> String {
    let help_link = super::help::help_link(super::help::Topic::Reports);
    let mut html = String::new();
    let _ = write!(
        html,
        "{}",
        super::frame::page_head(&super::frame::Head {
            eyebrow: "Reports · history",
            help: &help_link,
            title: "Browse past <em>days</em>",
            lead: "Every day your station has listened.",
            lede: "A stronger colour means a busier day — open one to see its hours and top species.",
            aside: "",
        }),
    );

    let Some(latest) = latest else {
        html.push_str(
            r#"<p class="hist-empty">No detection history yet. Once your station logs its first day, it'll appear here.</p>"#,
        );
        return html;
    };
    let sel = latest;
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
    let month_param = params.month.clone();
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            // The month asked for, clamped to the history's two ends, and that month's counts
            // only: every month click grouped the whole table to draw thirty days (UIC-21).
            let Some(bounds) = birdnet_db::sqlite::detection_date_bounds(conn)? else {
                return Ok((Vec::new(), None, None));
            };
            let span = MonthSpan::of(&bounds.0, &bounds.1);
            let month = span.clamp(month_param.as_deref());
            let (year, mon) = parse_month(&month);
            let days = birdnet_db::sqlite::detections_per_day_between(
                conn,
                &format!("{month}-01"),
                &format!("{month}-{:02}", days_in_month(year, mon)),
            )?;
            // The ledger only adds a statement ("off"); a failed read of it leaves the
            // calendar without that statement, never without the calendar.
            let ledger = super::listening::StationLedger::read(conn).ok();
            Ok::<_, birdnet_db::sqlite::DbError>((days, ledger, Some((span, month))))
        })
    })
    .await;
    let Ok(Ok((days, ledger, span))) = result else {
        // These four partials all answer 200, so the layout's
        // `htmx:responseError` fallback never runs and whatever is written
        // here is the reader's last word. `content()` in this same file
        // already routes its failure through the shared error state; these
        // did not, and said only that something had failed.
        return axum::response::Html(super::error_states::inline("this month's calendar"));
    };
    // `sel` is written back into the month buttons' links. "Ten characters long" is not a date:
    // `"><zq-p>12` is ten characters, and a station whose history spans two months renders those
    // buttons. Only a `YYYY-MM-DD` gets through.
    let sel = params
        .sel
        .as_deref()
        .filter(|s| crate::routes::is_valid_date(s));
    let Some((span, month)) = span else {
        return axum::response::Html(
            r#"<p class="hist-empty">No detection history yet.</p>"#.to_string(),
        );
    };
    let today = today_date_string();
    axum::response::Html(render_month(
        &days,
        &span,
        &month,
        sel,
        &DayStates {
            ledger: ledger.as_ref(),
            today: &today,
        },
    ))
}

/// The first and the last month with history (`YYYY-MM`): what the calendar's month buttons
/// may reach.
struct MonthSpan {
    earliest: String,
    latest: String,
}

impl MonthSpan {
    /// The span of two `YYYY-MM-DD` dates.
    fn of(first: &str, last: &str) -> Self {
        Self {
            earliest: first.get(0..7).unwrap_or("").to_owned(),
            latest: last.get(0..7).unwrap_or("").to_owned(),
        }
    }

    /// The month asked for, or the latest when none (or not a `YYYY-MM`) was, held inside the
    /// span.
    fn clamp(&self, asked: Option<&str>) -> String {
        let month = asked
            .filter(|m| m.len() == 7 && m.as_bytes()[4] == b'-')
            .unwrap_or(&self.latest);
        if month < self.earliest.as_str() {
            self.earliest.clone()
        } else if month > self.latest.as_str() {
            self.latest.clone()
        } else {
            month.to_owned()
        }
    }
}

/// What the calendar needs, beyond the counts, to tell a day's three empty states apart
/// (UIR-09; `docs/design/charts.md` honesty rule 3): the effort ledger, for "off", and today,
/// for "not yet".
struct DayStates<'a> {
    /// The station's ledger; `None` when it could not be read or the station has none, and
    /// then no day is called off.
    ledger: Option<&'a super::listening::StationLedger>,
    /// The local date today, `YYYY-MM-DD`.
    today: &'a str,
}

/// The calendar as the tests before UIR-09 drew it: no ledger, and no day to come.
#[cfg(test)]
fn render_calendar(
    days: &[birdnet_db::sqlite::DayCount],
    month_opt: Option<&str>,
    sel: Option<&str>,
) -> String {
    let (Some(first), Some(last)) = (days.first(), days.last()) else {
        return r#"<p class="hist-empty">No detection history yet.</p>"#.to_string();
    };
    let span = MonthSpan::of(&first.date, &last.date);
    let month = span.clamp(month_opt);
    render_month(
        days,
        &span,
        &month,
        sel,
        &DayStates {
            ledger: None,
            today: "9999-12-31",
        },
    )
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn render_month(
    days: &[birdnet_db::sqlite::DayCount],
    span: &MonthSpan,
    month: &str,
    sel: Option<&str>,
    states: &DayStates<'_>,
) -> String {
    let map: HashMap<&str, (i64, i64)> = days
        .iter()
        .map(|d| (d.date.as_str(), (d.count, d.species)))
        .collect();
    let earliest_month = span.earliest.as_str();
    let latest_month = span.latest.as_str();
    let (year, mon) = parse_month(month);

    // Month-local maximum drives the heat ramp so each month reads on its own.
    let cmax = days
        .iter()
        .filter(|d| d.date.starts_with(month))
        .map(|d| d.count)
        .max()
        .unwrap_or(1)
        .max(1);

    // Encoded where it is written, not only checked where it arrives: this is a function other
    // callers and the tests reach without the handler's check.
    let sel_param = simple_url_encode(sel.unwrap_or(""));
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
        r#"<div class="rp-cal-head">{prev_btn}<h2 class="rp-cal-h">{month_label} {year}</h2>{next_btn}</div>"#,
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
    let (mut any_off, mut any_future) = (false, false);
    for day in 1..=days_in_month(year, mon) {
        let date = format!("{year:04}-{mon:02}-{day:02}");
        let (count, species) = map.get(date.as_str()).copied().unwrap_or((0, 0));
        // UIR-09: three empty cells that are three different facts. A day to come is not a
        // link (there is nothing to open) and is outlined; a day the ledger says the station
        // did not listen to is hatched and says "off"; a listened day with nothing heard is
        // the plain cell. Today is never off: the recorder credits on a timer.
        let month_sr = format!(r#"<span class="sr-only"> {month_label},</span>"#);
        if date.as_str() > states.today {
            any_future = true;
            let _ = write!(
                html,
                r#"<div class="rp-day future"><span class="dn">{day}{month_sr}</span><span class="sr-only"> not yet</span></div>"#,
            );
            continue;
        }
        let off = count == 0
            && date.as_str() != states.today
            && states
                .ledger
                .and_then(|l| l.listened(&date))
                .is_some_and(|listened| !listened);
        if off {
            any_off = true;
            let current = if sel == Some(date.as_str()) {
                r#" aria-current="date""#
            } else {
                ""
            };
            let _ = write!(
                html,
                r#"<a class="rp-day off" href="/reports/day?date={date}" hx-get="/pages/history-chart?date={date}" hx-target='#rp-daydetail' hx-swap="innerHTML"{current}><span class="dn">{day}{month_sr}</span><span class="doff" aria-hidden="true">off</span><span class="sr-only"> station not listening</span></a>"#,
            );
            continue;
        }
        let heat = heat_bg(count, cmax);
        let sel_cls = if sel == Some(date.as_str()) {
            " sel"
        } else {
            ""
        };
        // A real link to the day's page, which htmx enhances by loading the day beside the
        // calendar; and the cell says in words what its colour says (UIC-28). The words are its
        // own text, with what the eye takes from the grid ("May", "detections") written for a
        // screen reader only, so the visible label is part of its name (WCAG 2.5.3): an
        // `aria-label` replaced the visible "23 sp" with "23 species" and axe reported every cell.
        let body = if count > 0 {
            format!(
                r#"<span class="dc"> {count}<span class="sr-only"> detection{s},</span></span><span class="dsp"> {species} sp<span class="sr-only">ecies</span></span>"#,
                count = super::group_thousands(count),
                s = if count == 1 { "" } else { "s" },
            )
        } else {
            r#"<span class="sr-only"> no detections</span>"#.to_string()
        };
        let current = if sel_cls.is_empty() {
            ""
        } else {
            r#" aria-current="date""#
        };
        let _ = write!(
            html,
            r#"<a class="rp-day{sel_cls}" href="/reports/day?date={date}" hx-get="/pages/history-chart?date={date}" hx-target='#rp-daydetail' hx-swap="innerHTML"{current} data-style="background:{heat}"><span class="dn">{day}{month_sr}</span>{body}</a>"#,
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
    html.push_str("more");
    if any_off {
        html.push_str(r#"<span class="rp-key"><span class="rp-legend-off" aria-hidden="true"></span>station not listening</span>"#);
    }
    if any_future {
        html.push_str(r#"<span class="rp-key"><span class="rp-legend-future" aria-hidden="true"></span>not yet</span>"#);
    }
    html.push_str("</div>");
    html
}

/// Map a day's count to an on-brand heat fill (moss over the neutral surface), in one of the
/// kit's seven square-root steps (`viz::kit::seq_step`), the scale every other heat map uses.
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
    // The kit's seven steps, by the square root of the count (`viz::kit::seq_step`), so one
    // detection beside two hundred is a step from none and ten a step from one; on the linear mix
    // this used to be (UIC-18) one detection beside two hundred was 8 % of moss and ten was 12 %.
    let step =
        i64::try_from(super::viz::kit::seq_step(count as f64 / cmax.max(1) as f64)).unwrap_or(7);
    let pct = 8 + (step - 1) * (HEAT_MAX_PCT - 8) / 6;
    format!("color-mix(in oklab, var(--moss) {pct}%, var(--surface-2))")
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
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            let hours = birdnet_db::sqlite::hourly_activity(conn, &date)?;
            let total = birdnet_db::sqlite::detection_count_for_date(conn, &date)?;
            let species = birdnet_db::sqlite::species_for_date(conn, &date)?;
            Ok::<_, birdnet_db::sqlite::DbError>((hours, total, species))
        })
    })
    .await;

    let html = match result {
        Ok(Ok((hours, total, species))) => {
            render_chart_content(&date2, total, &species, &hours, &names)
        }
        _ => super::error_states::inline("this day's hourly chart"),
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

    let page = params
        .page
        .as_deref()
        .and_then(|p| p.parse::<u32>().ok())
        .filter(|p| *p >= 1)
        .unwrap_or(1);
    let date2 = date.clone();
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            let hours = birdnet_db::sqlite::hourly_activity(conn, &date)?;
            let total = birdnet_db::sqlite::detection_count_for_date(conn, &date)?;
            let species = birdnet_db::sqlite::species_for_date(conn, &date)?;
            let rows = birdnet_db::sqlite::detections_by_date_page(
                conn,
                &date,
                DAY_PAGE_ROWS,
                (page - 1).saturating_mul(DAY_PAGE_ROWS),
            )?;
            let floor = super::confidence::station_floor(conn);
            Ok::<_, birdnet_db::sqlite::DbError>((hours, total, species, rows, floor))
        })
    })
    .await;

    let content = match result {
        Ok(Ok((hours, total, species, rows, floor))) => {
            let log = DayLog {
                rows: &rows,
                page,
                total,
            };
            render_day_page(&date2, &species, &hours, &log, &names, floor)
        }
        _ => format!(
            r#"<a class="action" href="/reports?tab=history">‹ Back to history</a>{}"#,
            super::error_states::could_not_load("this day")
        ),
    };
    let title = format!(
        "History · {}",
        crate::display::date_text(&date2).unwrap_or_else(|| date2.clone())
    );
    super::render_page_for_request(&title, &content, "reports", &headers)
}

/// HTMX partial: a flat list of dates with detections (legacy date browser).
async fn history_dates_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(birdnet_db::sqlite::distinct_detection_dates)
    })
    .await;

    let html = match result {
        Ok(Ok(dates)) => render_date_list(&dates),
        _ => super::error_states::inline("the list of days"),
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
    names: &birdnet_core::i18n::NameLocalizer,
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
        r#"<div class="section-header"><div><div class="bnb-eyebrow">{date}</div><h2 class="sh-h">{total} detections · {n} species</h2></div>{open_day}</div>"#,
        date = crate::display::date(date),
        n = species.len(),
    );
    let _ = write!(
        html,
        r#"<div class="rp-viz">{chart}</div>"#,
        chart = render_hourly_chart("history-hourly", hours),
    );
    html.push_str(r#"<div class="rp-h3">Top species that day</div>"#);
    if species.is_empty() {
        html.push_str(r#"<p class="bnb-meta">No detections on this day.</p>"#);
    } else {
        for (i, (com, sci, count)) in species.iter().take(6).enumerate() {
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
    html
}

/// The "Open day" full-page recap: a back link, the day's hourly shape, every
/// species heard that day, and the complete chronological detection log.
/// Read-only — managing detections stays on Today / Recordings.
fn render_day_page(
    date: &str,
    species: &[(String, String, i64)],
    hours: &[birdnet_db::sqlite::HourlyCount],
    log: &DayLog<'_>,
    names: &birdnet_core::i18n::NameLocalizer,
    floor: f64,
) -> String {
    let total = log.total;
    let mut html = String::new();
    // The shared head (UIR-27): the day's heading was an `sh-h` inside its first card.
    let head = super::frame::page_head(&super::frame::Head {
        eyebrow: &format!("Reports · {}", weekday_name(date)),
        help: &super::help::help_link(super::help::Topic::Reports),
        title: &crate::display::date(date),
        lead: "",
        lede: &format!(
            "{total} detections · {n} species. <a href=\"/reports?tab=history\">‹ Back to history</a>",
            n = species.len()
        ),
        aside: "",
    });
    let _ = write!(html, r#"{head}<div class="bnb-card pad">"#);

    if total == 0 {
        html.push_str(r#"<p class="bnb-meta">No detections were recorded on this day.</p></div>"#);
        return html;
    }

    let _ = write!(
        html,
        r#"<div class="rp-viz">{chart}</div></div>"#,
        chart = render_hourly_chart("history-day-hourly", hours),
    );

    // Every species heard that day (the panel only shows the top six).
    html.push_str(r#"<div class="bnb-card pad"><div class="rp-h3">Species heard</div>"#);
    for (i, (com, sci, count)) in species.iter().enumerate() {
        let shown = names.name(sci, com);
        let _ = write!(
            html,
            r#"<div class="rp-row"><span class="rk">{rank}</span>{av}<a class="nm" href="/species/detail?name={enc}">{name}</a><span class="ct">{count}</span></div>"#,
            rank = i + 1,
            av = avatar(com, shown, sci, ""),
            enc = simple_url_encode(com),
            name = escape_html(shown),
        );
    }
    html.push_str("</div>");

    // The day's log, newest first, a page at a time.
    let pager = log.pager(date);
    let _ = write!(
        html,
        r#"<div class="bnb-card pad" id="day-log"><div class="rp-h3">Every detection · newest first</div>{range}"#,
        range = log.range_line(),
    );
    html.push_str(&super::frame::counted(""));
    html.push_str(&super::confidence::legend(floor));
    for d in log.rows {
        render_day_log_row(&mut html, d, names, floor);
    }
    html.push_str(&pager);
    html.push_str("</div>");
    html
}

/// One page of a day's log, and where it sits in the day.
struct DayLog<'a> {
    /// This page's rows, newest first.
    rows: &'a [birdnet_db::sqlite::DetectionRow],
    /// The page, from 1.
    page: u32,
    /// Every detection of the day.
    total: i64,
}

impl DayLog<'_> {
    /// How many pages the day has (at least one).
    fn pages(&self) -> u32 {
        let total = u32::try_from(self.total.max(0)).unwrap_or(u32::MAX);
        total.div_ceil(DAY_PAGE_ROWS).max(1)
    }

    /// "Detections 201–400 of 450", only when the day has more than one page.
    fn range_line(&self) -> String {
        if self.pages() <= 1 {
            return String::new();
        }
        let first = (self.page - 1).saturating_mul(DAY_PAGE_ROWS) + 1;
        let shown = u32::try_from(self.rows.len()).unwrap_or(0);
        if shown == 0 {
            return format!(
                r#"<p class="bnb-meta">No detections on this page; the day has {} pages.</p>"#,
                self.pages()
            );
        }
        format!(
            r#"<p class="bnb-meta">Detections {first}–{last} of {total}, page {page} of {pages}.</p>"#,
            last = first + shown - 1,
            total = super::group_thousands(self.total),
            page = self.page,
            pages = self.pages(),
        )
    }

    /// Newer and older links: real links, which work without a script.
    fn pager(&self, date: &str) -> String {
        if self.pages() <= 1 {
            return String::new();
        }
        let d = simple_url_encode(date);
        let mut out =
            String::from(r#"<nav class="rp-pager" aria-label="Pages of this day's log">"#);
        if self.page > 1 {
            let _ = write!(
                out,
                r#"<a class="bnb-btn ghost" href="/reports/day?date={d}&amp;page={p}#day-log">‹ Newer</a> "#,
                p = self.page - 1,
            );
        }
        if self.page < self.pages() {
            let _ = write!(
                out,
                r#"<a class="bnb-btn ghost" href="/reports/day?date={d}&amp;page={p}#day-log">Older ›</a>"#,
                p = self.page + 1,
            );
        }
        out.push_str("</nav>");
        out
    }
}

/// One read-only row of the Open-day log: avatar · species (→ detail) ·
/// confidence · scientific name · the time (→ that detection's detail). The
/// `tdl-card` treatment matches the Today log; the lock/delete/play affordances
/// are dropped — a past day is for reading, not managing.
fn render_day_log_row(
    html: &mut String,
    d: &birdnet_db::sqlite::DetectionRow,
    names: &birdnet_core::i18n::NameLocalizer,
    floor: f64,
) {
    let enc_name = simple_url_encode(&d.com_name);
    let shown = names.name(&d.sci_name, &d.com_name);
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
         <a href=\"/detections/detail?date={date_enc}&time={time_enc}&name={enc_name}&file={file_enc}\" class=\"tdl-time\">{time}</a></div>\
         </div></div>",
        av = avatar(&d.com_name, shown, &d.sci_name, ""),
        conf = conf_bar(d.confidence, floor),
        com = escape_html(shown),
        sci = escape_html(&d.sci_name),
        time = crate::display::clock(&d.time),
        date_enc = simple_url_encode(&d.date),
        time_enc = simple_url_encode(&d.time),
        file_enc = simple_url_encode(d.file_name.as_deref().unwrap_or_default()),
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
///
/// Delegates to `birdnet_core::civil`, which is where the workspace's civil-date
/// arithmetic lives. This used to write the leap-year predicate out by hand,
/// which is one more place for a transcription slip in code that decides how
/// many cells a calendar has. The `i64` year is clamped rather than cast
/// blindly: a negative year is a broken clock, and the calendar it would draw
/// is meaningless either way.
fn days_in_month(year: i64, month: u32) -> u32 {
    let year = u32::try_from(year.max(0)).unwrap_or(u32::MAX);
    birdnet_core::civil::days_in_month(year, month)
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

    /// The month's heat is the kit's square-root scale, in its seven steps (UIC-18). It was a
    /// linear mix: one detection beside a busiest day of two hundred was 8 % of moss, ten was
    /// 12 %, and four points of a colour mix is not a difference anyone sees. Seen failing on the
    /// linear mix: "ten detections are a step from one beside 200: 8% and 12%".
    #[test]
    fn the_month_uses_the_kits_square_root_steps() {
        let mix = |c: i64| -> i64 {
            let s = heat_bg(c, 200);
            s.split("var(--moss) ")
                .nth(1)
                .and_then(|t| t.split('%').next())
                .and_then(|t| t.parse().ok())
                .unwrap_or_else(|| panic!("{s}"))
        };
        assert!(
            mix(10) - mix(1) >= 12,
            "ten detections are a step from one beside 200: {}% and {}%",
            mix(1),
            mix(10)
        );
        let steps: std::collections::BTreeSet<i64> = (1..=200).map(mix).collect();
        assert_eq!(steps.len(), 7, "the kit's seven steps: {steps:?}");
        assert_eq!(steps.first(), Some(&8));
        assert_eq!(steps.last(), Some(&HEAT_MAX_PCT));
        // Counterpart: none is still the empty cell, and more is never paler.
        assert_eq!(heat_bg(0, 200), "var(--surface-2)");
        assert!((1..200).all(|c| mix(c) <= mix(c + 1)));
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

    /// Two months of history, so the calendar draws the month buttons that carry `sel`. Fixed
    /// dates, not the clock: the sweep in `tests/a_query_value_is_never_echoed_as_markup.rs` seeded
    /// today and yesterday, which are in two months on the 1st only, so it was blind to this on
    /// every other day of the month.
    fn two_months() -> Vec<birdnet_db::sqlite::DayCount> {
        ["2026-10-31", "2026-11-01"]
            .iter()
            .map(|d| birdnet_db::sqlite::DayCount {
                date: (*d).into(),
                count: 3,
                species: 2,
            })
            .collect()
    }

    /// The probe the sweep uses is ten characters long, which is all the handler used to ask of
    /// `sel`. In the newest month the previous-month button carries it; in the oldest, the next.
    #[test]
    fn a_selected_day_is_never_put_into_the_month_buttons_as_markup() {
        let probe = "\"><zq-p>12";
        for month in ["2026-11", "2026-10"] {
            let html = render_calendar(&two_months(), Some(month), Some(probe));
            assert!(
                html.contains("hx-get=\"/pages/history-calendar?month="),
                "{month}: the page has a month button, or this tests nothing: {html}"
            );
            assert!(!html.contains("<zq-p>"), "{month}: a tag came back: {html}");
            assert!(
                !html.contains(probe),
                "{month}: the probe came back raw: {html}"
            );
            assert!(html.contains("sel=%22%3E%3Czq-p%3E12"), "{month}: {html}");
        }
    }

    /// The counterpart: a real date still goes into the buttons, unchanged, and the cell it names
    /// still carries `sel`; and a month with no buttons has nothing to carry it.
    #[test]
    fn a_real_selected_day_still_travels_in_the_month_buttons() {
        let html = render_calendar(&two_months(), Some("2026-11"), Some("2026-10-31"));
        assert!(html.contains("month=2026-10&amp;sel=2026-10-31"), "{html}");
        let html = render_calendar(&two_months(), Some("2026-10"), Some("2026-10-31"));
        assert!(html.contains("month=2026-11&amp;sel=2026-10-31"), "{html}");
        assert!(html.contains("rp-day sel"), "{html}");
        let one_month: Vec<_> = two_months().into_iter().take(1).collect();
        let html = render_calendar(&one_month, None, Some("2026-10-31"));
        assert!(
            !html.contains("sel=2026"),
            "one month has no month buttons: {html}"
        );
    }

    /// UIC-28. Every day cell is a real link to that day's page, which htmx enhances by loading
    /// the day beside the calendar, and it says in words what its colour says. It was `href='#'`
    /// with an `hx-get`, so without the script, or opened in a new tab, a cell went nowhere, and a
    /// screen reader heard the bare day number. Fixed dates: May 2026 (31 days), one busy day,
    /// one with one detection of one species, and every other day empty.
    #[test]
    fn every_day_cell_is_a_link_to_its_day_and_names_what_it_holds() {
        let days: Vec<_> = [("2026-05-12", 140, 23), ("2026-05-31", 1, 1)]
            .iter()
            .map(|&(date, count, species)| birdnet_db::sqlite::DayCount {
                date: date.into(),
                count,
                species,
            })
            .collect();
        let html = render_calendar(&days, Some("2026-05"), Some("2026-05-12"));
        // Each cell's opening tag, in order.
        let cells: Vec<&str> = html
            .split("<a class=\"rp-day")
            .skip(1)
            .map(|c| &c[..c.find('>').expect("a closed tag")])
            .collect();
        assert_eq!(cells.len(), 31, "one cell per day of May: {html}");
        for (i, tag) in cells.iter().enumerate() {
            let date = format!("2026-05-{:02}", i + 1);
            assert!(
                tag.contains(&format!("href=\"/reports/day?date={date}\"")),
                "{date}: the cell must be a link to the day's page: <a class=\"rp-day{tag}>"
            );
            assert!(
                tag.contains(&format!("hx-get=\"/pages/history-chart?date={date}\"")),
                "{date}: htmx still loads the day beside the calendar: {tag}"
            );
        }
        // What a screen reader hears is the cell's own text, all of it; what the eye reads is
        // that text less the screen-reader-only words. The second must be part of the first
        // (WCAG 2.5.3, axe's label-content-name-mismatch), so there is no `aria-label`.
        let whole: Vec<&str> = html
            .split("<a class=\"rp-day")
            .skip(1)
            .map(|c| &c[..c.find("</a>").expect("a closed link")])
            .collect();
        let text = |el: &str, sr: bool| -> String {
            let mut out = String::new();
            let mut rest = &el[el.find('>').expect("a tag") + 1..];
            while let Some(lt) = rest.find('<') {
                out.push_str(&rest[..lt]);
                let gt = rest[lt..].find('>').expect("a tag") + lt;
                if !sr && rest[lt..gt].contains("sr-only") {
                    // Skip the hidden words, up to this span's end.
                    let end = rest[gt..].find("</span>").expect("closed") + gt;
                    rest = &rest[end + "</span>".len()..];
                } else {
                    rest = &rest[gt + 1..];
                }
            }
            out.push_str(rest);
            out.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        for (cell, (heard, seen)) in [
            (11, ("12 May, 140 detections, 23 species", "12 140 23 sp")),
            (30, ("31 May, 1 detection, 1 species", "31 1 1 sp")),
            (0, ("1 May, no detections", "1")),
        ] {
            assert_eq!(text(whole[cell], true), heard, "{}", whole[cell]);
            assert_eq!(text(whole[cell], false), seen, "{}", whole[cell]);
            assert!(!cells[cell].contains("aria-label"), "{}", cells[cell]);
        }
        // The selected day says so to a screen reader too, and only that day.
        assert!(cells[11].starts_with(" sel\""), "{}", cells[11]);
        assert!(cells[11].contains("aria-current=\"date\""), "{}", cells[11]);
        assert_eq!(
            cells.iter().filter(|c| c.contains("aria-current")).count(),
            1
        );
    }

    /// What the handler lets through as `sel`: a date's shape and nothing else.
    #[test]
    fn only_a_date_shaped_value_is_a_selected_day() {
        let kept = |s: &str| crate::routes::is_valid_date(s);
        assert!(kept("2026-11-01"));
        for bad in [
            "\"><zq-p>12",
            "2026-11-0\"",
            "2026/11/01",
            "2026-11-011",
            "",
            "<script>1",
        ] {
            assert!(!kept(bad), "{bad:?}");
        }
    }

    /// The handler end to end, on a database whose days are fixed: a `sel` that is not a date is
    /// dropped before the page is drawn (nothing of it appears, raw or encoded), and a real one
    /// still travels. The unit tests above pin the encoding at the sink; this pins that the
    /// handler asks for a date at all.
    #[tokio::test(flavor = "current_thread")]
    async fn the_calendar_handler_drops_a_selected_day_that_is_not_a_date() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        conn.execute_batch(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name) VALUES
               ('2026-10-31', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.91, 'b.wav'),
               ('2026-11-01', '07:20:00', 'Erithacus rubecula', 'European Robin', 0.88, 'r.wav');",
        )
        .expect("seed");
        let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        let ask = |sel: &str| {
            let state = state.clone();
            let sel = sel.to_owned();
            async move {
                let response = history_calendar_partial(
                    State(state),
                    Query(CalendarParams {
                        month: None,
                        sel: Some(sel),
                    }),
                )
                .await
                .into_response();
                let body = axum::body::to_bytes(response.into_body(), 1 << 20)
                    .await
                    .expect("body");
                String::from_utf8(body.to_vec()).expect("utf-8")
            }
        };
        let html = ask("\"><zq-p>12").await;
        assert!(
            html.contains("month=2026-10"),
            "a month button, or no test: {html}"
        );
        assert!(!html.contains("zq-p") && !html.contains("%3Czq"), "{html}");
        let html = ask("2026-10-31").await;
        assert!(html.contains("month=2026-10&amp;sel=2026-10-31"), "{html}");
    }

    #[test]
    fn render_chart_content_no_data() {
        let html = render_chart_content(
            "2026-03-14",
            0,
            &[],
            &[],
            &birdnet_core::i18n::NameLocalizer::english(),
        );
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
            ..Default::default()
        }
    }

    #[test]
    fn open_day_link_present_only_with_detections() {
        let species = vec![("Robin".to_string(), "Erithacus".to_string(), 3_i64)];
        let hours = vec![birdnet_db::sqlite::HourlyCount {
            hour: "06".into(),
            count: 3,
        }];
        let with = render_chart_content(
            "2026-06-14",
            3,
            &species,
            &hours,
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        assert!(with.contains("Open day"));
        assert!(with.contains("/reports/day?date="));
        // An empty day has nothing to open.
        let without = render_chart_content(
            "2026-06-14",
            0,
            &[],
            &[],
            &birdnet_core::i18n::NameLocalizer::english(),
        );
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
        let html = render_day_page(
            "2026-06-14",
            &species,
            &hours,
            &DayLog {
                rows: &rows,
                page: 1,
                total: 7,
            },
            &birdnet_core::i18n::NameLocalizer::english(),
            0.70,
        );
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
        let html = render_day_page(
            "2026-06-14",
            &[],
            &[],
            &DayLog {
                rows: &[],
                page: 1,
                total: 0,
            },
            &birdnet_core::i18n::NameLocalizer::english(),
            0.70,
        );
        assert!(html.contains("No detections were recorded"));
        assert!(html.contains("Back to history"));
        assert!(!html.contains("tdl-card"));
    }

    // -----------------------------------------------------------------------
    // UIR-09: an off day, a quiet day and a day to come are three things
    // -----------------------------------------------------------------------

    /// The class list of the cell for `date`, which is either a link (`<a class=`) or, for a
    /// day to come, a plain box (`<div class=`).
    fn cell_class<'a>(html: &'a str, date: &str) -> &'a str {
        let day: u32 = date[8..].parse().expect("day");
        // Every cell's first text is its day number in `<span class="dn">`.
        let marker = format!(r#"<span class="dn">{day}<"#);
        let at = html
            .find(&marker)
            .unwrap_or_else(|| panic!("no cell {date}"));
        let open = html[..at]
            .rfind("class=\"rp-day")
            .expect("the cell's class");
        let rest = &html[open + 7..];
        &rest[..rest.find('"').expect("closing quote")]
    }

    /// November 2026: detections on the 5th and the 8th; the ledger from the 1st, with no
    /// audio on the 6th and 7th; today the 10th, so the 11th onward has not happened.
    fn november(ledger: bool) -> String {
        let days = vec![
            birdnet_db::sqlite::DayCount {
                date: "2026-11-05".into(),
                count: 30,
                species: 4,
            },
            birdnet_db::sqlite::DayCount {
                date: "2026-11-08".into(),
                count: 10,
                species: 2,
            },
        ];
        let ledger = ledger.then(|| {
            super::super::listening::StationLedger::from_daily(
                [1, 2, 3, 4, 5, 8, 9]
                    .iter()
                    .map(|d| (format!("2026-11-{d:02}"), 86_400.0))
                    .collect(),
            )
        });
        render_month(
            &days,
            &MonthSpan::of("2026-11-01", "2026-11-08"),
            "2026-11",
            None,
            &DayStates {
                ledger: ledger.as_ref(),
                today: "2026-11-10",
            },
        )
    }

    /// UIR-09. The 6th and 7th (no audio) were blank like the 23rd to the 30th (to come): the
    /// reader could not tell an outage from the future. An off day is hatched and says "off";
    /// a day to come is outlined, is not a link, and says "not yet"; a listened day with
    /// nothing heard is a plain cell that says so.
    #[test]
    fn an_off_day_does_not_look_like_a_day_to_come() {
        let html = november(true);
        assert_eq!(cell_class(&html, "2026-11-06"), "rp-day off");
        assert_eq!(cell_class(&html, "2026-11-07"), "rp-day off");
        assert!(html.contains(r#"<span class="doff" aria-hidden="true">off</span>"#));
        assert!(html.contains("<span class=\"sr-only\"> station not listening</span>"));
        assert_eq!(cell_class(&html, "2026-11-04"), "rp-day", "quiet, listened");
        assert_eq!(
            cell_class(&html, "2026-11-10"),
            "rp-day",
            "today is never off"
        );
        for d in 11..=30 {
            assert_eq!(
                cell_class(&html, &format!("2026-11-{d:02}")),
                "rp-day future"
            );
        }
        assert!(
            !html.contains("href=\"/reports/day?date=2026-11-11\""),
            "a day to come is not a link"
        );
        assert!(html.contains("<span class=\"sr-only\"> not yet</span>"));
        // And the legend names the two states.
        assert!(html.contains("station not listening"));
        assert!(html.contains("rp-legend-off") && html.contains("rp-legend-future"));
    }

    /// Counterpart: a station with no ledger (imported history) claims no day was off, and
    /// the future is still the future.
    #[test]
    fn without_a_ledger_no_day_is_called_off() {
        let html = november(false);
        assert_eq!(cell_class(&html, "2026-11-06"), "rp-day");
        assert!(!html.contains("rp-day off"));
        assert!(!html.contains("rp-legend-off"));
        assert_eq!(cell_class(&html, "2026-11-11"), "rp-day future");
    }

    /// Statements SQLite ran on the traced connection of the test below.
    static STATEMENTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    fn record(event: rusqlite::trace::TraceEvent<'_>) {
        if let rusqlite::trace::TraceEvent::Stmt(stmt, _) = event {
            STATEMENTS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(stmt.sql().into_owned());
        }
    }

    /// Two years of one detection a day, and the calendar asked for one month of it.
    fn two_years() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        conn.execute_batch(
            "WITH RECURSIVE d(n) AS (SELECT 0 UNION ALL SELECT n + 1 FROM d WHERE n < 730)
             INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             SELECT date('2024-11-01', '+' || n || ' days'), '06:10:00', 'Turdus merula',
                    'Eurasian Blackbird', 0.9, 'b' || n || '.wav' FROM d;",
        )
        .expect("seed");
        conn
    }

    async fn body(response: axum::response::Response) -> String {
        let body = axum::body::to_bytes(response.into_body(), 1 << 22)
            .await
            .expect("body");
        String::from_utf8(body.to_vec()).expect("utf-8")
    }

    /// UIC-21. Every month click ran `GROUP BY Date` over the whole table — every day the
    /// station has ever recorded, to draw thirty of them (measured at 1.27 s on a 3.3 M-row
    /// copy). The month's counts are now read for that month only. Read from SQLite's own
    /// statement trace, not inferred from the page.
    #[tokio::test(flavor = "current_thread")]
    async fn the_month_calendar_asks_only_for_its_month() {
        let mut conn = two_years();
        conn.trace_v2(
            rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
            Some(record),
        )
        .expect("trace");
        STATEMENTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        let html = body(
            history_calendar_partial(
                State(state),
                Query(CalendarParams {
                    month: Some("2025-06".into()),
                    sel: None,
                }),
            )
            .await
            .into_response(),
        )
        .await;
        // The counterpart: the month was drawn, with its days and both neighbours.
        assert!(html.contains("June 2025"), "{html}");
        assert!(html.contains("/reports/day?date=2025-06-30"), "{html}");
        assert!(
            html.contains(r#"aria-label="Previous month""#)
                && html.contains(r#"aria-label="Next month""#),
            "{html}"
        );
        let ran = STATEMENTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let per_day: Vec<&String> = ran.iter().filter(|q| q.contains("GROUP BY Date")).collect();
        assert!(
            !per_day.is_empty(),
            "the calendar read its counts: {ran:#?}"
        );
        for q in per_day {
            assert!(
                q.contains("Date BETWEEN"),
                "a month of the calendar must be read for that month only: {q}"
            );
        }
    }

    /// UIC-21. The day page drew every detection of the day in one response: a busy dawn on a
    /// two-model station is thousands of rows. It draws a page of them and links the next.
    #[tokio::test(flavor = "current_thread")]
    async fn a_day_page_lists_one_page_of_its_detections() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        conn.execute_batch(
            "WITH RECURSIVE d(n) AS (SELECT 0 UNION ALL SELECT n + 1 FROM d WHERE n < 449)
             INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             SELECT '2026-06-14', printf('%02d:%02d:00', 4 + n / 60, n % 60),
                    'Turdus merula', 'Eurasian Blackbird', 0.9, 'b' || n || '.wav' FROM d;",
        )
        .expect("seed");
        let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        let ask = |uri: &'static str| {
            let state = state.clone();
            async move {
                let q = Query::<HistoryParams>::try_from_uri(&uri.parse().expect("uri"))
                    .expect("query");
                let Html(html) = day_page(State(state), q, HeaderMap::new()).await;
                html
            }
        };
        let first = ask("http://x/reports/day?date=2026-06-14").await;
        let rows = first.matches("bnb-card tdl-card").count();
        assert!(rows > 0, "the day page drew its log: {first}");
        assert!(
            rows <= 200,
            "a day page must draw one page of the day's detections, not all 450: drew {rows}"
        );
        assert!(
            first.contains("450 detections"),
            "the head still counts the whole day"
        );
        assert!(
            first.contains("/reports/day?date=2026-06-14&amp;page=2"),
            "and links the next page"
        );
        // Newest first: page one opens on the day's last detection, 11:29.
        let newest = first.find("time=11%3A29%3A00").expect("the newest row");
        let older = first.find("time=11%3A28%3A00").expect("the next row");
        assert!(newest < older, "newest first");
        let third = ask("http://x/reports/day?date=2026-06-14&page=3").await;
        assert_eq!(third.matches("bnb-card tdl-card").count(), 50, "the rest");
        assert!(!third.contains("page=4"), "nothing after the last page");
        assert!(third.contains("page=2"), "the previous page");
    }

    /// UIR-20. History wrote a day three ways: "2026-06-14 · Sun" over the panel, a bare
    /// "2026-06-14" as the day page's heading, "Sun 26 Jul" in a calendar note. Prose dates are
    /// now one shape, "Sun 14 Jun 2026", in a `<time>` carrying the ISO date for a machine.
    #[test]
    fn a_day_in_prose_is_written_one_way() {
        let panel = render_chart_content(
            "2026-06-14",
            0,
            &[],
            &[],
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        assert!(
            panel.contains(r#"<time datetime="2026-06-14">Sun 14 Jun 2026</time>"#),
            "{panel}"
        );
        assert!(!panel.contains("2026-06-14 · "), "{panel}");
    }
}
