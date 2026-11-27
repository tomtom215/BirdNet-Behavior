//! Event rates per hour of a site's listening (MSC-02, RES-18, N26-06, UIC-02).
//!
//! # What is divided by what
//!
//! The numerator is **events** ([`super::events`], policy
//! [`super::events::EVENT_POLICY_VERSION`]): N sources at one site hearing one
//! call is one event, so adding a second microphone beside the first does not
//! double a rate. The denominator is **the site's listening**, which the hourly
//! ledger gives only as bounds ([`super::site_effort`]): at least the
//! best-covered source's seconds, at most their sum capped by the hour. So a
//! rate is a range, `events / upper` to `events / lower`, and a single-source
//! site has `lower == upper`. Summed sensor time is never the denominator: a
//! quiet source added to a site raises sensor time and would drag a pooled rate
//! down with no change in the birds (N26-06). Sensor time is reported beside
//! each point so a per-sensor rate can be formed by a reader who wants one.
//!
//! Sites are never pooled: two sites are two lines (decision 2 of 2026-11-25).
//!
//! Only events in a local hour the site has a ledger row for are counted;
//! events in an hour with no listening recorded are reported apart
//! ([`RatePoint::events_outside_listening`]), never divided by someone else's
//! hour.
//!
//! # The interval
//!
//! `ci_lower` / `ci_upper` are the exact 95 % Poisson (Garwood) interval of the
//! event count, divided by the upper and the lower bound of the hours
//! respectively, so they enclose both the counting and the bounds' uncertainty.
//! Events of one bird are clustered, so even this is a floor on the
//! uncertainty, not the whole of it.
//!
//! # What it is not
//!
//! A rate of acoustic events is not an abundance, a density or a count of
//! birds: a bird that sings more, a microphone placed nearer, a quieter day and
//! a change of model or threshold all move it. It is exploratory.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, params};

use super::effort::effort_hours;
use super::events::{self, Event, RowFilter, SiteMap};
use super::site_effort::{SiteHour, local_hour_capacity, site_hours};
use crate::sqlite::connection::DbError;

/// The rate definition, by name.
pub const RATE_METHOD: &str = "site-event-rate-1";

/// The longest range of local dates a rate read covers.
pub const MAX_RATE_DAYS: i64 = 400;

/// How points are binned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateBin {
    /// One point per local date.
    Day,
    /// One point per week, Monday first, labelled by its Monday.
    Week,
    /// One point per local hour of the day, pooled over the range.
    HourOfDay,
}

impl RateBin {
    /// Parse `day`, `week` or `hour_of_day`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            "hour_of_day" => Some(Self::HourOfDay),
            _ => None,
        }
    }

    /// The name a response states.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::HourOfDay => "hour_of_day",
        }
    }

    /// The bin of a local date and hour.
    fn key(self, date: &str, hour: u8) -> String {
        match self {
            Self::Day => date.to_owned(),
            Self::Week => monday_of(date).unwrap_or_else(|| date.to_owned()),
            Self::HourOfDay => format!("{hour:02}"),
        }
    }
}

/// Days since 1970-01-01 of `YYYY-MM-DD` (Howard Hinnant's `days_from_civil`).
fn epoch_day(date: &str) -> Option<i64> {
    let mut it = date.splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// The Monday on or before `date`.
fn monday_of(date: &str) -> Option<String> {
    let day = epoch_day(date)?;
    // 1970-01-01 was a Thursday: Monday is 0 of (day + 3) mod 7.
    Some(events::utc_date((day - (day + 3).rem_euclid(7)) * 86_400))
}

/// What a rate read asks.
#[derive(Debug, Clone)]
pub struct RateQuery {
    /// First local date, `YYYY-MM-DD`.
    pub from: String,
    /// Last local date, inclusive.
    pub to: String,
    /// How points are binned.
    pub bin: RateBin,
    /// The event tolerance.
    pub tolerance_secs: f64,
    /// Which site each source is at.
    pub sites: SiteMap,
    /// Which rows are read (source, confidence floor, species).
    pub filter: RowFilter,
}

/// One site, species and bin.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RatePoint {
    /// The site.
    pub site: String,
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// The bin's label: a date, a week's Monday, or an hour `00`–`23`.
    pub bin: String,
    /// Events in hours the site has a ledger row for: the numerator.
    pub events: u64,
    /// Rows in those events.
    pub rows: u64,
    /// Events in hours of this bin with no ledger row: not counted.
    pub events_outside_listening: u64,
    /// The site listened at least this long in the bin.
    pub listened_lower_secs: f64,
    /// The site listened at most this long in the bin.
    pub listened_upper_secs: f64,
    /// Sensor time (summed over sources): not the site's listening.
    pub sensor_secs: f64,
    /// `events / upper hours`; `None` without listening.
    pub rate_low: Option<f64>,
    /// `events / lower hours`; `None` without listening.
    pub rate_high: Option<f64>,
    /// Lower end of the interval, per hour.
    pub ci_lower: Option<f64>,
    /// Upper end of the interval, per hour.
    pub ci_upper: Option<f64>,
    /// `measured` (listening recorded: a rate, zero included) or `no_effort`
    /// (no listening recorded in the bin: no rate, which is not a rate of 0).
    pub status: &'static str,
}

/// A rate read.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RateRead {
    /// Points in order of site, species and bin.
    pub points: Vec<RatePoint>,
    /// Rows of the range with no instant, in no event.
    pub rows_without_instant: u64,
    /// Whether the events at the range's edges were read wide enough.
    pub edges_settled: bool,
}

/// Each instant's local date and hour, by quarter hour of real time, over
/// `[from, to)` (Unix seconds), on this host's clock: the clock the ledger is
/// written on.
fn local_quarters(
    conn: &Connection,
    from: i64,
    to: i64,
) -> Result<BTreeMap<i64, (String, u8)>, DbError> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE q(t) AS (
             SELECT ?1 - (?1 % 900) UNION ALL SELECT t + 900 FROM q WHERE t + 900 < ?2
         )
         SELECT t, date(t, 'unixepoch', 'localtime'),
                CAST(strftime('%H', t, 'unixepoch', 'localtime') AS INTEGER)
           FROM q",
    )?;
    let rows = stmt
        .query_map(params![from, to], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                (r.get::<_, String>(1)?, r.get::<_, u8>(2)?),
            ))
        })?
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(rows)
}

/// The UTC instants at which local `from` begins and local `to` ends.
fn utc_bounds(conn: &Connection, from: &str, to: &str) -> Result<(i64, i64), DbError> {
    Ok(conn.query_row(
        "SELECT CAST(strftime('%s', ?1, 'utc') AS INTEGER),
                CAST(strftime('%s', ?2, '+1 day', 'utc') AS INTEGER)",
        params![from, to],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

#[derive(Default)]
struct Effort {
    lower: f64,
    upper: f64,
    sensor: f64,
}

/// The rates (pure): `events` and the site's `hours`, binned by `bin`, each
/// event placed at the local date and hour `local` gives its first instant.
#[must_use]
pub fn rates(
    events: &[Event],
    hours: &[SiteHour],
    bin: RateBin,
    local: impl Fn(i64) -> Option<(String, u8)>,
) -> Vec<RatePoint> {
    let listened: BTreeSet<(&str, &str, u8)> = hours
        .iter()
        .filter(|h| h.lower_bound_secs > 0.0)
        .map(|h| (h.site.as_str(), h.date.as_str(), h.hour))
        .collect();
    let mut effort: BTreeMap<(String, String), Effort> = BTreeMap::new();
    for h in hours {
        let e = effort
            .entry((h.site.clone(), bin.key(&h.date, h.hour)))
            .or_default();
        e.lower += h.lower_bound_secs;
        e.upper += h.upper_bound_secs;
        e.sensor += h.sensor_secs;
    }
    // (site, species) -> common name, and (site, species, bin) -> counts.
    let mut names: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut counts: BTreeMap<(String, String, String), (u64, u64, u64)> = BTreeMap::new();
    for e in events {
        let Some(site) = e.site.as_deref() else {
            continue;
        };
        let Some((date, hour)) = local(e.first) else {
            continue;
        };
        names
            .entry((site.to_owned(), e.sci_name.clone()))
            .or_insert_with(|| e.com_name.clone());
        let c = counts
            .entry((site.to_owned(), e.sci_name.clone(), bin.key(&date, hour)))
            .or_default();
        if listened.contains(&(site, date.as_str(), hour)) {
            c.0 += 1;
            c.1 += e.members.len() as u64;
        } else {
            c.2 += 1;
        }
    }
    let mut out = Vec::new();
    for ((site, sci), com) in &names {
        let bins: BTreeSet<&String> = effort
            .keys()
            .filter(|(s, _)| s == site)
            .map(|(_, b)| b)
            .chain(
                counts
                    .keys()
                    .filter(|(s, sp, _)| s == site && sp == sci)
                    .map(|(_, _, b)| b),
            )
            .collect();
        for b in bins {
            let (ev, rows, outside) = counts
                .get(&(site.clone(), sci.clone(), b.clone()))
                .copied()
                .unwrap_or_default();
            let e = effort.get(&(site.clone(), b.clone()));
            let (lower, upper, sensor) =
                e.map_or((0.0, 0.0, 0.0), |e| (e.lower, e.upper, e.sensor));
            let measured = lower > 0.0;
            let k = u32::try_from(ev).unwrap_or(u32::MAX);
            let (lo, hi) = birdnet_behavioral::phenology::abundance::poisson_interval_exact(k);
            #[allow(clippy::cast_precision_loss, reason = "an event count, far below 2^52")]
            let evf = ev as f64;
            let (lh, uh) = (lower / 3600.0, upper / 3600.0);
            out.push(RatePoint {
                site: site.clone(),
                sci_name: sci.clone(),
                com_name: com.clone(),
                bin: b.clone(),
                events: ev,
                rows,
                events_outside_listening: outside,
                listened_lower_secs: lower,
                listened_upper_secs: upper,
                sensor_secs: sensor,
                rate_low: measured.then(|| evf / uh),
                rate_high: measured.then(|| evf / lh),
                ci_lower: measured.then(|| lo / uh),
                ci_upper: measured.then(|| hi / lh),
                status: if measured { "measured" } else { "no_effort" },
            });
        }
    }
    out
}

/// The event rates of the query.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn rates_between(conn: &Connection, q: &RateQuery) -> Result<RateRead, DbError> {
    let (from_utc, to_utc) = utc_bounds(conn, &q.from, &q.to)?;
    let read = events::events_between_filtered(
        conn,
        from_utc,
        to_utc,
        q.tolerance_secs,
        &q.sites,
        &q.filter,
    )?;
    let mut ledger = effort_hours(conn, &q.from, &q.to)?;
    if let Some(src) = q.filter.source.as_deref() {
        ledger.retain(|h| h.source == src);
    }
    let capacity = local_hour_capacity(conn, &q.from, &q.to)?;
    let hours = site_hours(&ledger, &q.sites, |d, h| {
        capacity.get(&(d.to_owned(), h)).copied()
    });
    let quarters = local_quarters(conn, from_utc, to_utc)?;
    let points = rates(&read.events, &hours, q.bin, |t| {
        quarters.get(&(t - t.rem_euclid(900))).cloned()
    });
    Ok(RateRead {
        points,
        rows_without_instant: read.rows_without_instant,
        edges_settled: read.edges_settled,
    })
}

/// Two windows' rates of one species at one site, and their ratio.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct RateRatio {
    /// Events in the recent window.
    pub recent_events: u64,
    /// Events in the earlier window.
    pub earlier_events: u64,
    /// Listening bounds of the recent window, seconds.
    pub recent_listened: (f64, f64),
    /// Listening bounds of the earlier window, seconds.
    pub earlier_listened: (f64, f64),
    /// Recent rate over earlier rate, at the bounds' midpoints; `None`
    /// without listening in either, or no event in the earlier one.
    pub ratio: Option<f64>,
    /// Lower end of the 95 % interval of the ratio (conditional binomial,
    /// Wilson, taken at the bounds that make it lowest).
    pub ratio_lower: Option<f64>,
    /// Upper end, at the bounds that make it highest; `None` when unbounded.
    pub ratio_upper: Option<f64>,
}

/// The ratio of a recent window's event rate to an earlier one's.
///
/// With `k1`
/// events over `t1` hours and `k2` over `t2`, `k1` given `k1 + k2` is binomial
/// with share `r1 t1 / (r1 t1 + r2 t2)`, so a Wilson interval on that share
/// gives one on `r1 / r2`. The hours are bounds, and each end of the interval
/// is taken at the bounds that push it outwards.
#[must_use]
pub fn rate_ratio(recent: (u64, f64, f64), earlier: (u64, f64, f64)) -> RateRatio {
    let (k1, l1, u1) = recent;
    let (k2, l2, u2) = earlier;
    let mut r = RateRatio {
        recent_events: k1,
        earlier_events: k2,
        recent_listened: (l1, u1),
        earlier_listened: (l2, u2),
        ratio: None,
        ratio_lower: None,
        ratio_upper: None,
    };
    if l1 <= 0.0 || l2 <= 0.0 {
        return r;
    }
    #[allow(clippy::cast_precision_loss, reason = "event counts, far below 2^52")]
    let (k1f, k2f) = (k1 as f64, k2 as f64);
    let (m1, m2) = (f64::midpoint(l1, u1), f64::midpoint(l2, u2));
    if k2 > 0 {
        r.ratio = Some((k1f / m1) / (k2f / m2));
    }
    let n = u32::try_from(k1 + k2).unwrap_or(u32::MAX);
    let k = u32::try_from(k1).unwrap_or(u32::MAX);
    if let Some((plo, phi)) = birdnet_behavioral::interval::wilson(k, n) {
        let odds = |p: f64| if p >= 1.0 { None } else { Some(p / (1.0 - p)) };
        // r1/r2 = odds · t2/t1.
        r.ratio_lower = odds(plo).map(|o| o * l2 / u1);
        r.ratio_upper = odds(phi).map(|o| o * u2 / l1);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_week_is_labelled_by_its_monday() {
        assert_eq!(monday_of("2026-11-27").as_deref(), Some("2026-11-23"));
        assert_eq!(monday_of("2026-11-23").as_deref(), Some("2026-11-23"));
        assert_eq!(monday_of("2026-01-01").as_deref(), Some("2025-12-29"));
    }

    #[test]
    fn equal_rates_give_a_ratio_of_one_with_an_interval_around_it() {
        let r = rate_ratio((30, 36_000.0, 36_000.0), (30, 36_000.0, 36_000.0));
        assert!((r.ratio.unwrap() - 1.0).abs() < 1e-12);
        assert!(
            r.ratio_lower.unwrap() < 1.0 && r.ratio_upper.unwrap() > 1.0,
            "{r:?}"
        );
        // Twice as much listening, the same events: half the rate.
        let r = rate_ratio((30, 72_000.0, 72_000.0), (30, 36_000.0, 36_000.0));
        assert!((r.ratio.unwrap() - 0.5).abs() < 1e-12);
        // No listening, no ratio.
        assert_eq!(rate_ratio((3, 0.0, 0.0), (3, 10.0, 10.0)).ratio, None);
    }
}
