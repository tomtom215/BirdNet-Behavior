//! birdnet-go's analytics chart endpoints, under `GET /api/v2/analytics/`.
//!
//! Same paths, parameters, defaults and field names as upstream — camelCase
//! here, because upstream's chart payloads are — so a client written against
//! either station reads both. The series are read by
//! `birdnet_db::sqlite::queries::charts`; the calendar arithmetic is here.
//!
//! Upstream's range rules, shared by every endpoint below: `start_date` is
//! required, `end_date` defaults to thirty days after it, both are inclusive
//! `YYYY-MM-DD` calendar days, and a reversed range is a 400.

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use serde::Deserialize;
use serde_json::json;

use super::insights::{bad_request, calendar_date, range_bound, read_failed};
use crate::state::AppState;

/// Days `end_date` lies after `start_date` when it is not given.
const DEFAULT_RANGE_DAYS: i64 = 30;
/// Histogram bins: upstream's default and bounds.
const CONFIDENCE_DEFAULT_BINS: usize = 20;
const CONFIDENCE_MIN_BINS: i64 = 5;
const CONFIDENCE_MAX_BINS: i64 = 50;
/// Species in a confidence distribution: upstream's default and maximum.
const CONFIDENCE_DEFAULT_SPECIES: u32 = 5;
const CONFIDENCE_MAX_SPECIES: u32 = 10;

/// The chart routes.
pub fn router() -> Router<AppState> {
    Router::new().route(
        "/analytics/confidence/distribution",
        get(confidence_distribution),
    )
}

/// The `YYYY-MM-DD` `days` after a calendar date.
fn days_after(date: &str, days: i64) -> String {
    use birdnet_core::civil::{date_string_from_days, days_from_civil, parse_civil};
    parse_civil(date, "00:00:00").map_or_else(
        || date.to_owned(),
        |c| date_string_from_days(days_from_civil(c.year, c.month, c.day) + days),
    )
}

/// Upstream's range: `start_date` required, `end_date` defaulting to thirty
/// days after it. `Err` is the message for the 400.
fn chart_range(start: Option<&str>, end: Option<&str>) -> Result<(String, String), &'static str> {
    let start = match start {
        None | Some("") => return Err("Missing required parameter: start_date"),
        Some(s) if calendar_date(s) => s.to_owned(),
        Some(_) => return Err("start_date must be a YYYY-MM-DD date"),
    };
    let end = range_bound(end)
        .map_err(|()| "end_date must be a YYYY-MM-DD date")?
        .unwrap_or_else(|| days_after(&start, DEFAULT_RANGE_DAYS));
    if start > end {
        return Err("start_date is after end_date");
    }
    Ok((start, end))
}

/// A single `species` value, trimmed; absent or blank is no filter.
fn species_filter(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Query parameters for the confidence distribution.
#[derive(Debug, Deserialize)]
struct ConfidenceQuery {
    start_date: Option<String>,
    end_date: Option<String>,
    species: Option<String>,
    bins: Option<String>,
    limit: Option<String>,
}

/// The number of histogram bins asked for: upstream's default of 20 when it
/// is absent or not an integer, clamped to 5..=50.
fn confidence_bins(raw: Option<&str>) -> usize {
    raw.and_then(|b| b.trim().parse::<i64>().ok())
        .map_or(CONFIDENCE_DEFAULT_BINS, |b| {
            usize::try_from(b.clamp(CONFIDENCE_MIN_BINS, CONFIDENCE_MAX_BINS))
                .unwrap_or(CONFIDENCE_DEFAULT_BINS)
        })
}

/// The species count asked for: upstream's `ParsePaginationLimit`, so a
/// value that is not a positive integer *or is above the maximum* is the
/// default, not the maximum.
fn confidence_limit(raw: Option<&str>) -> u32 {
    raw.and_then(|l| l.parse::<u32>().ok())
        .filter(|l| (1..=CONFIDENCE_MAX_SPECIES).contains(l))
        .unwrap_or(CONFIDENCE_DEFAULT_SPECIES)
}

/// `GET /api/v2/analytics/confidence/distribution?start_date=&end_date=&species=&bins=&limit=`
/// — each species' confidence distribution: the fraction of its detections
/// in each of `bins` equal-width bins over `[0, 1]`.
///
/// Upstream's parameters, defaults and camelCase fields: `start_date` is
/// required, `end_date` defaults to thirty days after it, `bins` to 20 (5 to
/// 50), `limit` to 5 species (at most 10). Without `species`, species with
/// fewer than 20 detections in the range are left out.
async fn confidence_distribution(
    State(state): State<AppState>,
    Query(q): Query<ConfidenceQuery>,
) -> Response {
    let (start, end) = match chart_range(q.start_date.as_deref(), q.end_date.as_deref()) {
        Ok(range) => range,
        Err(message) => return bad_request(message),
    };
    let species = species_filter(q.species.as_deref());
    let bins = confidence_bins(q.bins.as_deref());
    let limit = confidence_limit(q.limit.as_deref());
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                birdnet_db::sqlite::confidence_histograms(
                    conn,
                    &start,
                    &end,
                    species.as_deref(),
                    bins,
                    limit,
                )
            })
        })
        .await;
    match result {
        Ok(rows) => Json(
            rows.iter()
                .map(|h| {
                    json!({
                        "scientificName": h.sci_name,
                        "commonName": h.com_name,
                        "bins": h.bins,
                        "total": h.total,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => read_failed("confidence distribution", &e),
    }
}
