//! Migration (phenology) page + partials.
//!
//! Mounts:
//!   GET  /migration                            full page
//!   GET  /pages/migration-stats                4 KPI tiles
//!   GET  /pages/migration-ridgeline            the ridgeline and, under it, species heard per week
//!   GET  /pages/migration-card?kind=…          arrived/peaking/missing editorial card
//!
//! All renders are pure SVG strings, complete with no script (the chart kit adds the
//! hover and the keys; the ridgeline itself is `viz/ridge.rs`).
//! Designed to use only the existing `detections` table; no schema migration.
//!
//! # No panicking operations in this module
//!
//! `[profile.release]` sets `panic = "abort"` and the server mounts no
//! catch-panic layer, so a panic in a request handler does not produce a 500 —
//! it takes the whole process down, web server and detection daemon together,
//! and systemd restarts it. A reachable handler panic is therefore a station
//! outage, not a failed request.
//!
//! This module sorted and compared `f32` with `partial_cmp(..).unwrap()`. The
//! values are sums of integer detection counts, so no reachable input is `NaN`
//! today and it was latent rather than live — but the cost of that assessment
//! being wrong later is the whole station, so the comparisons use
//! [`f32::total_cmp`], which is total by construction. `unwrap`/`expect` are
//! denied here so the class cannot return unnoticed; a genuinely infallible
//! call may re-allow the lint locally with a comment saying why.

#![deny(clippy::unwrap_used, clippy::expect_used)]
// Adapted SVG-rendering module: int<->float coordinate casts, short math
// identifiers, and long path-builder functions are intrinsic to this code.
#![allow(clippy::pedantic, clippy::nursery)]

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::{Router, routing::get};
use serde::Deserialize;

use crate::analytics_cache::cached_fragment;
use crate::state::AppState;

use super::escape_html;
use super::viz::ridge::{self, Phenology, Ridge};

static PAGE_HTML: crate::seal::Sealed =
    crate::seal::Sealed::new(include_str!("../../../templates/migration.html"));

/// What a *failed* fragment renders, matching the three sibling analytics pages
/// (`heatmap`, `correlation`, `timeseries_dash`) which all use this wording.
const FRAGMENT_ERR: &str = r#"<p class="bnb-meta">Analytics temporarily unavailable.</p>"#;

/// Mount the migration (phenology) page and its HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/migration-stats", get(stats_partial))
        .route("/pages/migration-ridgeline", get(ridgeline_partial))
        .route("/pages/migration-card", get(card_partial))
}

/// The migration/phenology surface, rendered for embedding by
/// `homes::patterns` ("Migration" tab).
pub(super) fn content() -> String {
    let year = current_year();
    // Skeleton placeholders (O-16) shown until the htmx swap targets load.
    // O-20 help link drops the methodology shortcut into the eyebrow.
    // What the page covers, as a sentence: these were three pills styled like the range
    // buttons on the pages beside it, and a reader pressed them (UIR-36).
    let scope = format!(
        r#"<p class="bnb-meta mig-cover">Calendar year {year}, seasonal birds only, compared with last year.</p>"#
    );
    let head = super::frame::page_head(&super::frame::Head {
        eyebrow: "Patterns",
        help: &super::help::help_link(super::help::Topic::Phenology),
        title: "Arrivals and <em>departures</em>.",
        lead: "Each ridge is one species' year.",
        lede: "Week by week, scaled to its own busiest week so a small-population migrant stays visible. Its ink says whether it peaks in a migration window; where the peak stands says which, the spring one or the fall one, and both are marked above the chart. A bird is drawn when its busiest week stands well above its usual one per hour the station listened, so a week the station was off, or listened longer, does not make a resident look like a migrant. Under the ridges: how many different species were heard each week.",
        aside: &scope,
    });
    PAGE_HTML
        .replace("{{page_head}}", &head)
        .replace("{{skel_migration_stats}}", &super::skeletons::stat_row(4))
        .replace("{{skel_ridgeline}}", super::skeletons::ridgeline())
}

// ---------------------------------------------------------------------------
// Data shape
// ---------------------------------------------------------------------------

/// How many ridges the chart draws: past this the names stand on one another.
const MAX_RIDGES: usize = 12;

/// Fewer listened days than this in a week column and the week's index is unknown: one or
/// two days are a sample of anything (`viz::uncertainty::TOO_FEW_BELOW`, the same three).
const MIN_LISTENED_DAYS: u32 = 3;

/// A species that earned a ridge: its stored name, its detections in each week column and how
/// sharply seasonal it is.
struct Seasonal {
    name: String,
    counts: [f64; 52],
    /// Busiest week over the median week it was heard in, per listened time.
    seasonality: f64,
}

/// One species' year on the chart's week columns.
struct Weekly {
    name: String,
    /// Every detection in each column: what the ridge draws (the kit's tooltip and table
    /// carry these real counts).
    counts: [f64; 52],
    /// The detections on the days the index counts (see [`Listening`]).
    counted: [f64; 52],
    /// Detections per seven typical days of listening, or `None` where the column had fewer
    /// than [`MIN_LISTENED_DAYS`] days to count: unknown, never zero.
    index: [Option<f64>; 52],
}

impl Weekly {
    /// A station that listened to every day of every week, a typical day each: the index is
    /// the count. What the page assumed before it read the ledger (and the tests of the
    /// thresholds use).
    #[cfg(test)]
    fn fully_listened(name: String, counts: [f64; 52]) -> Self {
        Self {
            name,
            counts,
            counted: counts,
            index: counts.map(Some),
        }
    }
}

/// How sharply seasonal a fully listened year of weekly counts is (see [`seasonal_index`]).
#[cfg(test)]
fn seasonal_ratio(counts: &[f64; 52]) -> Option<f64> {
    seasonal_index(&counts.map(Some), counts)
}

/// How sharply seasonal a species' year is, or `None` if it has not earned a ridge.
///
/// `index` is detections per seven typical days of listening in each week column (`None`
/// where too little was listened to say), and `counted` the detections it was made from.
/// A species earns a ridge when it was heard in at least four weeks with an index, the
/// busiest of them (by index) held at least five detections, and that week's index was at
/// least three times its usual week's (the median of its weeks heard). That is a cheap
/// proxy for "migratory": a resident holds a steady ribbon and is left out, a bird that
/// passes through spikes. It is a proxy and the chart says "per-species weekly index", not
/// "migrants"; refine it with `birdnet_behavioral::ResidencyType::Migrant` when that is
/// wanted.
///
/// Per listened time and not per raw count (N24-08): a station that listened six hours a
/// day for most of the year and all day for six weeks heard a resident four times as often
/// in those six weeks, and the raw count drew it as a migrant.
fn seasonal_index(index: &[Option<f64>; 52], counted: &[f64; 52]) -> Option<f64> {
    let (peak_week, peak) = index
        .iter()
        .enumerate()
        .filter_map(|(w, v)| v.map(|v| (w, v)))
        .fold(
            (0, 0.0_f64),
            |best, (w, v)| if v > best.1 { (w, v) } else { best },
        );
    let mut heard: Vec<f64> = index
        .iter()
        .flatten()
        .copied()
        .filter(|v| *v > 0.0)
        .collect();
    if heard.len() < 4 || counted[peak_week] < 5.0 {
        return None;
    }
    heard.sort_by(f64::total_cmp);
    let median = heard[heard.len() / 2].max(1.0);
    let ratio = peak / median;
    (ratio >= 3.0).then_some(ratio)
}

/// The species to draw, from every species' weeks (fully listened): see [`pick`].
#[cfg(test)]
fn pick_ridges(all: Vec<(String, [f64; 52])>, max_species: usize) -> Vec<Seasonal> {
    pick(
        all.into_iter()
            .map(|(name, counts)| Weekly::fully_listened(name, counts))
            .collect(),
        max_species,
    )
}

/// The species to draw: those that earned a ridge, the `max_species` most sharply seasonal of
/// them, in the order of their busiest week so ridges read left to right.
///
/// The cut is by seasonality and not by the order of the year. It used to sort by peak week
/// and then truncate, which kept the twelve that peak *earliest*: with more than twelve
/// qualifying species the fall migrants were dropped without a word.
fn pick(all: Vec<Weekly>, max_species: usize) -> Vec<Seasonal> {
    let mut earned: Vec<Seasonal> = all
        .into_iter()
        .filter_map(|w| {
            seasonal_index(&w.index, &w.counted).map(|seasonality| Seasonal {
                name: w.name,
                counts: w.counts,
                seasonality,
            })
        })
        .collect();
    earned.sort_by(|a, b| {
        b.seasonality
            .total_cmp(&a.seasonality)
            .then(a.name.cmp(&b.name))
    });
    earned.truncate(max_species);
    let peak_week = |s: &Seasonal| {
        s.counts
            .iter()
            .enumerate()
            .fold(0, |best, (i, c)| if *c > s.counts[best] { i } else { best })
    };
    earned.sort_by(|a, b| peak_week(a).cmp(&peak_week(b)).then(a.name.cmp(&b.name)));
    earned
}

/// How much listening each week column of a year held, from the effort ledger
/// (`pages/listening.rs`, the one listened-day rule).
///
/// A day counts toward a week's index when it is before `today` (today's ledger is still
/// being credited) and the station listened to it: by the rule where the ledger covers the
/// date, and, where it does not (a station older than its ledger, or imported history), as
/// one typical day listened, which is the trend queries' choice for a date nobody measured,
/// so a year of imported records still has migrants. A counted day weighs its seconds over
/// the typical day (an all-day day on a six-hour station is four); a date the ledger does not
/// cover weighs one.
struct Listening {
    /// Counted days per column.
    days: [u32; 52],
    /// Typical days of listening per column.
    exposure: [f64; 52],
    /// The dates counted.
    counted: std::collections::BTreeSet<String>,
}

impl Listening {
    fn read(conn: &rusqlite::Connection, year: i32, today: &str) -> rusqlite::Result<Self> {
        let ledger = super::listening::StationLedger::read(conn)?;
        let mut out = Self {
            days: [0; 52],
            exposure: [0.0; 52],
            counted: std::collections::BTreeSet::new(),
        };
        let first = crate::routes::pages::date_to_epoch_days(&format!("{year}-01-01"));
        let last = crate::routes::pages::date_to_epoch_days(&format!("{year}-12-31"));
        for e in first..=last {
            let (y, m, d) = crate::routes::pages::days_to_date(e);
            let date = format!("{y}-{m:02}-{d:02}");
            if date.as_str() >= today {
                break;
            }
            let weight = match (ledger.listened(&date), ledger.typical()) {
                (Some(true), Some(t)) => ledger.seconds(&date) / t,
                (Some(_), _) => continue,
                (None, _) => 1.0,
            };
            let w = usize::from(week_of_year(&date));
            out.days[w] += 1;
            out.exposure[w] += weight;
            out.counted.insert(date);
        }
        Ok(out)
    }

    /// The index of one week: `counted` detections per seven typical days, or unknown.
    fn index(&self, week: usize, counted: f64) -> Option<f64> {
        (self.days[week] >= MIN_LISTENED_DAYS && self.exposure[week] > 0.0)
            .then(|| counted / self.exposure[week] * 7.0)
    }
}

/// Every species' weeks of `year` as of the local date `today`.
fn weekly(conn: &rusqlite::Connection, year: i32, today: &str) -> rusqlite::Result<Vec<Weekly>> {
    let listening = Listening::read(conn, year, today)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT Com_Name, Date, {WEEK_COLUMN} AS wk, COUNT(*) AS n \
         FROM detections_analytic \
         WHERE Date LIKE ?1 \
         GROUP BY Com_Name, Date"
    ))?;
    let prefix = format!("{year}-%");
    let rows = stmt.query_map([&prefix], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;

    use std::collections::HashMap;
    let mut by_species: HashMap<String, ([f64; 52], [f64; 52])> = HashMap::new();
    for row in rows {
        let (name, date, wk, n) = row?;
        let w = wk.clamp(0, 51) as usize; // WEEK_COLUMN is already 0..=51
        let e = by_species.entry(name).or_insert(([0.0; 52], [0.0; 52]));
        e.0[w] += n as f64;
        if listening.counted.contains(&date) {
            e.1[w] += n as f64;
        }
    }
    Ok(by_species
        .into_iter()
        .map(|(name, (counts, counted))| {
            let mut index = [None; 52];
            for (w, slot) in index.iter_mut().enumerate() {
                *slot = listening.index(w, counted[w]);
            }
            Weekly {
                name,
                counts,
                counted,
                index,
            }
        })
        .collect())
}

fn collect_ridges(
    conn: &rusqlite::Connection,
    year: i32,
    today: &str,
    max_species: usize,
) -> rusqlite::Result<Vec<Seasonal>> {
    Ok(pick(weekly(conn, year, today)?, max_species))
}

// ---------------------------------------------------------------------------
// Ridgeline
// ---------------------------------------------------------------------------

/// `None` means **the query failed**; an empty year returns its own body.
///
/// This used to return `None` for both, with the empty body as
/// `cached_fragment`'s fallback, so a database error on the Migration tab was
/// reported to the operator as "No migratory species detected yet this year."
/// The three sibling analytics pages already follow the contract restored
/// here — `Some` for anything renderable, `None` only for a failure — and
/// their fallback is an error message.
fn compute_ridgeline(state: &AppState) -> Option<String> {
    let year = current_year();
    // Every read propagates: a failed one is a failed fragment, not a quiet year.
    let (ridges, diversity, latitude) = state
        .with_read_db(|conn| {
            let today = crate::routes::pages::today_date_string();
            let ridges = collect_ridges(conn, year, &today, MAX_RIDGES)?;
            let diversity = weekly_diversity(conn, year)?;
            let latitude = super::station_location(conn).map(|l| l.lat);
            Ok::<_, rusqlite::Error>((ridges, diversity, latitude))
        })
        .ok()?;
    if ridges.is_empty() {
        return Some(super::empty_states::no_migrants());
    }
    Some(render_ridgeline(
        &ridges,
        &diversity,
        latitude,
        year,
        current_week(),
        state.names(),
    ))
}

async fn ridgeline_partial(State(state): State<AppState>) -> impl IntoResponse {
    let html = cached_fragment(
        &state,
        "migration-ridgeline".to_string(),
        FRAGMENT_ERR,
        compute_ridgeline,
    )
    .await;
    ok_html(html)
}

/// The ridgeline for `year` as of chart column `today_week`, over the species that earned one.
fn render_ridgeline(
    ridges: &[Seasonal],
    diversity: &[i64; 52],
    latitude: Option<f64>,
    year: i32,
    today_week: u8,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    let (spring, fall) = ridge::migration_windows(latitude);
    let drawn: Vec<Ridge> = ridges
        .iter()
        .map(|r| Ridge {
            name: names.name_of_stored(&r.name).to_owned(),
            counts: r.counts,
        })
        .collect();
    ridge::ridgeline(&Phenology {
        ridges: &drawn,
        diversity,
        year,
        today_week: Some(usize::from(today_week)),
        spring,
        fall,
    })
}

/// The chart column a `Date` falls in: `(day-of-year − 1) / 7`, the last
/// column absorbing the year's final eight or nine days.
///
/// Every query on this page and [`week_of_year`] use this one definition
/// (`ANA14b`). They used `%W`, which runs 0..=53 against 52 columns: the
/// diversity strip dropped weeks 52 and 53, the ridgeline folded them into
/// column 51, and the peak tile printed `w53` and `w54`.
const WEEK_COLUMN: &str = "MIN((CAST(strftime('%j', Date) AS INTEGER) - 1) / 7, 51)";

/// Distinct species per week of `year`, one slot per chart column.
fn weekly_diversity(conn: &rusqlite::Connection, year: i32) -> rusqlite::Result<[i64; 52]> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {WEEK_COLUMN} AS wk, COUNT(DISTINCT Com_Name) \
         FROM detections_analytic \
         WHERE Date LIKE ?1 \
         GROUP BY wk \
         ORDER BY wk"
    ))?;
    let prefix = format!("{year}-%");
    let rows = stmt.query_map([&prefix], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut weekly = [0i64; 52];
    for row in rows {
        let (w, n) = row?;
        if (0..52).contains(&w) {
            weekly[w as usize] = n;
        }
    }
    Ok(weekly)
}

// ---------------------------------------------------------------------------
// KPI tiles
// ---------------------------------------------------------------------------

async fn stats_partial(State(state): State<AppState>) -> impl IntoResponse {
    // A failed read renders `FRAGMENT_ERR` and is not cached. It used to be
    // defaulted to `(0, 0, 0, None, 0)` and cached, so one database error read
    // "First-of-year arrivals 0 · w0 · no overdue migrants" as fact, and kept
    // saying so after the database recovered.
    let html = cached_fragment(&state, "migration-stats".to_string(), FRAGMENT_ERR, |s| {
        let today = crate::routes::pages::today_date_string();
        s.with_read_db(|conn| stats_fragment(conn, &today, s.names()))
            .ok()
    })
    .await;
    ok_html(html)
}

/// What the station's own recording history lets this page compare with last year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LastYear {
    /// Nothing was recorded in the year before this one.
    NotRecorded,
    /// The station began part-way through it: a species' first date that year is then
    /// where the recording began, not where the bird arrived.
    PartlyRecorded,
    /// Recording was under way from the start of the year (within a fortnight of 1 January),
    /// so a first date in it is an arrival.
    Whole,
}

/// How much of the year before `year` the station recorded, from its first recorded day
/// (`None` when it has recorded nothing).
///
/// A first-heard date is only an arrival if the station could have heard the bird before it.
/// The "earliest vs last year" tile used to ask only that the station had been recording a
/// fortnight before the *species'* first date. A station installed on 26 October answered yes
/// for a nuthatch first heard on 24 December, compared it with the nuthatch's first date this
/// year (3 January) as if both were arrivals, and printed "−355 d": a resident, "arriving" 355
/// days early, because last year's calendar began in late October.
fn last_year_coverage(first_recorded: Option<&str>, year: i32) -> LastYear {
    let prior = year - 1;
    match first_recorded {
        None => LastYear::NotRecorded,
        Some(d) if year_of(d) >= year => LastYear::NotRecorded,
        // ISO dates sort as text.
        Some(d) if d <= format!("{prior}-01-14").as_str() => LastYear::Whole,
        Some(_) => LastYear::PartlyRecorded,
    }
}

/// SQL for each species' onset over the dates `from..=to` (two placeholders): the
/// `(n + 19) / 20`-th of its `n` distinct detection days, which is `ceil(0.05 n)`, the
/// phenology module's `quantile_disc(detection_date, 0.05)`. Rows: `Com_Name`, `d`.
fn onset_sql(from: &str, to: &str) -> String {
    format!(
        "SELECT Com_Name, d FROM ( \
           SELECT Com_Name, d, \
                  ROW_NUMBER() OVER (PARTITION BY Com_Name ORDER BY d) AS rn, \
                  COUNT(*) OVER (PARTITION BY Com_Name) AS n \
           FROM (SELECT DISTINCT Com_Name, Date AS d FROM detections_analytic \
                 WHERE Date >= {from} AND Date <= {to}) \
         ) WHERE rn = (n + 19) / 20"
    )
}

/// The chart column `week` (0-based) of `year` as a reader says it, "late Sep", and its
/// 1-based number, 39: the tile prints "late Sep" over "week 39" (UIR-23). A column is named by the day in its middle, the fourth of its seven (the last
/// column's eight or nine days by the middle one, rounded down): the 1st to the 10th of a
/// month is early, the 11th to the 20th mid, from the 21st late. The number is the column's
/// own, as the ridgeline's axis and key count them, so there is never a week 53.
fn week_words(year: i32, week: usize) -> (String, usize) {
    let week = week.min(ridge::WEEKS - 1);
    let (first, last) = ridge::week_days(year, week);
    let middle = u64::from((first + last) / 2);
    let jan1 = crate::routes::pages::date_to_epoch_days(&format!("{year}-01-01"));
    let (_, m, d) = crate::routes::pages::days_to_date(jan1 + middle - 1);
    let part = match d {
        1..=10 => "early",
        11..=20 => "mid",
        _ => "late",
    };
    let month = super::viz::kit::MONTHS[(m as usize).clamp(1, 12) - 1];
    (format!("{part} {month}"), week + 1)
}

/// The four KPI tiles, as of the local date `today`.
///
/// Every query propagates its error: the tiles are claims about the reader's
/// data, and a count that failed is not a count of zero.
fn stats_fragment(
    conn: &rusqlite::Connection,
    today: &str,
    names: &birdnet_core::i18n::NameLocalizer,
) -> rusqlite::Result<String> {
    use rusqlite::OptionalExtension as _;
    let year = year_of(today);
    let prior = year - 1;

    // First-of-year arrivals = species with first_date in this year so far.
    let foy: i64 = conn.query_row(
        "SELECT COUNT(*) FROM ( \
           SELECT Com_Name, MIN(Date) AS first FROM detections_analytic GROUP BY Com_Name \
         ) WHERE first LIKE ?1",
        [format!("{year}-%")],
        |r| r.get(0),
    )?;
    // Peak diversity week; `None` is a year with no detections yet.
    let peak: Option<(i64, i64)> = conn
        .query_row(
            &format!(
                "SELECT {WEEK_COLUMN} wk, COUNT(DISTINCT Com_Name) n \
                 FROM detections_analytic WHERE Date LIKE ?1 GROUP BY wk ORDER BY n DESC LIMIT 1"
            ),
            [format!("{year}-%")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    let first_recorded: Option<String> = conn.query_row(
        "SELECT MIN(Date) FROM detections WHERE date(Date) IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let coverage = last_year_coverage(first_recorded.as_deref(), year);

    // Earliest-vs-last-year: of the species that *arrived* in both years, the one whose
    // arrival came furthest ahead of last year's (negative: this year was earlier).
    //
    // An arrival is the phenology module's onset (`phenology/timing.rs`, `arrival_q05`): the
    // 5th percentile of the year's distinct detection days, the `ceil(0.05 n)`-th of `n`
    // (N05-02). That query is `DuckDB` SQL over the analytics copy and this tile reads the
    // `SQLite` store (and is drawn in a build without the analytics store), so the rule is
    // restated here in whole numbers, `(n + 19) / 20`, and held to the engine's by
    // `tests/the_migration_onset_is_the_phenology_onset.rs`. It was `MIN(Date)`, so one false
    // positive moved the tile: a stray January detection of a May swallow made it no arrival
    // at all, and a stray in March an arrival two months early. Like the phenology rows it
    // trims a stray day only once there are enough days to trim from (more than twenty).
    //
    // Last year is read to the same calendar date as today, so both onsets are taken over
    // the same stretch of their year: a percentile of a season half heard is earlier than
    // one of the whole season, and comparing the two would make every year look early.
    //
    // An arrival is an onset from the end of January on. A bird already singing in the
    // first four weeks of a year is a resident or a winter visitor, and its "first date" is
    // the start of the calendar, not a journey: comparing two of those printed "−1 d" for a
    // cardinal. And the prior year must have been recorded from its start (see
    // [`last_year_coverage`]).
    let earliest: Option<(String, i64)> = if coverage == LastYear::Whole {
        conn.query_row(
            &format!(
                "WITH first_this AS ({this}), first_prior AS ({prior}) \
                 SELECT t.Com_Name, \
                        CAST(julianday(t.d) - julianday(?3 || substr(p.d, 5)) AS INTEGER) AS delta \
                 FROM first_this t JOIN first_prior p USING (Com_Name) \
                 WHERE p.d >= ?4 AND t.d >= ?5 \
                 ORDER BY delta ASC LIMIT 1",
                this = onset_sql("?1", "?6"),
                prior = onset_sql("?2", "?7"),
            ),
            [
                format!("{year}-01-01"),
                format!("{prior}-01-01"),
                format!("{year}"),
                format!("{prior}-01-29"),
                format!("{year}-01-29"),
                today.to_owned(),
                format!("{prior}{}", today.get(4..).unwrap_or("-12-31")),
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
    } else {
        None
    };

    let earliest_html = match (coverage, earliest) {
        (LastYear::Whole, Some((species, delta))) => {
            let species = escape_html(names.name_of_stored(&species));
            match delta.cmp(&0) {
                std::cmp::Ordering::Less => format!(
                    r#"<span class="value">{delta} d</span><span class="bnb-meta">{species}</span>"#
                ),
                std::cmp::Ordering::Equal => format!(
                    r#"<span class="value">on time</span><span class="bnb-meta">{species}</span>"#
                ),
                std::cmp::Ordering::Greater => format!(
                    r#"<span class="value">+{delta} d</span><span class="bnb-meta">{species}</span>"#
                ),
            }
        }
        (LastYear::Whole, None) => {
            r#"<span class="value">—</span><span class="bnb-meta">no bird has arrived in both years yet</span>"#
                .to_string()
        }
        (LastYear::PartlyRecorded, _) => {
            r#"<span class="value">—</span><span class="bnb-meta">last year was only partly recorded</span>"#
                .to_string()
        }
        (LastYear::NotRecorded, _) => {
            r#"<span class="value">—</span><span class="bnb-meta">no prior-year data</span>"#.to_string()
        }
    };

    // Still expected = species that arrived in the prior year within the next
    // six weeks of today, but not heard yet this year. A zero is only said where there
    // was a last year to be expecting from.
    let heard_last_year: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT Com_Name) FROM detections_analytic WHERE Date LIKE ?1",
        [format!("{prior}-%")],
        |r| r.get(0),
    )?;
    let still_expected = still_expected_count(conn, today)?;
    let expected_html = if heard_last_year == 0 {
        r#"<span class="value">—</span><span class="bnb-meta">needs a year of records</span>"#
            .to_string()
    } else if still_expected > 0 {
        format!(
            r#"<span class="value">{still_expected}</span><span class="bnb-meta">arriving within 6 wk</span>"#
        )
    } else {
        r#"<span class="value">0</span><span class="bnb-meta">none outstanding for 6 wk</span>"#
            .to_string()
    };

    // A time of year a reader says, with the column's 1-based number as the ridgeline's axis
    // counts it (it printed the 0-based column once, so one week had two names on one page,
    // and then `w39`, which is a name only to whoever wrote it).
    let peak_html = peak.map_or_else(
        || r#"<span class="value mig-peak-val">—</span><span class="bnb-meta mono">no detections yet</span>"#.to_string(),
        |(wk, n)| {
            let (when, number) = week_words(year, usize::try_from(wk).unwrap_or(0));
            format!(
                r#"<span class="value mig-peak-val">{when}</span><span class="bnb-meta mono">week {number} · {n} species</span>"#
            )
        },
    );

    Ok(format!(
        r#"<div class="stat-tile"><span class="label">First-of-year arrivals</span><span class="value">{foy}</span></div>
<div class="stat-tile"><span class="label">Peak diversity week</span>{peak_html}</div>
<div class="stat-tile"><span class="label">Earliest vs last year</span>{earliest_html}</div>
<div class="stat-tile"><span class="label">Still expected</span>{expected_html}</div>"#,
    ))
}

// ---------------------------------------------------------------------------
// Editorial cards
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CardQuery {
    kind: String,
}

async fn card_partial(
    State(state): State<AppState>,
    Query(q): Query<CardQuery>,
) -> impl IntoResponse {
    let kind = q.kind;
    let kind_db = kind.clone();
    let names = state.names_handle();
    let permit = state.heavy_read_permit().await;
    let result = tokio::task::spawn_blocking(move || {
        let _slot = permit;
        let today = crate::routes::pages::today_date_string();
        state.with_read_db(|conn| card_content(conn, &kind_db, &today, &names))
    })
    .await;

    // "No row" is a quiet week; an error is not. `.ok()` made both the same, and
    // the card told a station whose database had failed that nothing had arrived.
    let (eyebrow, headline, sub) = match result {
        Ok(Ok(card)) => card,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, kind = %kind, "migration card: query failed");
            return ok_html(super::error_states::inline("this card"));
        }
        Err(e) => {
            tracing::warn!(error = %e, kind = %kind, "migration card: task failed");
            return ok_html(super::error_states::inline("this card"));
        }
    };
    let html = format!(
        r#"<div class="bnb-eyebrow" data-style="color:{accent}">{eyebrow}</div>
<div class="display mig-headline">{headline}</div>
<div class="bnb-meta mig-sub-line">{sub}</div>"#,
        accent = card_color(&kind),
        eyebrow = escape_html(eyebrow),
        headline = escape_html(&headline),
        sub = escape_html(&sub),
    );
    ok_html(html)
}

/// One editorial card: its eyebrow, its headline and the line under it, as of `today`.
///
/// Every state says what it is. "Missing" was a permanent "— / Forecast model pending" that
/// no station could ever fill: the data to answer it was already in the tiles' query (what
/// arrived around now last year and has not been heard yet), so it is that list now.
fn card_content(
    conn: &rusqlite::Connection,
    kind: &str,
    today: &str,
    names: &birdnet_core::i18n::NameLocalizer,
) -> rusqlite::Result<(&'static str, String, String)> {
    use rusqlite::OptionalExtension as _;
    let year = year_of(today);
    Ok(match kind {
        "arrived" => {
            // The most recent species whose first-ever detection is in this year.
            let row: Option<(String, String)> = conn
                .query_row(
                    "SELECT Com_Name, MIN(Date) FROM detections_analytic \
                     GROUP BY Com_Name HAVING MIN(Date) LIKE ?1 \
                     ORDER BY MIN(Date) DESC LIMIT 1",
                    [format!("{year}-%")],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match row {
                Some((name, date)) => (
                    "Just arrived",
                    names.name_of_stored(&name).to_owned(),
                    format!("First heard {date}"),
                ),
                None => ("Just arrived", "No arrivals yet".into(), "Watching…".into()),
            }
        }
        "peaking" => {
            let row: Option<(String, i64)> = conn
                .query_row(
                    "SELECT Com_Name, COUNT(*) AS n FROM detections_analytic \
                     WHERE Date >= date('now','localtime','-7 days') \
                     GROUP BY Com_Name ORDER BY n DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match row {
                Some((name, n)) => (
                    "Currently peaking",
                    names.name_of_stored(&name).to_owned(),
                    format!("{n} detections this week"),
                ),
                None => (
                    "Currently peaking",
                    "Quiet week".into(),
                    "No species over 10 detections".into(),
                ),
            }
        }
        _ => {
            let heard_last_year: i64 = conn.query_row(
                "SELECT COUNT(DISTINCT Com_Name) FROM detections_analytic WHERE Date LIKE ?1",
                [format!("{}-%", year - 1)],
                |r| r.get(0),
            )?;
            if heard_last_year == 0 {
                ("Missing", "—".into(), "needs a year of records".into())
            } else {
                let overdue = overdue_arrivals(conn, today)?;
                missing_card(&overdue, names)
            }
        }
    })
}

/// The "Missing" card for the species `overdue` (stored name, the date it was first heard last
/// year), most overdue first.
fn missing_card(
    overdue: &[(String, String)],
    names: &birdnet_core::i18n::NameLocalizer,
) -> (&'static str, String, String) {
    match overdue {
        [] => (
            "Missing",
            "Nothing overdue".into(),
            "Every bird that arrived in these weeks last year has been heard".into(),
        ),
        [(name, date), rest @ ..] => {
            let more = if rest.is_empty() {
                String::new()
            } else {
                format!(" · +{} more", rest.len())
            };
            (
                "Missing",
                names.name_of_stored(name).to_owned(),
                format!("First heard {date} last year, not yet this year{more}"),
            )
        }
    }
}

fn card_color(kind: &str) -> &'static str {
    match kind {
        "arrived" => "var(--moss-ink)",
        "peaking" => "var(--dawn-ink)",
        _ => "var(--rare)",
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// How far ahead the "still expected" tile looks.
const WINDOW_DAYS: u64 = 42;

/// Species that arrived in the prior year within the next [`WINDOW_DAYS`] and
/// have not been heard yet this year.
///
/// `today` is the station's **local** date, passed in rather than read from the
/// clock inside the query. The old form compared day-of-year numbers against
/// `strftime('%j','now')` and `strftime('%j','now','+42 days')`, which fails
/// three ways at once:
///
/// - **It empties itself every December.** Once the window's end falls in the
///   next calendar year its day number is *smaller* than the start's — 20
///   November 2026 gives `'324' … '001'` — so `BETWEEN` matches nothing. The
///   tile read a confident zero for the last 42 days of every year, which is
///   precisely when the winter arrivals it exists to announce are still due.
/// - **`'now'` is UTC.** Detections carry a local `Date`, so the window moved
///   off the data by the station's offset for part of every day.
/// - **Day-of-year is not comparable between years.** After February a leap
///   year runs one day ahead, so the window edges landed a day out whenever
///   exactly one of the two years was a leap year.
///
/// All three go away by not doing day-of-year arithmetic at all. The prior
/// year's arrival date is re-based onto this year *and* onto next year — the
/// same `year || substr(date, 5)` idiom the "earliest vs last year" query above
/// already uses — and both candidates are compared against the real window
/// bounds. The wrap is then just the second candidate matching.
///
/// The comparison is lexicographic over ISO dates rather than `julianday`,
/// which is what keeps a 29 February arrival from vanishing: re-basing it onto
/// a common year yields `2027-02-29`, a date that does not exist and that
/// `julianday` would turn into NULL, but that sorts exactly where a reader
/// expects it.
fn still_expected_count(conn: &rusqlite::Connection, today: &str) -> rusqlite::Result<i64> {
    let year = year_of(today);
    let prior = year - 1;
    let next = year + 1;
    conn.query_row(
        "WITH heard_this AS ( \
           SELECT DISTINCT Com_Name FROM detections_analytic WHERE Date LIKE ?1 \
         ), arrived_prior AS ( \
           SELECT Com_Name, MIN(Date) d \
           FROM detections_analytic WHERE Date LIKE ?2 GROUP BY Com_Name \
         ) \
         SELECT COUNT(*) FROM arrived_prior \
         WHERE ((?3 || substr(d, 5)) BETWEEN ?5 AND ?6 \
             OR (?4 || substr(d, 5)) BETWEEN ?5 AND ?6) \
           AND Com_Name NOT IN (SELECT Com_Name FROM heard_this)",
        rusqlite::params![
            format!("{year}-%"),
            format!("{prior}-%"),
            format!("{year}"),
            format!("{next}"),
            today,
            window_end(today, WINDOW_DAYS),
        ],
        |r| r.get(0),
    )
}

/// The date `days` after `today`, as `YYYY-MM-DD`.
fn window_end(today: &str, days: u64) -> String {
    let (y, m, d) =
        crate::routes::pages::days_to_date(crate::routes::pages::date_to_epoch_days(today) + days);
    format!("{y}-{m:02}-{d:02}")
}

/// The date `days` before `today`, as `YYYY-MM-DD`.
fn window_start(today: &str, days: u64) -> String {
    let (y, m, d) = crate::routes::pages::days_to_date(
        crate::routes::pages::date_to_epoch_days(today).saturating_sub(days),
    );
    format!("{y}-{m:02}-{d:02}")
}

/// Species whose arrival last year falls in the [`WINDOW_DAYS`] before `today` and that have
/// not been heard this year: `(stored name, the date it was first heard last year)`, the
/// earliest due first.
///
/// The window never reaches back before 1 January of this year: a species due in last
/// November has had the whole of that November to be heard in, and "heard this year" is a
/// statement about this calendar year. A species first heard in the station's first fortnight
/// of recording is not counted either: that date is where recording began, not where the bird
/// arrived, which is what [`last_year_coverage`] says of the tile beside it.
fn overdue_arrivals(
    conn: &rusqlite::Connection,
    today: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let year = year_of(today);
    let prior = year - 1;
    let jan1 = format!("{year}-01-01");
    let from = window_start(today, WINDOW_DAYS).max(jan1);
    let mut stmt = conn.prepare(
        "WITH heard_this AS ( \
           SELECT DISTINCT Com_Name FROM detections_analytic WHERE Date LIKE ?1 \
         ), arrived_prior AS ( \
           SELECT Com_Name, MIN(Date) d \
           FROM detections_analytic WHERE Date LIKE ?2 GROUP BY Com_Name \
         ), since AS ( \
           SELECT MIN(Date) d FROM detections WHERE date(Date) IS NOT NULL \
         ) \
         SELECT a.Com_Name, a.d \
         FROM arrived_prior a, since \
         WHERE (?3 || substr(a.d, 5)) >= ?4 AND (?3 || substr(a.d, 5)) < ?5 \
           AND since.d <= date(a.d, '-14 days') \
           AND a.Com_Name NOT IN (SELECT Com_Name FROM heard_this) \
         ORDER BY (?3 || substr(a.d, 5)), a.Com_Name",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![
            format!("{year}-%"),
            format!("{prior}-%"),
            format!("{year}"),
            from,
            today,
        ],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?;
    rows.collect()
}

/// The four-digit year of a `YYYY-MM-DD` string, or 1970 if there isn't one.
///
/// Degrades rather than panicking: this module aborts the process on a panic,
/// so a corrupt `Date` must not reach one.
fn year_of(date: &str) -> i32 {
    date.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(1970)
}

/// Chart column of a `YYYY-MM-DD` date, matching [`WEEK_COLUMN`].
///
/// This has to agree with [`WEEK_COLUMN`] because that is what every query on
/// this page buckets by, and the value positions the "today" marker against
/// those buckets. It did not: the marker was placed by `(unix_days % 365) / 7`, which
/// is not a week number in any calendar. It ignores leap days, so it had
/// drifted a fortnight by 2026, and it counts from 1 January 1970 rather than
/// from the current year, so on 31 December it returned week 1 and drew the
/// marker at the far left of a chart whose data ends at the far right.
///
#[allow(clippy::cast_possible_truncation)]
fn week_of_year(date: &str) -> u8 {
    let days = crate::routes::pages::date_to_epoch_days(date);
    let (y, _, _) = crate::routes::pages::days_to_date(days);
    let jan1 = crate::routes::pages::date_to_epoch_days(&format!("{y}-01-01"));
    let day_of_year_zero_based = days.saturating_sub(jan1);
    (day_of_year_zero_based / 7).min(51) as u8
}

/// The station's current **local** year.
///
/// Local, not UTC. Every query on this page selects rows with `Date LIKE
/// '{year}-%'` against a locally-dated column, and this is also the year
/// [`still_expected_count`] re-bases prior arrivals onto — a UTC year would put
/// the two out of step for the hours around New Year, which is exactly the
/// boundary the rest of this change is about.
fn current_year() -> i32 {
    year_of(&crate::routes::pages::today_date_string())
}

/// The station's current **local** chart column ([`WEEK_COLUMN`]).
fn current_week() -> u8 {
    week_of_year(&crate::routes::pages::today_date_string())
}

fn ok_html(body: String) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], body)
}

/// Pre-compute and cache the heavy phenology fragment (the ridgeline and the species-per-week
/// lane under it) so the first visit (and each background refresh) is instant. The stats tiles
/// cache per-request; the editorial cards are cheap single-row lookups.
pub fn prewarm(state: &AppState) {
    let cache = state.analytics_cache();
    if let Some(h) = compute_ridgeline(state) {
        cache.put("migration-ridgeline".to_string(), h);
    }
}

#[cfg(test)]
mod tests {
    // The module-level deny exists because a panic in a request handler aborts
    // the whole station. A test is not a request handler, and a failed
    // assertion is how a test reports.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    // -----------------------------------------------------------------------
    // Date handling across the year boundary
    // -----------------------------------------------------------------------

    /// A station holding `(common name, date)` first detections, behind the
    /// same `detections_analytic` view the page reads in production.
    fn station(rows: &[(&str, &str)]) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory SQLite");
        conn.execute_batch(
            "CREATE TABLE detections (Date TEXT, Com_Name TEXT, review_verdict TEXT);
             CREATE VIEW detections_analytic AS
                 SELECT * FROM detections WHERE review_verdict IS NOT 'rejected';",
        )
        .expect("schema");
        for (name, date) in rows {
            conn.execute(
                "INSERT INTO detections (Date, Com_Name, review_verdict) VALUES (?1, ?2, NULL)",
                rusqlite::params![date, name],
            )
            .expect("seed");
        }
        conn
    }

    /// `ANA14b`: the last days of December are a week on the chart like any
    /// other, named the way the chart names it.
    ///
    /// `%W` runs 0..=53 and every chart has 52 columns. The diversity strip
    /// dropped weeks 52 and 53 (from 28 December 2026, a Monday, on), the
    /// ridgeline folded them into column 51, and the "Peak diversity week"
    /// tile printed `w53` — a week the chart does not have.
    #[test]
    fn the_last_days_of_december_are_on_the_chart() {
        let conn = station(&[
            ("Bohemian Waxwing", "2026-12-29"),
            ("Common Redpoll", "2026-12-29"),
            ("Northern Shrike", "2026-12-30"),
        ]);
        let weekly = weekly_diversity(&conn, 2026).expect("diversity");
        assert_eq!(weekly[51], 3, "the last week was dropped: {weekly:?}");
        let html = stats_fragment(
            &conn,
            "2026-12-31",
            &birdnet_core::i18n::NameLocalizer::english(),
        )
        .expect("tiles");
        assert!(
            html.contains(">late Dec<") && html.contains(">week 52 · 3 species<"),
            "the peak week is off the chart: {html}"
        );
        assert_eq!(week_of_year("2026-12-31"), 51);

        // Counterparts: the first days of January are the first column, and a
        // mid-year date is where it always was.
        let conn = station(&[("Snow Bunting", "2026-01-01")]);
        assert_eq!(weekly_diversity(&conn, 2026).expect("diversity")[0], 1);
        assert_eq!(week_of_year("2026-01-01"), 0);
        assert_eq!(week_of_year("2026-07-02"), 26);
    }

    /// A first year that began mid-year is not a year of arrivals.
    ///
    /// A station installed on 2025-06-01 "first heard" its resident cardinal
    /// that day, and hearing it on 2026-01-01 read as an arrival 151 days
    /// early: the tile printed "−151 d". A species is compared only where the
    /// station was already recording a fortnight before last year's date.
    #[test]
    fn a_partial_first_year_is_not_an_early_arrival() {
        let conn = station(&[
            ("Northern Cardinal", "2025-06-01"),
            ("Northern Cardinal", "2026-01-01"),
        ]);
        let html = stats_fragment(
            &conn,
            "2026-03-01",
            &birdnet_core::i18n::NameLocalizer::english(),
        )
        .expect("tiles");
        assert!(!html.contains("-151 d"), "{html}");
        assert!(
            html.contains("last year was only partly recorded"),
            "{html}"
        );

        // Counterpart: a station that was running all of last year still
        // reports a genuinely early arrival.
        let conn = station(&[
            ("House Wren", "2024-12-01"),
            ("Barn Swallow", "2025-04-10"),
            ("Barn Swallow", "2026-04-01"),
        ]);
        let html = stats_fragment(
            &conn,
            "2026-04-20",
            &birdnet_core::i18n::NameLocalizer::english(),
        )
        .expect("tiles");
        assert!(html.contains("-9 d"), "{html}");
        assert!(html.contains("Barn Swallow"), "{html}");
    }

    fn english() -> birdnet_core::i18n::NameLocalizer {
        birdnet_core::i18n::NameLocalizer::english()
    }

    /// A station installed on 26 October compares nothing with "last year".
    ///
    /// The fixture's own Migration tab read "Earliest vs last year −356 d, White-breasted
    /// Nuthatch": the station began on 2025-10-26, the nuthatch was first heard on
    /// 2025-12-24 (two months after recording began, so the old guard "recording a fortnight
    /// before the species' first date" let it through) and again on 2026-01-02, and the tile
    /// subtracted one "arrival" from the other. A resident "arriving" 356 days early, because
    /// last year's calendar began in late October.
    ///
    /// Observed failing against that guard (`last_year_coverage` returning `Whole` for a
    /// station that began after 14 January of last year): the tile printed `-355 d`.
    #[test]
    fn a_year_that_began_in_october_is_not_a_year_of_arrivals() {
        let conn = station(&[
            ("Northern Cardinal", "2025-10-26"),
            ("White-breasted Nuthatch", "2025-12-24"),
            ("Northern Cardinal", "2026-01-01"),
            ("White-breasted Nuthatch", "2026-01-03"),
        ]);
        let html = stats_fragment(&conn, "2026-10-21", &english()).expect("tiles");
        assert!(
            !html.contains("-35"),
            "a resident is not 355 days early: {html}"
        );
        assert!(
            html.contains("last year was only partly recorded"),
            "{html}"
        );
        // Counterpart: the same two birds on a station that recorded from the first week of
        // last year have a first date that means the start of the calendar, which is still
        // not an arrival, so the tile says there is nothing to compare rather than "-1 d".
        let conn = station(&[
            ("Northern Cardinal", "2025-01-02"),
            ("White-breasted Nuthatch", "2025-01-03"),
            ("Northern Cardinal", "2026-01-01"),
            ("White-breasted Nuthatch", "2026-01-03"),
        ]);
        let html = stats_fragment(&conn, "2026-10-21", &english()).expect("tiles");
        assert!(!html.contains("-1 d") && !html.contains("-2 d"), "{html}");
        assert!(
            html.contains("no bird has arrived in both years yet"),
            "{html}"
        );
        // And a real arrival on that station is still reported.
        let conn = station(&[
            ("Northern Cardinal", "2025-01-02"),
            ("Barn Swallow", "2025-04-10"),
            ("Northern Cardinal", "2026-01-01"),
            ("Barn Swallow", "2026-04-01"),
        ]);
        let html = stats_fragment(&conn, "2026-10-21", &english()).expect("tiles");
        assert!(
            html.contains("-9 d") && html.contains("Barn Swallow"),
            "{html}"
        );
    }

    #[test]
    fn last_year_coverage_depends_on_the_first_recorded_day() {
        assert_eq!(last_year_coverage(None, 2026), LastYear::NotRecorded);
        assert_eq!(
            last_year_coverage(Some("2026-02-01"), 2026),
            LastYear::NotRecorded
        );
        assert_eq!(
            last_year_coverage(Some("2025-10-26"), 2026),
            LastYear::PartlyRecorded
        );
        assert_eq!(
            last_year_coverage(Some("2025-01-15"), 2026),
            LastYear::PartlyRecorded
        );
        assert_eq!(
            last_year_coverage(Some("2025-01-14"), 2026),
            LastYear::Whole
        );
        assert_eq!(
            last_year_coverage(Some("2024-06-01"), 2026),
            LastYear::Whole
        );
    }

    /// A zero is only printed where there was a last year to be expecting from.
    #[test]
    fn still_expected_says_so_when_there_is_no_last_year() {
        let conn = station(&[("Song Sparrow", "2026-03-01")]);
        let html = stats_fragment(&conn, "2026-05-01", &english()).expect("tiles");
        assert!(html.contains("needs a year of records"), "{html}");
        assert!(!html.contains("none outstanding"), "{html}");
        // Counterpart: with a last year, nothing outstanding is a statement.
        let conn = station(&[
            ("Song Sparrow", "2025-03-01"),
            ("Song Sparrow", "2026-03-01"),
        ]);
        let html = stats_fragment(&conn, "2026-05-01", &english()).expect("tiles");
        assert!(html.contains("none outstanding for 6 wk"), "{html}");
        assert!(!html.contains("needs a year of records"), "{html}");
    }

    /// "Missing" is the birds last year brought around now that this year has not.
    #[test]
    fn missing_lists_what_is_overdue_and_says_when_there_is_nothing() {
        let conn = station(&[
            // The station has been recording for years, so a first date means an arrival.
            ("Baseline Resident", "2024-01-02"),
            ("Overdue Warbler", "2025-04-20"),
            ("Overdue Vireo", "2025-04-25"),
            ("Heard Swallow", "2025-04-10"),
            ("Heard Swallow", "2026-04-15"),
            ("Too Early", "2025-03-01"),
            ("Not Due Yet", "2025-05-20"),
        ]);
        let over = overdue_arrivals(&conn, "2026-05-01").expect("query");
        assert_eq!(
            over,
            vec![
                ("Overdue Warbler".to_owned(), "2025-04-20".to_owned()),
                ("Overdue Vireo".to_owned(), "2025-04-25".to_owned()),
            ],
            "only what was due in the six weeks before today and has not been heard"
        );
        let card = card_content(&conn, "missing", "2026-05-01", &english()).expect("card");
        assert_eq!(card.0, "Missing");
        assert_eq!(card.1, "Overdue Warbler");
        assert!(
            card.2.contains("2025-04-20") && card.2.contains("+1 more"),
            "{card:?}"
        );

        // Nothing overdue: said as such.
        let (_, head, sub) = missing_card(&[], &english());
        assert_eq!(head, "Nothing overdue");
        assert!(sub.contains("has been heard"));
        // No last year: not "nothing overdue".
        let conn = station(&[("Song Sparrow", "2026-03-01")]);
        let card = card_content(&conn, "missing", "2026-05-01", &english()).expect("card");
        assert_eq!(card.1, "—");
        assert!(card.2.contains("needs a year of records"), "{card:?}");
        // The old card was a permanent "Forecast model pending".
        assert!(!card.2.contains("Forecast"));
    }

    /// The first fortnight of a station's recording is where it began, not where birds arrived.
    #[test]
    fn a_species_first_heard_when_recording_began_is_not_overdue() {
        let conn = station(&[
            ("Day One Bird", "2025-04-20"),
            ("Second Bird", "2025-06-01"),
        ]);
        // Recording began on the very day Day One Bird was first heard: not an arrival.
        let over = overdue_arrivals(&conn, "2026-05-01").expect("query");
        assert!(over.is_empty(), "{over:?}");
    }

    /// The ridgeline keeps the species that are most sharply seasonal, not the ones that peak
    /// earliest in the year.
    ///
    /// Observed failing against the old cut (sort by peak week, then truncate): with fourteen
    /// qualifying species the two latest (the fall birds) were the ones dropped.
    #[test]
    fn the_cut_keeps_the_most_seasonal_not_the_earliest() {
        let bird = |name: &str, peak_week: usize, height: f64| {
            // A spike over a thin tail of ordinary weeks: the shape of a bird that passes through.
            let mut c = [0.0; 52];
            for d in 1..=2 {
                c[peak_week - d] = 2.0;
                c[peak_week + d] = 2.0;
            }
            c[peak_week] = height;
            (name.to_owned(), c)
        };
        let mut all: Vec<(String, [f64; 52])> = (0..12)
            .map(|i| bird(&format!("Spring {i:02}"), 8 + i, 20.0 + i as f64))
            .collect();
        // Two fall birds with a much sharper peak (a higher peak over a lower usual week).
        all.push(bird("Fall Warbler", 40, 90.0));
        all.push(bird("Fall Thrush", 42, 80.0));
        let picked = pick_ridges(all, 12);
        assert_eq!(picked.len(), 12);
        let names: Vec<&str> = picked.iter().map(|p| p.name.as_str()).collect();
        assert!(
            names.contains(&"Fall Warbler") && names.contains(&"Fall Thrush"),
            "{names:?}"
        );
        // And they read left to right: the fall birds come last.
        assert_eq!(names[names.len() - 1], "Fall Thrush");

        // The thresholds: a steady bird, a bird heard in three weeks and a quiet one are out.
        let steady = [20.0; 52];
        assert!(seasonal_ratio(&steady).is_none());
        let mut three = [0.0; 52];
        three[10] = 30.0;
        three[11] = 9.0;
        three[12] = 4.0;
        assert!(
            seasonal_ratio(&three).is_none(),
            "heard in fewer than four weeks"
        );
        let mut quiet = [0.0; 52];
        quiet[5..10].fill(1.0);
        quiet[7] = 4.0;
        assert!(
            seasonal_ratio(&quiet).is_none(),
            "its busiest week is under five detections"
        );
    }

    /// The six-week look-ahead must not empty itself in December.
    ///
    /// The window was expressed as a day-of-year `BETWEEN` against
    /// `strftime('%j','now')` and `strftime('%j','now','+42 days')`. From 20
    /// November the end of the window falls in the next calendar year, so its
    /// day number is *smaller* than the start's — 20 November 2026 gives
    /// `'324' … '001'` — and the range matches nothing. "Still expected" was
    /// therefore a structural zero for the last 42 days of every year, which is
    /// exactly the stretch when the winter arrivals it exists to announce are
    /// still to come.
    #[test]
    fn still_expected_survives_the_year_boundary() {
        let conn = station(&[
            // Prior year (2025) first arrivals.
            ("Late Arriver", "2025-12-20"),
            ("New Year Arriver", "2025-01-05"),
            ("Spring Arriver", "2025-05-01"),
            ("Already Heard", "2025-12-22"),
            // Heard again this year, so no longer expected.
            ("Already Heard", "2026-12-06"),
        ]);

        let n = still_expected_count(&conn, "2026-12-05").expect("query runs");
        assert_eq!(
            n, 2,
            "expected Late Arriver (20 Dec, this year) and New Year Arriver \
             (5 Jan, next year); Spring Arriver is out of window and Already \
             Heard has been heard"
        );
    }

    /// The counterpart: the window the old query *did* answer still answers.
    ///
    /// A fix that widened the window until December worked could satisfy the
    /// gate above while counting species that are months away. This case never
    /// crosses a year boundary and its answer must not move.
    #[test]
    fn still_expected_mid_year_window_is_unchanged() {
        let conn = station(&[
            ("In Window", "2025-05-20"),
            ("Too Early", "2025-04-01"),
            ("Too Late", "2025-07-01"),
            ("Already Heard", "2025-05-21"),
            ("Already Heard", "2026-05-02"),
        ]);

        let n = still_expected_count(&conn, "2026-05-01").expect("query runs");
        assert_eq!(n, 1, "only In Window falls in 1 May - 12 June");
    }

    /// 29 February in the prior year has no counterpart in a common year.
    ///
    /// Re-basing it produces `2027-02-29`, a date that does not exist. The
    /// comparison is lexicographic over ISO strings rather than `julianday`,
    /// which places the impossible date exactly where the reader expects it
    /// — after the 28th, before 1 March — instead of turning it into a NULL
    /// that drops the species out of the count.
    #[test]
    fn a_leap_day_arrival_still_counts_in_a_common_year() {
        let conn = station(&[("Leap Arriver", "2024-02-29")]);
        let n = still_expected_count(&conn, "2025-02-20").expect("query runs");
        assert_eq!(n, 1, "29 Feb 2024 falls inside 20 Feb - 3 Apr 2025");
    }

    /// `week_of_year` must agree with the [`WEEK_COLUMN`] the queries bucket by.
    ///
    /// The chart's "today" marker was placed by `(unix_days % 365) / 7`, which
    /// is not a week number: it ignores leap days, so it had drifted a fortnight
    /// by 2026, and it is anchored to 1 January 1970 rather than to the current
    /// year, so on 31 December it read week 1 and drew the marker at the far
    /// left of a chart whose data ends at the far right.
    #[test]
    fn week_of_year_agrees_with_sqlite() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory SQLite");
        for date in [
            "2026-01-01",
            "2026-01-04",
            "2026-01-05",
            "2026-08-18",
            "2026-12-31",
            "2024-02-29",
            "2024-12-31",
            "2021-01-03",
        ] {
            let sqlite: i64 = conn
                .query_row(
                    &format!("SELECT {} FROM (SELECT ?1 AS Date)", super::WEEK_COLUMN),
                    [date],
                    |r| r.get(0),
                )
                .expect("strftime");
            assert_eq!(i64::from(week_of_year(date)), sqlite, "week of {date}");
        }
    }

    /// The look-ahead end date is 42 days on, across a year boundary and a
    /// leap day alike.
    #[test]
    fn window_end_crosses_years_and_leap_days() {
        assert_eq!(window_end("2026-12-05", 42), "2027-01-16");
        assert_eq!(window_end("2026-05-01", 42), "2026-06-12");
        // 2024 is a leap year: 42 days from 20 February includes 29 February.
        assert_eq!(window_end("2024-02-20", 42), "2024-04-02");
        assert_eq!(window_end("2025-02-20", 42), "2025-04-03");
    }

    #[test]
    fn year_of_reads_the_leading_four_digits() {
        assert_eq!(year_of("2026-12-05"), 2026);
        // Degrades rather than panicking: this module aborts the process on a
        // panic, so a corrupt date must not reach one.
        assert_eq!(year_of("oops"), 1970);
    }

    /// UIR-36. What the page covers is a sentence, not three pills styled like the range
    /// buttons a reader will try to press; and the caption names the seasons in words, not by
    /// a colour ("amber in the fall" over an orange-red ridge).
    #[test]
    fn the_scope_is_a_caption_and_the_seasons_are_words() {
        let html = content();
        assert!(!html.contains("bnb-pill"), "{html}");
        assert!(!html.contains("pill-row"), "{html}");
        let year = current_year();
        assert!(
            html.contains(&format!(
                "Calendar year {year}, seasonal birds only, compared with last year."
            )),
            "{html}"
        );
        // Whole words: "compared" holds "red".
        let words: Vec<String> = html
            .split(|c: char| !c.is_ascii_alphabetic())
            .map(str::to_ascii_lowercase)
            .collect();
        for colour in ["green", "amber", "orange", "red"] {
            assert!(
                !words.iter().any(|w| w == colour),
                "a season named by {colour}: {html}"
            );
        }
        assert!(
            html.contains("where the peak stands says which, the spring one or the fall one"),
            "{html}"
        );
    }

    /// UIR-23. The peak diversity week is a time of year a reader says, with the chart's
    /// week number beside it. A column is named by the day in its middle (the fourth of its
    /// seven, or the middle of the last column's eight or nine): days 1–10 of a month are
    /// "early", 11–20 "mid", 21 on "late".
    ///
    /// Worked by hand: 2026-09-27 is day 270, column (270 − 1) / 7 = 38, days 267–273, middle
    /// 270 = 27 Sep: "late Sep (week 39)". 2026-01-01 is column 0, middle day 4 = 4 Jan:
    /// "early Jan (week 1)". 2024-12-31 is day 366 of a leap year, column 51 (never a 53rd),
    /// days 358–366, middle 362 = 27 Dec: "late Dec (week 52)". 2024-02-29 is day 60, column
    /// 8, days 57–63, middle 60: in a leap year 29 Feb, "late Feb (week 9)"; the same column
    /// of 2026 has middle 60 = 1 Mar, "early Mar (week 9)".
    #[test]
    fn the_peak_week_is_a_date_a_reader_says() {
        for (date, when, week) in [
            ("2026-09-27", "late Sep", 39),
            ("2026-01-01", "early Jan", 1),
            ("2024-12-31", "late Dec", 52),
            ("2024-02-29", "late Feb", 9),
            ("2026-03-01", "early Mar", 9),
            ("2026-05-14", "mid May", 20),
        ] {
            let conn = station(&[("Song Sparrow", date)]);
            let html = stats_fragment(&conn, date, &english()).expect("tiles");
            assert!(
                html.contains(&format!(
                    r#"<span class="value mig-peak-val">{when}</span><span class="bnb-meta mono">week {week} · 1 species</span>"#
                )),
                "{date}: {html}"
            );
            // The old label, `w39`, is gone.
            assert!(!html.contains(&format!(">w{week}<")), "{date}: {html}");
        }
    }

    // -----------------------------------------------------------------------
    // N24-08: the weekly index is per listened time
    // -----------------------------------------------------------------------

    /// Every date of 2025 with its day of the year (1-based).
    fn days_of_2025() -> Vec<(String, u32)> {
        let lens = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut out = Vec::new();
        let mut doy = 0;
        for (m, len) in lens.iter().enumerate() {
            for d in 1..=*len {
                doy += 1;
                out.push((format!("2025-{:02}-{d:02}", m + 1), doy));
            }
        }
        out
    }

    /// A station with the real schema: `seconds(doy)` of listening on each date of 2025
    /// (`None`: no ledger row) and `heard(name, doy)` detections of each species.
    fn ledger_station(
        seconds: impl Fn(u32) -> Option<f64>,
        heard: &[(&str, &dyn Fn(u32) -> u32)],
    ) -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = birdnet_db::sqlite::open_or_create(&dir.path().join("birds.db")).expect("db");
        for (date, doy) in days_of_2025() {
            if let Some(s) = seconds(doy) {
                conn.execute(
                    "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, 'local', ?2)",
                    rusqlite::params![date, s],
                )
                .expect("effort");
            }
            for (name, n) in heard {
                for i in 0..n(doy) {
                    conn.execute(
                        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                         VALUES (?1, ?2, 'Sci ' || ?3, ?3, 0.9, ?3 || ?1 || ?4 || '.wav')",
                        rusqlite::params![date, format!("{:02}:00:00", i % 24), name, i],
                    )
                    .expect("detection");
                }
            }
        }
        (dir, conn)
    }

    /// Week column (0-based) of a day of the year, as [`WEEK_COLUMN`] has it.
    fn col(doy: u32) -> u32 {
        ((doy - 1) / 7).min(51)
    }

    /// A station that listened six hours a day (21 600 s) all of 2025 but for six weeks of
    /// all-day listening in spring (columns 14–19, 9 Apr – 20 May) and six weeks switched off
    /// in summer (columns 30–35, 30 Jul – 9 Sep, no ledger row).
    fn spring_all_day(doy: u32) -> Option<f64> {
        match col(doy) {
            14..=19 => Some(86_400.0),
            30..=35 => None,
            _ => Some(21_600.0),
        }
    }

    /// A Carolina Wren is resident: one detection an hour listened, 6 a day on a six-hour
    /// day and 24 on an all-day one, none while the station was off.
    fn wren(doy: u32) -> u32 {
        match spring_all_day(doy) {
            Some(s) => (s / 3_600.0) as u32,
            None => 0,
        }
    }

    /// A Swainson's Thrush passes through in the fall: one a day in columns 38, 39, 41 and
    /// 42 and ten a day in column 40.
    fn thrush(doy: u32) -> u32 {
        match col(doy) {
            40 => 10,
            38 | 39 | 41 | 42 => 1,
            _ => 0,
        }
    }

    /// N24-08. On raw weekly counts the wren read 42 a week on six-hour weeks and 168 on the
    /// all-day ones: a busiest week four times its usual one, a spring migrant. Per listened
    /// time it is one detection an hour throughout.
    ///
    /// Worked by hand: the typical day is the median of 323 dates with audio, 281 of them
    /// 21 600 s, so 21 600; every date with a row is a listened day (the all-day ones are four
    /// typical days each). A six-hour week is 7 typical days and 42 detections, 42 per seven
    /// typical days; an all-day week is 28 typical days and 168 detections, 168 / 28 × 7 = 42;
    /// column 51 (24–31 Dec) is 8 typical days and 48 detections, 42. The index is 42 in every
    /// week with listening, so the wren's busiest week is its usual week and it has no ridge.
    /// The thrush (counterpart) is 7, 7, 70, 7, 7 per seven typical days in columns 38–42:
    /// its busiest week ten times its usual week, so it keeps its ridge.
    #[test]
    fn a_resident_is_not_a_migrant_because_the_station_listened_longer() {
        let (_dir, conn) = ledger_station(
            spring_all_day,
            &[("Carolina Wren", &wren), ("Swainson's Thrush", &thrush)],
        );
        let picked = collect_ridges(&conn, 2025, "2026-01-01", MAX_RIDGES).expect("ridges");
        let names: Vec<&str> = picked.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["Swainson's Thrush"],
            "a resident was drawn as a migrant"
        );
    }

    /// A week the station was off is unknown, not a week the wren went quiet: the six weeks
    /// of the outage (columns 30–35) have no index, and every other week's is 42. A week
    /// with fewer than three days to count is unknown too, and today is not counted: with
    /// today Monday 13 Oct 2025 (day 286, column 40, days 281–287) column 40 has 281–285,
    /// five days, and is known; with today 16 Oct (day 289) column 41 (days 288–294) has
    /// only the 15th, and is not.
    #[test]
    fn a_week_nobody_listened_to_has_no_index() {
        let (_dir, conn) = ledger_station(spring_all_day, &[("Carolina Wren", &wren)]);
        let all = weekly(&conn, 2025, "2026-01-01").expect("weeks");
        let w = &all[0];
        for (c, v) in w.index.iter().enumerate() {
            if (30..=35).contains(&c) {
                assert_eq!(*v, None, "column {c}");
            } else {
                let v = v.unwrap_or_else(|| panic!("column {c} has no index"));
                assert!((v - 42.0).abs() < 1e-9, "column {c}: {v}");
            }
        }
        // Today's week: 13 Oct 2025 is day 286; 281–285 are counted, today is not.
        let all = weekly(&conn, 2025, "2025-10-13").expect("weeks");
        assert!(all[0].index[40].is_some_and(|v| (v - 42.0).abs() < 1e-9));
        assert_eq!(all[0].index[41], None, "a week that has not begun");
        let all = weekly(&conn, 2025, "2025-10-16").expect("weeks");
        assert_eq!(all[0].index[41], None, "one day is too few to say");
    }

    /// Counterpart: on a station that listened all day, every day, a real migrant keeps its
    /// ridge and a resident heard at a constant rate has none.
    #[test]
    fn with_full_listening_a_migrant_is_still_a_migrant() {
        let (_dir, conn) = ledger_station(
            |_| Some(86_400.0),
            &[("Carolina Wren", &|_| 24), ("Swainson's Thrush", &thrush)],
        );
        let picked = collect_ridges(&conn, 2025, "2026-01-01", MAX_RIDGES).expect("ridges");
        let names: Vec<&str> = picked.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Swainson's Thrush"]);
    }

    // -----------------------------------------------------------------------
    // N05-02: "Earliest vs last year" is the arrival of the bulk, not the first bird
    // -----------------------------------------------------------------------

    /// `name` on every date from `from` to `to` (inclusive, one year), one row a day.
    fn daily(name: &'static str, from: &str, to: &str) -> Vec<(&'static str, String)> {
        let (a, b) = (
            crate::routes::pages::date_to_epoch_days(from),
            crate::routes::pages::date_to_epoch_days(to),
        );
        (a..=b)
            .map(|e| {
                let (y, m, d) = crate::routes::pages::days_to_date(e);
                (name, format!("{y}-{m:02}-{d:02}"))
            })
            .collect()
    }

    fn station_of(rows: &[(&'static str, String)]) -> rusqlite::Connection {
        let refs: Vec<(&str, &str)> = rows.iter().map(|(n, d)| (*n, d.as_str())).collect();
        station(&refs)
    }

    /// A station recording from 1 Jan 2025 (a resident Cardinal every day to today, 31 Aug
    /// 2026), and a Barn Swallow heard every day from 1 May to 28 Aug in both years: 120
    /// detection days each year.
    fn swallow_years(extra_2026: &[(&'static str, String)]) -> rusqlite::Connection {
        let mut rows = daily("Northern Cardinal", "2025-01-01", "2026-08-31");
        rows.extend(daily("Barn Swallow", "2025-05-01", "2025-08-28"));
        rows.extend(daily("Barn Swallow", "2026-05-01", "2026-08-28"));
        rows.extend_from_slice(extra_2026);
        station_of(&rows)
    }

    /// N05-02. One stray detection of the swallow on 10 Jan 2026 (a false positive) does not
    /// move the tile.
    ///
    /// The onset is the phenology module's (`phenology/timing.rs`): the 5th percentile of the
    /// year's distinct detection days, `quantile_disc`, the `ceil(0.05 n)`-th day. Worked by
    /// hand, last year to 31 Aug: 120 days, ceil(6) = 6th, 6 May. This year: 121 days (the
    /// stray and the 120), ceil(6.05) = 7th: 10 Jan, then 1–6 May, so 6 May. 0 d: on time.
    /// The Cardinal's onset is in January in both years, so it is no arrival.
    ///
    /// On `MIN(Date)` the swallow's first date this year was 10 January, before the end of
    /// January, so it was not an arrival at all and the tile lost it.
    #[test]
    fn one_stray_january_detection_does_not_move_the_arrival() {
        let conn = swallow_years(&[("Barn Swallow", "2026-01-10".to_owned())]);
        let html = stats_fragment(&conn, "2026-08-31", &english()).expect("tiles");
        assert!(
            html.contains(
                r#"<span class="value">on time</span><span class="bnb-meta">Barn Swallow</span>"#
            ),
            "{html}"
        );
    }

    /// Counterpart: a real early arrival of several days moves it. The swallow is heard every
    /// day from 25 Apr 2026: 126 days, ceil(6.3) = 7th, 1 May, against last year's 6 May:
    /// 5 days early.
    #[test]
    fn a_real_early_arrival_still_moves_it() {
        let conn = swallow_years(&daily("Barn Swallow", "2026-04-25", "2026-04-30"));
        let html = stats_fragment(&conn, "2026-08-31", &english()).expect("tiles");
        assert!(
            html.contains(
                r#"<span class="value">-5 d</span><span class="bnb-meta">Barn Swallow</span>"#
            ),
            "{html}"
        );
    }
}
