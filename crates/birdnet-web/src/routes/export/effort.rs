//! The effort ledger and a species' detection history, as files (RES-02).
//!
//! `GET /api/v2/effort/export?from=&to=` is the hourly ledger: per local hour and
//! source, the audio the pipeline analysed and the time the source was up.
//!
//! `GET /api/v2/detections/export/history?sci_name=&from=&to=&grain=hour|day&min_seconds=`
//! is one species' detection history, the input occupancy models take: one row per
//! occasion (an hour or a day), `1` when the species was detected, `0` when the
//! station listened and did not detect it, and `NA` when it did not listen for at
//! least `min_seconds` of the occasion (default: half of it, 1800 s an hour or
//! 43 200 s a day). A day without effort is not a day without birds, and an export
//! that wrote it as `0` would tell a model so. Each source is its own site: an
//! occasion is `1` only when a source that itself listened for `min_seconds`
//! detected the species (review F14), and `&source=` gives one source's history.

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use super::escape_csv;
use crate::routes::is_valid_date;
use crate::state::AppState;

/// Longest range either export covers, in days: a year of hours is 8 784 rows.
const MAX_DAYS: i64 = 366;

#[derive(Deserialize)]
pub(super) struct EffortQuery {
    from: String,
    to: String,
}

#[derive(Deserialize)]
pub(super) struct HistoryQuery {
    sci_name: String,
    from: String,
    to: String,
    /// `hour` or `day` (default `day`).
    grain: Option<String>,
    /// Audio analysed below which an occasion is `NA`.
    min_seconds: Option<f64>,
    /// One source's history (each source is its own site); absent for the
    /// station.
    source: Option<String>,
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": message }).to_string(),
    )
        .into_response()
}

/// Both dates valid, in order, and at most [`MAX_DAYS`] apart.
fn check_range(from: &str, to: &str) -> Result<(), &'static str> {
    if !is_valid_date(from) || !is_valid_date(to) {
        return Err("from and to are required, as YYYY-MM-DD");
    }
    let span = crate::routes::pages::viz::kit::epoch_day(to)
        .zip(crate::routes::pages::viz::kit::epoch_day(from))
        .map(|(t, f)| t - f);
    match span {
        Some(d) if (0..MAX_DAYS).contains(&d) => Ok(()),
        _ => Err("from must not be after to, and the range is at most 366 days"),
    }
}

fn csv_file(name: &str, body: String) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        body,
    )
        .into_response()
}

fn internal<E: std::fmt::Display>(e: &E) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        json!({"error": crate::routes::log_internal("internal error", e)}).to_string(),
    )
        .into_response()
}

pub(super) async fn export_effort(
    State(state): State<AppState>,
    Query(q): Query<EffortQuery>,
) -> Response {
    if let Err(message) = check_range(&q.from, &q.to) {
        return bad_request(message);
    }
    let result = state
        .run_blocking(move |s| {
            s.with_read_db(|conn| birdnet_db::sqlite::effort_hours(conn, &q.from, &q.to))
        })
        .await;
    match result {
        Ok(rows) => {
            let mut out = String::from("Date,Hour,Source,Seconds_Analysed,Seconds_Up\n");
            for r in rows {
                let _ = writeln!(
                    out,
                    "{},{},{},{:.1},{:.1}",
                    r.date,
                    r.hour,
                    escape_csv(&r.source),
                    r.seconds_analysed,
                    r.seconds_up
                );
            }
            csv_file("effort-by-hour.csv", out)
        }
        Err(e) => internal(&e),
    }
}

/// `GET /api/v2/effort/export/utc?from=&to=` — the ledger by UTC hour (RES-13):
/// one row per UTC hour and source, the analysed seconds by the instant the
/// audio began, and the time up split by the clock's state while it was
/// (`Seconds_Up_Clock_Synced`, `_Unsynced`, `_Unknown`). `from` and `to` are UTC
/// dates, inclusive. Hours before the upgrade that added it are absent, not
/// zero. An hour stamped while the clock was not synchronised is itself only as
/// right as that clock.
pub(super) async fn export_effort_utc(
    State(state): State<AppState>,
    Query(q): Query<EffortQuery>,
) -> Response {
    if let Err(message) = check_range(&q.from, &q.to) {
        return bad_request(message);
    }
    let (Some(from), Some(to)) = (
        birdnet_core::civil::unix_secs_from_local(&q.from, "00:00:00", 0),
        birdnet_core::civil::unix_secs_from_local(&q.to, "00:00:00", 0),
    ) else {
        return bad_request("from and to are required, as YYYY-MM-DD");
    };
    let result = state
        .run_blocking(move |s| {
            s.with_read_db(|conn| {
                birdnet_db::capture_ledger::effort_utc_hours(conn, from, to + 86_400)
            })
        })
        .await;
    match result {
        Ok(rows) => {
            let mut out = String::from(
                "Hour_Start_UTC,Source,Seconds_Analysed,Seconds_Up,Seconds_Up_Clock_Synced,\
                 Seconds_Up_Clock_Unsynced,Seconds_Up_Clock_Unknown\n",
            );
            for r in rows {
                let _ = writeln!(
                    out,
                    "{},{},{:.1},{:.1},{:.1},{:.1},{:.1}",
                    birdnet_core::civil::rfc3339_utc(r.hour_start_utc),
                    escape_csv(&r.source),
                    r.seconds_analysed,
                    r.seconds_up,
                    r.seconds_up_clock_synced,
                    r.seconds_up_clock_unsynced,
                    r.seconds_up_clock_unknown(),
                );
            }
            csv_file("effort-by-utc-hour.csv", out)
        }
        Err(e) => internal(&e),
    }
}

pub(super) async fn export_history(
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Response {
    if let Err(message) = check_range(&q.from, &q.to) {
        return bad_request(message);
    }
    let hourly = match q.grain.as_deref() {
        None | Some("day") => false,
        Some("hour") => true,
        Some(_) => return bad_request("grain is hour or day"),
    };
    let occasion = if hourly { 3600.0 } else { 86_400.0 };
    let min_seconds = q.min_seconds.unwrap_or(occasion / 2.0);
    if !(min_seconds.is_finite() && min_seconds > 0.0 && min_seconds <= occasion) {
        return bad_request("min_seconds is above 0 and at most the occasion's length");
    }
    if q.sci_name.trim().is_empty() {
        return bad_request("sci_name is required");
    }
    let result = state
        .run_blocking(move |s| {
            s.with_read_db(|conn| {
                birdnet_db::sqlite::detection_history(
                    conn,
                    q.sci_name.trim(),
                    &q.from,
                    &q.to,
                    hourly,
                    min_seconds,
                    q.source.as_deref().map(str::trim).filter(|s| !s.is_empty()),
                )
            })
        })
        .await;
    match result {
        Ok(rows) => {
            let mut out = String::from("Date,Hour,Seconds_Analysed,Detections,Detected\n");
            for r in rows {
                let _ = writeln!(
                    out,
                    "{},{},{:.1},{},{}",
                    r.date,
                    r.hour.map_or_else(String::new, |h| h.to_string()),
                    r.seconds_analysed,
                    r.detections,
                    match r.detected {
                        Some(true) => "1",
                        Some(false) => "0",
                        None => "NA",
                    }
                );
            }
            csv_file("detection-history.csv", out)
        }
        Err(e) => internal(&e),
    }
}
