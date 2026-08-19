//! Detection read queries: counts, listings, pagination, today's feed, and
//! the multi-stream corroboration lookup.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;
use crate::sqlite::types::{
    ConcurrentDetection, DETECTION_COLS, DayCount, DetectionRow, SourceActivity, map_detection_row,
};

use super::search::{SearchTerm, parse_search_term};

/// SQL predicate for "this detection has audio you can actually play".
///
/// One definition, because every surface that offers a play button has to agree
/// with every other one and with the retention pass that removes the audio. It
/// was previously spelled out at eight call sites, which is exactly the shape
/// of thing that drifts.
///
/// Two conditions, and they mean different things:
///
///   * no `File_Name` — the detection never had a clip (a BirdNET-Pi import, a
///     quarantine approval re-inserted without re-extraction, a station with
///     extraction disabled);
///   * `Clip_Pruned_At` set — it had one, and retention reclaimed the file. The
///     name is deliberately kept (see migration 22): the row still records that
///     audio existed and what it was called, which is provenance an analysis
///     may need long after the disk space was recovered.
///
/// Either way there is nothing to play, so both are excluded here — while
/// everything that counts, groups or charts detections still sees every row.
pub const CLIP_AVAILABLE: &str =
    "File_Name IS NOT NULL AND TRIM(File_Name) <> '' AND Clip_Pruned_At IS NULL";

/// Get the total number of detection **rows**, rejected ones included.
///
/// This is the store's row count, not an analytic. It is what
/// `AppState`'s SQLite-vs-`DuckDB` reconciliation compares (paired with
/// [`crate::sqlite::rejected_detection_count`]), so it must keep counting every
/// row. Anything that shows an operator "how many detections" wants
/// [`analytic_detection_count`] instead.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_count(conn: &Connection) -> Result<i64, DbError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM detections", [], |row| row.get(0))?;
    Ok(count)
}

/// Get the number of detections an operator would count — rejected ones
/// excluded.
///
/// The distinction is not pedantry. The dashboard's headline tile row showed
/// six numbers drawn from both sides of it: "Species", "Last hour" and the
/// 12-day sparkline read `detections_analytic` and excluded rejections, while
/// "Detections", "Today" and "Species today" counted every row. Adjacent tiles
/// on one screen therefore disagreed about the same day, and the disagreement
/// grew with every rejection the operator recorded.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn analytic_detection_count(conn: &Connection) -> Result<i64, DbError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM detections_analytic", [], |row| {
        row.get(0)
    })?;
    Ok(count)
}

/// Detections on `date` an operator would count — rejected ones excluded.
///
/// See [`analytic_detection_count`] for why this exists beside
/// [`detection_count_for_date`].
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn analytic_detection_count_for_date(conn: &Connection, date: &str) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT COUNT(*) FROM detections_analytic WHERE Date = ?1",
        params![date],
        |row| row.get(0),
    )
    .map_err(DbError::Sqlite)
}

/// Distinct species on `date` an operator would count — rejected ones excluded.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn analytic_species_count_for_date(conn: &Connection, date: &str) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT COUNT(DISTINCT Com_Name) FROM detections_analytic WHERE Date = ?1",
        params![date],
        |row| row.get(0),
    )
    .map_err(DbError::Sqlite)
}

/// Get the total number of detections for a specific date.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_count_for_date(conn: &Connection, date: &str) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE Date = ?1",
        params![date],
        |row| row.get(0),
    )
    .map_err(DbError::Sqlite)
}

/// Get the number of detections for a specific species on a specific date.
///
/// Used to detect "first detection of species today" (count == 1 after insert).
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_count_for_species_date(
    conn: &Connection,
    date: &str,
    sci_name: &str,
) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT COUNT(*) FROM detections_analytic WHERE Date = ?1 AND Sci_Name = ?2",
        params![date, sci_name],
        |row| row.get(0),
    )
    .map_err(DbError::Sqlite)
}

/// Query detections for a specific date, ordered by time descending.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detections_by_date(conn: &Connection, date: &str) -> Result<Vec<DetectionRow>, DbError> {
    let sql = format!("SELECT {DETECTION_COLS} FROM detections WHERE Date = ?1 ORDER BY Time DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![date], map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Seconds elapsed since the most recent stored detection, or `None` when
/// the station has never detected anything.
///
/// The end-to-end freshness signal for the detection-deadman watchdog: it
/// proves the whole audio -> capture -> inference -> insert chain produced a
/// row recently, which no per-component gauge can. Detection `Date`/`Time`
/// are naive local-time strings (they come from capture filenames stamped
/// with the system timezone), so the elapsed math runs inside SQLite
/// against `'now','localtime'` — the same clock lens — rather than against
/// a Rust UTC timestamp that would skew by the TZ offset.
///
/// Clamped at zero: a detection apparently in the future (clock stepped
/// back after NTP sync) reads as "fresh", the fail-open choice.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn seconds_since_last_detection(conn: &Connection) -> Result<Option<u64>, DbError> {
    use rusqlite::OptionalExtension as _;
    let secs: Option<f64> = conn
        .query_row(
            "SELECT (julianday('now','localtime') - julianday(Date || ' ' || Time)) * 86400.0
             FROM detections ORDER BY Date DESC, Time DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        // A malformed Date/Time (julianday -> NULL) folds to None too: no
        // verdict is better than a bogus one.
        .flatten();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(secs.map(|s| if s.is_finite() { s.max(0.0) as u64 } else { 0 }))
}

/// Query the most recent detections up to `limit`.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn recent_detections(conn: &Connection, limit: u32) -> Result<Vec<DetectionRow>, DbError> {
    let sql =
        format!("SELECT {DETECTION_COLS} FROM detections ORDER BY Date DESC, Time DESC LIMIT ?1");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![limit], map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Highest-confidence detections for a given date that have a playable clip.
///
/// Powers the dashboard "best recordings" at-a-glance widget (BirdNET-Pi
/// parity): the day's most confident detections, each with audio. Rows with no
/// recording file are excluded so every result is playable.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn best_detections_for_date(
    conn: &Connection,
    date: &str,
    limit: u32,
) -> Result<Vec<DetectionRow>, DbError> {
    let sql = format!(
        "SELECT {DETECTION_COLS} FROM detections_analytic \
         WHERE Date = ?1 AND {CLIP_AVAILABLE} \
         ORDER BY Confidence DESC, Time DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![date, limit], map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Detections of the same species from **other** audio sources within
/// `window_secs` of the given detection's instant.
///
/// Powers the multi-stream "also heard by" corroboration display — purely
/// read-only, never collapses or hides anything. Only rows with a non-NULL
/// `Source` different from `exclude_source` are returned, ordered by closeness
/// in time. A ±1-day `Date` bound keeps the scan tight (and correct across a
/// midnight-straddling window) so a very common species stays cheap.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn concurrent_detections_from_other_sources(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
    exclude_source: &str,
    window_secs: f64,
    limit: u32,
) -> Result<Vec<ConcurrentDetection>, DbError> {
    let sql = "SELECT Source, Time, Confidence FROM detections \
               WHERE Sci_Name = ?1 \
                 AND Source IS NOT NULL AND Source <> ?2 \
                 AND Date >= date(?3, '-1 day') AND Date <= date(?3, '+1 day') \
                 AND ABS((julianday(Date || ' ' || Time) - julianday(?3 || ' ' || ?4)) * 86400.0) <= ?5 \
               ORDER BY ABS(julianday(Date || ' ' || Time) - julianday(?3 || ' ' || ?4)) ASC, Source \
               LIMIT ?6";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(
            params![sci_name, exclude_source, date, time, window_secs, limit],
            |row| {
                Ok(ConcurrentDetection {
                    source: row.get(0)?,
                    time: row.get(1)?,
                    confidence: row.get(2)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Query recent detections with limit and offset for pagination.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn recent_detections_page(
    conn: &Connection,
    limit: u32,
    offset: u32,
) -> Result<Vec<DetectionRow>, DbError> {
    let sql = format!(
        "SELECT {DETECTION_COLS} FROM detections \
         ORDER BY Date DESC, Time DESC LIMIT ?1 OFFSET ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![limit, offset], map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Query all detections, optionally filtered by an inclusive date range.
///
/// Returns rows ordered by date/time descending.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn all_detections(
    conn: &Connection,
    from: Option<&str>,
    to: Option<&str>,
    max_rows: u32,
) -> Result<(Vec<DetectionRow>, bool), DbError> {
    // Bound peak memory: the (public, unauthenticated) export endpoints load
    // the whole result set into a `Vec` and build the entire CSV string from
    // it, so an unfiltered export on a long-running station (millions of rows)
    // would OOM a small Pi. Fetch one row past `max_rows` to detect truncation
    // without a separate COUNT; callers surface an error and ask the user to
    // narrow the date range. `max_rows + 1` is a typed integer interpolated
    // into `LIMIT` — no injection surface.
    let fetch = u64::from(max_rows).saturating_add(1);
    let (where_sql, param_values): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match (from, to) {
        (Some(f), Some(t)) => (
            "WHERE Date >= ?1 AND Date <= ?2",
            vec![Box::new(f.to_string()), Box::new(t.to_string())],
        ),
        (Some(f), None) => ("WHERE Date >= ?1", vec![Box::new(f.to_string())]),
        (None, Some(t)) => ("WHERE Date <= ?1", vec![Box::new(t.to_string())]),
        (None, None) => ("", vec![]),
    };
    let sql = format!(
        "SELECT {DETECTION_COLS} FROM detections {where_sql} \
         ORDER BY Date DESC, Time DESC LIMIT {fetch}"
    );

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt
        .query_map(params_ref.as_slice(), map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let truncated = u64::try_from(rows.len()).unwrap_or(u64::MAX) > u64::from(max_rows);
    if truncated {
        rows.truncate(max_rows as usize);
    }
    Ok((rows, truncated))
}

/// Query recent detections for a specific species by common name.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detections_by_species(
    conn: &Connection,
    com_name: &str,
    limit: u32,
) -> Result<Vec<DetectionRow>, DbError> {
    let sql = format!(
        "SELECT {DETECTION_COLS} FROM detections \
         WHERE Com_Name = ?1 ORDER BY Date DESC, Time DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![com_name, limit], map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Category filter for the Today page's segmented control.
///
/// The definitions reuse the vocabulary the UI already ships: "first today"
/// matches the feed-row badge (species never heard before this date), "rare"
/// matches the `/feeds/rare.rss` definition (a confident first-ever record),
/// and "high confidence" matches the `bnb-conf high` threshold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TodayFilter {
    /// No category filter.
    #[default]
    All,
    /// Species first heard ever on the queried date.
    FirstToday,
    /// First-ever record with confidence > 0.85 (the rare-feed definition).
    Rare,
    /// Confidence ≥ 0.90 (the confidence bar's "high" threshold).
    HighConfidence,
}

impl TodayFilter {
    /// Parse the UI's filter token; unknown tokens fall back to `All`.
    #[must_use]
    pub fn from_token(token: Option<&str>) -> Self {
        match token.map(str::trim) {
            Some("first") => Self::FirstToday,
            Some("rare") => Self::Rare,
            Some("high") => Self::HighConfidence,
            _ => Self::All,
        }
    }

    /// Extra `AND …` clause for queries whose `?1` is the queried date.
    const fn sql_clause(self) -> &'static str {
        match self {
            Self::All => "",
            Self::FirstToday => {
                " AND NOT EXISTS (SELECT 1 FROM detections d2 \
                 WHERE d2.Com_Name = detections.Com_Name AND d2.Date < ?1)"
            }
            Self::Rare => {
                " AND Confidence > 0.85 AND NOT EXISTS (SELECT 1 FROM detections d2 \
                 WHERE d2.Com_Name = detections.Com_Name AND d2.Date < ?1)"
            }
            Self::HighConfidence => " AND Confidence >= 0.9",
        }
    }
}

/// Search today's detections with optional text filter, category filter,
/// limit, and offset.
///
/// If `search` starts with "NOT " (case-insensitive), the rest is used as an
/// exclusion filter (species name NOT LIKE pattern). Otherwise it is an
/// inclusion filter.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn todays_detections(
    conn: &Connection,
    date: &str,
    search: Option<&str>,
    filter: TodayFilter,
    limit: u32,
    offset: u32,
) -> Result<Vec<DetectionRow>, DbError> {
    let extra = filter.sql_clause();
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        match parse_search_term(search) {
            Some(SearchTerm::Exclude(rest)) => {
                let pattern = format!("%{rest}%");
                (
                    format!(
                        "SELECT {DETECTION_COLS} FROM detections \
                         WHERE Date = ?1 AND Com_Name NOT LIKE ?2{extra} \
                         ORDER BY Time DESC LIMIT ?3 OFFSET ?4"
                    ),
                    vec![
                        Box::new(date.to_string()),
                        Box::new(pattern),
                        Box::new(limit),
                        Box::new(offset),
                    ],
                )
            }
            Some(SearchTerm::Include(term)) => {
                let pattern = format!("%{term}%");
                (
                    format!(
                        "SELECT {DETECTION_COLS} FROM detections \
                         WHERE Date = ?1 AND (Com_Name LIKE ?2 OR Sci_Name LIKE ?2){extra} \
                         ORDER BY Time DESC LIMIT ?3 OFFSET ?4"
                    ),
                    vec![
                        Box::new(date.to_string()),
                        Box::new(pattern),
                        Box::new(limit),
                        Box::new(offset),
                    ],
                )
            }
            None => (
                format!(
                    "SELECT {DETECTION_COLS} FROM detections \
                     WHERE Date = ?1{extra} ORDER BY Time DESC LIMIT ?2 OFFSET ?3"
                ),
                vec![
                    Box::new(date.to_string()),
                    Box::new(limit),
                    Box::new(offset),
                ],
            ),
        };

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_ref.as_slice(), map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Count today's detections with optional text and category filters.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn todays_detection_count(
    conn: &Connection,
    date: &str,
    search: Option<&str>,
    filter: TodayFilter,
) -> Result<i64, DbError> {
    let extra = filter.sql_clause();
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        match parse_search_term(search) {
            Some(SearchTerm::Exclude(rest)) => {
                let pattern = format!("%{rest}%");
                (
                    format!(
                        "SELECT COUNT(*) FROM detections \
                         WHERE Date = ?1 AND Com_Name NOT LIKE ?2{extra}"
                    ),
                    vec![Box::new(date.to_string()), Box::new(pattern)],
                )
            }
            Some(SearchTerm::Include(term)) => {
                let pattern = format!("%{term}%");
                (
                    format!(
                        "SELECT COUNT(*) FROM detections \
                         WHERE Date = ?1 AND (Com_Name LIKE ?2 OR Sci_Name LIKE ?2){extra}"
                    ),
                    vec![Box::new(date.to_string()), Box::new(pattern)],
                )
            }
            None => (
                format!("SELECT COUNT(*) FROM detections WHERE Date = ?1{extra}"),
                vec![Box::new(date.to_string())],
            ),
        };

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();
    let count: i64 = conn.query_row(&sql, params_ref.as_slice(), |row| row.get(0))?;
    Ok(count)
}

/// Category filter for the cross-date Recordings clip browser.
///
/// Reuses the Today log's vocabulary where it overlaps so the two surfaces
/// agree on what the words mean: `Best` is the confidence bar's "high"
/// threshold and `Rare` is the rare-feed definition (a confident first-ever
/// record). `Locked` surfaces clips an operator pinned against the disk
/// purge. Unlike [`TodayFilter`] these clauses are date-agnostic — the
/// browser spans every day — so `Rare` keys on the row's own date through a
/// correlated subquery instead of a bound `?1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordingsFilter {
    /// No category filter (every clip that saved an audio file).
    #[default]
    All,
    /// Confidence ≥ 0.90 (the confidence bar's "high" threshold).
    Best,
    /// First-ever record of the species with confidence > 0.85.
    Rare,
    /// Clips locked against the disk purge.
    Locked,
}

impl RecordingsFilter {
    /// Parse the UI's filter token; unknown or missing tokens fall back to
    /// `All`.
    #[must_use]
    pub fn from_token(token: Option<&str>) -> Self {
        match token.map(str::trim) {
            Some("best") => Self::Best,
            Some("rare") => Self::Rare,
            Some("locked") => Self::Locked,
            _ => Self::All,
        }
    }

    /// The canonical token for this filter — the inverse of [`Self::from_token`],
    /// used to build the filter-chip links.
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Best => "best",
            Self::Rare => "rare",
            Self::Locked => "locked",
        }
    }

    /// Extra `AND …` clause appended after the "has a playable clip" predicate.
    const fn sql_clause(self) -> &'static str {
        match self {
            Self::All => "",
            Self::Best => " AND Confidence >= 0.9",
            Self::Rare => {
                " AND Confidence > 0.85 AND NOT EXISTS (SELECT 1 FROM detections d2 \
                 WHERE d2.Com_Name = detections.Com_Name AND d2.Date < detections.Date)"
            }
            Self::Locked => " AND COALESCE(is_locked, 0) = 1",
        }
    }
}

/// Browse recent clips — detections that saved an audio file — across every
/// day, newest first, with an optional category filter, text search, and
/// pagination. Powers the Recordings home's Clips view.
///
/// Only rows with a non-empty `File_Name` are returned, so every result is
/// playable: the same "has a clip" rule as [`best_detections_for_date`]. The
/// `search` term follows the shared include/`NOT `-exclude grammar.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn recent_clips(
    conn: &Connection,
    filter: RecordingsFilter,
    search: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<DetectionRow>, DbError> {
    let extra = filter.sql_clause();
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        match parse_search_term(search) {
            Some(SearchTerm::Exclude(rest)) => {
                let pattern = format!("%{rest}%");
                (
                    format!(
                        "SELECT {DETECTION_COLS} FROM detections \
                         WHERE {CLIP_AVAILABLE} \
                         AND Com_Name NOT LIKE ?1{extra} \
                         ORDER BY Date DESC, Time DESC LIMIT ?2 OFFSET ?3"
                    ),
                    vec![Box::new(pattern), Box::new(limit), Box::new(offset)],
                )
            }
            Some(SearchTerm::Include(term)) => {
                let pattern = format!("%{term}%");
                (
                    format!(
                        "SELECT {DETECTION_COLS} FROM detections \
                         WHERE {CLIP_AVAILABLE} \
                         AND (Com_Name LIKE ?1 OR Sci_Name LIKE ?1){extra} \
                         ORDER BY Date DESC, Time DESC LIMIT ?2 OFFSET ?3"
                    ),
                    vec![Box::new(pattern), Box::new(limit), Box::new(offset)],
                )
            }
            None => (
                format!(
                    "SELECT {DETECTION_COLS} FROM detections \
                     WHERE {CLIP_AVAILABLE}{extra} \
                     ORDER BY Date DESC, Time DESC LIMIT ?1 OFFSET ?2"
                ),
                vec![Box::new(limit), Box::new(offset)],
            ),
        };

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_ref.as_slice(), map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Count the clips [`recent_clips`] would return for the same filter and
/// search (its total, ignoring limit/offset) — drives the "Show more" gate
/// and the filter-chip badges.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn recent_clips_count(
    conn: &Connection,
    filter: RecordingsFilter,
    search: Option<&str>,
) -> Result<i64, DbError> {
    let extra = filter.sql_clause();
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        match parse_search_term(search) {
            Some(SearchTerm::Exclude(rest)) => {
                let pattern = format!("%{rest}%");
                (
                    format!(
                        "SELECT COUNT(*) FROM detections \
                         WHERE {CLIP_AVAILABLE} \
                         AND Com_Name NOT LIKE ?1{extra}"
                    ),
                    vec![Box::new(pattern)],
                )
            }
            Some(SearchTerm::Include(term)) => {
                let pattern = format!("%{term}%");
                (
                    format!(
                        "SELECT COUNT(*) FROM detections \
                         WHERE {CLIP_AVAILABLE} \
                         AND (Com_Name LIKE ?1 OR Sci_Name LIKE ?1){extra}"
                    ),
                    vec![Box::new(pattern)],
                )
            }
            None => (
                format!(
                    "SELECT COUNT(*) FROM detections \
                     WHERE {CLIP_AVAILABLE}{extra}"
                ),
                vec![],
            ),
        };

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();
    let count: i64 = conn.query_row(&sql, params_ref.as_slice(), |row| row.get(0))?;
    Ok(count)
}

/// Per-source detection activity for `date`: how many detections each audio
/// source contributed and the most recent one's time, busiest source first.
///
/// Powers the Station Health per-source panel. See [`SourceActivity`] for why
/// this is an activity signal rather than the supervisor's live stream state.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn todays_source_activity(
    conn: &Connection,
    date: &str,
) -> Result<Vec<SourceActivity>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Source, COUNT(*) AS n, MAX(Time) AS last FROM detections \
         WHERE Date = ?1 GROUP BY Source ORDER BY n DESC, Source",
    )?;
    let rows = stmt
        .query_map(params![date], |row| {
            Ok(SourceActivity {
                source: row.get(0)?,
                count: row.get(1)?,
                last_time: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get a list of distinct dates that have detections, ordered descending.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_dates(conn: &Connection, limit: u32) -> Result<Vec<String>, DbError> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT Date FROM detections_analytic ORDER BY Date DESC LIMIT ?1")?;
    let rows = stmt
        .query_map(params![limit], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// Get species list with counts for a given date.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_for_date(
    conn: &Connection,
    date: &str,
) -> Result<Vec<(String, String, i64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Com_Name, Sci_Name, COUNT(*) as cnt FROM detections_analytic \
         WHERE Date = ?1 GROUP BY Com_Name, Sci_Name ORDER BY cnt DESC",
    )?;
    let rows = stmt
        .query_map(params![date], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Per-day detection counts with distinct-species totals, oldest day first.
///
/// Powers the Reports History heat-calendar: one cell per day, coloured by
/// `count` and annotated with how many distinct species were heard.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detections_per_day(conn: &Connection) -> Result<Vec<DayCount>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, COUNT(*) AS n, COUNT(DISTINCT Com_Name) AS sp \
         FROM detections_analytic GROUP BY Date ORDER BY Date",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DayCount {
                date: row.get(0)?,
                count: row.get(1)?,
                species: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;
    use crate::sqlite::queries::detections::insert_detection;
    use crate::sqlite::queries::detections::test_support::temp_db_with_data;
    use crate::sqlite::types::DetectionRecord;

    /// The deadman freshness signal: a detection stamped two hours ago (in
    /// SQLite's own localtime lens, so no TZ skew) reads as ~7200 s of
    /// silence; an empty table reads as `None`, never zero.
    #[test]
    fn seconds_since_last_detection_measures_local_silence() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_or_create(&dir.path().join("t.db")).unwrap();
        assert_eq!(seconds_since_last_detection(&conn).unwrap(), None);

        let (date, time): (String, String) = conn
            .query_row(
                "SELECT date('now','localtime','-2 hours'), time('now','localtime','-2 hours')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let record = DetectionRecord {
            date: &date,
            time: &time,
            sci_name: "Pica pica",
            com_name: "Eurasian Magpie",
            confidence: 0.9,
            lat: None,
            lon: None,
            cutoff: None,
            week: None,
            sensitivity: None,
            overlap: None,
            file_name: "t.wav",
            chunk_offset_secs: Some(0.0),
            correlation_id: None,
            source: None,
            duration_secs: None,
        };
        insert_detection(&conn, &record).unwrap();

        let secs = seconds_since_last_detection(&conn).unwrap().unwrap();
        assert!(
            (7_100..=7_300).contains(&secs),
            "two hours of silence measured, got {secs}s"
        );
    }

    /// The three `analytic_*` counts must read the view, and must return the
    /// number they computed.
    ///
    /// Found by mutation testing: `analytic_species_count_for_date` survived
    /// being replaced with `Ok(-1)`. It *is* asserted — in
    /// `tests/review_verdicts_apply.rs`, which belongs to the binary crate, so
    /// the `package: birdnet-db` mutation row never runs it. A function whose
    /// only coverage lives in another crate is uncovered from where the gate
    /// stands.
    ///
    /// Both halves are here on purpose. Exact counts before any rejection kill
    /// the "return a constant" mutants; the shift after a rejection is what
    /// distinguishes reading `detections_analytic` from reading `detections` —
    /// asserting only the first would pass for a function that ignores verdicts
    /// entirely, which is the exact defect these three were added to fix.
    #[test]
    fn the_analytic_counts_exclude_a_rejection_and_report_real_numbers() {
        let (_tmp, conn) = temp_db_with_data();

        // Fixture: 4 detections. 2026-03-11 has 3 across 2 species (Blackbird
        // twice, Robin once); 2026-03-10 has 1.
        assert_eq!(analytic_detection_count(&conn).unwrap(), 4);
        assert_eq!(
            analytic_detection_count_for_date(&conn, "2026-03-11").unwrap(),
            3
        );
        assert_eq!(
            analytic_species_count_for_date(&conn, "2026-03-11").unwrap(),
            2
        );

        // Reject the Robin: it is the only one of its species that day, so all
        // three counts have somewhere to move.
        conn.execute(
            "UPDATE detections SET review_verdict = 'rejected'
              WHERE Date = '2026-03-11' AND Com_Name = 'European Robin'",
            [],
        )
        .unwrap();

        assert_eq!(
            analytic_detection_count(&conn).unwrap(),
            3,
            "a rejected detection is still in the all-time analytic count"
        );
        assert_eq!(
            analytic_detection_count_for_date(&conn, "2026-03-11").unwrap(),
            2,
            "a rejected detection is still in the per-day analytic count"
        );
        assert_eq!(
            analytic_species_count_for_date(&conn, "2026-03-11").unwrap(),
            1,
            "a species whose only detection that day was rejected is still counted"
        );

        // The counterpart: the raw counts are unmoved, so the assertions above
        // are about the view and not about the row having been deleted.
        assert_eq!(detection_count(&conn).unwrap(), 4);
        assert_eq!(
            detection_count_for_date(&conn, "2026-03-11").unwrap(),
            3,
            "the raw per-day count must still see the rejected detection"
        );
    }

    #[test]
    fn detections_by_date_ordered() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = detections_by_date(&conn, "2026-03-11").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].time, "07:00:00");
    }

    #[test]
    fn detections_per_day_groups_counts_and_species() {
        // Fixture: 2026-03-10 has 1 detection (1 species); 2026-03-11 has 3
        // detections across 2 species. Rows come back oldest-first.
        let (_tmp, conn) = temp_db_with_data();
        let rows = detections_per_day(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, "2026-03-10");
        assert_eq!(rows[0].count, 1);
        assert_eq!(rows[0].species, 1);
        assert_eq!(rows[1].date, "2026-03-11");
        assert_eq!(rows[1].count, 3);
        assert_eq!(rows[1].species, 2);
    }

    #[test]
    fn best_detections_for_date_orders_by_confidence_and_requires_clip() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        for (date, time, sci, com, conf, file) in [
            // Target day, with clips:
            (
                "2026-03-11",
                "06:30:00",
                "Turdus merula",
                "Blackbird",
                0.80,
                Some("a.wav"),
            ),
            (
                "2026-03-11",
                "06:45:00",
                "Erithacus rubecula",
                "Robin",
                0.95,
                Some("b.wav"),
            ),
            // Target day, highest confidence but NO clip → must be excluded:
            (
                "2026-03-11",
                "07:00:00",
                "Parus major",
                "Great Tit",
                0.99,
                None,
            ),
            // Another day → must be excluded by the date filter:
            (
                "2026-03-10",
                "18:00:00",
                "Cyanistes caeruleus",
                "Blue Tit",
                0.99,
                Some("c.wav"),
            ),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name) VALUES (?1,?2,?3,?4,?5,?6)",
                params![date, time, sci, com, conf, file],
            )
            .unwrap();
        }
        let rows = best_detections_for_date(&conn, "2026-03-11", 5).unwrap();
        // Only the two clipped target-day rows, most confident first.
        assert_eq!(
            rows.len(),
            2,
            "clip-less and other-day rows must be excluded"
        );
        assert_eq!(rows[0].com_name, "Robin"); // 0.95
        assert_eq!(rows[1].com_name, "Blackbird"); // 0.80
    }

    #[test]
    fn concurrent_detections_finds_other_sources_within_window() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let insert = |time: &str, sci: &str, src: Option<&str>, conf: f64| {
            let r = DetectionRecord {
                date: "2026-05-19",
                time,
                sci_name: sci,
                com_name: "X",
                confidence: conf,
                lat: None,
                lon: None,
                cutoff: None,
                week: None,
                sensitivity: None,
                overlap: None,
                file_name: "f.wav",
                chunk_offset_secs: Some(0.0),
                correlation_id: None,
                source: src,
                duration_secs: None,
            };
            insert_detection(&conn, &r).unwrap();
        };
        // "This" detection on cam1, plus: a concurrent cam2 (within window),
        // a far-off cam3 (outside window), a same-source cam1 (excluded), and a
        // different species on cam2 (excluded).
        insert("06:00:00", "Pica pica", Some("cam1"), 0.90);
        insert("06:00:05", "Pica pica", Some("cam2"), 0.85);
        insert("06:02:00", "Pica pica", Some("cam3"), 0.80);
        insert("06:00:03", "Pica pica", Some("cam1"), 0.70);
        insert("06:00:04", "Erithacus rubecula", Some("cam2"), 0.95);

        let hits = concurrent_detections_from_other_sources(
            &conn,
            "2026-05-19",
            "06:00:00",
            "Pica pica",
            "cam1",
            30.0,
            8,
        )
        .unwrap();
        assert_eq!(
            hits.len(),
            1,
            "only cam2 qualifies (same species, within 30 s, other source)"
        );
        assert_eq!(hits[0].source, "cam2");
        assert_eq!(hits[0].time, "06:00:05");
    }

    #[test]
    fn recent_detections_respects_limit() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = recent_detections(&conn, 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, "2026-03-11");
    }

    #[test]
    fn pagination_pages_correctly() {
        let (_tmp, conn) = temp_db_with_data();
        let page1 = recent_detections_page(&conn, 2, 0).unwrap();
        let page2 = recent_detections_page(&conn, 2, 2).unwrap();
        let page3 = recent_detections_page(&conn, 2, 4).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 2);
        assert!(page3.is_empty());
        assert_ne!(page1[0].time, page2[0].time);
    }

    #[test]
    fn all_detections_no_filter() {
        let (_tmp, conn) = temp_db_with_data();
        let (rows, truncated) = all_detections(&conn, None, None, 10_000).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(!truncated);
    }

    #[test]
    fn all_detections_date_range() {
        let (_tmp, conn) = temp_db_with_data();
        let (rows, _) =
            all_detections(&conn, Some("2026-03-11"), Some("2026-03-11"), 10_000).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn all_detections_from_only() {
        let (_tmp, conn) = temp_db_with_data();
        let (rows, _) = all_detections(&conn, Some("2026-03-11"), None, 10_000).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn all_detections_to_only() {
        let (_tmp, conn) = temp_db_with_data();
        let (rows, _) = all_detections(&conn, None, Some("2026-03-10"), 10_000).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn all_detections_truncates_at_max_rows() {
        let (_tmp, conn) = temp_db_with_data();
        // 4 rows exist; cap at 2 → truncated, exactly 2 returned.
        let (rows, truncated) = all_detections(&conn, None, None, 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(truncated);
        // Cap exactly at the row count → not truncated.
        let (rows, truncated) = all_detections(&conn, None, None, 4).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(!truncated);
    }

    #[test]
    fn detections_by_species_filters() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = detections_by_species(&conn, "Eurasian Blackbird", 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|d| d.com_name == "Eurasian Blackbird"));
    }

    #[test]
    fn detection_count_for_date_filters_by_date() {
        let (_tmp, conn) = temp_db_with_data();
        assert_eq!(detection_count_for_date(&conn, "2026-03-11").unwrap(), 3);
        assert_eq!(detection_count_for_date(&conn, "2026-03-10").unwrap(), 1);
        assert_eq!(detection_count_for_date(&conn, "2026-03-09").unwrap(), 0);
    }

    #[test]
    fn detection_count_for_species_date_counts_per_species() {
        let (_tmp, conn) = temp_db_with_data();
        assert_eq!(
            detection_count_for_species_date(&conn, "2026-03-11", "Turdus merula").unwrap(),
            2
        );
        assert_eq!(
            detection_count_for_species_date(&conn, "2026-03-11", "Erithacus rubecula").unwrap(),
            1
        );
        assert_eq!(
            detection_count_for_species_date(&conn, "2026-03-11", "Pica pica").unwrap(),
            0
        );
    }

    #[test]
    fn species_for_date_groups_by_species_descending_by_count() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = species_for_date(&conn, "2026-03-11").unwrap();
        // Two Turdus merula entries, one Erithacus rubecula → Turdus
        // merula is first.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "Eurasian Blackbird");
        assert_eq!(rows[0].1, "Turdus merula");
        assert_eq!(rows[0].2, 2);
        assert_eq!(rows[1].2, 1);
    }

    #[test]
    fn detection_dates_returns_distinct_descending() {
        let (_tmp, conn) = temp_db_with_data();
        let dates = detection_dates(&conn, 10).unwrap();
        assert_eq!(dates, vec!["2026-03-11", "2026-03-10"]);
    }

    #[test]
    fn detection_dates_respects_limit() {
        let (_tmp, conn) = temp_db_with_data();
        let dates = detection_dates(&conn, 1).unwrap();
        assert_eq!(dates, vec!["2026-03-11"]);
    }

    #[test]
    fn todays_detections_filters_by_date_and_search() {
        let (_tmp, conn) = temp_db_with_data();
        // No search → all rows for that date.
        let rows = todays_detections(&conn, "2026-03-11", None, TodayFilter::All, 10, 0).unwrap();
        assert_eq!(rows.len(), 3);

        // Include pattern (Com_Name LIKE).
        let rows =
            todays_detections(&conn, "2026-03-11", Some("Robin"), TodayFilter::All, 10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].com_name, "European Robin");

        // Exclusion pattern (NOT LIKE).
        let rows = todays_detections(
            &conn,
            "2026-03-11",
            Some("NOT Robin"),
            TodayFilter::All,
            10,
            0,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.com_name != "European Robin"));
    }

    #[test]
    fn todays_detections_pagination() {
        let (_tmp, conn) = temp_db_with_data();
        let page1 = todays_detections(&conn, "2026-03-11", None, TodayFilter::All, 2, 0).unwrap();
        let page2 = todays_detections(&conn, "2026-03-11", None, TodayFilter::All, 2, 2).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 1);
        assert_ne!(page1[0].time, page2[0].time);
    }

    #[test]
    fn todays_detections_whitespace_search_treated_as_none() {
        let (_tmp, conn) = temp_db_with_data();
        // A blank search term should not collapse the result set.
        let rows =
            todays_detections(&conn, "2026-03-11", Some("   "), TodayFilter::All, 10, 0).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn todays_detections_search_matches_sci_name_too() {
        // The inclusion path matches either Com_Name or Sci_Name LIKE.
        let (_tmp, conn) = temp_db_with_data();
        let rows = todays_detections(
            &conn,
            "2026-03-11",
            Some("Erithacus"),
            TodayFilter::All,
            10,
            0,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sci_name, "Erithacus rubecula");
    }

    #[test]
    fn todays_detection_count_filters_match_query_path() {
        let (_tmp, conn) = temp_db_with_data();
        assert_eq!(
            todays_detection_count(&conn, "2026-03-11", None, TodayFilter::All).unwrap(),
            3
        );
        assert_eq!(
            todays_detection_count(&conn, "2026-03-11", Some("Robin"), TodayFilter::All).unwrap(),
            1
        );
        assert_eq!(
            todays_detection_count(&conn, "2026-03-11", Some("NOT Robin"), TodayFilter::All)
                .unwrap(),
            2
        );
        assert_eq!(
            todays_detection_count(&conn, "2026-03-11", Some("   "), TodayFilter::All).unwrap(),
            3
        );
    }

    #[test]
    fn today_filter_categories_select_the_right_rows() {
        // Two days of data: the Wren is brand new today (high confidence →
        // also "rare" by the feed definition); the Robin was already known
        // yesterday; the Dunnock is new today but too uncertain for "rare".
        let dir = tempfile::tempdir().unwrap();
        let conn = open_or_create(&dir.path().join("t.db")).unwrap();
        let insert = |date: &str, time: &str, sci: &str, com: &str, conf: f64| {
            let record = DetectionRecord {
                date,
                time,
                sci_name: sci,
                com_name: com,
                confidence: conf,
                lat: None,
                lon: None,
                cutoff: None,
                week: None,
                sensitivity: None,
                overlap: None,
                file_name: "t.wav",
                chunk_offset_secs: Some(0.0),
                correlation_id: None,
                source: None,
                duration_secs: None,
            };
            insert_detection(&conn, &record).unwrap();
        };
        insert(
            "2026-06-10",
            "07:00:00",
            "Erithacus rubecula",
            "Robin",
            0.95,
        );
        insert(
            "2026-06-11",
            "06:00:00",
            "Erithacus rubecula",
            "Robin",
            0.97,
        );
        insert(
            "2026-06-11",
            "06:10:00",
            "Troglodytes aedon",
            "House Wren",
            0.93,
        );
        insert(
            "2026-06-11",
            "06:20:00",
            "Prunella modularis",
            "Dunnock",
            0.60,
        );

        let names = |filter: TodayFilter| -> Vec<String> {
            todays_detections(&conn, "2026-06-11", None, filter, 10, 0)
                .unwrap()
                .into_iter()
                .map(|d| d.com_name)
                .collect()
        };
        assert_eq!(names(TodayFilter::All).len(), 3);
        assert_eq!(
            names(TodayFilter::FirstToday),
            vec!["Dunnock", "House Wren"]
        );
        assert_eq!(names(TodayFilter::Rare), vec!["House Wren"]);
        assert_eq!(
            names(TodayFilter::HighConfidence),
            vec!["House Wren", "Robin"]
        );

        // Counts agree with the listing for every category.
        for f in [
            TodayFilter::All,
            TodayFilter::FirstToday,
            TodayFilter::Rare,
            TodayFilter::HighConfidence,
        ] {
            let listed = i64::try_from(names(f).len()).unwrap();
            let counted = todays_detection_count(&conn, "2026-06-11", None, f).unwrap();
            assert_eq!(listed, counted, "count mismatch for {f:?}");
        }
    }

    #[test]
    fn today_filter_token_parsing_is_total() {
        assert_eq!(
            TodayFilter::from_token(Some("first")),
            TodayFilter::FirstToday
        );
        assert_eq!(TodayFilter::from_token(Some("rare")), TodayFilter::Rare);
        assert_eq!(
            TodayFilter::from_token(Some("high")),
            TodayFilter::HighConfidence
        );
        assert_eq!(TodayFilter::from_token(Some("bogus")), TodayFilter::All);
        assert_eq!(TodayFilter::from_token(None), TodayFilter::All);
    }

    #[test]
    fn recent_detections_page_pagination_terminates() {
        // Pin that the page beyond the data is empty (boundary case
        // that mutation testing on `LIMIT ?1 OFFSET ?2` would otherwise
        // flip).
        let (_tmp, conn) = temp_db_with_data();
        let beyond = recent_detections_page(&conn, 10, 100).unwrap();
        assert!(beyond.is_empty());
    }

    #[test]
    fn recent_clips_filters_and_file_gate() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_or_create(&dir.path().join("t.db")).unwrap();
        let insert = |date: &str, time: &str, sci: &str, com: &str, conf: f64, file: &str| {
            let record = DetectionRecord {
                date,
                time,
                sci_name: sci,
                com_name: com,
                confidence: conf,
                lat: None,
                lon: None,
                cutoff: None,
                week: None,
                sensitivity: None,
                overlap: None,
                file_name: file,
                chunk_offset_secs: Some(0.0),
                correlation_id: None,
                source: None,
                duration_secs: None,
            };
            insert_detection(&conn, &record).unwrap();
        };
        // Robin known yesterday (its first-ever record, but below the rare
        // confidence floor); Robin again today (best, not first-ever); a new
        // House Wren today (best AND rare); a new Dunnock today (too uncertain
        // for rare); a high-confidence Crow today with NO clip on disk.
        insert(
            "2026-06-10",
            "07:00:00",
            "Erithacus rubecula",
            "Robin",
            0.82,
            "r1.wav",
        );
        insert(
            "2026-06-11",
            "06:00:00",
            "Erithacus rubecula",
            "Robin",
            0.97,
            "r2.wav",
        );
        insert(
            "2026-06-11",
            "06:10:00",
            "Troglodytes aedon",
            "House Wren",
            0.93,
            "w.wav",
        );
        insert(
            "2026-06-11",
            "06:20:00",
            "Prunella modularis",
            "Dunnock",
            0.60,
            "d.wav",
        );
        insert(
            "2026-06-11",
            "06:30:00",
            "Corvus corone",
            "Carrion Crow",
            0.99,
            "",
        );
        super::super::lock_detection(&conn, "2026-06-11", "06:10:00", "Troglodytes aedon").unwrap();

        let names = |filter: RecordingsFilter| -> Vec<String> {
            recent_clips(&conn, filter, None, 50, 0)
                .unwrap()
                .into_iter()
                .map(|d| d.com_name)
                .collect()
        };
        // The clip-less Crow is excluded from every view, newest clip first.
        let all = names(RecordingsFilter::All);
        assert_eq!(all, vec!["Dunnock", "House Wren", "Robin", "Robin"]);
        assert_eq!(names(RecordingsFilter::Best), vec!["House Wren", "Robin"]);
        assert_eq!(names(RecordingsFilter::Rare), vec!["House Wren"]);
        assert_eq!(names(RecordingsFilter::Locked), vec!["House Wren"]);

        // Search rides the shared include / NOT-exclude grammar.
        assert_eq!(
            recent_clips(&conn, RecordingsFilter::All, Some("Robin"), 50, 0)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            recent_clips(&conn, RecordingsFilter::All, Some("NOT Robin"), 50, 0)
                .unwrap()
                .len(),
            2
        );

        // Count agrees with the listing for every category (and the file gate).
        for f in [
            RecordingsFilter::All,
            RecordingsFilter::Best,
            RecordingsFilter::Rare,
            RecordingsFilter::Locked,
        ] {
            let listed = i64::try_from(names(f).len()).unwrap();
            assert_eq!(
                recent_clips_count(&conn, f, None).unwrap(),
                listed,
                "count mismatch {f:?}"
            );
        }

        // Pagination terminates past the end.
        assert!(
            recent_clips(&conn, RecordingsFilter::All, None, 10, 100)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn recordings_filter_token_round_trips() {
        for f in [
            RecordingsFilter::All,
            RecordingsFilter::Best,
            RecordingsFilter::Rare,
            RecordingsFilter::Locked,
        ] {
            assert_eq!(RecordingsFilter::from_token(Some(f.as_token())), f);
        }
        assert_eq!(
            RecordingsFilter::from_token(Some("bogus")),
            RecordingsFilter::All
        );
        assert_eq!(RecordingsFilter::from_token(None), RecordingsFilter::All);
    }

    #[test]
    fn todays_source_activity_groups_and_orders_by_count() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_or_create(&dir.path().join("t.db")).unwrap();
        let insert = |time: &str, sci: &str, source: Option<&str>| {
            let record = DetectionRecord {
                date: "2026-06-13",
                time,
                sci_name: sci,
                com_name: "Bird",
                confidence: 0.9,
                lat: None,
                lon: None,
                cutoff: None,
                week: None,
                sensitivity: None,
                overlap: None,
                file_name: "c.wav",
                chunk_offset_secs: Some(0.0),
                correlation_id: None,
                source,
                duration_secs: None,
            };
            insert_detection(&conn, &record).unwrap();
        };
        insert("06:00:00", "A", Some("cam1"));
        insert("06:30:00", "B", Some("cam1"));
        insert("07:15:00", "C", Some("cam1"));
        insert("08:00:00", "D", Some("local"));
        insert("05:00:00", "E", None); // pre-tagging row → grouped under NULL

        let rows = todays_source_activity(&conn, "2026-06-13").unwrap();
        // Three groups, busiest first: cam1 (3), then local (1) and NULL (1).
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].source.as_deref(), Some("cam1"));
        assert_eq!(rows[0].count, 3);
        // MAX(Time) is the freshest detection for the source.
        assert_eq!(rows[0].last_time.as_deref(), Some("07:15:00"));
        assert!(rows.iter().any(|r| r.source.is_none() && r.count == 1));
        // A different day sees none of it.
        assert!(
            todays_source_activity(&conn, "2026-06-14")
                .unwrap()
                .is_empty()
        );
    }
}
