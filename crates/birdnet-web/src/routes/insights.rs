//! `GET /api/v2/insights/*` — the dawn chorus's running order, recent
//! arrivals, and species gone quiet.
//!
//! The counterparts of birdnet-go's endpoints of the same names, with the same
//! parameters, defaults and field names, so a client written against either
//! station reads both. The computations are in
//! `birdnet_db::sqlite::queries::insights`.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use birdnet_db::sqlite::queries::insights::{DAWN_END_HOUR, DAWN_START_HOUR};
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;

/// Upstream's defaults and limits.
const DAWN_PERIOD_DAYS: u32 = 30;
const DAWN_MAX_PERIOD_DAYS: u32 = 365;
const DAWN_MIN_DAYS: u32 = 3;
const MIGRATION_RECENT_DAYS: u32 = 14;
const MIGRATION_MIN_TOTAL: u32 = 5;
const EXPECTED_WINDOW_DAYS: u32 = 3;
const EXPECTED_MAX_YEARS: u32 = 10;

/// The insight routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/insights/dawn-chorus", get(dawn_chorus))
        .route("/insights/migration", get(migration))
        .route("/insights/expected-today", get(expected_today))
        .route(
            "/insights/expected-today/regional",
            get(expected_today_regional),
        )
}

/// Query parameters for the dawn chorus. Strings, so that a value that is not
/// a number is answered as upstream answers it rather than by the extractor.
#[derive(Debug, Deserialize)]
struct DawnQuery {
    period_days: Option<String>,
    min_days: Option<String>,
}

/// A day count from a query value: absent is `default`, a value that is not a
/// positive integer is refused, and the result is clamped to `max` (the
/// default too, so `min_days` never exceeds `period_days`).
fn bounded_days(raw: Option<&str>, default: u32, max: u32) -> Result<u32, ()> {
    let n = match raw {
        None | Some("") => default,
        Some(s) => s.parse::<u32>().ok().filter(|n| *n >= 1).ok_or(())?,
    };
    Ok(n.min(max))
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

fn read_failed(what: &str, e: &birdnet_db::sqlite::connection::DbError) -> Response {
    tracing::warn!(error = %e, "insight could not be read: {what}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": format!("the {what} could not be read") })),
    )
        .into_response()
}

/// `HH:MM` for seconds after midnight.
fn hh_mm(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

fn thumbnail_url(sci_name: &str) -> String {
    format!(
        "/api/v2/species/image/{}",
        crate::routes::pages::simple_url_encode(sci_name)
    )
}

/// `GET /api/v2/insights/dawn-chorus?period_days=&min_days=` — each species'
/// typical first song of the morning, earliest riser first.
async fn dawn_chorus(State(state): State<AppState>, Query(q): Query<DawnQuery>) -> Response {
    let Ok(period_days) = bounded_days(
        q.period_days.as_deref(),
        DAWN_PERIOD_DAYS,
        DAWN_MAX_PERIOD_DAYS,
    ) else {
        return bad_request("period_days must be a positive integer");
    };
    let Ok(min_days) = bounded_days(q.min_days.as_deref(), DAWN_MIN_DAYS, period_days) else {
        return bad_request("min_days must be a positive integer");
    };
    let today = crate::routes::pages::today_date_string();
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                birdnet_db::sqlite::dawn_first_songs(conn, &today, period_days, min_days)
            })
        })
        .await;
    match result {
        Ok(songs) => Json(json!({
            "species": songs.iter().map(|s| json!({
                "scientific_name": s.sci_name,
                "common_name": s.com_name,
                "avg_first_detection": hh_mm(s.avg_first_secs),
                "earliest_detection": hh_mm(s.earliest_secs),
                "days_observed": s.days_observed,
                "thumbnail_url": thumbnail_url(&s.sci_name),
            })).collect::<Vec<_>>(),
            "period_days": period_days,
            "min_days": min_days,
            "start_hour": DAWN_START_HOUR,
            "end_hour": DAWN_END_HOUR,
        }))
        .into_response(),
        Err(e) => read_failed("dawn chorus", &e),
    }
}

/// `GET /api/v2/insights/migration` — species first heard in the last 14
/// days, and species heard at least five times with none in the last 14.
async fn migration(State(state): State<AppState>) -> Response {
    let today = crate::routes::pages::today_date_string();
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                let arrivals =
                    birdnet_db::sqlite::new_arrivals(conn, &today, MIGRATION_RECENT_DAYS)?;
                let quiet = birdnet_db::sqlite::gone_quiet(
                    conn,
                    &today,
                    MIGRATION_RECENT_DAYS,
                    MIGRATION_MIN_TOTAL,
                )?;
                Ok::<_, birdnet_db::sqlite::connection::DbError>((arrivals, quiet))
            })
        })
        .await;
    match result {
        Ok((arrivals, quiet)) => Json(json!({
            "new_arrivals": arrivals.iter().map(|a| json!({
                "scientific_name": a.sci_name,
                "common_name": a.com_name,
                "first_detected": a.first_detected,
                "detection_count": a.detection_count,
                "thumbnail_url": thumbnail_url(&a.sci_name),
            })).collect::<Vec<_>>(),
            "gone_quiet": quiet.iter().map(|g| json!({
                "scientific_name": g.sci_name,
                "common_name": g.com_name,
                "last_detected": g.last_detected,
                "days_since": g.days_since,
                "total_detections": g.total_detections,
                "thumbnail_url": thumbnail_url(&g.sci_name),
            })).collect::<Vec<_>>(),
            "recent_days": MIGRATION_RECENT_DAYS,
            "min_total_detections": MIGRATION_MIN_TOTAL,
        }))
        .into_response(),
        Err(e) => read_failed("migration insight", &e),
    }
}

/// A binomial written the usual way, genus capitalised: the eBird snapshot
/// stores names lowercased, for matching.
fn binomial_case(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// Day of the year (1-based) of a `YYYY-MM-DD` date.
fn day_of_year(date: &str) -> Option<i64> {
    use birdnet_core::civil::days_from_civil;
    let mut parts = date.split('-').map(str::parse::<u32>);
    let (Some(Ok(y)), Some(Ok(m)), Some(Ok(d))) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    Some(days_from_civil(y, m, d) - days_from_civil(y, 1, 1) + 1)
}

/// The species this station has heard around today's date in each of the
/// last ten years, and how many of those years it has data for.
fn read_expected(
    state: &AppState,
    today: &str,
) -> Result<(Vec<birdnet_db::sqlite::ExpectedSpecies>, u32), birdnet_db::sqlite::connection::DbError>
{
    state.with_db(|conn| {
        birdnet_db::sqlite::expected_today(conn, today, EXPECTED_WINDOW_DAYS, EXPECTED_MAX_YEARS)
    })
}

/// `GET /api/v2/insights/expected-today` — species heard within three days
/// of today's date in any of the last ten years, the most regular first.
async fn expected_today(State(state): State<AppState>) -> Response {
    let today = crate::routes::pages::today_date_string();
    let doy = day_of_year(&today);
    let result = state.run_blocking(move |s| read_expected(s, &today)).await;
    match result {
        Ok((species, years)) => Json(json!({
            "species": species.iter().map(|e| json!({
                "scientific_name": e.sci_name,
                "common_name": e.com_name,
                "years_seen": e.years_seen,
                "last_seen_date": e.last_seen_date,
                "thumbnail_url": thumbnail_url(&e.sci_name),
            })).collect::<Vec<_>>(),
            "day_of_year": doy,
            "years_of_data": years,
        }))
        .into_response(),
        Err(e) => read_failed("expected species", &e),
    }
}

/// `GET /api/v2/insights/expected-today/regional` — what eBird observers have
/// reported near the station that it does not normally hear at this time of
/// year. `available` is false when there is no eBird snapshot (no key, no
/// location, or not fetched yet).
///
/// Unlike upstream, a failed read of the station's own history is an error
/// rather than the unfiltered eBird list: that list would include the
/// station's regulars, presented as birds it does not hear.
async fn expected_today_regional(State(state): State<AppState>) -> Response {
    let Some(snapshot) = state.nearby() else {
        return Json(json!({ "species": [], "available": false })).into_response();
    };
    let today = crate::routes::pages::today_date_string();
    let result = state.run_blocking(move |s| read_expected(s, &today)).await;
    let local = match result {
        Ok((species, _)) => species,
        Err(e) => return read_failed("station's own expected species", &e),
    };
    // eBird's spelling, for each local species eBird also reported: the same
    // matching the rest of the station uses (`Snapshot::reported`).
    let heard_here: std::collections::HashSet<&str> = local
        .iter()
        .filter_map(|e| snapshot.reported(&e.sci_name))
        .map(|n| n.sci_name.as_str())
        .collect();
    let species: Vec<_> = snapshot
        .species
        .iter()
        .filter(|n| !heard_here.contains(n.sci_name.as_str()))
        .map(|n| {
            json!({
                "scientific_name": binomial_case(&n.sci_name),
                "common_name": n.com_name,
                "observation_date": n.last_seen,
                "location_name": n.locality,
            })
        })
        .collect();
    Json(json!({ "species": species, "available": true })).into_response()
}

#[cfg(test)]
mod tests {
    use super::{bounded_days, day_of_year};

    #[test]
    fn day_of_year_counts_from_the_first_of_january() {
        assert_eq!(day_of_year("2026-01-01"), Some(1));
        assert_eq!(day_of_year("2026-10-13"), Some(286));
        assert_eq!(day_of_year("2024-12-31"), Some(366));
        assert_eq!(day_of_year("not-a-date"), None);
    }

    /// Upstream's rule: absent takes the default, anything but a positive
    /// integer is refused, and the result is clamped — `min_days` to the
    /// `period_days` actually applied.
    #[test]
    fn a_day_count_is_defaulted_refused_or_clamped() {
        assert_eq!(bounded_days(None, 30, 365), Ok(30));
        assert_eq!(bounded_days(Some(""), 30, 365), Ok(30));
        assert_eq!(bounded_days(Some("7"), 30, 365), Ok(7));
        assert_eq!(bounded_days(Some("900"), 30, 365), Ok(365));
        assert_eq!(bounded_days(Some("10"), 3, 7), Ok(7));
        assert_eq!(bounded_days(None, 3, 2), Ok(2));
        for bad in ["0", "-3", "seven", "1.5"] {
            assert_eq!(bounded_days(Some(bad), 30, 365), Err(()), "{bad}");
        }
    }
}
