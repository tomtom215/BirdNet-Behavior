//! Species-level aggregation queries.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;
use crate::sqlite::types::{
    DETECTION_COLS, DailyCount, DetectionRow, HourlyCount, SpeciesCount, SpeciesSummary,
    map_detection_row,
};

/// The rollup, when it may be read, and the truth when it may not.
///
/// # Why this exists
///
/// `species_summary` (migration 30) is a materialised rollup keyed
/// `(Com_Name, Sci_Name, hour)` and maintained by triggers, so the species list
/// aggregates a few thousand rows instead of millions and still does in year
/// ten. Its triggers filter on one thing: `review_verdict IS NOT 'rejected'`.
///
/// Migration 34 then gave `detections_analytic` a *second* rule — imported rows
/// are excluded when the operator sets `analytics_exclude_imports` — and the
/// rollup could not learn it. Not by oversight: the rule depends on a setting
/// the operator can flip at any moment, and a trigger-maintained rollup keyed
/// without a provenance dimension cannot answer both questions from the same
/// rows. So the rollup answers "everything not rejected", full stop.
///
/// The result, measured on a station with two of its own detections and three
/// imported, with the setting on: `detections_analytic` reports 1 species and
/// 2 rows, while `species_count` reported 2 and `top_species` ranked the
/// imported species **first**, at 3 detections — a species that station never
/// heard, presented as its commonest bird, after the operator had explicitly
/// asked for it to be excluded. `species_summary()` on the detail page reads
/// the view directly and was right all along, so the list and the detail page
/// disagreed about the same species.
///
/// # What this does
///
/// Returns a `FROM` source with the rollup's exact column shape
/// (`Com_Name, Sci_Name, hour, detections, confidence_sum`) — either the rollup
/// table itself, or an equivalent aggregate over `detections_analytic`, which
/// carries both rules.
///
/// The substitute is used only when the station has imported rows **and** has
/// asked for them to be excluded. Every other station keeps the rollup and its
/// bounded cost; the one that has opted into the exclusion pays migration 30's
/// old scan for correct numbers, which is the right way round. `EXISTS` over
/// `import_batch_id IS NOT NULL` rides the partial index migration 33 built for
/// exactly that predicate, so the extra check is not a scan.
///
/// The lasting fix is a provenance dimension in the rollup's key, so both
/// answers come from it; that is a schema migration and a trigger rewrite, and
/// is recorded in `docs/UNATTENDED_DEPLOYMENT_AUDIT.md` rather than half-built
/// here.
const SUMMARY_FROM_DETECTIONS: &str = "(SELECT Com_Name, Sci_Name, \
     SUBSTR(Time, 1, 2) AS hour, COUNT(*) AS detections, \
     SUM(Confidence) AS confidence_sum \
     FROM detections_analytic GROUP BY Com_Name, Sci_Name, SUBSTR(Time, 1, 2))";

/// Pick the source [`SUMMARY_FROM_DETECTIONS`] documents.
fn summary_source(conn: &Connection) -> Result<&'static str, DbError> {
    let substitute: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM settings
                        WHERE key = 'analytics_exclude_imports' AND value = 'true')
            AND EXISTS(SELECT 1 FROM detections WHERE import_batch_id IS NOT NULL)",
        [],
        |row| row.get(0),
    )?;
    Ok(if substitute {
        SUMMARY_FROM_DETECTIONS
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
             SELECT Com_Name, Sci_Name, SUBSTR(Time, 1, 2) AS hour, COUNT(*) AS n
               FROM detections
              WHERE review_verdict IS NOT 'rejected'
              GROUP BY Com_Name, Sci_Name, SUBSTR(Time, 1, 2)
         )
         SELECT COALESCE(s.Com_Name, t.Com_Name),
                COALESCE(s.Sci_Name, t.Sci_Name),
                COALESCE(s.hour, t.hour),
                COALESCE(s.detections, 0),
                COALESCE(t.n, 0)
           FROM species_summary s
           FULL OUTER JOIN truth t
             ON s.Com_Name = t.Com_Name AND s.Sci_Name = t.Sci_Name AND s.hour = t.hour
          WHERE COALESCE(s.detections, 0) <> COALESCE(t.n, 0)
          ORDER BY 1, 2, 3",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SummaryDrift {
                com_name: row.get(0)?,
                sci_name: row.get(1)?,
                hour: row.get(2)?,
                summary_count: row.get(3)?,
                actual_count: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Recompute `species_summary` from `detections`, discarding what was there.
///
/// The repair half of [`species_summary_drift`]. It is the same statement
/// migration 30 runs to backfill, so a rebuilt summary is indistinguishable
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
        "INSERT INTO species_summary (Com_Name, Sci_Name, hour, detections, confidence_sum)
             SELECT Com_Name, Sci_Name, SUBSTR(Time, 1, 2), COUNT(*), SUM(Confidence)
               FROM detections
              WHERE review_verdict IS NOT 'rejected'
              GROUP BY Com_Name, Sci_Name, SUBSTR(Time, 1, 2)",
        [],
    )?;
    tx.commit()?;
    Ok(n)
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
