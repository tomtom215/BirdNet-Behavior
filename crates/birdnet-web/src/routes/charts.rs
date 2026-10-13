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

use axum::extract::{Query, RawQuery, State};
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
/// A day's chorus onset is its `ONSET_RANK`-th earliest detection, read only
/// on a day with at least `ONSET_MIN_DETECTIONS`: upstream's
/// `onsetDetectionRank` and `minOnsetDetections`.
const ONSET_RANK: u32 = 3;
const ONSET_MIN_DETECTIONS: i64 = 5;
/// Species in the hour-of-day ridgeline (default, maximum) and in the
/// succession streamgraph, and the most a `species` selection may name:
/// upstream's constants.
const RIDGELINE_SPECIES: (u32, u32) = (5, 10);
const SUCCESSION_SPECIES: (u32, u32) = (6, 10);
const MAX_SPECIES_SELECTION: usize = 10;
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
        .route("/analytics/time/dawn-onset", get(dawn_onset))
        .route("/analytics/time/heatmap", get(heatmap))
        .route(
            "/analytics/time/distribution/species",
            get(species_hour_distribution),
        )
        .route("/analytics/time/succession", get(succession))
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

/// Civil dawn on `date` at `location`, as minutes after local midnight:
/// `None` when the sun does not reach 6° below the horizon that day (polar
/// day and night) or the clock cannot place it.
///
/// Local time is SQLite's `localtime`: the same zone, rules and daylight
/// saving the `Date` and `Time` columns were written in, for that day rather
/// than today.
fn civil_dawn_minute(
    conn: &rusqlite::Connection,
    location: birdnet_scheduler::Location,
    date: &str,
) -> Option<i64> {
    use birdnet_core::civil::parse_civil;
    let c = parse_civil(date, "00:00:00")?;
    let dawn = birdnet_scheduler::SolarDay::for_date(location, c.year, c.month, c.day)
        .ok()?
        .civil_dawn_utc_min?;
    place_on_local_day(date, dawn, |instant| {
        conn.query_row(
            "SELECT date(?1, 'unixepoch', 'localtime'),
                    CAST(strftime('%H', ?1, 'unixepoch', 'localtime') AS INTEGER) * 60
                    + CAST(strftime('%M', ?1, 'unixepoch', 'localtime') AS INTEGER)",
            [instant],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()
    })
}

/// The local minute of the day of an event the solver placed at
/// `utc_minute` of a UTC day, on local `date`.
///
/// For a station far from Greenwich, the local date of a dawn is the UTC day
/// before or after it — Sydney's is the previous UTC evening. Of the three
/// instants it could be, this takes the one `to_local` (Unix seconds to
/// local date and minute) puts on `date`.
fn place_on_local_day(
    date: &str,
    utc_minute: u32,
    to_local: impl Fn(i64) -> Option<(String, i64)>,
) -> Option<i64> {
    use birdnet_core::civil::{days_from_civil, parse_civil};
    let c = parse_civil(date, "00:00:00")?;
    let day = days_from_civil(c.year, c.month, c.day);
    [day, day - 1, day + 1].into_iter().find_map(|utc_day| {
        let (local_date, minute) = to_local(utc_day * 86_400 + i64::from(utc_minute) * 60)?;
        (local_date == date).then_some(minute)
    })
}

/// Minutes after midnight of an `HH:MM:SS`.
fn minute_of_day(time: &str) -> Option<i64> {
    let hours: i64 = time.get(0..2)?.parse().ok()?;
    let minutes: i64 = time.get(3..5)?.parse().ok()?;
    Some(hours * 60 + minutes)
}

/// Query parameters shared by the per-day charts.
#[derive(Debug, Deserialize)]
struct DayChartQuery {
    start_date: Option<String>,
    end_date: Option<String>,
    species: Option<String>,
}

/// `GET /api/v2/analytics/time/dawn-onset?start_date=&end_date=&species=` —
/// for each day of the range, when the dawn chorus began, relative to civil
/// dawn.
///
/// Upstream's rule and shape: a day's onset is its third-earliest
/// detection, read only on a day with at least five; `onsetRelMinutes` is
/// that minute of the day less civil dawn's (negative is before dawn), and
/// `null` on a sparse day, a day with no civil dawn, or at a station with no
/// location. Every day of the range is listed, quiet ones with a count of 0,
/// so a chart's line breaks rather than joins across them.
async fn dawn_onset(State(state): State<AppState>, Query(q): Query<DayChartQuery>) -> Response {
    let (start, end) = match chart_range(q.start_date.as_deref(), q.end_date.as_deref()) {
        Ok(range) => range,
        Err(message) => return bad_request(message),
    };
    let species = species_filter(q.species.as_deref());
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                let days = birdnet_db::sqlite::day_onsets(
                    conn,
                    &start,
                    &end,
                    species.as_deref(),
                    ONSET_RANK,
                )?;
                let location = crate::routes::pages::station_location(conn);
                Ok::<_, birdnet_db::sqlite::connection::DbError>(dawn_onset_rows(
                    &start,
                    &end,
                    &days,
                    |date| location.and_then(|l| civil_dawn_minute(conn, l, date)),
                ))
            })
        })
        .await;
    match result {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => read_failed("dawn onset", &e),
    }
}

/// One row per day of `start..=end`: its date, detection count, and onset
/// relative to `dawn(date)` where both are known.
fn dawn_onset_rows(
    start: &str,
    end: &str,
    days: &[birdnet_db::sqlite::DayOnset],
    dawn: impl Fn(&str) -> Option<i64>,
) -> Vec<serde_json::Value> {
    use birdnet_core::civil::{date_string_from_days, days_from_civil, parse_civil};
    let day_number =
        |d: &str| parse_civil(d, "00:00:00").map(|c| days_from_civil(c.year, c.month, c.day));
    let (Some(first), Some(last)) = (day_number(start), day_number(end)) else {
        return Vec::new();
    };
    let by_date: std::collections::HashMap<&str, &birdnet_db::sqlite::DayOnset> =
        days.iter().map(|d| (d.date.as_str(), d)).collect();
    (first..=last)
        .map(|n| {
            let date = date_string_from_days(n);
            let day = by_date.get(date.as_str());
            let onset = day
                .filter(|d| d.count >= ONSET_MIN_DETECTIONS)
                .and_then(|d| d.onset_time.as_deref())
                .and_then(minute_of_day)
                .and_then(|onset| dawn(&date).map(|dawn| onset - dawn));
            json!({
                "date": date,
                "onsetRelMinutes": onset,
                "detectionCount": day.map_or(0, |d| d.count),
            })
        })
        .collect()
}

/// The heatmap's slot width for a range of `days` days: upstream's
/// `heatmapSlotResolution` — 15 minutes up to 90 days, 30 up to 180, then
/// 60, so a year is not drawn as 35 000 mostly empty cells.
const fn heatmap_slot_minutes(days: i64) -> u32 {
    if days > 180 {
        60
    } else if days > 90 {
        30
    } else {
        15
    }
}

/// Query parameters for the heatmap.
#[derive(Debug, Deserialize)]
struct HeatmapQuery {
    start_date: Option<String>,
    end_date: Option<String>,
    species: Option<String>,
    format: Option<String>,
}

/// `GET /api/v2/analytics/time/heatmap?start_date=&end_date=&species=&format=`
/// — detections by day and time of day.
///
/// Upstream's columnar, sparse shape: `{dates, slotResolutionMinutes,
/// cells: {dateIndex, slot, count}}`, where `dates` is every day of the
/// range, the three cell arrays run in parallel, and only cells with a
/// detection are listed, by day then slot. `format=csv` (any case) answers
/// the same cells as CSV — `date,slot,slot_start,count` — as an attachment.
async fn heatmap(State(state): State<AppState>, Query(q): Query<HeatmapQuery>) -> Response {
    use birdnet_core::civil::{date_string_from_days, days_from_civil, parse_civil};
    use std::fmt::Write as _;
    let (start, end) = match chart_range(q.start_date.as_deref(), q.end_date.as_deref()) {
        Ok(range) => range,
        Err(message) => return bad_request(message),
    };
    let day_number =
        |d: &str| parse_civil(d, "00:00:00").map_or(0, |c| days_from_civil(c.year, c.month, c.day));
    let (first, last) = (day_number(&start), day_number(&end));
    let slot_minutes = heatmap_slot_minutes(last - first + 1);
    let species = species_filter(q.species.as_deref());
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                birdnet_db::sqlite::slot_counts(
                    conn,
                    &start,
                    &end,
                    species.as_deref(),
                    slot_minutes,
                )
            })
        })
        .await;
    let cells = match result {
        Ok(cells) => cells,
        Err(e) => return read_failed("heatmap", &e),
    };
    let dates: Vec<String> = (first..=last).map(date_string_from_days).collect();
    if q.format
        .as_deref()
        .is_some_and(|f| f.eq_ignore_ascii_case("csv"))
    {
        let mut csv = String::from("date,slot,slot_start,count\n");
        for (date, slot, count) in &cells {
            let minutes = slot * slot_minutes;
            let _ = writeln!(
                csv,
                "{date},{slot},{:02}:{:02},{count}",
                minutes / 60,
                minutes % 60
            );
        }
        return (
            [
                (axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    "attachment; filename=\"activity-heatmap.csv\"",
                ),
            ],
            csv,
        )
            .into_response();
    }
    let index: std::collections::HashMap<&str, usize> = dates
        .iter()
        .enumerate()
        .map(|(i, d)| (d.as_str(), i))
        .collect();
    let (mut date_index, mut slot, mut count) = (Vec::new(), Vec::new(), Vec::new());
    for (date, s, n) in &cells {
        if let Some(&i) = index.get(date.as_str()) {
            date_index.push(i);
            slot.push(*s);
            count.push(*n);
        }
    }
    Json(json!({
        "dates": dates,
        "slotResolutionMinutes": slot_minutes,
        "cells": {"dateIndex": date_index, "slot": slot, "count": count},
    }))
    .into_response()
}

/// What an hour-of-day chart was asked for.
#[derive(Debug, PartialEq, Eq)]
struct HourChartRequest {
    start: String,
    end: String,
    species: Vec<String>,
    limit: u32,
}

/// Read an hour-of-day chart's query: upstream's range rules, its
/// `ParsePaginationLimit` against `(default, max)`, and a repeated
/// `species` (`?species=A&species=B`) trimmed, emptied of blanks, and
/// de-duplicated without regard to case, keeping the first spelling.
/// More than ten is a 400, as upstream's `validateBatchSize` answers.
fn hour_chart_request(query: Option<&str>, limits: (u32, u32)) -> Result<HourChartRequest, String> {
    let mut start = None;
    let mut end = None;
    let mut limit = None;
    let mut species: Vec<String> = Vec::new();
    for (key, value) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "start_date" => start = Some(value.into_owned()),
            "end_date" => end = Some(value.into_owned()),
            "limit" => limit = Some(value.into_owned()),
            "species" => {
                let name = value.trim();
                if !name.is_empty()
                    && !species
                        .iter()
                        .any(|s| s.to_lowercase() == name.to_lowercase())
                {
                    species.push(name.to_owned());
                }
            }
            _ => {}
        }
    }
    let (start, end) = chart_range(start.as_deref(), end.as_deref()).map_err(str::to_owned)?;
    if species.len() > MAX_SPECIES_SELECTION {
        return Err(format!(
            "Too many items requested. Maximum: {MAX_SPECIES_SELECTION}"
        ));
    }
    let limit = limit
        .and_then(|l| l.parse::<u32>().ok())
        .filter(|l| (1..=limits.1).contains(l))
        .unwrap_or(limits.0);
    Ok(HourChartRequest {
        start,
        end,
        species,
        limit,
    })
}

/// Answer an hour-of-day chart: read the request, the species' hourly
/// counts, and render each species with `row`.
async fn hour_chart(
    state: &AppState,
    query: Option<&str>,
    limits: (u32, u32),
    row: fn(&birdnet_db::sqlite::SpeciesHours) -> serde_json::Value,
) -> Response {
    let request = match hour_chart_request(query, limits) {
        Ok(request) => request,
        Err(message) => return bad_request(&message),
    };
    let result = state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                birdnet_db::sqlite::species_hours(
                    conn,
                    &request.start,
                    &request.end,
                    &request.species,
                    request.limit,
                )
            })
        })
        .await;
    match result {
        Ok(rows) => Json(rows.iter().map(row).collect::<Vec<_>>()).into_response(),
        Err(e) => read_failed("hourly activity", &e),
    }
}

/// `GET /api/v2/analytics/time/distribution/species?start_date=&end_date=&species=&limit=`
/// — who sings when: each species' detections by local hour as fractions
/// of its total, so species of any volume compare by shape.
///
/// Upstream's shape: `[{scientificName, commonName, buckets, total}]` with
/// 24 buckets, midnight first; the 5 most detected species by default, at
/// most 10, or those named by a repeated `species`.
async fn species_hour_distribution(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> Response {
    hour_chart(&state, query.as_deref(), RIDGELINE_SPECIES, |r| {
        #[allow(clippy::cast_precision_loss)]
        let buckets: Vec<f64> = r.hours.iter().map(|&h| h as f64 / r.total as f64).collect();
        json!({
            "scientificName": r.sci_name,
            "commonName": r.com_name,
            "buckets": buckets,
            "total": r.total,
        })
    })
    .await
}

/// `GET /api/v2/analytics/time/succession?start_date=&end_date=&species=&limit=`
/// — each species' detections by local hour, as counts, for a streamgraph
/// whose band widths are volumes.
///
/// Upstream's shape: `[{scientificName, commonName, counts, total}]` with 24
/// counts, midnight first; the 6 most detected species by default, at most
/// 10, or those named by a repeated `species`.
async fn succession(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    hour_chart(&state, query.as_deref(), SUCCESSION_SPECIES, |r| {
        json!({
            "scientificName": r.sci_name,
            "commonName": r.com_name,
            "counts": r.hours,
            "total": r.total,
        })
    })
    .await
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

    fn onset(date: &str, count: i64, time: Option<&str>) -> birdnet_db::sqlite::DayOnset {
        birdnet_db::sqlite::DayOnset {
            date: date.to_owned(),
            count,
            onset_time: time.map(str::to_owned),
        }
    }

    /// Every day of the range is listed; the onset is relative to that day's
    /// civil dawn, and null on a sparse day or one with no dawn.
    #[test]
    fn each_day_gets_its_onset_against_its_own_dawn() {
        let days = [
            onset("2026-05-01", 5, Some("04:30:00")),
            onset("2026-05-02", 4, Some("04:00:00")),
            onset("2026-05-04", 9, Some("05:00:00")),
        ];
        let dawn = |d: &str| match d {
            "2026-05-01" => Some(4 * 60 + 45),
            "2026-05-04" => None,
            _ => Some(5 * 60),
        };
        let rows = dawn_onset_rows("2026-05-01", "2026-05-04", &days, dawn);
        assert_eq!(
            serde_json::Value::Array(rows),
            json!([
                {"date": "2026-05-01", "onsetRelMinutes": -15, "detectionCount": 5},
                {"date": "2026-05-02", "onsetRelMinutes": null, "detectionCount": 4},
                {"date": "2026-05-03", "onsetRelMinutes": null, "detectionCount": 0},
                {"date": "2026-05-04", "onsetRelMinutes": null, "detectionCount": 9},
            ])
        );
    }

    fn sqlite() -> rusqlite::Connection {
        rusqlite::Connection::open_in_memory().unwrap()
    }

    /// Civil dawn lands on the requested local day and before sunrise, at a
    /// mid-latitude station; and there is none in an Arctic midsummer.
    #[test]
    fn civil_dawn_is_found_on_the_local_day_and_not_in_polar_day() {
        let conn = sqlite();
        // Process zone: compare against the solver's own UTC answer shifted
        // by the zone's offset on that date, so the test holds in any TZ.
        let london = birdnet_scheduler::Location::new(51.5, -0.13).unwrap();
        let minute = civil_dawn_minute(&conn, london, "2026-06-21").expect("a dawn in June");
        let solar = birdnet_scheduler::SolarDay::for_date(london, 2026, 6, 21).unwrap();
        let offset_min: i64 = conn
            .query_row(
                "SELECT CAST(ROUND((julianday('2026-06-21 03:00:00', 'localtime')
                                   - julianday('2026-06-21 03:00:00')) * 1440) AS INTEGER)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let expected = (i64::from(solar.civil_dawn_utc_min.unwrap()) + offset_min).rem_euclid(1440);
        assert_eq!(minute, expected);
        assert!(
            i64::from(solar.civil_dawn_utc_min.unwrap())
                < i64::from(solar.sunrise_utc_min.unwrap()),
            "precondition: civil dawn precedes sunrise"
        );
        let svalbard = birdnet_scheduler::Location::new(78.2, 15.6).unwrap();
        assert_eq!(civil_dawn_minute(&conn, svalbard, "2026-06-21"), None);
        assert_eq!(civil_dawn_minute(&conn, london, "2026-02-30"), None);
    }

    /// A zone `offset_hours` east of UTC, as `place_on_local_day` reads one.
    fn zone(offset_hours: i64) -> impl Fn(i64) -> Option<(String, i64)> {
        move |instant| {
            let local = instant + offset_hours * 3600;
            Some((
                birdnet_core::civil::date_string_from_days(local.div_euclid(86_400)),
                local.rem_euclid(86_400) / 60,
            ))
        }
    }

    /// The instant on the local day is the one taken: Sydney's dawn (UTC+10)
    /// is the previous UTC evening, Honolulu's evening (UTC-10) the next UTC
    /// morning, and at UTC the same day.
    #[test]
    fn an_event_is_placed_on_its_local_day() {
        // 19:30 UTC is 05:30 the next day in Sydney.
        assert_eq!(
            place_on_local_day("2026-06-21", 19 * 60 + 30, zone(10)),
            Some(5 * 60 + 30)
        );
        // 04:00 UTC is 18:00 the previous day in Honolulu.
        assert_eq!(
            place_on_local_day("2026-06-21", 4 * 60, zone(-10)),
            Some(18 * 60)
        );
        assert_eq!(
            place_on_local_day("2026-06-21", 4 * 60, zone(0)),
            Some(4 * 60)
        );
        assert_eq!(place_on_local_day("2026-06-21", 4 * 60, |_| None), None);
    }

    /// The date check is what matters across a change of offset. Sydney
    /// leaves daylight saving at 03:00 on 5 April 2026 (16:00 UTC on the
    /// 4th). Its dawn on the 4th, 06:00 AEDT, is 19:00 UTC on the 3rd; the
    /// same UTC minute a day later is already 05:00 AEST on the 5th. Taking
    /// the first candidate without checking its date would report 05:00.
    #[test]
    fn across_a_change_of_offset_the_instant_on_the_local_day_is_taken() {
        let change = birdnet_core::civil::days_from_civil(2026, 4, 4) * 86_400 + 16 * 3600;
        let sydney = |instant: i64| {
            let offset = if instant < change { 11 } else { 10 };
            zone(offset)(instant)
        };
        assert_eq!(
            place_on_local_day("2026-04-04", 19 * 60, sydney),
            Some(6 * 60)
        );
    }

    /// Upstream's thresholds: up to 90 days in 15-minute slots, up to 180 in
    /// 30, beyond that in hours.
    #[test]
    fn the_heatmap_coarsens_as_the_range_widens() {
        assert_eq!(heatmap_slot_minutes(1), 15);
        assert_eq!(heatmap_slot_minutes(90), 15);
        assert_eq!(heatmap_slot_minutes(91), 30);
        assert_eq!(heatmap_slot_minutes(180), 30);
        assert_eq!(heatmap_slot_minutes(181), 60);
        assert_eq!(heatmap_slot_minutes(366), 60);
    }

    /// Upstream's reading of a repeated `species`: trimmed, blanks dropped,
    /// case-insensitive duplicates collapsed to the first spelling; more
    /// than ten refused. And its limit: out of range is the default.
    #[test]
    fn an_hour_chart_reads_its_species_and_limit_like_upstream() {
        let r = hour_chart_request(
            Some("start_date=2026-05-01&species=%20Turdus%20merula%20&species=&species=turdus+MERULA&species=Robin&species=%C3%96&species=%C3%B6&limit=3"),
            (5, 10),
        )
        .expect("valid");
        assert_eq!(
            r.species,
            ["Turdus merula", "Robin", "Ö"],
            "not ASCII alone"
        );
        assert_eq!(r.limit, 3);
        assert_eq!(r.end, "2026-05-31");
        for (limit, expected) in [("0", 5), ("10", 10), ("11", 5), ("x", 5)] {
            let r = hour_chart_request(
                Some(&format!("start_date=2026-05-01&limit={limit}")),
                (5, 10),
            )
            .expect("valid");
            assert_eq!(r.limit, expected, "limit={limit}");
        }
        let eleven = (0..11)
            .map(|i| format!("&species=S{i}"))
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(
            hour_chart_request(Some(&format!("start_date=2026-05-01{eleven}")), (5, 10)),
            Err("Too many items requested. Maximum: 10".to_owned())
        );
        let ten = (0..10)
            .map(|i| format!("&species=S{i}"))
            .collect::<Vec<_>>()
            .concat();
        assert!(hour_chart_request(Some(&format!("start_date=2026-05-01{ten}")), (5, 10)).is_ok());
        assert!(
            hour_chart_request(None, (5, 10)).is_err(),
            "start_date is required"
        );
    }
}
