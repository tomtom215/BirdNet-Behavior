//! The soundscape terrain: the station's last 30, 90 or 365 days as a landscape.
//!
//! Hour of day runs one way, the day the other, and the height and colour of the ground at
//! each hour of each day are how much was heard then. It is the Patterns "When active" tab's
//! lead card; `static/terrain.js` draws it, in WebGL when the reader asks for the 3D view and
//! as a flat grid otherwise (and always when there is no WebGL), and `static/vendor/three/` is
//! the one library it loads, only then.
//!
//! This module is the server's half: the counts, which the page asks for as JSON, a table of the
//! same numbers, and the card they live in. The terrain's shape is a pure function of the
//! counts (`build`), which is what the tests hold.
//!
//! | Path | Purpose |
//! |------|---------|
//! | (embedded)                     | Patterns home, "When active" tab (lead card) |
//! | `GET /pages/terrain-data`      | the counts as JSON (`days` 30, 90 or 365)     |
//! | `GET /pages/terrain-table`     | the same numbers as a table (HTMX partial)    |

use std::fmt::Write as _;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::routing::get;
use serde::Deserialize;

use super::escape_html;
use crate::analytics_cache::cached_fragment;
use crate::state::AppState;

/// The ranges the card offers. Anything else asked for is the nearest smaller one, so a request
/// cannot ask the station for ten years of hours.
pub(super) const RANGES: [u32; 3] = [30, 90, 365];

/// The default range.
const DEFAULT_DAYS: u32 = 90;

/// Species named in the readout, most heard first. A cell whose top species is further down the
/// list than this reads "another species": the counts are all there, only the name is not.
const NAMED_SPECIES: usize = 48;

/// What the terrain shows when the query failed: the page's script reads this as "unavailable".
const TERRAIN_ERR: &str = r#"{"error":"unavailable"}"#;

/// What the table fragment shows when the query failed (not cached, so a transient failure does
/// not stay on the page for the cache's life). Named so the error-wording gate can read it.
const FRAGMENT_ERR: &str = r#"<p class="bnb-meta">Analytics temporarily unavailable.</p>"#;

/// Mount the terrain's data routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/terrain-data", get(terrain_data))
        .route("/pages/terrain-table", get(terrain_table))
}

#[derive(Deserialize)]
struct TerrainQuery {
    days: Option<u32>,
}

/// The range a request asks for, clamped to one the card offers.
fn range_of(days: Option<u32>) -> u32 {
    let want = days.unwrap_or(DEFAULT_DAYS);
    RANGES
        .iter()
        .rev()
        .copied()
        .find(|&r| r <= want)
        .unwrap_or(RANGES[0])
}

// ---------------------------------------------------------------------------
// The shape
// ---------------------------------------------------------------------------

/// One day of the terrain.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DayCells {
    /// The local date, `YYYY-MM-DD`.
    pub(crate) date: String,
    /// Detections in each hour of the day.
    pub(crate) n: [u32; 24],
    /// The species most heard in each hour (an index into [`Terrain::species`]), `None` for a
    /// silent hour.
    pub(crate) top: [Option<u16>; 24],
    /// How many detections that species had in the hour.
    pub(crate) top_n: [u32; 24],
    /// The station was not listening this day (the recording ledger says so): its zeros are not
    /// quiet, and the terrain does not draw them as quiet.
    pub(crate) off: bool,
}

/// The terrain: a run of days, oldest first, ending on `end`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Terrain {
    /// The last day, local date.
    pub(crate) end: String,
    /// The local hour now: the hours of the last day after it have not happened yet.
    pub(crate) hour_now: u8,
    /// The species named, most heard first, as the station displays them.
    pub(crate) species: Vec<String>,
    /// The days.
    pub(crate) days: Vec<DayCells>,
    /// Sunrise and sunset on each day, as fractional hours on the station's local clock, oldest
    /// day first; `None` for a day the sun does not rise or set. Empty when the station has no
    /// location, in which case nothing is drawn or said about the sun.
    pub(crate) sun: Vec<Option<(f32, f32)>>,
}

impl Terrain {
    /// This terrain with the sun's times on each of its days.
    pub(crate) fn with_sun(mut self, sun: Vec<Option<(f32, f32)>>) -> Self {
        self.sun = if sun.len() == self.days.len() {
            sun
        } else {
            Vec::new()
        };
        self
    }
}

/// How much of what was heard fell close to sunrise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SunShare {
    /// The share of detections inside the window, 0 to 1.
    pub(crate) share: f64,
    /// The share an even spread over the day would put there: the window's length over 24.
    pub(crate) chance: f64,
    /// Detections counted.
    pub(crate) total: u64,
}

/// Half the window around sunrise the takeaway measures: two hours either side.
pub(crate) const SUNRISE_HALF_WINDOW: f32 = 2.0;

/// The share of what was heard that fell within `half_window` hours either side of sunrise.
///
/// Each hour's cell covers `[h, h + 1)` and counts for the part of it inside the window, so a
/// window that starts at 04:40 takes two-thirds of the 04:00 cell and not all of it. Only days
/// with a sunrise and a station that was listening count, and on the last day only the hours
/// that have happened: a quiet stretch the station was not there for is not evidence that
/// nothing sang. `None` when there is no sun to measure from, fewer than a week of days to
/// measure, or too little heard to say anything.
pub(crate) fn sunrise_share(t: &Terrain, half_window: f32) -> Option<SunShare> {
    if t.sun.is_empty() {
        return None;
    }
    let (mut inside, mut total, mut days) = (0.0_f64, 0_u64, 0_usize);
    for (i, day) in t.days.iter().enumerate() {
        let Some(Some((rise, _))) = t.sun.get(i) else {
            continue;
        };
        if day.off {
            continue;
        }
        let last = i + 1 == t.days.len();
        let (lo, hi) = (f64::from(rise - half_window), f64::from(rise + half_window));
        days += 1;
        for (h, &n) in day.n.iter().enumerate() {
            if last && h > usize::from(t.hour_now) {
                continue;
            }
            let start = f64::from(u32::try_from(h).unwrap_or(0));
            let overlap = (hi.min(start + 1.0) - lo.max(start)).max(0.0);
            inside += f64::from(n) * overlap;
            total += u64::from(n);
        }
    }
    if days < 7 || i64::try_from(total).unwrap_or(i64::MAX) < super::insight::MIN_DETECTIONS {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let share = inside / total as f64;
    Some(SunShare {
        share,
        chance: f64::from(2.0 * half_window) / 24.0,
        total,
    })
}

/// One row of the query: `(date, hour, species, detections)`.
pub(crate) type Row = (String, u32, String, u32);

/// The terrain from query rows.
///
/// * Every one of the `days` days ending on `end` is there, oldest first, whether or not it had a
///   detection: a quiet day is a flat day, not a missing one.
/// * `listened` is the recording ledger's answer, per day oldest first (`None`: the ledger has
///   nothing, so nothing is claimed). A day it says was not listened to is `off`.
/// * Species are ranked by how much each was heard over the whole range, and a cell's top
///   species is the one with the most detections in it (the alphabetically first on a tie, so
///   the page and its cache agree).
pub(crate) fn build(
    rows: &[Row],
    end: &str,
    days: usize,
    hour_now: u8,
    listened: Option<&[bool]>,
    display: &dyn Fn(&str) -> String,
) -> Terrain {
    use std::collections::BTreeMap;
    let end_day = super::date_to_epoch_days(end);
    let first_day = end_day.saturating_sub(days as u64 - 1);

    // Which species were heard how much overall.
    let mut totals: BTreeMap<&str, u64> = BTreeMap::new();
    for (_, _, sp, n) in rows {
        *totals.entry(sp.as_str()).or_insert(0) += u64::from(*n);
    }
    let mut ranked: Vec<(&str, u64)> = totals.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    ranked.truncate(NAMED_SPECIES);
    let index: BTreeMap<&str, u16> = ranked
        .iter()
        .enumerate()
        .map(|(i, (n, _))| (*n, u16::try_from(i).unwrap_or(u16::MAX)))
        .collect();

    // Per day, per hour, per species.
    let mut cells: BTreeMap<(u64, u32), BTreeMap<&str, u32>> = BTreeMap::new();
    for (date, hour, sp, n) in rows {
        let d = super::date_to_epoch_days(date);
        if d < first_day || d > end_day || *hour > 23 {
            continue;
        }
        *cells
            .entry((d - first_day, *hour))
            .or_default()
            .entry(sp.as_str())
            .or_insert(0) += n;
    }

    let mut out = Vec::with_capacity(days);
    for i in 0..days {
        let (y, m, d) = super::days_to_date(first_day + i as u64);
        let mut day = DayCells {
            date: format!("{y}-{m:02}-{d:02}"),
            n: [0; 24],
            top: [None; 24],
            top_n: [0; 24],
            off: listened.is_some_and(|l| l.get(i).is_some_and(|on| !on)),
        };
        for h in 0..24_u32 {
            let Some(here) = cells.get(&(i as u64, h)) else {
                continue;
            };
            day.n[h as usize] = here.values().sum();
            // The most detections; the alphabetically first species on a tie.
            let best = here
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                .map(|(sp, n)| (*sp, *n));
            if let Some((sp, n)) = best {
                day.top[h as usize] = index.get(sp).copied();
                day.top_n[h as usize] = n;
            }
        }
        out.push(day);
    }
    Terrain {
        end: end.to_owned(),
        hour_now,
        species: ranked.iter().map(|(n, _)| display(n)).collect(),
        days: out,
        sun: Vec::new(),
    }
}

/// `x` to two decimals (a hundredth of an hour is thirty-six seconds).
fn round2(x: f32) -> f64 {
    (f64::from(x) * 100.0).round() / 100.0
}

/// The terrain as the JSON `static/terrain.js` reads: flat arrays of `days × 24` (day-major,
/// oldest first), so a year is three arrays of 8 760 small integers and not 8 760 objects.
pub(crate) fn to_json(t: &Terrain) -> String {
    let n: Vec<u32> = t.days.iter().flat_map(|d| d.n).collect();
    let top: Vec<i32> = t
        .days
        .iter()
        .flat_map(|d| d.top.map(|s| s.map_or(-1, i32::from)))
        .collect();
    let top_n: Vec<u32> = t.days.iter().flat_map(|d| d.top_n).collect();
    let off: Vec<u8> = t.days.iter().map(|d| u8::from(d.off)).collect();
    serde_json::json!({
        "v": 1,
        "end": t.end,
        "hour_now": t.hour_now,
        "days": t.days.len(),
        "dates": t.days.iter().map(|d| d.date.as_str()).collect::<Vec<_>>(),
        "species": t.species,
        "n": n,
        "top": top,
        "top_n": top_n,
        "off": off,
        // Sunrise and sunset per day, `null` where unknown; `null` altogether without a location.
        "sun": if t.sun.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::Array(
                t.sun
                    .iter()
                    .map(|d| d.map_or(serde_json::Value::Null, |(r, s)| {
                        serde_json::json!([round2(r), round2(s)])
                    }))
                    .collect(),
            )
        },
        // What the picture found, as plain text the page sets under the title.
        "take": sunrise_share(t, SUNRISE_HALF_WINDOW)
            .and_then(|s| super::insight::sunrise_sentence(s.share, s.chance, SUNRISE_HALF_WINDOW)),
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// The detections of the `days` days ending today, by date, hour and species.
///
/// The same view every other aggregate reads, so a rejected detection or an excluded import does
/// not count here either.
fn collect_rows(conn: &rusqlite::Connection, days: u32) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn.prepare(
        "SELECT Date, CAST(substr(Time, 1, 2) AS INTEGER) AS hr, Com_Name, COUNT(*) \
         FROM detections_analytic \
         WHERE Date >= date('now', 'localtime', ?1) \
         GROUP BY Date, hr, Com_Name",
    )?;
    let modifier = format!("-{} days", days.saturating_sub(1));
    let rows = stmt.query_map([&modifier], |r| {
        Ok((
            r.get::<_, String>(0)?,
            u32::try_from(r.get::<_, i64>(1)?.clamp(0, 23)).unwrap_or(0),
            r.get::<_, String>(2)?,
            u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX),
        ))
    })?;
    rows.collect()
}

/// Sunrise and sunset on each of `dates` (`YYYY-MM-DD`), as fractional hours on the station's
/// local clock; empty when the station has no location.
///
/// The offset is the one in force *on that date*, asked of SQLite's own time zone: a sunrise in
/// March and one in October are an hour apart on the wall clock for the same sun, and a year of
/// them on today's offset would put half the lines an hour out.
fn sun_by_day(conn: &rusqlite::Connection, dates: &[String]) -> Vec<Option<(f32, f32)>> {
    let Some(location) = super::station_location(conn) else {
        return Vec::new();
    };
    let Ok(mut offset) = conn.prepare(
        "SELECT CAST(ROUND((julianday(?1 || ' 12:00:00', 'localtime') \
                          - julianday(?1 || ' 12:00:00')) * 86400.0) AS INTEGER)",
    ) else {
        return Vec::new();
    };
    dates
        .iter()
        .map(|date| {
            let year: u32 = date.get(0..4)?.parse().ok()?;
            let month: u32 = date.get(5..7)?.parse().ok()?;
            let day: u32 = date.get(8..10)?.parse().ok()?;
            let solar = birdnet_scheduler::SolarDay::for_date(location, year, month, day).ok()?;
            let secs: i64 = offset.query_row([date], |r| r.get(0)).ok()?;
            #[allow(clippy::cast_precision_loss)]
            let offset_h = secs as f64 / 3600.0;
            #[allow(clippy::cast_possible_truncation)] // a time of day, under 24
            let local = |m: u32| (f64::from(m) / 60.0 + offset_h).rem_euclid(24.0) as f32;
            Some((local(solar.sunrise_utc_min?), local(solar.sunset_utc_min?)))
        })
        .collect()
}

/// The terrain for `days` days, from the database; `None` is a read that failed.
fn compute_terrain(state: &AppState, days: u32) -> Option<Terrain> {
    let today = super::today_date_string();
    #[allow(clippy::cast_possible_truncation)]
    let hour_now =
        u8::try_from(super::now_hour_local().floor().clamp(0.0, 23.0) as i64).unwrap_or(0);
    let names = state.names();
    state
        .with_read_db(|conn| -> rusqlite::Result<Terrain> {
            let rows = collect_rows(conn, days)?;
            // The ledger is a refinement: without it nothing is claimed about the station being off.
            let listened = super::listening::first_date(&today, days as usize)
                .and_then(|from| super::listening::effort_by_date(conn, &from, &today).ok())
                .and_then(|effort| super::listening::listened_days(&effort, &today, days as usize));
            let terrain = build(
                &rows,
                &today,
                days as usize,
                hour_now,
                listened.as_deref(),
                &|n| names.name_of_stored(n).to_owned(),
            );
            let dates: Vec<String> = terrain.days.iter().map(|d| d.date.clone()).collect();
            let sun = sun_by_day(conn, &dates);
            Ok(terrain.with_sun(sun))
        })
        .ok()
}

async fn terrain_data(
    State(state): State<AppState>,
    Query(q): Query<TerrainQuery>,
) -> impl axum::response::IntoResponse {
    let days = range_of(q.days);
    let body = cached_fragment(&state, format!("terrain:{days}"), TERRAIN_ERR, move |s| {
        compute_terrain(s, days).map(|t| to_json(&t))
    })
    .await;
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
}

/// The table: for a month one row per day, for longer one per week.
fn render_table(t: &Terrain) -> String {
    let weekly = t.days.len() > 31;
    let mut html = String::from(
        r#"<div class="viz-table-scroll"><table class="viz-table tr-table"><thead><tr><th scope="col">"#,
    );
    html.push_str(if weekly { "Week of" } else { "Day" });
    html.push_str(
        r#"</th><th scope="col" class="num">Detections</th><th scope="col">Busiest hour</th><th scope="col">Most heard there</th></tr></thead><tbody>"#,
    );
    let step = if weekly { 7 } else { 1 };
    // Newest first, so what happened this week is at the top.
    let starts: Vec<usize> = (0..t.days.len()).step_by(step).collect();
    for &start in starts.iter().rev() {
        let end = (start + step).min(t.days.len());
        let chunk = &t.days[start..end];
        let total: u64 = chunk.iter().flat_map(|d| d.n).map(u64::from).sum();
        let off = chunk.iter().all(|d| d.off);
        // The busiest hour of the chunk, summed over its days.
        let mut by_hour = [0_u64; 24];
        let mut top_in_hour: Vec<std::collections::BTreeMap<Option<u16>, u64>> =
            vec![std::collections::BTreeMap::default(); 24];
        for d in chunk {
            for h in 0..24 {
                by_hour[h] += u64::from(d.n[h]);
                *top_in_hour[h].entry(d.top[h]).or_insert(0) += u64::from(d.top_n[h]);
            }
        }
        let busiest = (0..24)
            .max_by(|&a, &b| by_hour[a].cmp(&by_hour[b]).then(b.cmp(&a)))
            .unwrap_or(0);
        let _ = write!(
            html,
            r#"<tr><th scope="row">{}</th>"#,
            escape_html(&chunk[0].date)
        );
        if off {
            html.push_str(
                r#"<td class="num">—</td><td colspan="2">The station was not listening</td></tr>"#,
            );
            continue;
        }
        let top = top_in_hour[busiest]
            .iter()
            .filter(|(k, _)| k.is_some())
            .max_by(|a, b| a.1.cmp(b.1))
            .and_then(|(k, _)| k.and_then(|i| t.species.get(usize::from(i))))
            .map_or_else(|| "—".to_owned(), |s| escape_html(s));
        let busiest_cell = if total == 0 {
            "—".to_owned()
        } else {
            crate::display::clock_hour(busiest)
        };
        let _ = write!(
            html,
            r#"<td class="num">{total}</td><td>{busiest_cell}</td><td>{top}</td></tr>"#,
            total = super::group_thousands(i64::try_from(total).unwrap_or(i64::MAX)),
        );
    }
    html.push_str("</tbody></table></div>");
    html
}

async fn terrain_table(
    State(state): State<AppState>,
    Query(q): Query<TerrainQuery>,
) -> impl axum::response::IntoResponse {
    let days = range_of(q.days);
    let html = cached_fragment(
        &state,
        format!("terrain-table:{days}"),
        FRAGMENT_ERR,
        move |s| compute_terrain(s, days).map(|t| render_table(&t)),
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// The card
// ---------------------------------------------------------------------------

static CARD: crate::seal::Sealed =
    crate::seal::Sealed::new(include_str!("../../../templates/terrain.html"));

/// The card the "When active" tab leads with. The page holds no data: `static/terrain.js` reads
/// `data-src`, fetches it, and draws the grid; the 3D view (and three.js with it) is loaded when,
/// and only when, the reader asks for it.
pub(super) fn content() -> String {
    CARD.replace("{{version}}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(date: &str, hour: u32, sp: &str, n: u32) -> Row {
        (date.to_owned(), hour, sp.to_owned(), n)
    }

    fn ident(s: &str) -> String {
        s.to_owned()
    }

    #[test]
    fn a_request_gets_a_range_the_card_offers() {
        assert_eq!(range_of(None), 90);
        assert_eq!(range_of(Some(30)), 30);
        assert_eq!(range_of(Some(90)), 90);
        assert_eq!(range_of(Some(365)), 365);
        assert_eq!(range_of(Some(400)), 365, "never more than a year");
        assert_eq!(range_of(Some(200)), 90, "the nearest smaller choice");
        assert_eq!(range_of(Some(1)), 30);
        assert_eq!(range_of(Some(0)), 30);
    }

    /// Every day of the range is there, oldest first, and a day with nothing is a flat day: a
    /// quiet day dropped from the list would shorten the terrain and move every later day.
    #[test]
    fn every_day_is_there_even_the_quiet_ones() {
        let rows = [
            row("2026-10-19", 6, "Robin", 4),
            row("2026-10-21", 7, "Wren", 2),
        ];
        let t = build(&rows, "2026-10-21", 5, 9, None, &ident);
        assert_eq!(t.days.len(), 5);
        assert_eq!(t.days[0].date, "2026-10-17");
        assert_eq!(t.days[4].date, "2026-10-21");
        assert_eq!(t.days[2].n[6], 4);
        assert_eq!(
            t.days[3].n.iter().sum::<u32>(),
            0,
            "20 Oct was quiet, and is there"
        );
        assert_eq!(t.days[4].n[7], 2);
        // The year boundary and a leap day do not shorten a range.
        let t = build(&[], "2025-03-02", 4, 0, None, &ident);
        assert_eq!(
            t.days.iter().map(|d| d.date.as_str()).collect::<Vec<_>>(),
            ["2025-02-27", "2025-02-28", "2025-03-01", "2025-03-02"]
        );
        let t = build(&[], "2024-03-01", 3, 0, None, &ident);
        assert_eq!(t.days[0].date, "2024-02-28");
        assert_eq!(t.days[1].date, "2024-02-29");
    }

    /// A cell's total is every species' count in it and its top species is the one with the most.
    #[test]
    fn a_cell_says_who_was_heard_most() {
        let rows = [
            row("2026-10-21", 6, "Wren", 3),
            row("2026-10-21", 6, "Robin", 9),
            row("2026-10-21", 6, "Jay", 9),
            row("2026-10-21", 7, "Wren", 1),
        ];
        let t = build(&rows, "2026-10-21", 1, 23, None, &ident);
        let d = &t.days[0];
        assert_eq!(d.n[6], 21);
        // Robin and Jay tie at nine: the alphabetically first wins, so the page and its cache agree.
        assert_eq!(
            t.species[usize::from(d.top[6].expect("a top species"))],
            "Jay"
        );
        assert_eq!(d.top_n[6], 9);
        assert_eq!(
            t.species[usize::from(d.top[7].expect("a top species"))],
            "Wren"
        );
        assert!(
            d.top[5].is_none() && d.top_n[5] == 0,
            "a silent hour has no top species"
        );
        // Species are ranked by how much they were heard over the whole range.
        assert_eq!(
            t.species,
            vec!["Jay".to_owned(), "Robin".to_owned(), "Wren".to_owned()]
        );
    }

    /// Rows outside the range, and hours that do not exist, are not counted anywhere.
    #[test]
    fn rows_outside_the_range_are_ignored() {
        let rows = [
            row("2026-10-10", 6, "Robin", 100),
            row("2026-10-21", 25, "Robin", 100),
            row("2026-10-22", 6, "Robin", 100),
            row("2026-10-21", 6, "Robin", 1),
        ];
        let t = build(&rows, "2026-10-21", 3, 23, None, &ident);
        let total: u32 = t.days.iter().flat_map(|d| d.n).sum();
        assert_eq!(total, 1, "{t:?}");
    }

    /// The recording ledger decides which days are "off", and with no ledger nothing is claimed.
    #[test]
    fn a_day_the_station_was_off_is_off_not_quiet() {
        let t = build(&[], "2026-10-21", 3, 9, Some(&[true, false, true]), &ident);
        assert_eq!(
            t.days.iter().map(|d| d.off).collect::<Vec<_>>(),
            [false, true, false]
        );
        let t = build(&[], "2026-10-21", 3, 9, None, &ident);
        assert!(t.days.iter().all(|d| !d.off), "no ledger: no claim");
    }

    /// The JSON has `days × 24` of each flat array and says which days are off; a species name
    /// is data, not markup.
    #[test]
    fn the_json_is_flat_and_complete() {
        let rows = [row("2026-10-20", 6, "<b>Robin</b>", 4)];
        let t = build(&rows, "2026-10-21", 2, 9, Some(&[false, true]), &ident);
        let v: serde_json::Value = serde_json::from_str(&to_json(&t)).expect("json");
        assert_eq!(v["days"], 2);
        assert_eq!(v["n"].as_array().expect("n").len(), 48);
        assert_eq!(v["top"].as_array().expect("top").len(), 48);
        assert_eq!(v["top_n"].as_array().expect("top_n").len(), 48);
        assert_eq!(v["off"], serde_json::json!([1, 0]));
        assert_eq!(v["n"][6], 4);
        assert_eq!(v["top"][6], 0);
        assert_eq!(v["top"][7], -1);
        assert_eq!(v["dates"][1], "2026-10-21");
        assert_eq!(
            v["species"][0], "<b>Robin</b>",
            "data is carried as it is; the page sets it with textContent"
        );
        assert_eq!(v["hour_now"], 9);
    }

    /// The table has a row per day for a month and one per week beyond, newest first, and a day
    /// the station was off says so instead of printing a zero.
    #[test]
    fn the_table_is_the_same_numbers() {
        let rows = [
            row("2026-10-21", 6, "Robin", 4),
            row("2026-10-20", 6, "Wren", 1),
        ];
        let t = build(
            &rows,
            "2026-10-21",
            3,
            23,
            Some(&[false, true, true]),
            &ident,
        );
        let html = render_table(&t);
        assert_eq!(html.matches("<tr><th scope=\"row\">").count(), 3);
        let newest = html.find("2026-10-21").expect("newest");
        let oldest = html.find("2026-10-19").expect("oldest");
        assert!(newest < oldest, "newest first");
        assert!(html.contains("The station was not listening"));
        assert!(html.contains("Robin"));
        // Longer ranges are by the week.
        let t = build(&[], "2026-10-21", 90, 23, None, &ident);
        assert!(render_table(&t).contains("Week of"));
        assert_eq!(
            render_table(&t).matches("<tr><th scope=\"row\">").count(),
            13
        );
    }

    /// A terrain of `days` days, every one with sunrise at `rise`, and `n` heard in each hour in
    /// `busy` (and nothing elsewhere).
    fn sunny(days: usize, rise: f32, busy: &[usize], n: u32) -> Terrain {
        let day = |i: usize| {
            let mut cells = DayCells {
                date: format!("2026-04-{:02}", i + 1),
                n: [0; 24],
                top: [None; 24],
                top_n: [0; 24],
                off: false,
            };
            for &h in busy {
                cells.n[h] = n;
            }
            cells
        };
        Terrain {
            end: format!("2026-04-{days:02}"),
            hour_now: 23,
            species: Vec::new(),
            days: (0..days).map(day).collect(),
            sun: Vec::new(),
        }
        .with_sun(vec![Some((rise, rise + 12.0)); days])
    }

    /// Hours 4 to 7 are the window around a 06:00 sunrise: all of what was heard there is all of it.
    #[test]
    fn song_inside_the_sunrise_window_is_all_of_the_share() {
        let t = sunny(10, 6.0, &[4, 5, 6, 7], 10);
        let s = sunrise_share(&t, SUNRISE_HALF_WINDOW).unwrap();
        assert!((s.share - 1.0).abs() < 1e-9, "{s:?}");
        assert!((s.chance - 4.0 / 24.0).abs() < 1e-9);
        assert_eq!(s.total, 400);
        // The counterpart: the same counts at midday are none of it.
        let noon = sunny(10, 6.0, &[11, 12, 13, 14], 10);
        assert!(sunrise_share(&noon, SUNRISE_HALF_WINDOW).unwrap().share < 1e-9);
    }

    /// A cell counts for the part of it inside the window: a window from 04:40 takes two-thirds of
    /// the 04:00 cell, not all of it and not none.
    #[test]
    fn a_cell_half_inside_the_window_counts_for_half() {
        // Sunrise at 06:30: the window is 04:30 to 08:30, so hours 4 and 8 are half inside.
        let t = sunny(10, 6.5, &[4, 8], 10);
        let s = sunrise_share(&t, SUNRISE_HALF_WINDOW).unwrap();
        assert!((s.share - 0.5).abs() < 1e-9, "{s:?}");
    }

    /// A day the station was not listening, and the hours of today that have not come, are not
    /// evidence that nothing sang.
    #[test]
    fn days_off_and_hours_not_yet_come_are_not_counted() {
        let mut t = sunny(10, 6.0, &[5, 12], 10);
        t.days[0].off = true;
        let all = sunny(10, 6.0, &[5, 12], 10);
        assert_eq!(
            sunrise_share(&t, SUNRISE_HALF_WINDOW).unwrap().total,
            sunrise_share(&all, SUNRISE_HALF_WINDOW).unwrap().total - 20,
            "the off day is left out"
        );
        let mut early = sunny(10, 6.0, &[5, 12], 10);
        early.hour_now = 8; // it is nine in the morning on the last day: the 12:00 cell has not happened
        assert_eq!(
            sunrise_share(&early, SUNRISE_HALF_WINDOW).unwrap().total,
            sunrise_share(&all, SUNRISE_HALF_WINDOW).unwrap().total - 10
        );
    }

    #[test]
    fn nothing_is_said_without_a_sun_a_week_or_enough_birds() {
        let mut no_sun = sunny(10, 6.0, &[5], 10);
        no_sun.sun.clear();
        assert!(
            sunrise_share(&no_sun, SUNRISE_HALF_WINDOW).is_none(),
            "no location"
        );
        assert!(
            sunrise_share(&sunny(6, 6.0, &[5], 10), SUNRISE_HALF_WINDOW).is_none(),
            "six days is not a pattern"
        );
        assert!(
            sunrise_share(&sunny(10, 6.0, &[5], 1), SUNRISE_HALF_WINDOW).is_none(),
            "ten detections is not enough to say"
        );
    }

    #[test]
    fn the_json_carries_the_sun_and_the_finding_and_nulls_without_them() {
        let t = sunny(10, 6.0, &[4, 5, 6, 7], 10);
        let v: serde_json::Value = serde_json::from_str(&to_json(&t)).unwrap();
        assert_eq!(v["sun"].as_array().unwrap().len(), 10);
        assert_eq!(v["sun"][0], serde_json::json!([6.0, 18.0]));
        assert!(
            v["take"]
                .as_str()
                .unwrap()
                .contains("100% of what was heard")
        );
        let mut bare = t;
        bare.sun.clear();
        let v: serde_json::Value = serde_json::from_str(&to_json(&bare)).unwrap();
        assert!(v["sun"].is_null() && v["take"].is_null(), "{v}");
    }

    /// A sun list that does not match the days is dropped rather than drawn against the wrong
    /// days.
    #[test]
    fn a_sun_list_of_the_wrong_length_is_dropped() {
        let t = sunny(10, 6.0, &[5], 10).with_sun(vec![Some((6.0, 18.0)); 3]);
        assert!(t.sun.is_empty());
    }

    fn station(lat_lon: Option<(&str, &str)>) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        if let Some((lat, lon)) = lat_lon {
            birdnet_db::settings::set(
                &conn,
                "latitude",
                lat,
                birdnet_db::settings::SettingsCategory::Location,
            )
            .unwrap();
            birdnet_db::settings::set(
                &conn,
                "longitude",
                lon,
                birdnet_db::settings::SettingsCategory::Location,
            )
            .unwrap();
        }
        conn
    }

    /// With a location every day gets a sunrise before its sunset, on the 24-hour clock, and the
    /// sun is up longer in June than in December at 51 N. Without one nothing is claimed.
    /// (What this does not hold is the offset: the test runner's own time zone is whatever it is,
    /// and changing it for a test is not something this crate may do.)
    #[test]
    fn a_located_station_gets_a_sun_for_each_day_and_an_unlocated_one_none() {
        let dates: Vec<String> = ["2026-06-21", "2026-12-21"].map(String::from).to_vec();
        let sun = sun_by_day(&station(Some(("51.48", "-0.13"))), &dates);
        assert_eq!(sun.len(), 2);
        let (june, december) = (sun[0].unwrap(), sun[1].unwrap());
        for (rise, set) in [june, december] {
            assert!(
                (0.0..24.0).contains(&rise) && (0.0..24.0).contains(&set),
                "{rise} {set}"
            );
            assert!(rise < set);
        }
        assert!(
            june.1 - june.0 > december.1 - december.0 + 6.0,
            "{june:?} {december:?}"
        );
        assert!(sun_by_day(&station(None), &dates).is_empty());
    }

    #[test]
    fn a_date_that_is_not_a_date_has_no_sun_and_does_not_shift_the_others() {
        let dates = vec![
            "2026-06-21".to_owned(),
            "garbage".to_owned(),
            "2026-06-23".to_owned(),
        ];
        let sun = sun_by_day(&station(Some(("51.48", "-0.13"))), &dates);
        assert_eq!(sun.len(), 3);
        assert!(sun[0].is_some() && sun[1].is_none() && sun[2].is_some());
    }
}
