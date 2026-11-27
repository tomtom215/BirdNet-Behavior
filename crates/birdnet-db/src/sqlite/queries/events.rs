//! Events: the detections of one call, heard by several sources, counted once
//! (decision 1 of 2026-11-25; MSC-02, MSC-07, RES-18).
//!
//! # What an event is
//!
//! N sources that hear one call write N rows. An event is the set of rows that
//! are, by this module's rule, the same call: computed when it is read, never
//! stored on a row, so changing the tolerance or the site map rewrites nothing.
//! **An event is a call, not a bird.** One bird singing for a minute is twenty
//! events; two birds calling together at one site can be one. Nothing here
//! counts individuals, and no caller should name an event count "birds".
//!
//! # The rule (policy [`EVENT_POLICY_VERSION`])
//!
//! 1. Rows are matched only within one **site** and one **species**
//!    (`Sci_Name`). A source's site comes from the [`SiteMap`]; a source the
//!    map does not name is its own site (decision 2), so with the default map
//!    nothing is ever merged across sources. A row with no recorded `Source`
//!    (imported history, rows from before sources were recorded) cannot be
//!    placed anywhere and is an event of its own.
//! 2. A row's **instant** is `detected_at_utc`. A row whose instant is NULL
//!    (its local `Date`/`Time` named no instant, or named the hour that a
//!    spring-forward night skips) is **excluded and counted**, never guessed
//!    from its local time: [`EventsRead::rows_without_instant`].
//! 3. An event holds **at most one row per source**, and all of its members lie
//!    within `tolerance_secs` of one another (its span is at most the
//!    tolerance). That is what keeps a chorus from growing into one long
//!    event: A at 0, B at 3 and C at 6 with a tolerance of 4 are two events,
//!    never one chain.
//! 4. Pairing is **nearest first**. Every pair of rows from two different
//!    sources of one site and species within the tolerance is a candidate.
//!    Candidates are taken in order of their distance in time, and a candidate
//!    joins the two rows' events when the joined event would still satisfy
//!    rule 3; otherwise it is skipped. So A at 0, B at 3 and A at 3 pair B with
//!    the A at 3, not the earlier A.
//! 5. **Ties** are broken by the rows' order: rows are ordered by instant, then
//!    by their stable row id ([`EventRow::row_id`]), and of two candidates at
//!    the same distance the one whose earlier row comes first is taken first
//!    (then the one whose later row comes first). So with A at 0 and A at 2
//!    both one second from B at 1, B pairs with the A at 0, and the A at 2 is
//!    an event of its own.
//!
//! The result depends only on the set of rows, not on the order they arrive
//! in, because every decision above is taken in a total order over the rows.
//! The events that come out are also maximal: no two of them could be joined
//! without breaking rule 3 (a candidate between them was considered, and a
//! join refused for a subset is refused for the whole).
//!
//! # Identity
//!
//! An event's id ([`event_id`]) is a hash of its members' sorted row ids and
//! the policy version, so the same rows under the same policy always have the
//! same id, and a change of policy changes every id rather than silently
//! reusing one. It is an identifier, not a security hash (64-bit FNV-1a).
//!
//! # The tolerance
//!
//! A setting, defaulting to the model's window plus one second
//! ([`DEFAULT_TOLERANCE_SECS`], 4 s for `BirdNET` V2.4; V3.0's would be 5.5 s).
//! That is the lower bound the code fixes: a row's time is its segment's
//! whole-second stamp plus the window offset, truncated, so two sources on one
//! host clock can differ by up to a window and a second before any latency.
//! The latency difference between sources (RTSP in particular, MSC-14) is
//! unmeasured, so the default can be too small for a camera stream.
//!
//! # One definition
//!
//! This is the only definition of an event in the repository. The `DuckDB`
//! copy has none of its own; analytics that needs events should call
//! [`group_events`] over the rows it reads rather than write a second one in
//! SQL.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

/// The version of the event rule this module implements. Every response that
/// counts events states it, and it is hashed into every [`event_id`].
pub const EVENT_POLICY_VERSION: &str = "events-1";

/// `BirdNET` V2.4's analysis window, in seconds.
pub const V24_WINDOW_SECS: f64 = 3.0;

/// The default tolerance: the model's window plus one second (decision 1).
pub const DEFAULT_TOLERANCE_SECS: f64 = V24_WINDOW_SECS + 1.0;

/// The largest tolerance a caller may ask for. Ten minutes is far beyond any
/// plausible clock and latency difference; it bounds the work a request does.
pub const MAX_TOLERANCE_SECS: f64 = 600.0;

/// Which site each source is at. A source the map does not name is its own
/// site (decision 2 of 2026-11-25): nothing is grouped until an owner says two
/// sources are in the same place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiteMap {
    explicit: BTreeMap<String, String>,
}

impl SiteMap {
    /// Every source its own site.
    #[must_use]
    pub fn identity() -> Self {
        Self::default()
    }

    /// Put `source` at `site`.
    #[must_use]
    pub fn with(mut self, source: &str, site: &str) -> Self {
        self.explicit.insert(source.to_owned(), site.to_owned());
        self
    }

    /// The site `source` is at: the map's entry, or the source's own name.
    #[must_use]
    pub fn site_of<'a>(&'a self, source: &'a str) -> &'a str {
        self.explicit.get(source).map_or(source, String::as_str)
    }

    /// Parse `source:site` pairs, comma-separated, split at the last colon so a
    /// source name may contain one. A source named twice with different sites
    /// is refused. The form `/api/v2/events?sites=` and `SOURCE_SITES` share.
    ///
    /// # Errors
    ///
    /// A message naming the entry that is not `source:site`, or the source
    /// given two sites.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut map = Self::identity();
        for entry in raw.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let Some((source, site)) = entry.rsplit_once(':') else {
                return Err(format!("sites entry {entry:?} is not source:site"));
            };
            let (source, site) = (source.trim(), site.trim());
            if source.is_empty() || site.is_empty() {
                return Err(format!("sites entry {entry:?} is not source:site"));
            }
            if let Some(prev) = map.explicit.get(source)
                && prev != site
            {
                return Err(format!("source {source:?} is given two sites"));
            }
            map = map.with(source, site);
        }
        Ok(map)
    }

    /// The entries an owner gave; every other source is its own site.
    #[must_use]
    pub const fn explicit(&self) -> &BTreeMap<String, String> {
        &self.explicit
    }

    /// The map as it applies to `sources`: each one with the site it is at,
    /// whether or not the map names it, plus every explicit entry.
    #[must_use]
    pub fn resolved<'a>(
        &self,
        sources: impl IntoIterator<Item = &'a str>,
    ) -> BTreeMap<String, String> {
        let mut out = self.explicit.clone();
        for s in sources {
            out.entry(s.to_owned())
                .or_insert_with(|| self.site_of(s).to_owned());
        }
        out
    }
}

/// One detection row, as the event rule reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    /// A stable identity for the row: the same row has the same id in every
    /// read. [`events_between`] uses the table's unique key
    /// (`Date|Time|Sci_Name|File_Name|chunk_offset_secs`), not the `rowid`,
    /// which a `VACUUM` may renumber.
    pub row_id: String,
    /// `detected_at_utc`, Unix seconds.
    pub instant: i64,
    /// Scientific name: rows match only within one species.
    pub sci_name: String,
    /// Common name, carried for display; never part of matching.
    pub com_name: String,
    /// The source that wrote the row; `None` when none was recorded.
    pub source: Option<String>,
    /// The row's confidence, carried for display.
    pub confidence: f64,
}

/// One event: rows of one species at one site that are the same call.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// [`event_id`] of the members.
    pub event_id: String,
    /// The site; `None` for a row with no recorded source.
    pub site: Option<String>,
    /// Scientific name.
    pub sci_name: String,
    /// Common name of the first member.
    pub com_name: String,
    /// Instant of the earliest member, Unix seconds.
    pub first: i64,
    /// Instant of the latest member, Unix seconds.
    pub last: i64,
    /// The members, in row order (instant, then row id).
    pub members: Vec<EventRow>,
}

/// The id of an event with members `row_ids` under policy `version`: 64-bit
/// FNV-1a over the version and the sorted ids, each length-prefixed, as 16
/// hex digits. Independent of the order the ids are given in.
#[must_use]
pub fn event_id<S: AsRef<str>>(row_ids: &[S], version: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut ids: Vec<&str> = row_ids.iter().map(AsRef::as_ref).collect();
    ids.sort_unstable();
    let mut h = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for b in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
            h ^= u64::from(*b);
            h = h.wrapping_mul(PRIME);
        }
    };
    feed(version.as_bytes());
    for id in ids {
        feed(id.as_bytes());
    }
    format!("{h:016x}")
}

/// The tolerance as whole seconds: instants are whole seconds, so `|dt| <=
/// tolerance` is `|dt| <= floor(tolerance)`. A tolerance that is not finite or
/// is negative matches nothing but a row with itself.
fn whole_seconds(tolerance_secs: f64) -> i64 {
    if !tolerance_secs.is_finite() || tolerance_secs < 0.0 {
        return -1;
    }
    // Bounded above by MAX_TOLERANCE_SECS at the API; a larger value is
    // clamped to a day, which no plausible tolerance reaches.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to a day of seconds above, far inside i64"
    )]
    let secs = tolerance_secs.min(86_400.0).floor() as i64;
    secs
}

/// The total order the rule decides in: instant, then row id, then source.
fn row_order(a: &EventRow, b: &EventRow) -> std::cmp::Ordering {
    a.instant
        .cmp(&b.instant)
        .then_with(|| a.row_id.cmp(&b.row_id))
        .then_with(|| a.source.cmp(&b.source))
}

/// Group `rows` into events by the rule in the module documentation.
///
/// Events are returned in order of their first instant, then site, species and
/// id. Every row is in exactly one event.
#[must_use]
pub fn group_events(rows: &[EventRow], tolerance_secs: f64, sites: &SiteMap) -> Vec<Event> {
    let tol = whole_seconds(tolerance_secs);
    // (site, species) -> rows. A row with no source is set aside: it is an
    // event of its own.
    let mut groups: BTreeMap<(&str, &str), Vec<&EventRow>> = BTreeMap::new();
    let mut events = Vec::new();
    for row in rows {
        match row.source.as_deref() {
            Some(source) => groups
                .entry((sites.site_of(source), row.sci_name.as_str()))
                .or_default()
                .push(row),
            None => events.push(make_event(None, vec![row.clone()])),
        }
    }
    for ((site, _), mut group) in groups {
        group.sort_by(|a, b| row_order(a, b));
        for members in pair_nearest(&group, tol) {
            events.push(make_event(Some(site.to_owned()), members));
        }
    }
    events.sort_by(|a, b| {
        a.first
            .cmp(&b.first)
            .then_with(|| a.site.cmp(&b.site))
            .then_with(|| a.sci_name.cmp(&b.sci_name))
            .then_with(|| a.event_id.cmp(&b.event_id))
    });
    events
}

/// An event of `members`, which is never empty: every caller builds it from at
/// least one row.
fn make_event(site: Option<String>, mut members: Vec<EventRow>) -> Event {
    members.sort_by(row_order);
    let ids: Vec<&str> = members.iter().map(|m| m.row_id.as_str()).collect();
    let event_id = event_id(&ids, EVENT_POLICY_VERSION);
    let head = &members[0];
    let (sci_name, com_name, first) = (head.sci_name.clone(), head.com_name.clone(), head.instant);
    let last = members.last().map_or(first, |m| m.instant);
    Event {
        event_id,
        site,
        sci_name,
        com_name,
        first,
        last,
        members,
    }
}

/// The root of `x` in a union-find, halving the path as it goes.
const fn find_root(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// One site and species' rows, already in row order, paired nearest first.
///
/// Candidates are the pairs of rows from different sources within `tol` of
/// each other, taken by (distance, index of the earlier row, index of the
/// later row): the indices are row order, so this is the documented tie-break.
/// A candidate joins its two rows' events when they are different events,
/// share no source, and the joined span is within `tol`.
fn pair_nearest(group: &[&EventRow], tol: i64) -> Vec<Vec<EventRow>> {
    let n = group.len();
    let mut pairs: Vec<(i64, usize, usize)> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let dt = group[j].instant - group[i].instant;
            if dt > tol {
                break;
            }
            if group[i].source != group[j].source {
                pairs.push((dt, i, j));
            }
        }
    }
    pairs.sort_unstable();

    // A small union-find; each root carries its event's span and sources.
    let mut parent: Vec<usize> = (0..n).collect();
    let mut span: Vec<(i64, i64)> = group.iter().map(|r| (r.instant, r.instant)).collect();
    let mut sources: Vec<BTreeSet<&str>> = group
        .iter()
        .map(|r| r.source.as_deref().into_iter().collect())
        .collect();
    for (_, i, j) in pairs {
        let (a, b) = (find_root(&mut parent, i), find_root(&mut parent, j));
        if a == b {
            continue;
        }
        let lo = span[a].0.min(span[b].0);
        let hi = span[a].1.max(span[b].1);
        if hi - lo > tol || !sources[a].is_disjoint(&sources[b]) {
            continue;
        }
        // Keep the smaller index as the root, so the result never depends on
        // which side a merge happened to come from.
        let (keep, gone) = if a < b { (a, b) } else { (b, a) };
        parent[gone] = keep;
        span[keep] = (lo, hi);
        let moved = std::mem::take(&mut sources[gone]);
        sources[keep].extend(moved);
    }
    let mut events: BTreeMap<usize, Vec<EventRow>> = BTreeMap::new();
    for (i, row) in group.iter().enumerate() {
        let r = find_root(&mut parent, i);
        events.entry(r).or_default().push((*row).clone());
    }
    events.into_values().collect()
}

/// Events read from the database for a UTC range.
#[derive(Debug, Clone, PartialEq)]
pub struct EventsRead {
    /// The events whose first member is in the range, in [`group_events`]'s
    /// order.
    pub events: Vec<Event>,
    /// Rows the range holds with no instant (`detected_at_utc` NULL), counted
    /// by their local `Date` against the range's UTC dates, since they have no
    /// instant to place. They are in no event.
    pub rows_without_instant: u64,
    /// The rows actually read, `[read_from, read_to)`: the range widened until
    /// no event reported could reach a row outside it.
    pub read_from: i64,
    /// End of the rows read, exclusive.
    pub read_to: i64,
    /// `false` when the widening stopped at its limit (a day each side) with
    /// rows still within the tolerance of its edge: an event at the very edge
    /// of the range may then pair differently from a read of a wider range.
    pub edges_settled: bool,
}

/// How far [`events_between`] widens the rows it reads, at most, each side.
const MAX_WIDENING_SECS: i64 = 86_400;

/// Which rows an event read admits, before any grouping (UIC-11, MSC-18).
///
/// Filtering happens on rows, not on events: a source left out is not a
/// member of any event, so an event of two sources read for one of them is
/// that source's row alone. The default admits every row
/// `detections_analytic` holds.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct RowFilter {
    /// Only this source's rows.
    pub source: Option<String>,
    /// Only rows at or above this confidence (a score, not a probability).
    pub min_confidence: Option<f64>,
    /// Only this species, by scientific name.
    pub sci_name: Option<String>,
}

fn read_rows(
    conn: &Connection,
    from: i64,
    to: i64,
    filter: &RowFilter,
) -> Result<Vec<EventRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, Time, Sci_Name, COALESCE(File_Name, ''), chunk_offset_secs,
                detected_at_utc, Com_Name, Source, Confidence
           FROM detections_analytic
          WHERE detected_at_utc >= ?1 AND detected_at_utc < ?2
            AND (?3 IS NULL OR Source = ?3)
            AND (?4 IS NULL OR Confidence >= ?4)
            AND (?5 IS NULL OR Sci_Name = ?5)",
    )?;
    let rows = stmt
        .query_map(
            params![
                from,
                to,
                filter.source,
                filter.min_confidence,
                filter.sci_name
            ],
            |r| {
                let date: String = r.get(0)?;
                let time: String = r.get(1)?;
                let sci: String = r.get(2)?;
                let file: String = r.get(3)?;
                let offset: f64 = r.get(4)?;
                Ok(EventRow {
                    row_id: format!("{date}|{time}|{sci}|{file}|{offset}"),
                    instant: r.get(5)?,
                    sci_name: sci,
                    com_name: r.get(6)?,
                    source: r.get(7)?,
                    confidence: r.get(8)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Whether some row within `tol` of an edge of `[lo, hi)` is in the same
/// run of rows (one site and species, consecutive rows no more than `tol`
/// apart) as a row of the reported range `[from, to)`: a row beyond that edge
/// could then change how the reported rows pair.
fn edge_reaches_range(
    rows: &[EventRow],
    sites: &SiteMap,
    tol: i64,
    bounds: (i64, i64, i64, i64),
) -> bool {
    let (lo, hi, from, to) = bounds;
    let mut groups: BTreeMap<(&str, &str), Vec<i64>> = BTreeMap::new();
    for r in rows {
        if let Some(s) = r.source.as_deref() {
            groups
                .entry((sites.site_of(s), r.sci_name.as_str()))
                .or_default()
                .push(r.instant);
        }
    }
    for mut ts in groups.into_values() {
        ts.sort_unstable();
        let mut start = 0;
        for i in 0..ts.len() {
            let ends_run = i + 1 == ts.len() || ts[i + 1] - ts[i] > tol;
            if ends_run {
                let run = &ts[start..=i];
                let near_edge = run[0] - tol < lo || run[run.len() - 1] + tol >= hi;
                let in_range = run.iter().any(|t| (from..to).contains(t));
                if near_edge && in_range {
                    return true;
                }
                start = i + 1;
            }
        }
    }
    false
}

/// The events of `[from, to)` (Unix seconds, UTC), from `detections_analytic`
/// (a detection a reviewer rejected is in none).
///
/// Rows are read from a range widened on each side until no reported event
/// could reach a row beyond it (at most a day each side;
/// [`EventsRead::edges_settled`] says whether that was enough), and an event
/// is reported when its first member is in `[from, to)`.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn events_between(
    conn: &Connection,
    from: i64,
    to: i64,
    tolerance_secs: f64,
    sites: &SiteMap,
) -> Result<EventsRead, DbError> {
    events_between_filtered(conn, from, to, tolerance_secs, sites, &RowFilter::default())
}

/// [`events_between`] over the rows `filter` admits.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn events_between_filtered(
    conn: &Connection,
    from: i64,
    to: i64,
    tolerance_secs: f64,
    sites: &SiteMap,
    filter: &RowFilter,
) -> Result<EventsRead, DbError> {
    let tol = whole_seconds(tolerance_secs).max(0);
    let mut widen = tol.max(1);
    let (rows, lo, hi, settled) = loop {
        let lo = from.saturating_sub(widen);
        let hi = to.saturating_add(widen);
        let rows = read_rows(conn, lo, hi, filter)?;
        if !edge_reaches_range(&rows, sites, tol, (lo, hi, from, to)) {
            break (rows, lo, hi, true);
        }
        if widen >= MAX_WIDENING_SECS {
            break (rows, lo, hi, false);
        }
        widen = widen.saturating_mul(4).min(MAX_WIDENING_SECS);
    };
    let events = group_events(&rows, tolerance_secs, sites)
        .into_iter()
        .filter(|e| (from..to).contains(&e.first))
        .collect();
    let rows_without_instant: i64 = conn.query_row(
        "SELECT COUNT(*) FROM detections_analytic
          WHERE detected_at_utc IS NULL
            AND Date >= date(?1, 'unixepoch') AND Date <= date(?2 - 1, 'unixepoch')
            AND (?3 IS NULL OR Source = ?3)
            AND (?4 IS NULL OR Confidence >= ?4)
            AND (?5 IS NULL OR Sci_Name = ?5)",
        params![
            from,
            to,
            filter.source,
            filter.min_confidence,
            filter.sci_name
        ],
        |r| r.get(0),
    )?;
    Ok(EventsRead {
        events,
        rows_without_instant: u64::try_from(rows_without_instant).unwrap_or(0),
        read_from: lo,
        read_to: hi,
        edges_settled: settled,
    })
}

/// The UTC date (`YYYY-MM-DD`) of a Unix instant.
#[must_use]
pub fn utc_date(instant: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = instant.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Events and rows at one site (a point, in point-count terms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteCount {
    /// The site; `None` for rows with no recorded source.
    pub site: Option<String>,
    /// The sources whose rows are in its events.
    pub sources: BTreeSet<String>,
    /// Events.
    pub events: u64,
    /// Rows in those events.
    pub rows: u64,
}

/// Events and rows per site.
#[must_use]
pub fn site_counts(events: &[Event]) -> Vec<SiteCount> {
    let mut by: BTreeMap<Option<String>, SiteCount> = BTreeMap::new();
    for e in events {
        let c = by.entry(e.site.clone()).or_insert_with(|| SiteCount {
            site: e.site.clone(),
            sources: BTreeSet::new(),
            events: 0,
            rows: 0,
        });
        c.events += 1;
        c.rows += e.members.len() as u64;
        c.sources
            .extend(e.members.iter().filter_map(|m| m.source.clone()));
    }
    by.into_values().collect()
}

/// Events of one species at one site on one UTC date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaySpeciesCount {
    /// UTC date of the events' first members.
    pub date: String,
    /// The site; `None` for rows with no recorded source.
    pub site: Option<String>,
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Events.
    pub events: u64,
    /// Rows in those events.
    pub rows: u64,
}

/// Events per UTC date, site and species, in that order. A site is never
/// pooled with another: two sites hearing one species are two lines.
#[must_use]
pub fn day_species_counts(events: &[Event]) -> Vec<DaySpeciesCount> {
    let mut by: BTreeMap<(String, Option<String>, String), DaySpeciesCount> = BTreeMap::new();
    for e in events {
        let date = utc_date(e.first);
        let c = by
            .entry((date.clone(), e.site.clone(), e.sci_name.clone()))
            .or_insert_with(|| DaySpeciesCount {
                date,
                site: e.site.clone(),
                sci_name: e.sci_name.clone(),
                com_name: e.com_name.clone(),
                events: 0,
                rows: 0,
            });
        c.events += 1;
        c.rows += e.members.len() as u64;
    }
    by.into_values().collect()
}
