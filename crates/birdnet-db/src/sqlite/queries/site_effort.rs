//! How long a site listened, as bounds, and the one listened-day rule (decision 2
//! of 2026-11-25; N06-02, N26-05, N26-06, RES-26).
//!
//! # Why bounds
//!
//! The hourly ledger (`recording_effort_hourly`) holds seconds of audio analysed
//! per local date, hour and **source**. Two microphones at one site that each
//! analysed half an hour may have heard the same half hour or the two different
//! halves; the ledger does not say which, so the time the *site* listened (the
//! union of its sources' intervals) is not recoverable. What is certain:
//!
//! * at least the best-covered source's seconds (**lower bound**: the max);
//! * at most the sum, and at most the hour itself (**upper bound**:
//!   `min(capacity, sum)`, never below the lower bound).
//!
//! **Sensor time** (the sum over sources) is a different quantity, reported
//! separately: it is the denominator of a per-sensor rate, and adding a source
//! that hears less raises it (N26-06), so it must never be presented as the
//! site's listening.
//!
//! The capacity of an hour is its real length on this host's local clock: 3600 s,
//! 7200 s for the hour a fall-back night repeats, and nothing for the hour a
//! spring-forward night skips ([`local_hour_capacity`]). The ledger credits a
//! segment to the hour it starts in, so one source can be credited slightly more
//! than an hour holds; the upper bound is then the lower bound, never less.
//! Both bounds are on the ledger's credits, which are audio *analysed*
//! (`seconds_analysed`), not time a source was up.
//!
//! # One listened-day rule
//!
//! [`LISTENED_DAY`] is the rule, by name; [`is_listened_day`] applies it. It is
//! the rule `birdnet_timeseries::queries::trend::is_listened_day` and its SQL
//! apply (half the station's typical day, the median over dates with audio), with
//! the share held equal to `trend::EFFORT_FLOOR_SHARE` by a test in
//! `birdnet-web`, which depends on both crates.
//!
//! The share is `birdnet_behavioral::phenology::LISTENED_DAY_SHARE`, which
//! phenology's effort-gap flag (`phenology_timing_sql`) also reads, and the
//! calendar (`birdnet-web` `viz::heat::calendar`) takes a map already judged by
//! this rule: the two other rules those surfaces applied (any day above 0 s, and
//! 60 s) are gone (N26-05). What still asks another question:
//!
//! * `effort::detection_history` and `/api/v2/detections/export/history` ask a
//!   different question (was an *occasion* surveyed, by its best-covered source,
//!   against a caller's `min_seconds`), and are not a listened-day rule.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;

use super::effort::{EffortHour, effort_hours};
use super::events::SiteMap;
use crate::sqlite::connection::DbError;

/// The share of the station's typical day a listened day needs.
pub const LISTENED_DAY_SHARE: f64 = birdnet_behavioral::phenology::LISTENED_DAY_SHARE;

/// A rule for "the station listened to this day".
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ListenedDayRule {
    /// At least [`LISTENED_DAY_SHARE`] of the typical day (the median, over the
    /// dates with any audio, of each date's seconds). No typical day, no
    /// listened day. The rule.
    HalfOfTypicalDay,
    /// Any audio at all (more than 0 s).
    AnyAudio,
    /// At least this many seconds.
    AtLeastSeconds(f64),
}

/// The listened-day rule every surface should apply.
pub const LISTENED_DAY: ListenedDayRule = ListenedDayRule::HalfOfTypicalDay;

impl ListenedDayRule {
    /// The rule's name, as a response states it.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::HalfOfTypicalDay => "half-of-typical-day".to_owned(),
            Self::AnyAudio => "any-audio".to_owned(),
            Self::AtLeastSeconds(s) => format!("at-least-{s}-seconds"),
        }
    }

    /// Whether a day with `seconds` of audio is listened to under this rule,
    /// given the station's `typical` day.
    #[must_use]
    pub fn is_listened(self, seconds: f64, typical: Option<f64>) -> bool {
        match self {
            Self::HalfOfTypicalDay => typical.is_some_and(|t| seconds >= LISTENED_DAY_SHARE * t),
            Self::AnyAudio => seconds > 0.0,
            Self::AtLeastSeconds(floor) => seconds >= floor,
        }
    }
}

/// [`LISTENED_DAY`] applied: whether a day with `seconds` of audio (summed over
/// sources) is a day the station listened to.
#[must_use]
pub fn is_listened_day(seconds: f64, typical: Option<f64>) -> bool {
    LISTENED_DAY.is_listened(seconds, typical)
}

/// One site's listening in one local hour.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteHour {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    /// Local hour, 0–23.
    pub hour: u8,
    /// The site.
    pub site: String,
    /// The site's sources with a ledger row this hour.
    pub sources: BTreeSet<String>,
    /// The site listened at least this long: the best-covered source.
    pub lower_bound_secs: f64,
    /// The site listened at most this long: `min(capacity, sum)`, not below the
    /// lower bound.
    pub upper_bound_secs: f64,
    /// Sensor time: the sum over sources. Not the site's listening.
    pub sensor_secs: f64,
    /// The hour's real length on the local clock, when known.
    pub capacity_secs: Option<f64>,
}

/// The ledger's hours grouped by site, with bounds (pure).
///
/// `capacity` gives an hour's real length; `None` leaves the upper bound at the
/// sum, which is always an upper bound.
#[must_use]
pub fn site_hours(
    hours: &[EffortHour],
    sites: &SiteMap,
    capacity: impl Fn(&str, u8) -> Option<f64>,
) -> Vec<SiteHour> {
    let mut by: BTreeMap<(String, u8, String), SiteHour> = BTreeMap::new();
    for h in hours {
        let site = sites.site_of(&h.source).to_owned();
        let e = by
            .entry((h.date.clone(), h.hour, site.clone()))
            .or_insert_with(|| SiteHour {
                date: h.date.clone(),
                hour: h.hour,
                site,
                sources: BTreeSet::new(),
                lower_bound_secs: 0.0,
                upper_bound_secs: 0.0,
                sensor_secs: 0.0,
                capacity_secs: capacity(&h.date, h.hour),
            });
        e.sources.insert(h.source.clone());
        e.sensor_secs += h.seconds_analysed;
        e.lower_bound_secs = e.lower_bound_secs.max(h.seconds_analysed);
        let cap = e.capacity_secs.unwrap_or(e.sensor_secs);
        e.upper_bound_secs = e.sensor_secs.min(cap).max(e.lower_bound_secs);
    }
    by.into_values().collect()
}

/// The real length of each local hour of `from..=to` (local dates) on this
/// host's clock, from `SQLite`'s `localtime`: the clock the ledger's hours are
/// written on. An hour the clock skips is absent.
///
/// Computed by stepping through the range in quarter hours of real time and
/// counting the steps that land in each local hour, which is exact for every
/// zone whose offsets are whole quarter hours (all in current use).
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn local_hour_capacity(
    conn: &Connection,
    from: &str,
    to: &str,
) -> Result<BTreeMap<(String, u8), f64>, DbError> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE steps(t) AS (
             SELECT CAST(strftime('%s', ?1, '-1 day') AS INTEGER)
             UNION ALL
             SELECT t + 900 FROM steps
              WHERE t + 900 < CAST(strftime('%s', ?2, '+2 days') AS INTEGER)
         ),
         local AS (
             SELECT date(t, 'unixepoch', 'localtime') AS d,
                    CAST(strftime('%H', t, 'unixepoch', 'localtime') AS INTEGER) AS h
               FROM steps
         )
         SELECT d, h, 900.0 * COUNT(*) FROM local
          WHERE d >= ?1 AND d <= ?2
          GROUP BY d, h",
    )?;
    let rows = stmt
        .query_map([from, to], |r| {
            Ok((
                (r.get::<_, String>(0)?, r.get::<_, u8>(1)?),
                r.get::<_, f64>(2)?,
            ))
        })?
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(rows)
}

/// Each site's listening per local hour over `from..=to` (local dates), with
/// bounds, from the hourly ledger.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn site_effort_hours(
    conn: &Connection,
    from: &str,
    to: &str,
    sites: &SiteMap,
) -> Result<Vec<SiteHour>, DbError> {
    let hours = effort_hours(conn, from, to)?;
    let capacity = local_hour_capacity(conn, from, to)?;
    Ok(site_hours(&hours, sites, |d, h| {
        capacity.get(&(d.to_owned(), h)).copied()
    }))
}
