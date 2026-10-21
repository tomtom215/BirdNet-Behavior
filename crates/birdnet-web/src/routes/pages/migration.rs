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
    PAGE_HTML
        .replace("{{year}}", &year.to_string())
        .replace("{{skel_migration_stats}}", &super::skeletons::stat_row(4))
        .replace("{{skel_ridgeline}}", super::skeletons::ridgeline())
        .replace(
            "{{help_link}}",
            &super::help::help_link(super::help::Topic::Phenology),
        )
}

// ---------------------------------------------------------------------------
// Data shape
// ---------------------------------------------------------------------------

/// How many ridges the chart draws: past this the names stand on one another.
const MAX_RIDGES: usize = 12;

/// A species that earned a ridge: its stored name, its detections in each week column and how
/// sharply seasonal it is.
struct Seasonal {
    name: String,
    counts: [f64; 52],
    /// Busiest week over the median week it was heard in.
    seasonality: f64,
}

/// How sharply seasonal a species' year is, or `None` if it has not earned a ridge.
///
/// A species earns one when it was heard in at least four different weeks, its busiest week
/// held at least five detections, and that week was at least three times the usual week it was
/// heard in (the median of its non-empty weeks). That is a cheap proxy for "migratory": a
/// resident holds a steady ribbon and is left out, a bird that passes through spikes. It is a
/// proxy and the chart says "per-species weekly index", not "migrants"; refine it with
/// `birdnet_behavioral::ResidencyType::Migrant` when that is wanted.
fn seasonal_ratio(counts: &[f64; 52]) -> Option<f64> {
    let peak = counts.iter().copied().fold(0.0_f64, f64::max);
    let mut heard: Vec<f64> = counts.iter().copied().filter(|v| *v > 0.0).collect();
    if heard.len() < 4 || peak < 5.0 {
        return None;
    }
    heard.sort_by(f64::total_cmp);
    let median = heard[heard.len() / 2].max(1.0);
    let ratio = peak / median;
    (ratio >= 3.0).then_some(ratio)
}

/// The species to draw, from every species' weeks: those that earned a ridge, the
/// `max_species` most sharply seasonal of them, in the order of their busiest week so ridges
/// read left to right.
///
/// The cut is by seasonality and not by the order of the year. It used to sort by peak week
/// and then truncate, which kept the twelve that peak *earliest*: with more than twelve
/// qualifying species the fall migrants were dropped without a word.
fn pick_ridges(all: Vec<(String, [f64; 52])>, max_species: usize) -> Vec<Seasonal> {
    let mut earned: Vec<Seasonal> = all
        .into_iter()
        .filter_map(|(name, counts)| {
            seasonal_ratio(&counts).map(|seasonality| Seasonal {
                name,
                counts,
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

fn collect_ridges(
    conn: &rusqlite::Connection,
    year: i32,
    max_species: usize,
) -> rusqlite::Result<Vec<Seasonal>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT Com_Name, {WEEK_COLUMN} AS wk, COUNT(*) AS n \
         FROM detections_analytic \
         WHERE Date LIKE ?1 \
         GROUP BY Com_Name, wk"
    ))?;
    let prefix = format!("{year}-%");
    let rows = stmt.query_map([&prefix], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;

    use std::collections::HashMap;
    let mut by_species: HashMap<String, [f64; 52]> = HashMap::new();
    for row in rows {
        let (name, wk, n) = row?;
        let w = wk.clamp(0, 51) as usize; // WEEK_COLUMN is already 0..=51
        by_species.entry(name).or_insert([0.0; 52])[w] += n as f64;
    }
    Ok(pick_ridges(by_species.into_iter().collect(), max_species))
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
            let ridges = collect_ridges(conn, year, MAX_RIDGES)?;
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
    // An arrival is a first date from the end of January on. A bird already singing in the
    // first four weeks of a year is a resident or a winter visitor, and its "first date" is
    // the start of the calendar, not a journey: comparing two of those printed "−1 d" for a
    // cardinal. And the prior year must have been recorded from its start (see
    // [`last_year_coverage`]).
    let earliest: Option<(String, i64)> = if coverage == LastYear::Whole {
        conn.query_row(
            "WITH first_this AS ( \
               SELECT Com_Name, MIN(Date) d \
               FROM detections_analytic WHERE Date LIKE ?1 GROUP BY Com_Name \
             ), first_prior AS ( \
               SELECT Com_Name, MIN(Date) d \
               FROM detections_analytic WHERE Date LIKE ?2 GROUP BY Com_Name \
             ) \
             SELECT t.Com_Name, \
                    CAST(julianday(t.d) - julianday(?3 || substr(p.d, 5)) AS INTEGER) AS delta \
             FROM first_this t JOIN first_prior p USING (Com_Name) \
             WHERE p.d >= ?4 AND t.d >= ?5 \
             ORDER BY delta ASC LIMIT 1",
            [
                format!("{year}-%"),
                format!("{prior}-%"),
                format!("{year}"),
                format!("{prior}-01-29"),
                format!("{year}-01-29"),
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

    // The column is 0-based; the ridgeline on this page prints its peak as
    // `w{pw + 1}`, and the tile printed the raw number, so one week had two
    // names on one page.
    let peak_html = peak.map_or_else(
        || r#"<span class="value mig-peak-val">—</span><span class="bnb-meta mono">no detections yet</span>"#.to_string(),
        |(wk, n)| {
            format!(
                r#"<span class="value mig-peak-val">w{w}</span><span class="bnb-meta mono">{n} species</span>"#,
                w = wk + 1
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
            html.contains(">w52<"),
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
        assert!(html.contains("last year was only partly recorded"), "{html}");

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
        assert!(!html.contains("-35"), "a resident is not 355 days early: {html}");
        assert!(html.contains("last year was only partly recorded"), "{html}");
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
        assert!(html.contains("no bird has arrived in both years yet"), "{html}");
        // And a real arrival on that station is still reported.
        let conn = station(&[
            ("Northern Cardinal", "2025-01-02"),
            ("Barn Swallow", "2025-04-10"),
            ("Northern Cardinal", "2026-01-01"),
            ("Barn Swallow", "2026-04-01"),
        ]);
        let html = stats_fragment(&conn, "2026-10-21", &english()).expect("tiles");
        assert!(html.contains("-9 d") && html.contains("Barn Swallow"), "{html}");
    }

    #[test]
    fn last_year_coverage_depends_on_the_first_recorded_day() {
        assert_eq!(last_year_coverage(None, 2026), LastYear::NotRecorded);
        assert_eq!(last_year_coverage(Some("2026-02-01"), 2026), LastYear::NotRecorded);
        assert_eq!(last_year_coverage(Some("2025-10-26"), 2026), LastYear::PartlyRecorded);
        assert_eq!(last_year_coverage(Some("2025-01-15"), 2026), LastYear::PartlyRecorded);
        assert_eq!(last_year_coverage(Some("2025-01-14"), 2026), LastYear::Whole);
        assert_eq!(last_year_coverage(Some("2024-06-01"), 2026), LastYear::Whole);
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
        assert!(card.2.contains("2025-04-20") && card.2.contains("+1 more"), "{card:?}");

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
            let mut c = [0.0; 52];
            for (w, v) in c.iter_mut().enumerate() {
                let d = (w as f64 - peak_week as f64).abs();
                *v = (height * (-(d / 2.0).powi(2)).exp()).round();
            }
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
        assert!(names.contains(&"Fall Warbler") && names.contains(&"Fall Thrush"), "{names:?}");
        // And they read left to right: the fall birds come last.
        assert_eq!(names[names.len() - 1], "Fall Thrush");

        // The thresholds: a steady bird, a bird heard in three weeks and a quiet one are out.
        let steady = [20.0; 52];
        assert!(seasonal_ratio(&steady).is_none());
        let mut three = [0.0; 52];
        three[10] = 30.0;
        three[11] = 9.0;
        three[12] = 4.0;
        assert!(seasonal_ratio(&three).is_none(), "heard in fewer than four weeks");
        let mut quiet = [0.0; 52];
        for w in 5..10 {
            quiet[w] = 1.0;
        }
        quiet[7] = 4.0;
        assert!(seasonal_ratio(&quiet).is_none(), "its busiest week is under five detections");
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
}
