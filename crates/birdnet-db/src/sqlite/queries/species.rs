//! Species-level aggregation queries.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;
use crate::sqlite::queries::detections::CLIP_AVAILABLE;
use crate::sqlite::types::{
    DETECTION_COLS, DailyCount, DetectionRow, HourlyCount, SpeciesCount, SpeciesSummary,
    map_detection_row,
};

/// The rollup, read whole or read for this station's own rows.
///
/// `species_summary` (migration 30, re-keyed by migration 42) is a
/// materialised rollup maintained by triggers, so the species list aggregates
/// a few thousand rows instead of millions and still does in year ten. Its
/// key carries `is_import` — whether the row came in through an import batch
/// — so the second rule migration 34 gave `detections_analytic` (imported rows
/// are excluded when the operator sets `analytics_exclude_imports`) is a
/// `WHERE` on the rollup rather than a reason to abandon it.
///
/// Before migration 42 the key had no provenance dimension, and this function
/// substituted an aggregate over `detections_analytic` for the stations that
/// both held imported rows and excluded them (RC-3). That made the numbers
/// right by putting exactly the largest stations back onto the whole-history
/// scan migration 30 existed to remove. `a_station_that_excludes_imports_still_reads_the_rollup`
/// holds that both sources are now the rollup.
///
/// Returns a `FROM` source with the rollup's column shape
/// (`Com_Name, Sci_Name, hour, detections, confidence_sum`). Only the exact
/// string `'true'` excludes, which is the same rule the view applies.
const SUMMARY_OWN_ROWS: &str = "(SELECT Com_Name, Sci_Name, hour, detections, confidence_sum \
     FROM species_summary WHERE is_import = 0)";

/// Pick the source [`SUMMARY_OWN_ROWS`] documents.
fn summary_source(conn: &Connection) -> Result<&'static str, DbError> {
    let exclude: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM settings
                        WHERE key = 'analytics_exclude_imports' AND value = 'true')",
        [],
        |row| row.get(0),
    )?;
    Ok(if exclude {
        SUMMARY_OWN_ROWS
    } else {
        "species_summary"
    })
}

/// Get the number of unique species (by scientific name).
///
/// Reads `species_summary`, the per-species aggregate migration 30 maintains
/// on write, rather than counting distinct names across the whole detection
/// history. See [`species_summary_drift`] for how the two are kept honest.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_count(conn: &Connection) -> Result<i64, DbError> {
    let count: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT Sci_Name) FROM {}",
            summary_source(conn)?
        ),
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Get top species by detection count.
///
/// Reads `species_summary` (migration 30), which holds one row per
/// (common name, scientific name, hour) and is maintained by triggers on every
/// write to `detections`. A station with 200 species holds at most 4 800 rows
/// here, so this aggregates thousands of rows instead of millions and keeps
/// doing so in year ten — which the query it replaced did not.
///
/// Ordering breaks ties on common name. The `detections_analytic` aggregate
/// this replaced left ties in whatever order the scan produced, so the species
/// list could reorder between two loads that returned the same numbers.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn top_species(conn: &Connection, limit: u32) -> Result<Vec<SpeciesCount>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT Com_Name, Sci_Name, SUM(detections) as count,
                SUM(confidence_sum) / SUM(detections) as avg_conf
         FROM {} GROUP BY Com_Name, Sci_Name
         ORDER BY count DESC, Com_Name ASC LIMIT ?1",
        summary_source(conn)?
    ))?;
    let rows = stmt
        .query_map(params![limit], |row| {
            Ok(SpeciesCount {
                com_name: row.get(0)?,
                sci_name: row.get(1)?,
                count: row.get(2)?,
                avg_confidence: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Search species by name (case-insensitive substring match on common or scientific name).
///
/// Reads the maintained `species_summary`, like [`top_species`], and breaks
/// ties on common name for the same reason.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn search_species(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<SpeciesCount>, DbError> {
    let pattern = format!("%{query}%");
    let mut stmt = conn.prepare(&format!(
        "SELECT Com_Name, Sci_Name, SUM(detections) as count,
                SUM(confidence_sum) / SUM(detections) as avg_conf
         FROM {}
         WHERE Com_Name LIKE ?1 COLLATE NOCASE OR Sci_Name LIKE ?1 COLLATE NOCASE
         GROUP BY Com_Name, Sci_Name ORDER BY count DESC, Com_Name ASC LIMIT ?2",
        summary_source(conn)?
    ))?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| {
            Ok(SpeciesCount {
                com_name: row.get(0)?,
                sci_name: row.get(1)?,
                count: row.get(2)?,
                avg_confidence: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get species summary (count, avg confidence, first/last seen) by common name.
///
/// Returns `None` if no detections exist for the species.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_summary(
    conn: &Connection,
    com_name: &str,
) -> Result<Option<SpeciesSummary>, DbError> {
    let result = conn.query_row(
        "SELECT Com_Name, Sci_Name, COUNT(*) as count,
                AVG(Confidence) as avg_conf,
                MIN(Date) as first_seen,
                MAX(Date) as last_seen
         FROM detections_analytic WHERE Com_Name = ?1 GROUP BY Com_Name",
        params![com_name],
        |row| {
            Ok(SpeciesSummary {
                com_name: row.get(0)?,
                sci_name: row.get(1)?,
                count: row.get(2)?,
                avg_confidence: row.get(3)?,
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        },
    );
    match result {
        Ok(summary) => Ok(Some(summary)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(DbError::Sqlite(e)),
    }
}

/// Get daily detection counts for a specific species (most recent `days` dates).
///
/// Returns rows in chronological order.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_daily_counts(
    conn: &Connection,
    com_name: &str,
    days: u32,
) -> Result<Vec<DailyCount>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, COUNT(*) as count
         FROM detections_analytic WHERE Com_Name = ?1
         GROUP BY Date ORDER BY Date DESC LIMIT ?2",
    )?;
    let mut rows: Vec<DailyCount> = stmt
        .query_map(params![com_name, days], |row| {
            Ok(DailyCount {
                date: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.reverse(); // chronological order
    Ok(rows)
}

/// Get hourly activity for a specific species (across all dates).
///
/// Reads `species_summary`, which is already grouped by hour-of-day, so this
/// is a lookup of at most 24 rows rather than a scan of every detection of the
/// species. The `hour` key is `SUBSTR(Time, 1, 2)` stored verbatim — including
/// for a malformed imported timestamp, which lands in its own bucket exactly as
/// the aggregate this replaced reported it.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_hourly_activity(
    conn: &Connection,
    com_name: &str,
) -> Result<Vec<HourlyCount>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT hour, SUM(detections) as count
         FROM {} WHERE Com_Name = ?1
         GROUP BY hour ORDER BY hour",
        summary_source(conn)?
    ))?;
    let rows = stmt
        .query_map(params![com_name], |row| {
            Ok(HourlyCount {
                hour: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Hourly activity (0–23 buckets) for several species in a single grouped scan.
///
/// Replaces N per-species [`species_hourly_activity`] calls: the dawn-chorus
/// polar needs the top handful of species and previously ran one full table
/// scan per species (an N+1). Returns a map from common name to its 24-hour
/// histogram; a species with no detections is simply absent from the map.
///
/// Also reads the maintained `species_summary`. The hour is `CAST` to an
/// integer here and out-of-range values dropped, which is what this function
/// has always done and is why a malformed imported timestamp cannot land in the
/// array — [`species_hourly_activity`] reports that bucket instead.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_hourly_activity_batch(
    conn: &Connection,
    com_names: &[String],
) -> Result<std::collections::HashMap<String, [i64; 24]>, DbError> {
    let mut out: std::collections::HashMap<String, [i64; 24]> = std::collections::HashMap::new();
    if com_names.is_empty() {
        return Ok(out);
    }
    // One bind placeholder per species for the `IN (…)` list.
    let placeholders = vec!["?"; com_names.len()].join(",");
    let source = summary_source(conn)?;
    let sql = format!(
        "SELECT Com_Name, CAST(hour AS INTEGER) AS h, SUM(detections) AS cnt
         FROM {source}
         WHERE Com_Name IN ({placeholders})
         GROUP BY Com_Name, h"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(com_names.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (name, hour, cnt) = row?;
        if let Ok(idx) = usize::try_from(hour)
            && idx < 24
        {
            out.entry(name).or_insert_with(|| [0_i64; 24])[idx] = cnt;
        }
    }
    Ok(out)
}

/// Query recent detections for a specific species by common name.
///
/// Alias for `crate::sqlite::queries::detections::detections_by_species`
/// provided here for ergonomic use in species-level handlers.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn recent_by_species(
    conn: &Connection,
    com_name: &str,
    limit: u32,
) -> Result<Vec<DetectionRow>, DbError> {
    let sql = format!(
        "SELECT {DETECTION_COLS} FROM detections_analytic \
         WHERE Com_Name = ?1 ORDER BY Date DESC, Time DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![com_name, limit], map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get 7-day sparkline data for all species (daily counts per common name).
///
/// Returns a map from common name to a vector of 7 daily counts (oldest first).
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_sparklines(
    conn: &Connection,
    days: u32,
) -> Result<std::collections::HashMap<String, Vec<i64>>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Com_Name, Date, COUNT(*) as count
         FROM detections_analytic
         WHERE Date >= date('now', 'localtime', '-' || ?1 || ' days')
         GROUP BY Com_Name, Date
         ORDER BY Com_Name, Date",
    )?;

    let mut map: std::collections::HashMap<String, Vec<(String, i64)>> =
        std::collections::HashMap::new();
    let rows = stmt.query_map(params![days], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    for row in rows {
        let (name, date, count) = row?;
        map.entry(name).or_default().push((date, count));
    }

    // Build date list for the last N days.
    let mut date_set: Vec<String> = Vec::new();
    let mut date_stmt = conn.prepare(
        "WITH RECURSIVE dates(d) AS (
             SELECT date('now', 'localtime', '-' || (?1 - 1) || ' days')
             UNION ALL
             SELECT date(d, '+1 day') FROM dates WHERE d < date('now', 'localtime')
         ) SELECT d FROM dates",
    )?;
    let date_rows = date_stmt.query_map(params![days], |row| row.get::<_, String>(0))?;
    for d in date_rows {
        date_set.push(d?);
    }

    // Normalize: fill in zeros for missing dates.
    let mut result: std::collections::HashMap<String, Vec<i64>> = std::collections::HashMap::new();
    for (name, counts) in &map {
        let count_map: std::collections::HashMap<&str, i64> =
            counts.iter().map(|(d, c)| (d.as_str(), *c)).collect();
        let sparkline: Vec<i64> = date_set
            .iter()
            .map(|d| count_map.get(d.as_str()).copied().unwrap_or(0))
            .collect();
        result.insert(name.clone(), sparkline);
    }

    Ok(result)
}

/// First-ever detection *instant* per scientific name, as `"YYYY-MM-DD HH:MM:SS"`.
///
/// [`species_first_seen`] returns only a date, which is all the life list and
/// the year-in-review need. The "first ever" badge needs more: with a date
/// alone the only question answerable is "was this species new on this day?",
/// which is true of *every* detection of it that day — so a station that heard
/// 133 blackcaps today badged all 133 as the first. Comparing a row's own
/// `Date`+`Time` against this value marks exactly the one detection that was.
///
/// `MIN(Date || ' ' || Time)` is safe here because both columns are stored
/// zero-padded (`YYYY-MM-DD`, `HH:MM:SS`), so lexicographic order over the
/// concatenation is chronological order.
///
/// # Errors
///
/// Returns [`DbError`] if the query fails.
pub fn species_first_detection(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, String>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Sci_Name, MIN(Date || ' ' || Time) FROM detections_analytic GROUP BY Sci_Name",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<std::collections::HashMap<String, String>, _>>()?;
    Ok(rows)
}

/// Get the first-seen *date* for each species (by scientific name).
///
/// Returns a map from scientific name to its first detection date. For marking
/// the single detection that was a species' first, use
/// [`species_first_detection`] — a date cannot distinguish it from every other
/// detection that same day.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_first_seen(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, String>, DbError> {
    let mut stmt =
        conn.prepare("SELECT Sci_Name, MIN(Date) FROM detections_analytic GROUP BY Sci_Name")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<std::collections::HashMap<String, String>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// The maintained summary: verifying it, and repairing it
// ---------------------------------------------------------------------------

/// One (species, hour) bucket where `species_summary` disagrees with
/// `detections`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryDrift {
    /// Common name of the bucket that disagrees.
    pub com_name: String,
    /// Scientific name of the bucket that disagrees.
    pub sci_name: String,
    /// Hour-of-day key, as stored (`SUBSTR(Time, 1, 2)`).
    pub hour: String,
    /// Provenance key: `true` for rows that came in through an import batch.
    pub is_import: bool,
    /// What `species_summary` claims the count is.
    pub summary_count: i64,
    /// What counting `detections` directly says it is.
    pub actual_count: i64,
}

/// Compare the maintained summary against the detections it summarises.
///
/// A materialised aggregate that can drift is worse than a slow query, because
/// nothing about a wrong number looks wrong. Migration 30 maintains
/// `species_summary` with triggers precisely so no write path can bypass it —
/// but "no path can bypass it" is a claim, and this is how the claim is
/// checked rather than believed.
///
/// This costs a full aggregate over `detections`: the same work the summary
/// exists to avoid. It is therefore not something a page load may call. It is
/// for `--doctor` and for the daily maintenance job, which run it once and act
/// on the answer.
///
/// Returns one entry per disagreeing bucket, including buckets present on only
/// one side. An empty vector means the two agree exactly.
///
/// Only counts are compared, not `confidence_sum`. The trigger maintains the
/// sum by repeated addition and `SUM()` adds in scan order, so the two can
/// differ in the last bits of a float for reasons that are not drift; the
/// count is an integer and cannot.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_summary_drift(conn: &Connection) -> Result<Vec<SummaryDrift>, DbError> {
    let mut stmt = conn.prepare(
        "WITH truth AS (
             SELECT Com_Name, Sci_Name, SUBSTR(Time, 1, 2) AS hour,
                    (import_batch_id IS NOT NULL) AS is_import, COUNT(*) AS n
               FROM detections
              WHERE review_verdict IS NOT 'rejected'
              GROUP BY Com_Name, Sci_Name, SUBSTR(Time, 1, 2), (import_batch_id IS NOT NULL)
         )
         SELECT COALESCE(s.Com_Name, t.Com_Name),
                COALESCE(s.Sci_Name, t.Sci_Name),
                COALESCE(s.hour, t.hour),
                COALESCE(s.is_import, t.is_import),
                COALESCE(s.detections, 0),
                COALESCE(t.n, 0)
           FROM species_summary s
           FULL OUTER JOIN truth t
             ON s.Com_Name = t.Com_Name AND s.Sci_Name = t.Sci_Name
            AND s.hour = t.hour AND s.is_import = t.is_import
          WHERE COALESCE(s.detections, 0) <> COALESCE(t.n, 0)
          ORDER BY 1, 2, 3, 4",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SummaryDrift {
                com_name: row.get(0)?,
                sci_name: row.get(1)?,
                hour: row.get(2)?,
                is_import: row.get(3)?,
                summary_count: row.get(4)?,
                actual_count: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Recompute `species_summary` from `detections`, discarding what was there.
///
/// The repair half of [`species_summary_drift`]. It is the same statement
/// migration 42 runs to backfill, so a rebuilt summary is indistinguishable
/// from a freshly migrated one.
///
/// Returns the number of buckets written.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn rebuild_species_summary(conn: &Connection) -> Result<usize, DbError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM species_summary", [])?;
    let n = tx.execute(
        "INSERT INTO species_summary (Com_Name, Sci_Name, hour, is_import, detections, confidence_sum)
             SELECT Com_Name, Sci_Name, SUBSTR(Time, 1, 2),
                    (import_batch_id IS NOT NULL), COUNT(*), SUM(Confidence)
               FROM detections
              WHERE review_verdict IS NOT 'rejected'
              GROUP BY Com_Name, Sci_Name, SUBSTR(Time, 1, 2), (import_batch_id IS NOT NULL)",
        [],
    )?;
    tx.commit()?;
    Ok(n)
}

// ---------------------------------------------------------------------------
// The station's per-species footprint, and the bulk actions over it (`N-4`)
// ---------------------------------------------------------------------------

/// One species' footprint on this station.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeciesUsage {
    /// Common name, as the classifier labels it.
    pub com_name: String,
    /// Scientific name — the key every list and threshold is stored under.
    pub sci_name: String,
    /// Detections that count: rejected reviews excluded, the way every other
    /// number an operator is shown excludes them.
    pub detections: i64,
    /// How many still have a playable clip on disk.
    pub clips: i64,
    /// How many are locked. Locked rows are never touched by the bulk actions,
    /// so this is what an operator needs to see *before* pressing one —
    /// otherwise the count that comes back is a surprise.
    pub locked: i64,
    /// The most recent date it was detected, `YYYY-MM-DD`.
    pub last_seen: String,
}

/// Every species this station has recorded, with its footprint.
///
/// Ordered by detection count descending: the species costing the most is the
/// one the operator came to this page about.
///
/// Rejected detections are excluded from `detections`, so the number agrees
/// with every other species count in the product. They are **not** excluded
/// from `clips`: a rejected detection's clip is still a file taking up space,
/// and a page about disk usage that hid it would answer a different question
/// from the one being asked.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_disk_usage(conn: &Connection) -> Result<Vec<SpeciesUsage>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT Com_Name, Sci_Name,
                SUM(CASE WHEN review_verdict IS NOT 'rejected' THEN 1 ELSE 0 END),
                SUM(CASE WHEN {CLIP_AVAILABLE} THEN 1 ELSE 0 END),
                SUM(CASE WHEN is_locked = 1 THEN 1 ELSE 0 END),
                MAX(Date)
           FROM detections
          GROUP BY Sci_Name, Com_Name
          ORDER BY 3 DESC, Com_Name ASC"
    ))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SpeciesUsage {
                com_name: row.get(0)?,
                sci_name: row.get(1)?,
                detections: row.get(2)?,
                clips: row.get(3)?,
                locked: row.get(4)?,
                last_seen: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The clip files one species still has on disk, relative to the recordings
/// directory.
///
/// Locked detections are excluded: their clips are the evidence the lock
/// exists to protect, and the only caller is the bulk clip removal.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_clip_files(conn: &Connection, sci_name: &str) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT File_Name FROM detections
          WHERE Sci_Name = ?1 AND is_locked = 0 AND {CLIP_AVAILABLE}"
    ))?;
    let rows = stmt
        .query_map(params![sci_name], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// What one bulk action did, and what it deliberately left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BulkOutcome {
    /// Rows changed.
    pub affected: usize,
    /// Rows skipped because the operator had locked them.
    ///
    /// Reported rather than swallowed. A bulk action is issued against a
    /// species, not against rows the operator is looking at, so "3 998 of your
    /// 4 000, and the two you locked are still here" is the answer — and an
    /// operator not told would later find detections of a species they believe
    /// they removed.
    pub locked_skipped: usize,
}

/// Delete every **unlocked** detection of one species.
///
/// Locked rows survive. [`crate::sqlite::delete_detection`] does not check the
/// lock, because there the operator is looking at the row they named; a bulk
/// delete is issued against a species and will sweep up rows nobody is
/// thinking about.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn delete_species_detections(
    conn: &Connection,
    sci_name: &str,
) -> Result<BulkOutcome, DbError> {
    let locked: i64 = conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE Sci_Name = ?1 AND is_locked = 1",
        params![sci_name],
        |row| row.get(0),
    )?;
    let affected = conn.execute(
        "DELETE FROM detections WHERE Sci_Name = ?1 AND is_locked = 0",
        params![sci_name],
    )?;
    Ok(BulkOutcome {
        affected,
        locked_skipped: usize::try_from(locked).unwrap_or(usize::MAX),
    })
}

/// Mark every **unlocked** clip of one species as reclaimed.
///
/// The row and its `File_Name` are kept — see migration 22: the name records
/// that audio existed and what it was called, which is provenance an analysis
/// may need long after the space was recovered. Only `Clip_Pruned_At` is set.
///
/// Marked **before** the files are removed, deliberately. A row marked pruned
/// whose file still exists wastes disk; a file removed without the mark offers
/// an operator a player for audio that is gone. The first is the cheaper thing
/// to be wrong about.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn prune_species_clips(
    conn: &Connection,
    sci_name: &str,
    now_unix: i64,
) -> Result<BulkOutcome, DbError> {
    let locked: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM detections
              WHERE Sci_Name = ?1 AND is_locked = 1 AND {CLIP_AVAILABLE}"
        ),
        params![sci_name],
        |row| row.get(0),
    )?;
    let affected = conn.execute(
        &format!(
            "UPDATE detections SET Clip_Pruned_At = ?2
              WHERE Sci_Name = ?1 AND is_locked = 0 AND {CLIP_AVAILABLE}"
        ),
        params![sci_name, now_unix],
    )?;
    Ok(BulkOutcome {
        affected,
        locked_skipped: usize::try_from(locked).unwrap_or(usize::MAX),
    })
}

// ---------------------------------------------------------------------------
// Per-species confidence thresholds
// ---------------------------------------------------------------------------

/// A per-species confidence threshold override.
#[derive(Debug, Clone)]
pub struct SpeciesThreshold {
    /// Scientific name of the species.
    pub sci_name: String,
    /// Custom confidence threshold (0.0–1.0).
    pub confidence_threshold: f64,
    /// When this threshold was created.
    pub created_at: String,
}

/// Get all per-species confidence thresholds.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn get_species_thresholds(conn: &Connection) -> Result<Vec<SpeciesThreshold>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT sci_name, confidence_threshold, created_at FROM species_thresholds ORDER BY sci_name",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SpeciesThreshold {
                sci_name: row.get(0)?,
                confidence_threshold: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get all per-species confidence thresholds as a map (`sci_name` → threshold).
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn get_species_threshold_map(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, f64>, DbError> {
    let mut stmt = conn.prepare("SELECT sci_name, confidence_threshold FROM species_thresholds")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?
        .collect::<Result<std::collections::HashMap<String, f64>, _>>()?;
    Ok(rows)
}

/// Set a per-species confidence threshold (upsert).
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn set_species_threshold(
    conn: &Connection,
    sci_name: &str,
    threshold: f64,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO species_thresholds (sci_name, confidence_threshold) VALUES (?1, ?2)
         ON CONFLICT(sci_name) DO UPDATE SET confidence_threshold = ?2",
        params![sci_name, threshold],
    )?;
    Ok(())
}

/// Remove a per-species confidence threshold.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn delete_species_threshold(conn: &Connection, sci_name: &str) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM species_thresholds WHERE sci_name = ?1",
        params![sci_name],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The whole-history species aggregates must be served by a covering index.
    ///
    /// These three run on every load of the species list, the life list and a
    /// species page, over the *entire* detection history with no time bound —
    /// so their cost grows with how long the station has been running, which is
    /// exactly backwards for a multi-year deployment. On a seeded three-year
    /// station (2 755 374 detections, 1.43 GB) they took 4.96 s, 4.12 s and
    /// 4.82 s before migration 29's covering indexes and 1.31 s, 0.58 s and
    /// 1.15 s after.
    ///
    /// A timing assertion would be flaky, so this pins the mechanism instead:
    /// SQLite must report a COVERING INDEX, which is the thing that stops the
    /// plan going back to the table row by row. Dropping either index, or
    /// removing a column from one, turns the plan back into a plain
    /// `SCAN … USING INDEX` and fails here.
    #[test]
    fn the_whole_history_species_aggregates_are_index_only() {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        let plan = |sql: &str| -> String {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let rows: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows.join(" | ")
        };
        for (name, sql) in [
            (
                "species list",
                "SELECT Com_Name, Sci_Name, COUNT(*) c, AVG(Confidence) \
                 FROM detections_analytic GROUP BY Com_Name, Sci_Name ORDER BY c DESC LIMIT 200",
            ),
            (
                "life-list firsts",
                "SELECT Sci_Name, MIN(Date || ' ' || Time) FROM detections_analytic \
                 GROUP BY Sci_Name",
            ),
            (
                "per-species hour histogram",
                "SELECT Com_Name, CAST(SUBSTR(Time, 1, 2) AS INTEGER) h, COUNT(*) \
                 FROM detections_analytic GROUP BY Com_Name, h",
            ),
        ] {
            let p = plan(sql);
            assert!(
                p.contains("COVERING INDEX"),
                "the {name} aggregate must be index-only; plan was: {p}"
            );
        }
    }

    use super::*;
    use crate::sqlite::connection::open_or_create;
    use rusqlite::params;

    fn temp_db_with_data() -> (tempfile::NamedTempFile, Connection) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        for (date, time, sci, com, conf) in [
            (
                "2026-03-11",
                "06:30:00",
                "Turdus merula",
                "Eurasian Blackbird",
                0.87,
            ),
            (
                "2026-03-11",
                "06:45:00",
                "Erithacus rubecula",
                "European Robin",
                0.92,
            ),
            (
                "2026-03-11",
                "07:00:00",
                "Turdus merula",
                "Eurasian Blackbird",
                0.75,
            ),
            ("2026-03-10", "18:00:00", "Parus major", "Great Tit", 0.80),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES (?1,?2,?3,?4,?5)",
                params![date, time, sci, com, conf],
            ).unwrap();
        }
        (tmp, conn)
    }

    #[test]
    fn species_count_distinct() {
        let (_tmp, conn) = temp_db_with_data();
        assert_eq!(species_count(&conn).unwrap(), 3);
    }

    #[test]
    fn top_species_ordered_by_count() {
        let (_tmp, conn) = temp_db_with_data();
        let species = top_species(&conn, 10).unwrap();
        assert_eq!(species.len(), 3);
        assert_eq!(species[0].com_name, "Eurasian Blackbird");
        assert_eq!(species[0].count, 2);
    }

    #[test]
    fn search_species_by_common_name() {
        let (_tmp, conn) = temp_db_with_data();
        let results = search_species(&conn, "blackbird", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].com_name, "Eurasian Blackbird");
    }

    #[test]
    fn search_species_by_scientific_name() {
        let (_tmp, conn) = temp_db_with_data();
        let results = search_species(&conn, "Turdus", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].sci_name, "Turdus merula");
    }

    #[test]
    fn search_species_case_insensitive() {
        let (_tmp, conn) = temp_db_with_data();
        let results = search_species(&conn, "ROBIN", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn species_summary_found() {
        let (_tmp, conn) = temp_db_with_data();
        let s = species_summary(&conn, "Eurasian Blackbird")
            .unwrap()
            .unwrap();
        assert_eq!(s.count, 2);
        assert!((s.avg_confidence - 0.81).abs() < 0.01);
    }

    #[test]
    fn species_summary_not_found() {
        let (_tmp, conn) = temp_db_with_data();
        assert!(species_summary(&conn, "Flamingo").unwrap().is_none());
    }

    #[test]
    fn species_daily_counts_chronological() {
        let (_tmp, conn) = temp_db_with_data();
        let days = species_daily_counts(&conn, "Eurasian Blackbird", 7).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].count, 2);
    }

    #[test]
    fn species_hourly_activity_groups_correctly() {
        let (_tmp, conn) = temp_db_with_data();
        let hours = species_hourly_activity(&conn, "Eurasian Blackbird").unwrap();
        assert_eq!(hours.len(), 2);
        assert_eq!(hours[0].hour, "06");
        assert_eq!(hours[1].hour, "07");
    }
}

#[cfg(test)]
mod footprint_tests {
    use super::{
        BulkOutcome, delete_species_detections, prune_species_clips, species_clip_files,
        species_disk_usage,
    };
    use rusqlite::Connection;

    /// A station with three species: a resident, a phantom with many clips,
    /// and one whose only detection the operator has locked.
    fn station() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::migration::migrate(&conn).expect("migrate");
        let add = |date: &str,
                   time: &str,
                   sci: &str,
                   com: &str,
                   file: Option<&str>,
                   locked: i64,
                   verdict: Option<&str>| {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                      File_Name, chunk_offset_secs, is_locked, review_verdict)
                 VALUES (?1, ?2, ?3, ?4, 0.9, 0.7, 36, 1.25, 0.0, ?5, 0, ?6, ?7)",
                rusqlite::params![date, time, sci, com, file, locked, verdict],
            )
            .expect("insert");
        };
        add(
            "2026-09-01",
            "06:00:00",
            "Turdus merula",
            "Blackbird",
            Some("a.wav"),
            0,
            None,
        );
        add(
            "2026-09-02",
            "06:00:00",
            "Turdus merula",
            "Blackbird",
            Some("b.wav"),
            0,
            None,
        );
        // The phantom. Deliberately MORE detections than the blackbird, and a
        // name that sorts after it, so "worst first" and "alphabetical" give
        // different answers. The first version of this fixture had the two tie,
        // which let a mutation replacing the ordering pass unnoticed.
        add(
            "2026-09-03",
            "07:00:00",
            "Phantomus fictus",
            "Not A Bird",
            Some("p1.wav"),
            0,
            None,
        );
        add(
            "2026-09-04",
            "07:00:00",
            "Phantomus fictus",
            "Not A Bird",
            Some("p2.wav"),
            0,
            None,
        );
        // One with no clip at all: a detection can exist without audio.
        add(
            "2026-09-04",
            "07:30:00",
            "Phantomus fictus",
            "Not A Bird",
            None,
            0,
            None,
        );
        add(
            "2026-09-05",
            "07:00:00",
            "Phantomus fictus",
            "Not A Bird",
            Some("p3.wav"),
            0,
            Some("rejected"),
        );
        // The locked one.
        add(
            "2026-09-06",
            "08:00:00",
            "Rara avis",
            "Rare Bird",
            Some("r.wav"),
            1,
            None,
        );
        conn
    }

    /// The table an operator reads before acting, worst first — the species
    /// costing the most is the one they came to this page about.
    ///
    /// The fixture makes the phantom the most-detected species *and* gives it
    /// a name sorting after the blackbird, so count order and alphabetical
    /// order disagree. An earlier fixture had them tie, and a mutation
    /// replacing the ordering with `ORDER BY Com_Name` passed against it: the
    /// gate was green for a reason that had nothing to do with its name.
    ///
    /// Observed failing, after that fix, with the ordering changed to
    /// `ORDER BY Com_Name ASC` — the blackbird came first.
    #[test]
    fn the_footprint_lists_every_species_worst_first() {
        let rows = species_disk_usage(&station()).expect("usage");
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(
            rows[0].com_name, "Not A Bird",
            "the most-detected species comes first, not the first alphabetically: {rows:?}"
        );
        assert_eq!(
            rows[0].detections, 3,
            "one of its four rows is a rejected review and must not count"
        );
        assert_eq!(
            rows[0].clips, 3,
            "a rejected detection's clip is still a file; the clipless row has none"
        );
        assert_eq!(rows[0].last_seen, "2026-09-05");

        let blackbird = rows
            .iter()
            .find(|r| r.sci_name == "Turdus merula")
            .expect("b");
        assert_eq!(blackbird.detections, 2);
        assert_eq!(blackbird.clips, 2);
        assert_eq!(blackbird.last_seen, "2026-09-02");
    }

    /// The locked count is shown *before* the operator acts, because it is
    /// what makes the number that comes back afterwards unsurprising.
    #[test]
    fn a_locked_detection_is_counted_and_shown() {
        let rows = species_disk_usage(&station()).expect("usage");
        let rare = rows.iter().find(|r| r.sci_name == "Rara avis").expect("r");
        assert_eq!(rare.locked, 1);
        assert_eq!(rare.detections, 1);
    }

    /// **The safety gate.** A bulk delete is issued against a species, not
    /// against rows the operator is looking at, so it must not sweep up one
    /// they deliberately locked.
    ///
    /// Observed failing with `is_locked = 0` removed from the DELETE: the
    /// locked row was gone and the surviving-row assertion went red.
    #[test]
    fn a_bulk_delete_leaves_locked_detections_alone_and_says_so() {
        let conn = station();
        let outcome = delete_species_detections(&conn, "Rara avis").expect("delete");
        assert_eq!(
            outcome,
            BulkOutcome {
                affected: 0,
                locked_skipped: 1
            },
            "the only row is locked, so nothing is deleted and the operator is told why"
        );
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Sci_Name = 'Rara avis'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(left, 1, "a locked detection survived a bulk delete");
    }

    /// The counterpart: unlocked rows really are deleted, or the gate above
    /// would pass against a delete that does nothing at all.
    #[test]
    fn a_bulk_delete_removes_the_unlocked_rows() {
        let conn = station();
        let outcome = delete_species_detections(&conn, "Phantomus fictus").expect("delete");
        assert_eq!(
            outcome.affected, 4,
            "including the rejected review and the row with no clip"
        );
        assert_eq!(outcome.locked_skipped, 0);
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Sci_Name = 'Phantomus fictus'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(left, 0);
        // And it touched nothing else.
        let others: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .expect("count");
        assert_eq!(others, 3, "a species delete must not reach other species");
    }

    /// Pruning clips keeps the row and the filename — migration 22's point —
    /// and only sets the reclaim stamp.
    ///
    /// Observed failing with the UPDATE replaced by a DELETE: the row count
    /// dropped and the surviving-name assertion went red.
    #[test]
    fn pruning_clips_keeps_the_row_and_the_name() {
        let conn = station();
        let outcome = prune_species_clips(&conn, "Phantomus fictus", 1_800_000_000).expect("prune");
        assert_eq!(outcome.affected, 3);

        let (rows, named, pruned): (i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*),
                        SUM(CASE WHEN File_Name IS NOT NULL THEN 1 ELSE 0 END),
                        SUM(CASE WHEN Clip_Pruned_At IS NOT NULL THEN 1 ELSE 0 END)
                   FROM detections WHERE Sci_Name = 'Phantomus fictus'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("row");
        assert_eq!(rows, 4, "the detections themselves are not deleted");
        assert_eq!(named, 3, "the filename is provenance and is kept");
        assert_eq!(pruned, 3, "every clip is marked; the clipless row has none");

        // And it is idempotent: a second pass finds nothing left to do.
        let again = prune_species_clips(&conn, "Phantomus fictus", 1_800_000_001).expect("again");
        assert_eq!(again.affected, 0);
    }

    /// A locked detection's clip is the evidence the lock protects, so it is
    /// neither listed for removal nor marked.
    #[test]
    fn a_locked_clip_is_never_listed_or_pruned() {
        let conn = station();
        assert!(
            species_clip_files(&conn, "Rara avis")
                .expect("files")
                .is_empty(),
            "a locked clip must not be offered for deletion"
        );
        let outcome = prune_species_clips(&conn, "Rara avis", 1_800_000_000).expect("prune");
        assert_eq!(outcome.affected, 0);
        assert_eq!(outcome.locked_skipped, 1);
    }

    /// The files handed to the caller are exactly the unlocked, still-present
    /// clips — no more, so nothing outside the species is ever unlinked.
    #[test]
    fn the_clip_list_is_exactly_this_species_unlocked_files() {
        let conn = station();
        let mut files = species_clip_files(&conn, "Phantomus fictus").expect("files");
        files.sort();
        assert_eq!(files, ["p1.wav", "p2.wav", "p3.wav"]);

        // After pruning they are gone from the list: `CLIP_AVAILABLE` is false
        // once the stamp is set, so a second removal pass unlinks nothing.
        prune_species_clips(&conn, "Phantomus fictus", 1_800_000_000).expect("prune");
        assert!(
            species_clip_files(&conn, "Phantomus fictus")
                .expect("f")
                .is_empty()
        );
    }
}
