//! Species co-occurrence and correlation queries.
//!
//! Analyses which species tend to appear together: in the same five-minute
//! block **at the same site** for the pair views, on the same day at the same
//! site for the companion lookup.
//!
//! # Exploratory and associational
//!
//! Two species heard together are not shown to interact, to share a flock or
//! to respond to each other: they may simply be common at the same hours and in
//! the same season. So every pair carries what chance would give
//! ([`SpeciesPair::expected_shared`], [`SpeciesPair::chance_overlap`]) from a
//! null model that keeps each species' prevalence, time of day, season and
//! day-to-day persistence and breaks only their alignment
//! ([`birdnet_behavioral::null_model::shift_null`]), a p-value against it, a
//! false-discovery-adjusted q-value over every pair looked at, and a
//! [`Verdict`]. A figure that has nothing beyond chance to show should say so.
//!
//! # Sites (N06-02)
//!
//! Two sources pointed at different places hear different birds; a robin at
//! one and a wren at the other in the same five minutes were never together.
//! Blocks are keyed by site ([`SiteMap`]; a source the map does not name is its
//! own site, decision 2 of 2026-11-25), so a pair is a pair at one site, and the
//! days a site's null shifts over are that site's own. Rows with no recorded
//! source (imported history) are one site of their own.
//!
//! Presence in a block is not changed by how many sources at one site heard
//! it, so a pair count is already a count of occasions, not rows: N sources
//! hearing one call at one site is one occasion.

use std::collections::BTreeMap;

pub use birdnet_behavioral::null_model::Verdict;
use birdnet_behavioral::null_model::{
    self, DEFAULT_DRAWS, DEFAULT_SEED, DaySlots, PairNull, SHIFT_NULL_METHOD, STRATUM_DAYS,
};
use rusqlite::{Connection, params};

use super::events::SiteMap;
use crate::sqlite::connection::DbError;

/// The definition of "together" the pair views use, by name.
pub const COOCCURRENCE_METHOD: &str = "same-site-five-minute-block-1";

/// The most pairs one analysis tests against its null. Pairs beyond it (the
/// ones sharing fewest blocks) are not reported, and
/// [`CooccurrenceAnalysis::pairs_untested`] says how many.
pub const MAX_TESTED_PAIRS: usize = 400;

/// Two species heard together, and how much of their time that is.
///
/// "Together" is the same five-minute block of the same day at the same site
/// (`Time` 07:00–07:04 is one block). The strength of a pair is its
/// [`overlap`](Self::overlap): shared blocks as a share of the blocks in which
/// either was heard, so two common birds are not a strong pair merely for being
/// common (`ANA7`); and [`chance_overlap`](Self::chance_overlap) is what that
/// share would be by chance. The default (for figures' fixtures) has no
/// null computed and claims nothing ([`Verdict::TooLittleData`]).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SpeciesPair {
    /// First species common name (alphabetically first).
    pub species_a: String,
    /// Second species common name.
    pub species_b: String,
    /// The site the two were heard at; `None` for rows with no recorded
    /// source.
    pub site: Option<String>,
    /// Five-minute blocks in which both were heard.
    pub shared_blocks: i64,
    /// Blocks in which `species_a` was heard.
    pub blocks_a: i64,
    /// Blocks in which `species_b` was heard.
    pub blocks_b: i64,
    /// `shared / (blocks_a + blocks_b − shared)`, from 0 to 1 (Jaccard).
    pub overlap: f64,
    /// The shared blocks chance gives: the mean over the null's shifts.
    pub expected_shared: f64,
    /// The overlap at the chance level of shared blocks.
    pub chance_overlap: f64,
    /// `shared_blocks / expected_shared`; `None` when chance gives none.
    pub lift: Option<f64>,
    /// One-sided permutation p-value for "more together than chance".
    pub p_value: f64,
    /// Benjamini–Hochberg q-value over every pair the analysis tested.
    pub q_value: f64,
    /// What a figure may say about the pair.
    pub verdict: Verdict,
}

/// A species heard on the same days as another (the trigger), and how much of their days that is.
///
/// "The same day" is the same calendar day at the same site, in either order: not *after* the
/// trigger, which an earlier version of this comment said. Each source is its own site (decision 2
/// of 2026-11-25), so the days counted are site-days: a trigger at one microphone and a companion
/// at another on one date were not heard on the same day in one place (N06-02). The strength is [`overlap`](Self::overlap), the shared
/// days as a share of the days either was heard, so a resident heard every day is not every
/// species' first companion merely for being common (UIC-04).
#[derive(Debug, Clone, serde::Serialize)]
pub struct FollowOn {
    /// The trigger species.
    pub trigger: String,
    /// A species heard on at least one of the trigger's days.
    pub companion: String,
    /// Days on which both were heard.
    pub shared_days: i64,
    /// Days on which the trigger was heard, in the window.
    pub trigger_days: i64,
    /// Days on which the companion was heard, in the window.
    pub companion_days: i64,
    /// `shared / (trigger_days + companion_days − shared)`, from 0 to 1 (Jaccard).
    pub overlap: f64,
    /// Average confidence of the companion's detections on the shared days.
    pub avg_confidence: f64,
}

/// What a co-occurrence analysis asks.
#[derive(Debug, Clone)]
pub struct CooccurrenceQuery {
    /// The last `days` days, today included.
    pub days: u32,
    /// Pairs to return, strongest overlap first.
    pub limit: usize,
    /// A pair needs this many shared blocks to be tested or returned.
    pub min_shared: u32,
    /// Which site each source is at.
    pub sites: SiteMap,
    /// Only this source's rows; `None` reads every source.
    pub source: Option<String>,
    /// Only rows at or above this confidence; `None` takes every row the
    /// station stored.
    pub min_confidence: Option<f64>,
    /// The null's seed.
    pub seed: u64,
    /// Random shifts per pair, when they are not enumerated.
    pub draws: usize,
}

impl CooccurrenceQuery {
    /// The defaults: every source its own site, every source and stored row,
    /// [`DEFAULT_SEED`] and [`DEFAULT_DRAWS`].
    #[must_use]
    pub fn new(days: u32, limit: usize, min_shared: u32) -> Self {
        Self {
            days,
            limit,
            min_shared,
            sites: SiteMap::identity(),
            source: None,
            min_confidence: None,
            seed: DEFAULT_SEED,
            draws: DEFAULT_DRAWS,
        }
    }
}

/// A co-occurrence analysis, with everything needed to re-derive it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CooccurrenceAnalysis {
    /// The pairs, strongest overlap first, at most the query's limit.
    pub pairs: Vec<SpeciesPair>,
    /// [`COOCCURRENCE_METHOD`].
    pub method: &'static str,
    /// [`SHIFT_NULL_METHOD`].
    pub null_method: &'static str,
    /// The seed the null drew from.
    pub seed: u64,
    /// Shifts per pair (the most any pair used).
    pub draws: usize,
    /// Days per stratum the shifts stay within.
    pub stratum_days: usize,
    /// Pairs tested against the null (the denominator of the adjustment).
    pub pairs_tested: usize,
    /// Pairs at or above `min_shared` that were not tested
    /// ([`MAX_TESTED_PAIRS`]).
    pub pairs_untested: usize,
    /// The days each site heard anything in the window: what its null shifts
    /// over. The key is the site, `""` for rows with no recorded source.
    pub eligible_days: BTreeMap<String, usize>,
    /// Tested pairs whose verdict is [`Verdict::StandsOut`].
    pub stands_out: usize,
    /// Whether the window is too short for any pair to stand out: the null's
    /// smallest attainable p-value is above the false-discovery rate (fewer
    /// than about twenty days at one site).
    pub too_little_data: bool,
    /// The false-discovery rate the verdicts use.
    pub fdr_alpha: f64,
}

/// One site's species and their blocks, on that site's eligible days.
struct SiteBlocks {
    days: usize,
    species: BTreeMap<String, Vec<DaySlots>>,
}

/// The site key of a row's source: its site, `""` when none was recorded.
fn site_key(sites: &SiteMap, source: Option<&str>) -> String {
    source.map_or("", |s| sites.site_of(s)).to_owned()
}

/// Every site's species and their five-minute blocks over its own days.
fn read_blocks(
    conn: &Connection,
    q: &CooccurrenceQuery,
) -> Result<BTreeMap<String, SiteBlocks>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT Source, Date,
                CAST(SUBSTR(Time, 1, 2) AS INTEGER) * 12
                    + CAST(SUBSTR(Time, 4, 2) AS INTEGER) / 5 AS slot,
                Com_Name
         FROM detections_analytic
         WHERE Date >= DATE('now', 'localtime', '-' || (?1 - 1) || ' days')
           AND (?2 IS NULL OR Source = ?2)
           AND (?3 IS NULL OR Confidence >= ?3)",
    )?;
    let rows = stmt
        .query_map(params![q.days, q.source, q.min_confidence], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    // Site -> date -> index, in date order.
    let mut dates: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for (source, date, _, _) in &rows {
        dates
            .entry(site_key(&q.sites, source.as_deref()))
            .or_default()
            .insert(date.clone(), 0);
    }
    for days in dates.values_mut() {
        for (i, v) in days.values_mut().enumerate() {
            *v = i;
        }
    }
    let mut out: BTreeMap<String, SiteBlocks> = dates
        .iter()
        .map(|(site, d)| {
            (
                site.clone(),
                SiteBlocks {
                    days: d.len(),
                    species: BTreeMap::new(),
                },
            )
        })
        .collect();
    for (source, date, slot, name) in rows {
        let site = site_key(&q.sites, source.as_deref());
        let Some(day) = dates.get(&site).and_then(|d| d.get(&date)).copied() else {
            continue;
        };
        let Some(sb) = out.get_mut(&site) else {
            continue;
        };
        let n = sb.days;
        let series = sb.species.entry(name).or_insert_with(|| vec![[0; 5]; n]);
        if let Ok(slot) = usize::try_from(slot) {
            null_model::set_slot(&mut series[day], slot);
        }
    }
    Ok(out)
}

fn count_blocks(series: &[DaySlots]) -> i64 {
    series
        .iter()
        .flatten()
        .map(|w| i64::from(w.count_ones()))
        .sum()
}

fn shared_blocks(a: &[DaySlots], b: &[DaySlots]) -> i64 {
    a.iter()
        .zip(b)
        .flat_map(|(x, y)| {
            x.iter()
                .zip(y)
                .map(|(p, q)| i64::from((p & q).count_ones()))
        })
        .sum()
}

/// A pair at a site that shares enough blocks to be tested.
struct Candidate<'a> {
    site: &'a str,
    a: &'a str,
    b: &'a str,
    shared: i64,
}

/// Every pair at every site sharing at least `min_shared` blocks, most shared
/// first.
fn candidates(sites: &BTreeMap<String, SiteBlocks>, min_shared: i64) -> Vec<Candidate<'_>> {
    let mut out = Vec::new();
    for (site, sb) in sites {
        let names: Vec<&String> = sb.species.keys().collect();
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                let shared = shared_blocks(&sb.species[*a], &sb.species[*b]);
                if shared >= min_shared.max(1) {
                    out.push(Candidate { site, a, b, shared });
                }
            }
        }
    }
    out.sort_by(|x, y| {
        y.shared
            .cmp(&x.shared)
            .then_with(|| x.site.cmp(y.site))
            .then_with(|| x.a.cmp(y.a))
            .then_with(|| x.b.cmp(y.b))
    });
    out
}

/// One tested pair, with its null and q-value.
fn pair(sb: &SiteBlocks, c: &Candidate<'_>, n: &PairNull, q: f64, verdict: Verdict) -> SpeciesPair {
    let (blocks_a, blocks_b) = (
        count_blocks(&sb.species[c.a]),
        count_blocks(&sb.species[c.b]),
    );
    #[allow(
        clippy::cast_precision_loss,
        reason = "block counts of one range, far below 2^52"
    )]
    let overlap = c.shared as f64 / (blocks_a + blocks_b - c.shared) as f64;
    #[allow(
        clippy::cast_precision_loss,
        reason = "block counts of one range, far below 2^52"
    )]
    let either_by_chance = (blocks_a + blocks_b) as f64 - n.expected;
    let chance_overlap = if either_by_chance > 0.0 {
        n.expected / either_by_chance
    } else {
        0.0
    };
    #[allow(
        clippy::cast_precision_loss,
        reason = "block counts of one range, far below 2^52"
    )]
    let lift = (n.expected > 0.0).then(|| c.shared as f64 / n.expected);
    SpeciesPair {
        species_a: c.a.to_owned(),
        species_b: c.b.to_owned(),
        site: (!c.site.is_empty()).then(|| c.site.to_owned()),
        shared_blocks: c.shared,
        blocks_a,
        blocks_b,
        overlap,
        expected_shared: n.expected,
        chance_overlap,
        lift,
        p_value: n.p_value,
        q_value: q,
        verdict,
    }
}

/// The co-occurrence analysis of the query's window: pairs, their chance
/// levels and verdicts, and what they rest on.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn cooccurrence_analysis(
    conn: &Connection,
    q: &CooccurrenceQuery,
) -> Result<CooccurrenceAnalysis, DbError> {
    let sites = read_blocks(conn, q)?;
    let mut tested = candidates(&sites, i64::from(q.min_shared));
    let untested = tested.len().saturating_sub(MAX_TESTED_PAIRS);
    tested.truncate(MAX_TESTED_PAIRS);

    let nulls: Vec<PairNull> = tested
        .iter()
        .map(|c| {
            let sb = &sites[c.site];
            let key = format!("{}\u{1f}{}\u{1f}{}", c.site, c.a, c.b);
            let seed = null_model::item_seed(q.seed, &key);
            null_model::shift_null(&sb.species[c.a], &sb.species[c.b], q.draws, seed)
        })
        .collect();
    let qs = null_model::benjamini_hochberg(&nulls.iter().map(|n| n.p_value).collect::<Vec<_>>());
    let tests = tested.len();
    let min_p = nulls.iter().map(|n| n.min_p).fold(1.0_f64, f64::min);
    let too_little_data = tests > 0 && null_model::too_little_data(min_p);
    let mut pairs: Vec<SpeciesPair> = tested
        .iter()
        .zip(&nulls)
        .zip(&qs)
        .map(|((c, n), &qv)| {
            let verdict = null_model::verdict(qv, n.observed, n.expected, n.min_p);
            pair(&sites[c.site], c, n, qv, verdict)
        })
        .collect();
    let stands_out = pairs
        .iter()
        .filter(|p| p.verdict == Verdict::StandsOut)
        .count();
    pairs.sort_by(|x, y| {
        y.overlap
            .total_cmp(&x.overlap)
            .then(y.shared_blocks.cmp(&x.shared_blocks))
            .then_with(|| x.species_a.cmp(&y.species_a))
            .then_with(|| x.species_b.cmp(&y.species_b))
            .then_with(|| x.site.cmp(&y.site))
    });
    pairs.truncate(q.limit);
    Ok(CooccurrenceAnalysis {
        pairs,
        method: COOCCURRENCE_METHOD,
        null_method: SHIFT_NULL_METHOD,
        seed: q.seed,
        draws: nulls.iter().map(|n| n.draws).max().unwrap_or(0),
        stratum_days: STRATUM_DAYS,
        pairs_tested: tests,
        pairs_untested: untested,
        eligible_days: sites.iter().map(|(k, v)| (k.clone(), v.days)).collect(),
        stands_out,
        too_little_data,
        fdr_alpha: null_model::FDR_ALPHA,
    })
}

/// The `limit` strongest pairs over the last `days` days (today included),
/// strongest [`overlap`](SpeciesPair::overlap) first, each with its chance
/// level and verdict ([`cooccurrence_analysis`] at its defaults).
///
/// Only pairs sharing at least `min_shared` blocks are returned: with fewer,
/// two birds heard once, together, would rank as perfectly associated.
///
/// It used to count shared *days*, which is not what the page says ("heard
/// within five minutes of each other"), and ranks a dawn resident with a dusk
/// one as the strongest pair on the station.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn top_cooccurrence_pairs(
    conn: &Connection,
    days: u32,
    limit: usize,
    min_shared: u32,
) -> Result<Vec<SpeciesPair>, DbError> {
    Ok(cooccurrence_analysis(conn, &CooccurrenceQuery::new(days, limit, min_shared))?.pairs)
}

/// Where the listened blocks of [`CoStrata`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ListenedFrom {
    /// The hourly effort ledger: an hour holds as many blocks as it has five minutes of
    /// analysed audio (at most twelve), and never fewer than the blocks a detection proves.
    Ledger,
    /// No ledger in the window: every hour with any detection is taken as listened in full.
    /// A station with no ledger cannot say which quiet hours it listened to.
    DetectionHours,
}

/// The co-occurrence counts by hour of the day, for a chance level that keeps each bird's
/// daily rhythm (CHT-08).
///
/// A block is five minutes of one day. `listened[h]` is how many blocks of clock hour `h`
/// the station listened to in the window; `blocks[i][h]` how many of them held species `i`;
/// `shared[i][j][h]` how many held both `i` and `j`.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CoStrata {
    /// Species, most blocks first (ties by name).
    pub names: Vec<String>,
    /// Listened blocks per clock hour.
    pub listened: Vec<u64>,
    /// `blocks[i][h]`.
    pub blocks: Vec<Vec<u64>>,
    /// `shared[i][j][h]`, symmetric, zero on the diagonal.
    pub shared: Vec<Vec<Vec<u64>>>,
    /// Where `listened` came from.
    pub listened_from: Option<ListenedFrom>,
}

impl CoStrata {
    /// Total blocks species `i` was heard in.
    #[must_use]
    pub fn total(&self, i: usize) -> u64 {
        self.blocks.get(i).map_or(0, |r| r.iter().sum())
    }

    /// Total blocks `i` and `j` shared.
    #[must_use]
    pub fn total_shared(&self, i: usize, j: usize) -> u64 {
        self.shared
            .get(i)
            .and_then(|r| r.get(j))
            .map_or(0, |r| r.iter().sum())
    }
}

/// The co-occurrence counts of the `max_species` species heard in most five-minute blocks over
/// the last `days` days, by clock hour, with the blocks the station listened to in each hour.
///
/// The species kept are the busiest by blocks, not the most connected: choosing by
/// connectedness would pick the pairs to test by their result.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn cooccurrence_strata(
    conn: &Connection,
    days: u32,
    max_species: usize,
) -> Result<CoStrata, DbError> {
    use std::collections::{BTreeMap, HashMap, HashSet};

    // (day, hour) → slots with any detection; species → set of block keys.
    let mut stmt = conn.prepare(
        "SELECT DISTINCT Date,
                CAST(SUBSTR(Time, 1, 2) AS INTEGER) AS h,
                CAST(SUBSTR(Time, 4, 2) AS INTEGER) / 5 AS s,
                Com_Name
         FROM detections_analytic
         WHERE Date >= DATE('now', 'localtime', '-' || (?1 - 1) || ' days')",
    )?;
    let mut rows = stmt.query(params![days])?;
    let mut heard_slots: HashMap<(String, u8), HashSet<u8>> = HashMap::new();
    let mut of_species: HashMap<String, HashSet<(String, u8, u8)>> = HashMap::new();
    while let Some(row) = rows.next()? {
        let date: String = row.get(0)?;
        let h: i64 = row.get(1)?;
        let s: i64 = row.get(2)?;
        let name: String = row.get(3)?;
        let (Ok(h), Ok(s)) = (u8::try_from(h), u8::try_from(s)) else {
            continue;
        };
        if h > 23 || s > 11 {
            continue;
        }
        heard_slots.entry((date.clone(), h)).or_default().insert(s);
        of_species.entry(name).or_default().insert((date, h, s));
    }

    // The ledger: the best-covered source of each (day, hour).
    let mut stmt = conn.prepare(
        "SELECT date, hour, MAX(seconds_analysed) FROM recording_effort_hourly
          WHERE date >= DATE('now', 'localtime', '-' || (?1 - 1) || ' days')
          GROUP BY date, hour",
    )?;
    let ledger: BTreeMap<(String, u8), f64> = stmt
        .query_map(params![days], |r| {
            Ok((
                (r.get::<_, String>(0)?, r.get::<_, u8>(1)?),
                r.get::<_, f64>(2)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    let mut listened = vec![0_u64; 24];
    let listened_from = if ledger.is_empty() {
        for (_, h) in heard_slots.keys() {
            listened[usize::from(*h)] += 12;
        }
        (!heard_slots.is_empty()).then_some(ListenedFrom::DetectionHours)
    } else {
        let mut hours: HashSet<(String, u8)> = ledger.keys().cloned().collect();
        hours.extend(heard_slots.keys().cloned());
        for key in hours {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let from_ledger = ledger
                .get(&key)
                .map_or(0, |s| (s.max(0.0) / 300.0).ceil().min(12.0) as u64);
            let proven = heard_slots.get(&key).map_or(0, |s| s.len() as u64);
            listened[usize::from(key.1)] += from_ledger.max(proven);
        }
        Some(ListenedFrom::Ledger)
    };

    let mut ranked: Vec<(String, usize)> = of_species
        .iter()
        .map(|(n, b)| (n.clone(), b.len()))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(max_species);
    let names: Vec<String> = ranked.into_iter().map(|(n, _)| n).collect();
    let n = names.len();
    let mut blocks = vec![vec![0_u64; 24]; n];
    let mut shared = vec![vec![vec![0_u64; 24]; n]; n];
    for (i, name) in names.iter().enumerate() {
        let mine = &of_species[name];
        for (_, h, _) in mine {
            blocks[i][usize::from(*h)] += 1;
        }
        for j in i + 1..n {
            for key in mine.intersection(&of_species[&names[j]]) {
                let h = usize::from(key.1);
                shared[i][j][h] += 1;
                shared[j][i][h] += 1;
            }
        }
    }
    Ok(CoStrata {
        names,
        listened,
        blocks,
        shared,
        listened_from,
    })
}

/// The `limit` species heard on the same days as `trigger_species` over the last `days` days,
/// strongest [`overlap`](FollowOn::overlap) first (then most shared days, then surest, then by
/// name).
///
/// It used to rank by shared days alone, the measure the pair view left for the same reason:
/// the commonest residents share most of anyone's days, so they were every species' first
/// "companions" and the answer barely depended on the species asked about.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn companion_species(
    conn: &Connection,
    trigger_species: &str,
    days: u32,
    limit: usize,
) -> Result<Vec<FollowOn>, DbError> {
    let mut stmt = conn.prepare(
        "WITH win AS (
            SELECT Date, COALESCE(Source, '') AS site, Com_Name, Confidence
            FROM detections_analytic
            WHERE Date >= DATE('now', 'localtime', '-' || ?2 || ' days')
         ),
         trigger_dates AS (
            SELECT DISTINCT Date, site FROM win WHERE Com_Name = ?1
         ),
         days_heard AS (
            SELECT Com_Name, COUNT(*) AS n
            FROM (SELECT DISTINCT Com_Name, Date, site FROM win WHERE Com_Name != ?1)
            GROUP BY Com_Name
         ),
         shared AS (
            SELECT w.Com_Name,
                   COUNT(DISTINCT w.Date || char(31) || w.site) AS shared_days,
                   AVG(w.Confidence) AS avg_confidence
            FROM win w
            JOIN trigger_dates td ON w.Date = td.Date AND w.site = td.site
            WHERE w.Com_Name != ?1
            GROUP BY w.Com_Name
         )
         SELECT ?1, s.Com_Name, s.shared_days, (SELECT COUNT(*) FROM trigger_dates), h.n,
                s.avg_confidence
         FROM shared s
         JOIN days_heard h ON h.Com_Name = s.Com_Name",
    )?;

    let mut rows = stmt
        .query_map(params![trigger_species, days], |row| {
            let shared_days: i64 = row.get(2)?;
            let trigger_days: i64 = row.get(3)?;
            let companion_days: i64 = row.get(4)?;
            let either = trigger_days + companion_days - shared_days;
            #[allow(clippy::cast_precision_loss)]
            let overlap = if either > 0 {
                shared_days as f64 / either as f64
            } else {
                0.0
            };
            Ok(FollowOn {
                trigger: row.get(0)?,
                companion: row.get(1)?,
                shared_days,
                trigger_days,
                companion_days,
                overlap,
                avg_confidence: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.sort_by(|x, y| {
        y.overlap
            .total_cmp(&x.overlap)
            .then(y.shared_days.cmp(&x.shared_days))
            .then(y.avg_confidence.total_cmp(&x.avg_confidence))
            .then_with(|| x.companion.cmp(&y.companion))
    });
    rows.truncate(limit);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// `ANA7`: "together" means heard in the same five-minute block, and a
    /// pair is ranked by how much of its time it shares, not by how common
    /// its members are.
    ///
    /// Two residents heard every day — one at dawn, one at dusk, never within
    /// hours of each other — shared every *day*, so the page's "heard within
    /// five minutes of each other" ranked them as the strongest pair on the
    /// station. A rarer pair heard a minute apart three times is the real
    /// association.
    #[test]
    fn together_means_the_same_five_minutes_not_the_same_day() {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        let mut rows = Vec::new();
        for back in 1..=10 {
            let d = format!("DATE('now', 'localtime', '-{back} days')");
            rows.push(format!("({d},'05:30:00','Turdus merula','Blackbird',0.9)"));
            rows.push(format!("({d},'20:30:00','Strix aluco','Tawny Owl',0.9)"));
            if back <= 3 {
                rows.push(format!("({d},'07:01:00','Sitta europaea','Nuthatch',0.9)"));
                rows.push(format!(
                    "({d},'07:02:00','Certhia familiaris','Treecreeper',0.9)"
                ));
            }
        }
        conn.execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES {};",
            rows.join(",")
        ))
        .unwrap();

        let pairs = top_cooccurrence_pairs(&conn, 30, 10, 1).unwrap();
        let names: Vec<(&str, &str)> = pairs
            .iter()
            .map(|p| (p.species_a.as_str(), p.species_b.as_str()))
            .collect();
        assert_eq!(
            names.first(),
            Some(&("Nuthatch", "Treecreeper")),
            "the pair heard a minute apart is the strongest: {names:?}"
        );
        assert!(
            !names.contains(&("Blackbird", "Tawny Owl")),
            "dawn and dusk residents are never together: {names:?}"
        );
    }

    /// CHT-08: the strata hold each hour's listened blocks (from the ledger, never fewer than a
    /// detection proves), each species' blocks and each pair's shared blocks, by hour.
    #[test]
    fn the_strata_count_by_hour_against_what_was_listened() {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        let mut rows = Vec::new();
        let mut effort = Vec::new();
        for back in 1..=4 {
            let d = format!("DATE('now', 'localtime', '-{back} days')");
            rows.push(format!("({d},'05:01:00','A a','Alpha',0.9)"));
            rows.push(format!("({d},'05:03:00','B b','Beta',0.9)"));
            rows.push(format!("({d},'05:31:00','B b','Beta',0.9)"));
            // Hour 5 fully listened; hour 6 half an hour; hour 20 not in the ledger at all.
            effort.push(format!("({d},5,'mic',3600,3600)"));
            effort.push(format!("({d},6,'mic',1800,3600)"));
            effort.push(format!("({d},6,'usb',900,3600)"));
        }
        let d = "DATE('now', 'localtime', '-1 days')";
        rows.push(format!("({d},'20:10:00','A a','Alpha',0.9)"));
        conn.execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES {};
             INSERT INTO recording_effort_hourly (date, hour, source, seconds_analysed, seconds_up) VALUES {};",
            rows.join(","),
            effort.join(",")
        ))
        .unwrap();
        let s = cooccurrence_strata(&conn, 30, 10).unwrap();
        assert_eq!(s.names, ["Beta", "Alpha"], "most blocks first");
        assert_eq!(s.listened_from, Some(ListenedFrom::Ledger));
        assert_eq!(s.listened[5], 48, "12 blocks on each of 4 days");
        assert_eq!(
            s.listened[6], 24,
            "the best source's half hour: 6 blocks a day"
        );
        assert_eq!(
            s.listened[20], 1,
            "an hour the ledger missed holds the block a detection proves"
        );
        assert_eq!(s.blocks[0][5], 8);
        assert_eq!(s.blocks[1][5], 4);
        assert_eq!(s.shared[0][1][5], 4, "05:01 and 05:03 are one block");
        assert_eq!(s.shared[1][0][5], 4);
        assert_eq!(s.total_shared(0, 1), 4);
        assert_eq!(s.total(1), 5);

        // Counterpart: with no ledger, an hour with a detection is taken as listened in full.
        conn.execute_batch("DELETE FROM recording_effort_hourly")
            .unwrap();
        let s = cooccurrence_strata(&conn, 30, 10).unwrap();
        assert_eq!(s.listened_from, Some(ListenedFrom::DetectionHours));
        assert_eq!((s.listened[5], s.listened[6], s.listened[20]), (48, 0, 12));
    }

    fn setup() -> Connection {
        // Apply the full migration chain rather than a hand-coded CREATE TABLE:
        // ADR-16 flags the latter as the source of three of the PR #35 bugs
        // because the hand-coded schema silently drifts the moment a new
        // migration adds a column. The migration list is the single source
        // of truth.
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        // Dates are computed at insert time via SQLite's DATE('now', 'localtime', '-N days')
        // so the fixture stays within the 30-day window used by the queries
        // under test, regardless of when the suite runs.
        conn.execute_batch(
            "INSERT INTO detections
              (Date, Time, Sci_Name, Com_Name, Confidence,
               Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name)
            VALUES
              (DATE('now', 'localtime', '-7 days'),'07:00:00','A sp','Robin',  0.9, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-7 days'),'07:05:00','B sp','Wren',   0.8, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-7 days'),'08:00:00','C sp','Finch',  0.7, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-6 days'),'07:00:00','A sp','Robin',  0.9, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-6 days'),'07:10:00','B sp','Wren',   0.8, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-5 days'),'07:00:00','A sp','Robin',  0.9, NULL,NULL,NULL,NULL,NULL,NULL,'');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn top_pairs_basic() {
        let conn = setup();
        let pairs = top_cooccurrence_pairs(&conn, 30, 10, 1).unwrap();
        // Robin+Wren share 2 days, Robin+Finch share 1 day
        // Robin and Wren share two days but never a five-minute block (07:00
        // and 07:05, 07:00 and 07:10), so they are not a pair.
        assert!(pairs.is_empty(), "{pairs:?}");
    }

    #[test]
    fn companion_species_robin() {
        let conn = setup();
        let companions = companion_species(&conn, "Robin", 30, 10).unwrap();
        // Wren appears on 2 of 3 robin days, Finch on 1
        assert!(!companions.is_empty());
        assert_eq!(companions[0].companion, "Wren");
        assert_eq!(companions[0].shared_days, 2);
    }

    /// UIC-04. A companion is ranked by how much of their days the two share, not by how
    /// many: a resident heard every day shares every one of the trigger's days and is no
    /// companion for it. Nutcracker heard on ten days; Pine Grosbeak on eight, each one a
    /// Nutcracker day; House Sparrow on every day of the window.
    fn nutcracker_fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        let mut rows = Vec::new();
        for back in 1..=29 {
            let d = format!("DATE('now', 'localtime', '-{back} days')");
            rows.push(format!(
                "({d},'09:00:00','Passer domesticus','House Sparrow',0.9)"
            ));
            if back <= 10 {
                rows.push(format!(
                    "({d},'07:00:00','Nucifraga caryocatactes','Nutcracker',0.9)"
                ));
            }
            if back <= 8 {
                rows.push(format!(
                    "({d},'07:30:00','Pinicola enucleator','Pine Grosbeak',0.9)"
                ));
            }
        }
        conn.execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES {};",
            rows.join(",")
        ))
        .unwrap();
        conn
    }

    #[test]
    fn a_resident_heard_every_day_is_not_the_first_companion() {
        let conn = nutcracker_fixture();
        let names: Vec<String> = companion_species(&conn, "Nutcracker", 30, 10)
            .unwrap()
            .into_iter()
            .map(|c| c.companion)
            .collect();
        // Counterpart: the resident is still listed, below the true companion.
        assert_eq!(names, ["Pine Grosbeak", "House Sparrow"]);
    }

    /// The numbers a page prints beside a companion: "8 of the trigger's 10 days", and the share.
    #[test]
    fn a_companion_carries_the_days_behind_its_rank() {
        let conn = nutcracker_fixture();
        let c = companion_species(&conn, "Nutcracker", 30, 10).unwrap();
        let (grosbeak, sparrow) = (&c[0], &c[1]);
        assert_eq!(
            (
                grosbeak.shared_days,
                grosbeak.trigger_days,
                grosbeak.companion_days
            ),
            (8, 10, 8)
        );
        assert!(
            (grosbeak.overlap - 0.8).abs() < 1e-9,
            "{}",
            grosbeak.overlap
        );
        assert_eq!(
            (
                sparrow.shared_days,
                sparrow.trigger_days,
                sparrow.companion_days
            ),
            (10, 10, 29)
        );
        assert!(
            (sparrow.overlap - 10.0 / 29.0).abs() < 1e-9,
            "{}",
            sparrow.overlap
        );
        // The limit still cuts after the ranking, not before it.
        let one = companion_species(&conn, "Nutcracker", 30, 1).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].companion, "Pine Grosbeak");
    }
}
