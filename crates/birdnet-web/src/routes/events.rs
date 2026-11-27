//! Events and site effort, read-only (decisions 1 and 2 of 2026-11-25; MSC-02,
//! RES-18, N06-02).
//!
//! * `GET /api/v2/events?from=&to=&tolerance_secs=&sites=` — the events of a range
//!   of UTC dates (at most [`MAX_EVENT_DAYS`]), each with its member rows.
//! * `GET /api/v2/events/daily?from=&to=&tolerance_secs=&sites=` — events per UTC
//!   date, site and species (at most [`MAX_SUMMARY_DAYS`]).
//! * `GET /api/v2/effort/sites?from=&to=&sites=` — each site's listening per local
//!   hour as bounds, with sensor time apart (at most [`MAX_SUMMARY_DAYS`]).
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

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use birdnet_db::sqlite::queries::events::{
    self, DEFAULT_TOLERANCE_SECS, EVENT_POLICY_VERSION, Event, EventsRead, MAX_TOLERANCE_SECS,
    SiteMap,
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

    #[test]
    fn a_range_is_bounded() {
        assert!(parse_range(Some("2026-05-01"), Some("2026-05-07"), 7).is_ok());
        assert!(parse_range(Some("2026-05-01"), Some("2026-05-08"), 7).is_err());
        assert!(parse_range(Some("2026-05-02"), Some("2026-05-01"), 7).is_err());
        assert!(parse_range(None, Some("2026-05-01"), 7).is_err());
    }
}
