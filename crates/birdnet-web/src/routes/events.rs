//! Events and site effort, read-only (decisions 1 and 2 of 2026-11-25; MSC-02,
//! RES-18, N06-02).
//!
//! * `GET /api/v2/events?from=&to=&tolerance_secs=&sites=` — the events of a range
//!   of UTC dates (at most [`MAX_EVENT_DAYS`]), each with its member rows.
//! * `GET /api/v2/events/daily?from=&to=&tolerance_secs=&sites=` — events per UTC
//!   date, site and species (at most [`MAX_SUMMARY_DAYS`]).
//! * `GET /api/v2/effort/sites?from=&to=&sites=` — each site's listening per local
//!   hour as bounds, with sensor time apart (at most [`MAX_SUMMARY_DAYS`]).
//! * `GET /api/v2/analytics/rates?from=&to=&bin=&tolerance_secs=&sites=&source=&min_confidence=&species=`
//!   — events per hour of each site's listening, per site, species and bin (`day`,
//!   `week`, `hour_of_day`), with listening bounds and intervals (local dates, at
//!   most [`MAX_RATE_DAYS`]).
//! * `GET /api/v2/analytics/rates/compare?days=&end=&…` — the last `days` complete
//!   days against the `days` before, as a rate ratio with an interval.
//! * `GET /api/v2/analytics/sunrise-relative?from=&to=&bin_minutes=&…` — events by
//!   minutes from each day's own sunrise.
//! * `GET /api/v2/analytics/cooccurrence?days=&limit=&min_shared=&sites=&source=&min_confidence=&seed=`
//!   — pairs heard together at one site, with their chance level, q-value and
//!   verdict.
//!
//! The rule is `birdnet_db::sqlite::queries::events`; this module only parses,
//! validates and states the parameters. Every response that counts events states
//! the policy version, the tolerance, the site map it applied, and how many rows
//! were left out for want of an instant. An event is a call heard once or by
//! several sources, **not a bird**: nothing here names a count of events "birds".
//!
//! `sites` is the site map, `source:site` pairs separated by commas
//! (`sites=mic-north:yard,cam-1:yard`); a source it does not name is its own site,
//! which is also what an absent `sites` means. A source name containing a comma
//! cannot be named here.

use std::collections::{BTreeMap, BTreeSet};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use birdnet_db::sqlite::queries::correlation::{CooccurrenceQuery, cooccurrence_analysis};
use birdnet_db::sqlite::queries::events::{
    self, DEFAULT_TOLERANCE_SECS, EVENT_POLICY_VERSION, Event, EventsRead, MAX_TOLERANCE_SECS,
    RowFilter, SiteMap,
};
use birdnet_db::sqlite::queries::rates::{
    self, MAX_RATE_DAYS, RATE_METHOD, RateBin, RateQuery, RateRatio,
};
use birdnet_db::sqlite::queries::site_effort::{self, LISTENED_DAY, SiteHour};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::routes::is_valid_date;
use crate::state::AppState;

/// Longest range of `/events`, in days: a busy station's week is tens of
/// thousands of events, each with its members.
pub const MAX_EVENT_DAYS: i64 = 7;

/// Longest range of `/events/daily` and `/effort/sites`, in days.
pub const MAX_SUMMARY_DAYS: i64 = 31;

/// What an event is, in every response, so a count is not read as birds.
const UNIT: &str = "An event is the detections of one call: rows of one species at one site, \
at most one per source, all within the tolerance of one another. It is not an individual bird.";

/// Mount the routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/events", get(list_events))
        .route("/events/daily", get(daily_events))
        .route("/effort/sites", get(site_effort))
        .route("/analytics/rates", get(event_rates))
        .route("/analytics/rates/compare", get(compare_rates))
        .route("/analytics/cooccurrence", get(cooccurrence))
        .route("/analytics/sunrise-relative", get(sunrise_relative))
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    from: Option<String>,
    to: Option<String>,
    tolerance_secs: Option<f64>,
    sites: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EffortQuery {
    from: Option<String>,
    to: Option<String>,
    sites: Option<String>,
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

fn internal<E: std::fmt::Display>(e: &E) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": crate::routes::log_internal("internal error", e) })),
    )
        .into_response()
}

/// Parse `sites`: `source:site` pairs, comma-separated, split at the last colon
/// so a source name may contain one. A source named twice with different sites
/// is refused.
pub(crate) fn parse_sites(raw: Option<&str>) -> Result<SiteMap, String> {
    raw.map_or_else(|| Ok(SiteMap::identity()), SiteMap::parse)
}

/// `from..=to` as dates no more than `max_days` apart, as the epoch days of each.
fn parse_range(from: Option<&str>, to: Option<&str>, max_days: i64) -> Result<(i64, i64), String> {
    let (Some(from), Some(to)) = (from, to) else {
        return Err("from and to are required, as YYYY-MM-DD".to_owned());
    };
    if !is_valid_date(from) || !is_valid_date(to) {
        return Err("from and to are required, as YYYY-MM-DD".to_owned());
    }
    let day = crate::routes::pages::viz::kit::epoch_day;
    match (day(from), day(to)) {
        (Some(f), Some(t)) if (0..max_days).contains(&(t - f)) => Ok((f, t)),
        _ => Err(format!(
            "from must not be after to, and the range is at most {max_days} days"
        )),
    }
}

fn parse_tolerance(raw: Option<f64>) -> Result<f64, String> {
    let tol = raw.unwrap_or(DEFAULT_TOLERANCE_SECS);
    if tol.is_finite() && (0.0..=MAX_TOLERANCE_SECS).contains(&tol) {
        Ok(tol)
    } else {
        Err(format!(
            "tolerance_secs is a number of seconds from 0 to {MAX_TOLERANCE_SECS}"
        ))
    }
}

/// The parameters every event response states.
fn policy(tol: f64, sites: &SiteMap, events: &[Event]) -> Value {
    let seen = events
        .iter()
        .flat_map(|e| e.members.iter().filter_map(|m| m.source.as_deref()));
    json!({
        "version": EVENT_POLICY_VERSION,
        "tolerance_secs": tol,
        "default_tolerance_secs": DEFAULT_TOLERANCE_SECS,
        "site_map": sites.resolved(seen),
        "site_map_default": "every source not named is its own site",
        "unit": UNIT,
        "instant": "detected_at_utc; a row without one is in no event and is counted in rows_without_instant",
        "rows_without_source": "a row with no recorded source is an event of its own, at site null",
    })
}

fn site_counts_json(events: &[Event]) -> Value {
    Value::Array(
        events::site_counts(events)
            .into_iter()
            .map(|c| {
                json!({
                    "site": c.site,
                    "sources": c.sources,
                    "events": c.events,
                    "rows": c.rows,
                })
            })
            .collect(),
    )
}

/// A range's events, read, with the parameters they were counted with.
struct Counted {
    tol: f64,
    sites: SiteMap,
    read: EventsRead,
    range: Value,
}

impl Counted {
    /// The fields every event response carries, and `key` holding `value`.
    fn body(&self, key: &str, value: Value) -> Value {
        let mut body = json!({
            "policy": policy(self.tol, &self.sites, &self.read.events),
            "range": self.range,
            "rows_without_instant": self.read.rows_without_instant,
            "total_events": self.read.events.len(),
            "sites": site_counts_json(&self.read.events),
        });
        body[key] = value;
        body
    }
}

/// Validate, then read the events of `from..=to` (UTC dates).
async fn read_events(
    state: &AppState,
    q: EventsQuery,
    max_days: i64,
) -> Result<Counted, Box<Response>> {
    let bad = |m: String| Box::new(bad_request(&m));
    let (f, t) = parse_range(q.from.as_deref(), q.to.as_deref(), max_days).map_err(bad)?;
    let tol = parse_tolerance(q.tolerance_secs).map_err(bad)?;
    let sites = parse_sites(q.sites.as_deref()).map_err(bad)?;
    let (from_utc, to_utc) = (f * 86_400, (t + 1) * 86_400);
    let map = sites.clone();
    let read = state
        .run_blocking_heavy(move |s| {
            s.with_read_db(|conn| events::events_between(conn, from_utc, to_utc, tol, &map))
        })
        .await
        .map_err(|e| Box::new(internal(&e)))?;
    let range = json!({
        "from": q.from,
        "to": q.to,
        "from_utc": from_utc,
        "to_utc": to_utc,
        "read_from_utc": read.read_from,
        "read_to_utc": read.read_to,
        "edges_settled": read.edges_settled,
    });
    Ok(Counted {
        tol,
        sites,
        read,
        range,
    })
}

/// `GET /api/v2/events`.
async fn list_events(State(state): State<AppState>, Query(q): Query<EventsQuery>) -> Response {
    let counted = match read_events(&state, q, MAX_EVENT_DAYS).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let items: Vec<Value> = counted
        .read
        .events
        .iter()
        .map(|e| {
            json!({
                "event_id": e.event_id,
                "site": e.site,
                "sci_name": e.sci_name,
                "com_name": e.com_name,
                "first_utc": e.first,
                "last_utc": e.last,
                "members": e.members.iter().map(|m| json!({
                    "row_id": m.row_id,
                    "source": m.source,
                    "instant_utc": m.instant,
                    "confidence": m.confidence,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    (
        StatusCode::OK,
        Json(counted.body("events", Value::Array(items))),
    )
        .into_response()
}

/// `GET /api/v2/events/daily`.
async fn daily_events(State(state): State<AppState>, Query(q): Query<EventsQuery>) -> Response {
    let counted = match read_events(&state, q, MAX_SUMMARY_DAYS).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let days: Vec<Value> = events::day_species_counts(&counted.read.events)
        .into_iter()
        .map(|d| {
            json!({
                "date": d.date,
                "site": d.site,
                "sci_name": d.sci_name,
                "com_name": d.com_name,
                "events": d.events,
                "rows": d.rows,
            })
        })
        .collect();
    let mut body = counted.body("days", Value::Array(days));
    body["day"] = json!("the UTC date of an event's first member");
    (StatusCode::OK, Json(body)).into_response()
}

fn site_hour_json(h: &SiteHour) -> Value {
    json!({
        "date": h.date,
        "hour": h.hour,
        "site": h.site,
        "sources": h.sources,
        "listened_secs_lower_bound": h.lower_bound_secs,
        "listened_secs_upper_bound": h.upper_bound_secs,
        "sensor_secs": h.sensor_secs,
        "sensor_hours": h.sensor_secs / 3600.0,
        "hour_capacity_secs": h.capacity_secs,
    })
}

/// `GET /api/v2/effort/sites`.
async fn site_effort(State(state): State<AppState>, Query(q): Query<EffortQuery>) -> Response {
    if let Err(m) = parse_range(q.from.as_deref(), q.to.as_deref(), MAX_SUMMARY_DAYS) {
        return bad_request(&m);
    }
    let sites = match parse_sites(q.sites.as_deref()) {
        Ok(s) => s,
        Err(m) => return bad_request(&m),
    };
    let (from, to) = (
        q.from.clone().unwrap_or_default(),
        q.to.clone().unwrap_or_default(),
    );
    let map = sites.clone();
    let result = state
        .run_blocking(move |s| {
            s.with_read_db(|conn| site_effort::site_effort_hours(conn, &from, &to, &map))
        })
        .await;
    let hours = match result {
        Ok(h) => h,
        Err(e) => return internal(&e),
    };
    let seen = hours
        .iter()
        .flat_map(|h| h.sources.iter().map(String::as_str));
    let body = json!({
        "site_map": sites.resolved(seen),
        "site_map_default": "every source not named is its own site",
        "from": q.from,
        "to": q.to,
        "dates": "local dates and hours, as the effort ledger keeps them",
        "bounds": "The ledger keeps seconds analysed per source, so a site's listening (the union of its \
    sources' intervals) is not known: it is at least the best-covered source (listened_secs_lower_bound) and at \
    most the sum, capped at the hour's real length (listened_secs_upper_bound). sensor_secs is the sum, a \
    denominator for per-sensor rates, and is not the site's listening.",
        "listened_day_rule": LISTENED_DAY.name(),
        "hours": hours.iter().map(site_hour_json).collect::<Vec<_>>(),
    });
    (StatusCode::OK, Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// Event rates, their comparison, and co-occurrence with its chance level
// (MSC-02, RES-18, N26-06, UIC-02, UIC-11, UIR-34, RES-16, UIR-22, CHT-24)
// ---------------------------------------------------------------------------

/// What every exploratory analysis says about itself.
const EXPLORATORY: &str = "Exploratory and associational. A rate of acoustic events is not a count \
of birds, an abundance or a density; two species heard together are not shown to interact; a change \
over time is not shown to have a cause. Confidence is a classifier score, not a probability.";

/// Parameters every rate route reads.
#[derive(Debug, Deserialize)]
struct RatesQuery {
    from: Option<String>,
    to: Option<String>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    bin: Option<String>,
    tolerance_secs: Option<f64>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    sites: Option<String>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    source: Option<String>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    min_confidence: Option<f64>,
    /// Scientific name.
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    species: Option<String>,
    /// `/rates/compare`: days per window.
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    days: Option<u32>,
    /// `/rates/compare`: the last day of the recent window (default yesterday,
    /// so a day still under way is never compared).
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    end: Option<String>,
    /// `/sunrise-relative`: bin width in minutes.
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    bin_minutes: Option<u32>,
}

/// `min_confidence`, when given, is a score in `[0, 1]`.
fn parse_min_confidence(raw: Option<f64>) -> Result<Option<f64>, String> {
    match raw {
        Some(c) if !(c.is_finite() && (0.0..=1.0).contains(&c)) => {
            Err("min_confidence is a score from 0 to 1".to_owned())
        }
        other => Ok(other),
    }
}

fn row_filter(q: &RatesQuery) -> Result<RowFilter, String> {
    Ok(RowFilter {
        source: q.source.clone(),
        min_confidence: parse_min_confidence(q.min_confidence)?,
        sci_name: q.species.clone(),
    })
}

fn rate_policy(tol: f64, sites: &SiteMap, filter: &RowFilter, bin: RateBin) -> Value {
    json!({
        "method": RATE_METHOD,
        "event_policy": EVENT_POLICY_VERSION,
        "tolerance_secs": tol,
        "site_map": sites.explicit(),
        "site_map_default": "every source not named is its own site",
        "filter": filter,
        "bin": bin.name(),
        "unit": "events per hour of the site's listening",
        "numerator": "events (one per call heard, however many sources at the site heard it) in local \
    hours the site has a ledger row for",
        "denominator": "the site's listening, as bounds: at least its best-covered source, at most the \
    sum of its sources capped at the hour. rate_low divides by the upper bound and rate_high by the lower; \
    sensor_secs (the sum) is reported and is never the denominator",
        "interval": "exact 95 % Poisson interval of the events, over the upper and lower bounds; a floor \
    on the uncertainty, since events of one bird are clustered",
        "null_is_not_zero": "a point with status no_effort has no rate; a measured point with no events \
    has a rate of 0",
        "exploratory": EXPLORATORY,
    })
}

/// `GET /api/v2/analytics/rates`.
async fn event_rates(State(state): State<AppState>, Query(q): Query<RatesQuery>) -> Response {
    if let Err(m) = parse_range(q.from.as_deref(), q.to.as_deref(), MAX_RATE_DAYS) {
        return bad_request(&m);
    }
    let Some(bin) = q.bin.as_deref().map_or(Some(RateBin::Day), RateBin::parse) else {
        return bad_request("bin is day, week or hour_of_day");
    };
    let tol = match parse_tolerance(q.tolerance_secs) {
        Ok(t) => t,
        Err(m) => return bad_request(&m),
    };
    let (sites, filter) = match (parse_sites(q.sites.as_deref()), row_filter(&q)) {
        (Ok(s), Ok(f)) => (s, f),
        (Err(m), _) | (_, Err(m)) => return bad_request(&m),
    };
    let rq = RateQuery {
        from: q.from.clone().unwrap_or_default(),
        to: q.to.clone().unwrap_or_default(),
        bin,
        tolerance_secs: tol,
        sites: sites.clone(),
        filter: filter.clone(),
    };
    let result = state
        .run_blocking_heavy(move |s| s.with_read_db(|conn| rates::rates_between(conn, &rq)))
        .await;
    let read = match result {
        Ok(r) => r,
        Err(e) => return internal(&e),
    };
    let body = json!({
        "policy": rate_policy(tol, &sites, &filter, bin),
        "from": q.from,
        "to": q.to,
        "dates": "local dates and hours, as the effort ledger keeps them",
        "rows_without_instant": read.rows_without_instant,
        "edges_settled": read.edges_settled,
        "points": read.points,
    });
    (StatusCode::OK, Json(body)).into_response()
}

/// One window's events and listening bounds (seconds).
type Window = (u64, f64, f64);

/// Site, scientific name and common name.
type SpeciesAtSite = (String, String, String);

/// `GET /api/v2/analytics/rates/compare`: the last `days` complete days against
/// the `days` before them, per site and species and for the site's whole
/// output, as a rate ratio with an interval and a direction only when the
/// interval excludes no change.
async fn compare_rates(State(state): State<AppState>, Query(q): Query<RatesQuery>) -> Response {
    let days = i64::from(q.days.unwrap_or(30));
    if !(1..=MAX_RATE_DAYS / 2).contains(&days) {
        return bad_request(&format!("days is from 1 to {}", MAX_RATE_DAYS / 2));
    }
    if q.end.as_deref().is_some_and(|e| !is_valid_date(e)) {
        return bad_request("end is a date, YYYY-MM-DD");
    }
    let tol = match parse_tolerance(q.tolerance_secs) {
        Ok(t) => t,
        Err(m) => return bad_request(&m),
    };
    let (sites, filter) = match (parse_sites(q.sites.as_deref()), row_filter(&q)) {
        (Ok(s), Ok(f)) => (s, f),
        (Err(m), _) | (_, Err(m)) => return bad_request(&m),
    };
    let (map, f, end) = (sites.clone(), filter.clone(), q.end.clone());
    let result = state
        .run_blocking_heavy(move |s| {
            s.with_read_db(|conn| -> Result<_, birdnet_db::sqlite::DbError> {
                let end: String = match end {
                    Some(e) => e,
                    None => {
                        conn.query_row("SELECT date('now', 'localtime', '-1 day')", [], |r| {
                            r.get(0)
                        })?
                    }
                };
                // `end` is a validated date or SQLite's own; one that names no day
                // is refused rather than read as 1970.
                let Some(e) = crate::routes::pages::viz::kit::epoch_day(&end) else {
                    return Ok(None);
                };
                let (split, start) = (e - days + 1, e - 2 * days + 1);
                let rq = RateQuery {
                    from: events::utc_date(start * 86_400),
                    to: end.clone(),
                    bin: RateBin::Day,
                    tolerance_secs: tol,
                    sites: map,
                    filter: f,
                };
                let read = rates::rates_between(conn, &rq)?;
                Ok(Some((read, events::utc_date(split * 86_400), rq.from, end)))
            })
        })
        .await;
    let (read, split, start, end) = match result {
        Ok(Some(r)) => r,
        Ok(None) => return bad_request("end is a date, YYYY-MM-DD"),
        Err(e) => return internal(&e),
    };
    // (site, species) -> [earlier, recent] (events, lower, upper); and the site's
    // listening per window, counted once per day however many species.
    let mut by: BTreeMap<SpeciesAtSite, [Window; 2]> = BTreeMap::new();
    let mut listening: BTreeMap<(String, String), (usize, f64, f64)> = BTreeMap::new();
    let mut site_events: BTreeMap<String, [u64; 2]> = BTreeMap::new();
    let mut seen_day: BTreeSet<(String, String)> = BTreeSet::new();
    let mut species_heard: BTreeMap<String, [BTreeSet<String>; 2]> = BTreeMap::new();
    for p in &read.points {
        let w = usize::from(p.bin >= split);
        let slot = &mut by
            .entry((p.site.clone(), p.sci_name.clone(), p.com_name.clone()))
            .or_default()[w];
        slot.0 += p.events;
        slot.1 += p.listened_lower_secs;
        slot.2 += p.listened_upper_secs;
        site_events.entry(p.site.clone()).or_default()[w] += p.events;
        if p.events > 0 {
            species_heard.entry(p.site.clone()).or_default()[w].insert(p.sci_name.clone());
        }
        if seen_day.insert((p.site.clone(), p.bin.clone())) {
            let l = listening
                .entry((
                    p.site.clone(),
                    if w == 1 { "recent" } else { "earlier" }.to_owned(),
                ))
                .or_default();
            l.0 += usize::from(p.listened_lower_secs > 0.0);
            l.1 += p.listened_lower_secs;
            l.2 += p.listened_upper_secs;
        }
    }
    let window = |site: &str, name: &str| {
        listening
            .get(&(site.to_owned(), name.to_owned()))
            .copied()
            .unwrap_or_default()
    };
    let direction = |r: &RateRatio| match (r.ratio_lower, r.ratio_upper) {
        (Some(lo), _) if lo > 1.0 => "busier",
        (_, Some(hi)) if hi < 1.0 => "quieter",
        _ if r.ratio_lower.is_none() => "cannot_tell",
        _ => "no_clear_change",
    };
    let sites_json: Vec<Value> = site_events
        .iter()
        .map(|(site, ev)| {
            let (re, ea) = (window(site, "recent"), window(site, "earlier"));
            let r = rates::rate_ratio((ev[1], re.1, re.2), (ev[0], ea.1, ea.2));
            let heard = species_heard.get(site);
            json!({
                "site": site,
                "all_species": r,
                "direction": direction(&r),
                "listened_days": {"earlier": ea.0, "recent": re.0},
                "species_heard": {
                    "earlier": heard.map_or(0, |h| h[0].len()),
                    "recent": heard.map_or(0, |h| h[1].len()),
                },
            })
        })
        .collect();
    let species_json: Vec<Value> = by
        .iter()
        .map(|((site, sci, com), w)| {
            let r = rates::rate_ratio(w[1], w[0]);
            json!({
                "site": site, "sci_name": sci, "com_name": com,
                "ratio": r, "direction": direction(&r),
            })
        })
        .collect();
    let body = json!({
        "policy": rate_policy(tol, &sites, &filter, RateBin::Day),
        "windows": {
            "earlier": {"from": start, "to": events::utc_date((day_before(&split)) * 86_400)},
            "recent": {"from": split, "to": end},
            "days": days,
            "today": "excluded unless named as end: a day under way is not compared",
        },
        "ratio_method": "recent rate over earlier rate; 95 % interval from the conditional binomial of the \
    recent window's share of the events (Wilson), each end at the listening bounds that push it outwards. \
    direction is busier or quieter only when the interval excludes 1",
        "sites": sites_json,
        "species": species_json,
    });
    (StatusCode::OK, Json(body)).into_response()
}

fn day_before(date: &str) -> i64 {
    crate::routes::pages::viz::kit::epoch_day(date).map_or(0, |d| d - 1)
}

#[derive(Debug, Deserialize)]
struct CooccurrenceParams {
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    days: Option<u32>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    limit: Option<usize>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    min_shared: Option<u32>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    sites: Option<String>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    source: Option<String>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    min_confidence: Option<f64>,
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    seed: Option<u64>,
}

/// `GET /api/v2/analytics/cooccurrence`.
async fn cooccurrence(
    State(state): State<AppState>,
    Query(q): Query<CooccurrenceParams>,
) -> Response {
    let mut cq = CooccurrenceQuery::new(
        q.days.unwrap_or(90).clamp(1, 365),
        q.limit.unwrap_or(50).min(400),
        q.min_shared.unwrap_or(3),
    );
    cq.sites = match parse_sites(q.sites.as_deref()) {
        Ok(s) => s,
        Err(m) => return bad_request(&m),
    };
    cq.min_confidence = match parse_min_confidence(q.min_confidence) {
        Ok(c) => c,
        Err(m) => return bad_request(&m),
    };
    cq.source = q.source;
    if let Some(seed) = q.seed {
        cq.seed = seed;
    }
    let (days, limit, min_shared) = (cq.days, cq.limit, cq.min_shared);
    let result = state
        .run_blocking_heavy(move |s| s.with_read_db(|conn| cooccurrence_analysis(conn, &cq)))
        .await;
    let analysis = match result {
        Ok(a) => a,
        Err(e) => return internal(&e),
    };
    let body = json!({
        "days": days,
        "limit": limit,
        "min_shared": min_shared,
        "together": "the same five-minute block of the same day at the same site",
        "chance": "expected_shared is the mean over shifts of one species' days, circularly within \
    strata of consecutive days, which keeps each species' prevalence, time of day, season and persistence; \
    p_value is one-sided, q_value is Benjamini-Hochberg over pairs_tested",
        "verdicts": "stands_out (q at or below fdr_alpha and above chance), within_chance, or \
    too_little_data (no pair could reach fdr_alpha with these days)",
        "exploratory": EXPLORATORY,
        "analysis": analysis,
    });
    (StatusCode::OK, Json(body)).into_response()
}

/// The sunrise instant of UTC day `day` (days since 1970-01-01) at `location`,
/// from the scheduler's solar calculation; `None` on a day the sun does not
/// rise or set there.
fn sunrise_instant(location: birdnet_scheduler::Location, day: i64) -> Option<i64> {
    let date = events::utc_date(day * 86_400);
    let y: u32 = date.get(0..4)?.parse().ok()?;
    let m: u32 = date.get(5..7)?.parse().ok()?;
    let d: u32 = date.get(8..10)?.parse().ok()?;
    let solar = birdnet_scheduler::SolarDay::for_date(location, y, m, d).ok()?;
    Some(day * 86_400 + i64::from(solar.sunrise_utc_min?) * 60)
}

/// `GET /api/v2/analytics/sunrise-relative?from=&to=&bin_minutes=&tolerance_secs=&sites=&source=&min_confidence=&species=`:
/// each site and species' events by minutes from **that day's** sunrise
/// (RES-17, UIR-18), from four hours before to eight after, in bins of
/// `bin_minutes` (default 10). Without station coordinates there is no
/// sunrise to place against: 409, never a clock-hour stand-in.
async fn sunrise_relative(State(state): State<AppState>, Query(q): Query<RatesQuery>) -> Response {
    let (f, t) = match parse_range(q.from.as_deref(), q.to.as_deref(), MAX_RATE_DAYS) {
        Ok(r) => r,
        Err(m) => return bad_request(&m),
    };
    let bin = i64::from(q.bin_minutes.unwrap_or(10)).clamp(1, 120);
    let tol = match parse_tolerance(q.tolerance_secs) {
        Ok(t) => t,
        Err(m) => return bad_request(&m),
    };
    let (sites, filter) = match (parse_sites(q.sites.as_deref()), row_filter(&q)) {
        (Ok(s), Ok(f)) => (s, f),
        (Err(m), _) | (_, Err(m)) => return bad_request(&m),
    };
    let (map, fl) = (sites.clone(), filter.clone());
    let result = state
        .run_blocking_heavy(move |s| {
            s.with_read_db(|conn| {
                let location = crate::routes::pages::station_location(conn);
                let read = events::events_between_filtered(
                    conn,
                    f * 86_400,
                    (t + 1) * 86_400,
                    tol,
                    &map,
                    &fl,
                )?;
                Ok::<_, birdnet_db::sqlite::DbError>((location, read))
            })
        })
        .await;
    let (location, read) = match result {
        Ok(r) => r,
        Err(e) => return internal(&e),
    };
    let Some(location) = location else {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "the station has no coordinates, so there is no sunrise to place events against",
            })),
        )
            .into_response();
    };
    let mut by: BTreeMap<(Option<String>, String, String), Vec<i64>> = BTreeMap::new();
    for e in &read.events {
        by.entry((e.site.clone(), e.sci_name.clone(), e.com_name.clone()))
            .or_default()
            .push(e.first);
    }
    let sunrise = |d: i64| sunrise_instant(location, d);
    let rows: Vec<Value> = by
        .iter()
        .map(|((site, sci, com), instants)| {
            json!({
                "site": site, "sci_name": sci, "com_name": com,
                "events": instants.len(),
                "histogram": birdnet_behavioral::solar_relative::histogram(
                    instants, sunrise, bin, -240, 480,
                ),
            })
        })
        .collect();
    let body = json!({
        "policy": rate_policy(tol, &sites, &filter, RateBin::Day),
        "method": birdnet_behavioral::solar_relative::SUNRISE_RELATIVE_METHOD,
        "axis": "minutes from the sunrise of each event's own day (the nearest sunrise to it), \
    computed from the station's coordinates; not clock hours, and not today's sunrise",
        "counting": "events, by their first member's instant; not birds",
        "from": q.from, "to": q.to, "dates": "UTC dates",
        "rows_without_instant": read.rows_without_instant,
        "species": rows,
    });
    (StatusCode::OK, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_parse_as_pairs_and_refuse_contradictions() {
        let m = parse_sites(Some("a:yard, b:yard,rtsp:cam:field")).expect("parse");
        assert_eq!(m.site_of("a"), "yard");
        assert_eq!(m.site_of("rtsp:cam"), "field");
        assert_eq!(m.site_of("other"), "other");
        assert!(parse_sites(Some("a:yard,a:field")).is_err());
        assert!(parse_sites(Some("a:yard,a:yard")).is_ok());
        assert!(parse_sites(Some("nocolon")).is_err());
        assert!(parse_sites(Some(":yard")).is_err());
        assert_eq!(parse_sites(None).expect("none"), SiteMap::identity());
    }

    #[test]
    fn the_tolerance_is_bounded_and_defaults_to_the_window_and_a_second() {
        assert!((parse_tolerance(None).expect("default") - 4.0).abs() < f64::EPSILON);
        assert!(parse_tolerance(Some(-1.0)).is_err());
        assert!(parse_tolerance(Some(f64::NAN)).is_err());
        assert!(parse_tolerance(Some(MAX_TOLERANCE_SECS + 1.0)).is_err());
        assert!(parse_tolerance(Some(5.5)).is_ok());
    }

    /// RES-17 / UIR-18 acceptance: a species that always sings ten minutes before
    /// sunrise, every day for 60 days across the spring equinox in London, is one bin
    /// on the sunrise-relative axis; on the clock (UTC, so no daylight saving even
    /// helps it) it smears over more than an hour, because sunrise moved.
    #[test]
    fn a_bird_at_sunrise_is_one_bin_against_each_days_own_sunrise() {
        let london = birdnet_scheduler::Location::new(51.5074, -0.1278).expect("location");
        let first = crate::routes::pages::viz::kit::epoch_day("2026-02-20").expect("day");
        // The fixture's sunrises come from the solar calculation directly, not from the
        // function under test, so a defect in that function cannot move both.
        let instants: Vec<i64> = (0..60)
            .map(|d| {
                let (y, m, dd) = birdnet_core::civil::civil_from_days(first + d);
                let solar = birdnet_scheduler::SolarDay::for_date(london, y, m, dd).expect("date");
                (first + d) * 86_400 + i64::from(solar.sunrise_utc_min.expect("sunrise")) * 60 - 600
            })
            .collect();
        let h = birdnet_behavioral::solar_relative::histogram(
            &instants,
            |d| sunrise_instant(london, d),
            10,
            -240,
            480,
        );
        let busy: Vec<usize> = (0..h.counts.len()).filter(|&i| h.counts[i] > 0).collect();
        assert_eq!(
            busy.len(),
            1,
            "a fixed offset from sunrise spread over {busy:?}: {h:?}"
        );
        assert_eq!(h.counts[busy[0]], 60);
        // The bin is -10..0 minutes: 23 bins of 10 after -240.
        assert_eq!(busy[0], 23, "{h:?}");
        // Counterpart: by clock minute the same instants span more than an hour.
        let minute = |t: &i64| t.rem_euclid(86_400) / 60;
        let span = instants.iter().map(minute).max().unwrap_or(0)
            - instants.iter().map(minute).min().unwrap_or(0);
        assert!(
            span > 60,
            "sunrise moved only {span} minutes: the fixture shows nothing"
        );
    }

    #[test]
    fn a_range_is_bounded() {
        assert!(parse_range(Some("2026-05-01"), Some("2026-05-07"), 7).is_ok());
        assert!(parse_range(Some("2026-05-01"), Some("2026-05-08"), 7).is_err());
        assert!(parse_range(Some("2026-05-02"), Some("2026-05-01"), 7).is_err());
        assert!(parse_range(None, Some("2026-05-01"), 7).is_err());
    }
}
