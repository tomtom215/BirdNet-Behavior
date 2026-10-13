//! `GET /api/v2/insights/*`, `GET /api/v2/dashboard/kpis` and
//! `GET /api/v2/analytics/species/summary`.
//!
//! The dawn chorus's running order, recent arrivals, species gone quiet,
//! likely misdetections, what is expected today, the dashboard's headline
//! numbers, and each species' figures over a date range.
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
const PHANTOM_PERIOD_DAYS: u32 = 30;
const PHANTOM_MIN_DETECTIONS: u32 = 3;
const PHANTOM_MAX_AVG_CONFIDENCE: f64 = 0.6;

/// The insight routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/insights/dawn-chorus", get(dawn_chorus))
        .route("/insights/migration", get(migration))
        .route("/insights/phantom-species", get(phantom_species))
        .route("/insights/expected-today", get(expected_today))
        .route(
            "/insights/expected-today/regional",
            get(expected_today_regional),
        )
        .route("/dashboard/kpis", get(dashboard_kpis))
        .route("/analytics/species/summary", get(species_summary))
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

/// `GET /api/v2/insights/phantom-species` — species detected at least three
/// times in the last 30 days with a mean confidence under 0.6, the least
/// confident first.
///
/// Upstream's rule. `/admin/quality/phantoms` asks a different question —
/// four shape signals over all history — and stays.
async fn phantom_species(State(state): State<AppState>) -> Response {
    let today = crate::routes::pages::today_date_string();
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                birdnet_db::sqlite::phantom_species(
                    conn,
                    &today,
                    PHANTOM_PERIOD_DAYS,
                    PHANTOM_MIN_DETECTIONS,
                    PHANTOM_MAX_AVG_CONFIDENCE,
                )
            })
        })
        .await;
    match result {
        Ok(species) => Json(json!({
            "species": species.iter().map(|p| json!({
                "scientific_name": p.sci_name,
                "common_name": p.com_name,
                "detection_count": p.detection_count,
                "avg_confidence": p.avg_confidence,
                "max_confidence": p.max_confidence,
                "thumbnail_url": thumbnail_url(&p.sci_name),
            })).collect::<Vec<_>>(),
            "period_days": PHANTOM_PERIOD_DAYS,
            "confidence_threshold": PHANTOM_MAX_AVG_CONFIDENCE,
            "min_detections": PHANTOM_MIN_DETECTIONS,
        }))
        .into_response(),
        Err(e) => read_failed("phantom species", &e),
    }
}

/// Query parameters for the species summary.
#[derive(Debug, Deserialize)]
struct SummaryQuery {
    start_date: Option<String>,
    end_date: Option<String>,
    limit: Option<String>,
}

/// A `YYYY-MM-DD` that names a real calendar day.
fn calendar_date(s: &str) -> bool {
    use birdnet_core::civil::{date_string_from_days, days_from_civil, parse_civil};
    parse_civil(s, "00:00:00")
        .is_some_and(|c| date_string_from_days(days_from_civil(c.year, c.month, c.day)) == s)
}

/// A date-range bound: absent or empty is open, anything but a calendar date
/// is refused.
fn range_bound(raw: Option<&str>) -> Result<Option<String>, ()> {
    match raw {
        None | Some("") => Ok(None),
        Some(s) if calendar_date(s) => Ok(Some(s.to_owned())),
        Some(_) => Err(()),
    }
}

/// `GET /api/v2/analytics/species/summary?start_date=&end_date=&limit=` —
/// each species' count, mean and highest confidence, and first and last
/// detection over the range, the most detected first.
///
/// Upstream's shape: a bare array; `first_heard`/`last_heard` in RFC 3339
/// with the offset in force at the time, left out when the row names no
/// instant. A `limit` that is not a positive integer is ignored, as upstream
/// ignores it.
async fn species_summary(State(state): State<AppState>, Query(q): Query<SummaryQuery>) -> Response {
    let Ok(start) = range_bound(q.start_date.as_deref()) else {
        return bad_request("start_date must be a YYYY-MM-DD date");
    };
    let Ok(end) = range_bound(q.end_date.as_deref()) else {
        return bad_request("end_date must be a YYYY-MM-DD date");
    };
    if matches!((&start, &end), (Some(s), Some(e)) if s > e) {
        return bad_request("start_date is after end_date");
    }
    let limit = q
        .limit
        .as_deref()
        .and_then(|l| l.parse::<usize>().ok())
        .filter(|l| *l > 0);
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                birdnet_db::sqlite::species_range_summary(conn, start.as_deref(), end.as_deref())
            })
        })
        .await;
    let rows = match result {
        Ok(rows) => rows,
        Err(e) => return read_failed("species summary", &e),
    };
    let heard = |p: &Option<(String, String, i64)>| {
        p.as_ref()
            .and_then(|(d, t, utc)| birdnet_core::civil::rfc3339_local(d, t, Some(*utc)))
    };
    let body: Vec<_> = rows
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .map(|r| {
            let mut o = json!({
                "scientific_name": r.sci_name,
                "common_name": r.com_name,
                "count": r.count,
                "avg_confidence": r.avg_confidence,
                "max_confidence": r.max_confidence,
                "thumbnail_url": thumbnail_url(&r.sci_name),
            });
            if let Some(f) = heard(&r.first) {
                o["first_heard"] = json!(f);
            }
            if let Some(l) = heard(&r.last) {
                o["last_heard"] = json!(l);
            }
            o
        })
        .collect();
    Json(body).into_response()
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

/// `GET /api/v2/dashboard/kpis` — species ever heard, detections today, the
/// busiest day since this date last year, and the run of consecutive days
/// with a detection ending today.
///
/// Upstream's shape, including its empty values: a station with no busiest
/// day answers `{"date": "", "count": 0}`, and no streak `start_date: ""`.
/// Unlike upstream, the streak is not capped at the 90 most recent dates.
async fn dashboard_kpis(State(state): State<AppState>) -> Response {
    let today = crate::routes::pages::today_date_string();
    let result = state
        .run_blocking(move |s| s.with_db(|conn| birdnet_db::sqlite::dashboard_kpis(conn, &today)))
        .await;
    match result {
        Ok(k) => {
            let (best_date, best_count) = k.best_day.unwrap_or_default();
            Json(json!({
                "lifetime_species": k.lifetime_species,
                "today_detections": k.today_detections,
                "best_day": { "date": best_date, "count": best_count },
                "detection_streak": {
                    "days": k.streak_days,
                    "start_date": k.streak_start.unwrap_or_default(),
                },
            }))
            .into_response()
        }
        Err(e) => read_failed("dashboard figures", &e),
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded_days, calendar_date, day_of_year, range_bound};

    #[test]
    fn day_of_year_counts_from_the_first_of_january() {
        assert_eq!(day_of_year("2026-01-01"), Some(1));
        assert_eq!(day_of_year("2026-10-13"), Some(286));
        assert_eq!(day_of_year("2024-12-31"), Some(366));
        assert_eq!(day_of_year("not-a-date"), None);
    }

    /// A range bound is a real calendar day or nothing: 30 February is
    /// refused, as Go's `time.Parse` refuses it upstream.
    #[test]
    fn a_range_bound_is_a_calendar_date() {
        assert!(calendar_date("2024-02-29"));
        assert!(calendar_date("2026-12-31"));
        for bad in [
            "2026-02-29",
            "2026-02-30",
            "2026-13-01",
            "2026-1-01",
            "26-01-01",
            "today",
        ] {
            assert!(!calendar_date(bad), "{bad}");
        }
        assert_eq!(range_bound(None), Ok(None));
        assert_eq!(range_bound(Some("")), Ok(None));
        assert_eq!(
            range_bound(Some("2026-10-13")),
            Ok(Some("2026-10-13".to_owned()))
        );
        assert_eq!(range_bound(Some("2026-02-30")), Err(()));
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
