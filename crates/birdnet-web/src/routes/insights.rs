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

/// The insight routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/insights/dawn-chorus", get(dawn_chorus))
        .route("/insights/migration", get(migration))
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

#[cfg(test)]
mod tests {
    use super::bounded_days;

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
