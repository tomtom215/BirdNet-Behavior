//! The weather the station has cached, as JSON (`P-21`).
//!
//! tphakala/birdnet-go serves `/api/v2/weather/{latest,hourly/{date},
//! hourly/{date}/{hour},detection/{id},daily/{date},sun/{date},moon/{date}}`.
//! This serves the four that are about *weather*:
//!
//! | Path | Answers |
//! |---|---|
//! | `GET /api/v2/weather/latest` | the newest cached hour |
//! | `GET /api/v2/weather/hourly/{date}` | every cached hour of one UTC day |
//! | `GET /api/v2/weather/hourly/{date}/{hour}` | one hour |
//! | `GET /api/v2/weather/detection?date&time&sci_name` | the hour a detection was heard in |
//!
//! There is no `sun` or `moon`: they are not weather, and a public sunrise time is
//! the station's longitude to within a minute of arc — the same data this API is
//! careful not to give out (`species/occurrence` rounds the position to a tenth
//! of a degree). There is no `daily` for the same reason (its content is
//! sunrise and sunset).
//!
//! # What is stored, and what is said about it
//!
//! The weather poll (off unless `BNB_WEATHER_ENABLED=1`) keeps one row per hour
//! with seven values, whichever provider supplied them: temperature (°C),
//! precipitation (mm), wind speed (knots) and direction (degrees), pressure at
//! sea level (hPa), cloud cover (%) and a WMO weather code. Any can be missing
//! from a provider (a personal weather station has no cloud cover), and a
//! missing one is `null`, never `0`. Units are in the field names and are
//! metric whatever the station's display preference: this is a data interface.
//! Not stored, so not offered: humidity, feels-like, gusts, visibility, a
//! description or an icon.
//!
//! Open like the other reads. A row carries no position; it is the weather the
//! provider gave for wherever the station is, which the response does not say.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use birdnet_db::weather::{WeatherRow, WeatherStore as _};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// The weather read routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/weather/latest", get(latest))
        .route("/weather/hourly/{date}", get(hourly_day))
        .route("/weather/hourly/{date}/{hour}", get(hourly_hour))
        .route("/weather/detection", get(for_detection))
}

/// One cached hour as JSON. Units are in the names.
fn row_json(r: &WeatherRow) -> Value {
    json!({
        "at": at_stamp(&r.at),
        "temperature_c": r.temp_c,
        "precipitation_mm": r.precip_mm,
        "wind_speed_kt": r.wind_kt,
        "wind_direction_deg": r.wind_dir_deg,
        "pressure_hpa": r.pressure_hpa,
        "cloud_cover_pct": r.cloud_pct,
        "weather_code": r.code,
    })
}

fn bad(message: &str) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message })))
}

fn not_cached(what: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": format!(
                "no cached weather for {what}. The station only has weather if its weather poll \
                 is on (BNB_WEATHER_ENABLED=1) and has run; it keeps 30 days."
            )
        })),
    )
}

fn unreadable(err: &birdnet_db::weather::WeatherError) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": crate::routes::log_internal("weather not readable", err) })),
    )
}

/// A `YYYY-MM-DD` that is a real calendar date.
///
/// `parse_civil` checks the shape and the month, not the length of the month, so
/// 30 February parses (and would read as an empty day). A real date survives a
/// round trip through seconds; a made-up one comes back as its neighbour.
fn parse_date(raw: &str) -> Result<&str, &'static str> {
    const WHAT: &str = "date must be a real calendar date, YYYY-MM-DD";
    if !crate::routes::is_valid_date(raw) {
        return Err(WHAT);
    }
    let civil = birdnet_core::civil::parse_civil(raw, "00:00:00").ok_or(WHAT)?;
    let back = birdnet_core::civil::civil_from_unix_secs(
        birdnet_core::civil::unix_secs_from_civil(&civil),
    );
    if (back.year, back.month, back.day) == (civil.year, civil.month, civil.day) {
        Ok(raw)
    } else {
        Err(WHAT)
    }
}

/// The hour as `…:00Z`, whichever way the provider wrote it. The poll already
/// widens `2026-10-21T14:00`; a row stored by an older version may not be.
fn at_stamp(at: &str) -> String {
    if at.len() == "YYYY-MM-DDTHH:MM".len() && at.as_bytes().get(10) == Some(&b'T') {
        format!("{at}:00Z")
    } else {
        at.to_owned()
    }
}

/// `GET /api/v2/weather/latest`.
async fn latest(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    state
        .run_blocking(|state| match state.with_read_db(|c| c.latest()) {
            Ok(Some(row)) => (StatusCode::OK, Json(row_json(&row))),
            Ok(None) => not_cached("any hour"),
            Err(e) => unreadable(&e),
        })
        .await
}

/// The cached hours of one UTC day, in order.
fn day_rows(
    state: &AppState,
    date: &str,
) -> Result<Vec<WeatherRow>, birdnet_db::weather::WeatherError> {
    // The same bounds the Today page's day strip uses, so the two agree about
    // which hours belong to a day.
    let (from, to) = (format!("{date}T00:00:00Z"), format!("{date}T23:59:59Z"));
    state.with_read_db(|c| c.range(&from, &to))
}

/// `GET /api/v2/weather/hourly/{date}`.
async fn hourly_day(
    State(state): State<AppState>,
    Path(date): Path<String>,
) -> (StatusCode, Json<Value>) {
    let date = match parse_date(&date) {
        Ok(d) => d.to_owned(),
        Err(m) => return bad(m),
    };
    state
        .run_blocking(move |state| match day_rows(state, &date) {
            Ok(rows) => (
                StatusCode::OK,
                Json(json!({
                    "date": date,
                    "timezone": "UTC",
                    "count": rows.len(),
                    "hours": rows.iter().map(row_json).collect::<Vec<_>>(),
                })),
            ),
            Err(e) => unreadable(&e),
        })
        .await
}

/// `GET /api/v2/weather/hourly/{date}/{hour}`.
async fn hourly_hour(
    State(state): State<AppState>,
    Path((date, hour)): Path<(String, String)>,
) -> (StatusCode, Json<Value>) {
    let date = match parse_date(&date) {
        Ok(d) => d.to_owned(),
        Err(m) => return bad(m),
    };
    let Some(hour) = hour.parse::<u8>().ok().filter(|h| *h < 24) else {
        return bad("hour must be a whole number from 0 to 23 (UTC)");
    };
    state
        .run_blocking(move |state| match hour_row(state, &date, hour) {
            Ok(Some(row)) => (StatusCode::OK, Json(row_json(&row))),
            Ok(None) => not_cached(&format!("{date} {hour:02}:00 UTC")),
            Err(e) => unreadable(&e),
        })
        .await
}

/// The row for one UTC hour. Matched on the hour's own prefix (`…T14`) rather
/// than an exact `at`, because providers differ in how they write the stamp
/// (`…T14:00` or `…T14:00:00Z`).
fn hour_row(
    state: &AppState,
    date: &str,
    hour: u8,
) -> Result<Option<WeatherRow>, birdnet_db::weather::WeatherError> {
    let prefix = format!("{date}T{hour:02}");
    Ok(day_rows(state, date)?
        .into_iter()
        .find(|r| r.at.starts_with(&prefix)))
}

/// Query for [`for_detection`]: the key a detection is identified by.
#[derive(Deserialize)]
struct DetectionQuery {
    date: Option<String>,
    time: Option<String>,
    sci_name: Option<String>,
}

/// `GET /api/v2/weather/detection?date=&time=&sci_name=` — the hour the
/// detection was heard in. The key is the one the detections API uses; there is
/// no surrogate id to ask by.
async fn for_detection(
    State(state): State<AppState>,
    Query(q): Query<DetectionQuery>,
) -> (StatusCode, Json<Value>) {
    let (Some(date), Some(time), Some(sci)) = (q.date, q.time, q.sci_name) else {
        return bad("date, time and sci_name are all required");
    };
    if !crate::routes::is_valid_date(&date) || time.len() != 8 {
        return bad("date is YYYY-MM-DD and time is HH:MM:SS");
    }
    state
        .run_blocking(move |state| {
            let instant = match state.with_read_db(|c| {
                birdnet_db::sqlite::detected_at_utc_for(c, &date, &time, &sci)
            }) {
                Ok(Some(i)) => i,
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({ "error": "no such detection, or one whose time is unreadable" })),
                    );
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": crate::routes::log_internal("detection not readable", &e) })),
                    );
                }
            };
            let c = birdnet_core::civil::civil_from_unix_secs(instant);
            let day = format!("{:04}-{:02}-{:02}", c.year, c.month, c.day);
            let hour = u8::try_from(c.hour).unwrap_or(0);
            match hour_row(state, &day, hour) {
                Ok(Some(row)) => (
                    StatusCode::OK,
                    Json(json!({
                        "detection": { "date": date, "time": time, "sci_name": sci },
                        "heard_at": birdnet_core::civil::rfc3339_utc(instant),
                        "weather": row_json(&row),
                    })),
                ),
                Ok(None) => not_cached(&format!("{day} {hour:02}:00 UTC, when it was heard")),
                Err(e) => unreadable(&e),
            }
        })
        .await
}
