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
    Router::new()
        .route(
            "/analytics/confidence/distribution",
            get(confidence_distribution),
        )
        .route("/analytics/time/year-over-year", get(year_over_year))
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

/// One day of the year-over-year comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
struct YearOverYearPoint {
    /// The day in the current year (`YYYY-MM-DD`).
    date: String,
    /// Detections from 1 January of the current year through it.
    this_year: i64,
    /// Detections from 1 January of the previous year through the same
    /// calendar day of that year.
    last_year: i64,
}

/// The two windows the comparison reads, as day numbers: 1 January through
/// `(year, month, day)`, and 1 January of the year before through the same
/// month and day — the 28th when that is 29 February.
///
/// Upstream also asks whether the previous year is a leap year. It never is
/// when the current date is 29 February — no two consecutive years are — so
/// the question is left out rather than carried as a branch nothing reaches.
const fn year_over_year_windows(year: u32, month: u32, day: u32) -> ((i64, i64), (i64, i64)) {
    use birdnet_core::civil::days_from_civil;
    let prev = year - 1;
    let prev_day = if month == 2 && day == 29 { 28 } else { day };
    (
        (
            days_from_civil(year, 1, 1),
            days_from_civil(year, month, day),
        ),
        (
            days_from_civil(prev, 1, 1),
            days_from_civil(prev, month, prev_day),
        ),
    )
}

/// Cumulative counts for each day of `current`, aligned by month and day
/// with the cumulative counts over `previous`: birdnet-go's
/// `buildYearOverYear`.
///
/// Aligned by calendar date, not day of the year, so the seasons line up
/// across a leap year. A current 29 February with no counterpart carries the
/// previous year's total from the 28th; a previous 29 February with none in
/// the current year is folded into the previous total from 1 March on.
/// Nothing is lost or counted twice, and both series only rise.
fn year_over_year_points(
    current: (i64, i64),
    previous: (i64, i64),
    per_day: &std::collections::HashMap<String, i64>,
) -> Vec<YearOverYearPoint> {
    use birdnet_core::civil::date_string_from_days;
    let month_day = |d: &str| d.get(5..).unwrap_or_default().to_owned();
    let mut prior_by_month_day = std::collections::HashMap::new();
    let mut running = 0;
    for day in previous.0..=previous.1 {
        let d = date_string_from_days(day);
        running += per_day.get(&d).copied().unwrap_or(0);
        prior_by_month_day.insert(month_day(&d), running);
    }
    let (mut this_year, mut last_year) = (0, 0);
    (current.0..=current.1)
        .map(|day| {
            let date = date_string_from_days(day);
            this_year += per_day.get(&date).copied().unwrap_or(0);
            if let Some(&p) = prior_by_month_day.get(&month_day(&date)) {
                last_year = p;
            }
            YearOverYearPoint {
                date,
                this_year,
                last_year,
            }
        })
        .collect()
}

/// Query parameters for the year-over-year tracker.
#[derive(Debug, Deserialize)]
struct YearOverYearQuery {
    date: Option<String>,
}

/// `GET /api/v2/analytics/time/year-over-year?date=` — cumulative detections
/// this year to `date` (default today) against the same span of last year,
/// one point per day.
///
/// Upstream's shape: `{currentYear, previousYear, points: [{date, monthDay,
/// thisYear, lastYear, delta}]}`. A `date` that is not a calendar day is a
/// 400.
async fn year_over_year(
    State(state): State<AppState>,
    Query(q): Query<YearOverYearQuery>,
) -> Response {
    use birdnet_core::civil::{date_string_from_days, parse_civil};
    let Ok(date) = range_bound(q.date.as_deref()) else {
        return bad_request("date must be a YYYY-MM-DD date");
    };
    let date = date.unwrap_or_else(crate::routes::pages::today_date_string);
    let Some(at) = parse_civil(&date, "00:00:00").filter(|c| c.year >= 2) else {
        return bad_request("date must be a YYYY-MM-DD date");
    };
    let (current, previous) = year_over_year_windows(at.year, at.month, at.day);
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                let mut per_day = std::collections::HashMap::new();
                for (start, end) in [previous, current] {
                    per_day.extend(birdnet_db::sqlite::day_counts_between(
                        conn,
                        &date_string_from_days(start),
                        &date_string_from_days(end),
                    )?);
                }
                Ok::<_, birdnet_db::sqlite::connection::DbError>(per_day)
            })
        })
        .await;
    let per_day = match result {
        Ok(per_day) => per_day,
        Err(e) => return read_failed("year-over-year counts", &e),
    };
    Json(json!({
        "currentYear": at.year,
        "previousYear": at.year - 1,
        "points": year_over_year_points(current, previous, &per_day)
            .iter()
            .map(|p| json!({
                "date": p.date,
                "monthDay": p.date.get(5..).unwrap_or_default(),
                "thisYear": p.this_year,
                "lastYear": p.last_year,
                "delta": p.this_year - p.last_year,
            }))
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(days: &[(&str, i64)]) -> std::collections::HashMap<String, i64> {
        days.iter().map(|(d, n)| ((*d).to_owned(), *n)).collect()
    }

    fn last(points: &[YearOverYearPoint], date: &str) -> (i64, i64) {
        let p = points.iter().find(|p| p.date == date).expect(date);
        (p.this_year, p.last_year)
    }

    /// Both years run from 1 January to the same calendar day, cumulatively.
    #[test]
    fn the_two_years_are_cumulative_and_aligned_by_calendar_day() {
        let (cur, prev) = year_over_year_windows(2026, 1, 10);
        let per_day = counts(&[
            ("2026-01-01", 2),
            ("2026-01-05", 3),
            ("2026-01-11", 100),
            ("2025-01-01", 1),
            ("2025-01-10", 4),
            ("2025-01-11", 100),
            ("2024-12-31", 100),
        ]);
        let points = year_over_year_points(cur, prev, &per_day);
        assert_eq!(points.len(), 10);
        assert_eq!(points[0].date, "2026-01-01");
        assert_eq!(last(&points, "2026-01-01"), (2, 1));
        assert_eq!(last(&points, "2026-01-04"), (2, 1));
        assert_eq!(last(&points, "2026-01-05"), (5, 1));
        assert_eq!(last(&points, "2026-01-10"), (5, 5));
    }

    /// A leap current year against a common one: 29 February has no
    /// counterpart, so last year's total from the 28th is carried over it, and
    /// the window last year ends on the 28th.
    #[test]
    fn a_current_29_february_carries_last_years_28th() {
        let (cur, prev) = year_over_year_windows(2028, 2, 29);
        assert_eq!(
            prev.1,
            birdnet_core::civil::days_from_civil(2027, 2, 28),
            "the window last year ends on the 28th"
        );
        let per_day = counts(&[("2027-02-28", 7), ("2027-03-01", 100), ("2028-02-29", 3)]);
        let points = year_over_year_points(cur, prev, &per_day);
        assert_eq!(points.len(), 60);
        assert_eq!(last(&points, "2028-02-28"), (0, 7));
        assert_eq!(last(&points, "2028-02-29"), (3, 7));
    }

    /// A leap previous year against a common one: last year's 29 February is
    /// folded into its total from 1 March, so nothing is lost.
    #[test]
    fn a_previous_29_february_is_folded_into_1_march() {
        let (cur, prev) = year_over_year_windows(2025, 3, 1);
        let per_day = counts(&[("2024-02-28", 1), ("2024-02-29", 5), ("2024-03-01", 2)]);
        let points = year_over_year_points(cur, prev, &per_day);
        assert_eq!(last(&points, "2025-02-28"), (0, 1));
        assert_eq!(last(&points, "2025-03-01"), (0, 8));
        assert_eq!(points.len(), 60);
    }
}
