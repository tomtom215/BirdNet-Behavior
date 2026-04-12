//! Analytics aggregation queries (hourly, daily, confidence, latest).

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;
use crate::sqlite::types::{DailyCount, HourlyCount};

/// Get detections grouped by hour for a given date.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn hourly_activity(conn: &Connection, date: &str) -> Result<Vec<HourlyCount>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT SUBSTR(Time, 1, 2) as hour, COUNT(*) as count
         FROM detections WHERE Date = ?1
         GROUP BY hour ORDER BY hour",
    )?;
    let rows = stmt
        .query_map(params![date], |row| {
            Ok(HourlyCount {
                hour: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get daily detection counts for the last N days, in chronological order.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn daily_counts(conn: &Connection, days: u32) -> Result<Vec<DailyCount>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, COUNT(*) as count
         FROM detections
         WHERE Date >= DATE('now', '-' || ?1 || ' days')
         GROUP BY Date ORDER BY Date ASC",
    )?;
    let rows = stmt
        .query_map(params![days], |row| {
            Ok(DailyCount {
                date: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get the most recent detection as `(date, time, common_name)`.
///
/// Returns `None` if the detections table is empty.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn latest_detection(conn: &Connection) -> Result<Option<(String, String, String)>, DbError> {
    let result = conn.query_row(
        "SELECT Date, Time, Com_Name FROM detections ORDER BY Date DESC, Time DESC LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );
    match result {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(DbError::Sqlite(e)),
    }
}

/// Return per-bucket detection counts across six confidence ranges.
///
/// Buckets: `[<50%, 50-60%, 60-70%, 70-80%, 80-90%, ≥90%]`.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn confidence_distribution(conn: &Connection) -> Result<[i64; 6], DbError> {
    let mut buckets = [0i64; 6];
    let mut stmt = conn.prepare(
        "SELECT
            CASE
                WHEN Confidence < 0.5 THEN 0
                WHEN Confidence < 0.6 THEN 1
                WHEN Confidence < 0.7 THEN 2
                WHEN Confidence < 0.8 THEN 3
                WHEN Confidence < 0.9 THEN 4
                ELSE 5
            END as bucket,
            COUNT(*) as count
         FROM detections GROUP BY bucket ORDER BY bucket",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (bucket, count) = row?;
        if let Ok(idx) = usize::try_from(bucket)
            && let Some(b) = buckets.get_mut(idx)
        {
            *b = count;
        }
    }
    Ok(buckets)
}

/// Top species by detection count within a date range `[week_start, week_end]`.
///
/// Returns `(sci_name, com_name, count)` tuples, most-detected first.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn weekly_top_species(
    conn: &Connection,
    week_start: &str,
    week_end: &str,
    limit: usize,
) -> Result<Vec<(String, String, i64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Sci_Name, Com_Name, COUNT(*) as cnt
         FROM detections
         WHERE Date >= ?1 AND Date <= ?2
         GROUP BY Sci_Name, Com_Name
         ORDER BY cnt DESC
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(
            params![
                week_start,
                week_end,
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Species detected for the first time within `[week_start, week_end]`.
///
/// Returns `(sci_name, com_name, first_date)` tuples ordered by first detection date.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn weekly_new_species(
    conn: &Connection,
    week_start: &str,
    week_end: &str,
) -> Result<Vec<(String, String, String)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT d.Sci_Name, d.Com_Name, MIN(d.Date) as first_date
         FROM detections d
         WHERE d.Date >= ?1 AND d.Date <= ?2
         GROUP BY d.Sci_Name, d.Com_Name
         HAVING MIN(d.Date) >= ?1
         AND NOT EXISTS (
             SELECT 1 FROM detections e
             WHERE e.Sci_Name = d.Sci_Name AND e.Date < ?1
         )
         ORDER BY first_date ASC",
    )?;
    let rows = stmt
        .query_map(params![week_start, week_end], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Total detection count within `[week_start, week_end]`.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn weekly_detection_count(
    conn: &Connection,
    week_start: &str,
    week_end: &str,
) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE Date >= ?1 AND Date <= ?2",
        params![week_start, week_end],
        |row| row.get(0),
    )
    .map_err(DbError::Sqlite)
}

/// Daily detection counts for a date range `[start, end]` in chronological order.
///
/// Used for weekly trend bars.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn range_daily_counts(
    conn: &Connection,
    start: &str,
    end: &str,
) -> Result<Vec<DailyCount>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, COUNT(*) as count
         FROM detections
         WHERE Date >= ?1 AND Date <= ?2
         GROUP BY Date ORDER BY Date ASC",
    )?;
    let rows = stmt
        .query_map(params![start, end], |row| {
            Ok(DailyCount {
                date: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Distinct dates that have at least one detection, ordered chronologically.
///
/// Used for date navigation in charts.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn distinct_detection_dates(conn: &Connection) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare("SELECT DISTINCT Date FROM detections ORDER BY Date ASC")?;
    let dates = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(dates)
}

// ---------------------------------------------------------------------------
// Data quality queries
// ---------------------------------------------------------------------------

/// Overall detection quality summary statistics.
#[derive(Debug, Clone)]
pub struct QualitySummary {
    /// Total number of detections in the database.
    pub total_detections: i64,
    /// Average confidence across all detections (0.0–1.0).
    pub avg_confidence: f64,
    /// Minimum confidence seen.
    pub min_confidence: f64,
    /// Maximum confidence seen.
    pub max_confidence: f64,
    /// Number of detections with confidence < 0.5 (potential false positives).
    pub low_confidence_count: i64,
    /// Number of distinct species detected.
    pub distinct_species: i64,
    /// Date of earliest detection, or empty string if none.
    pub earliest_date: String,
    /// Date of most recent detection, or empty string if none.
    pub latest_date: String,
}

/// Compute a high-level data quality summary.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn quality_summary(conn: &Connection) -> Result<QualitySummary, DbError> {
    let row = conn.query_row(
        "SELECT
            COUNT(*) as total,
            COALESCE(AVG(Confidence), 0.0) as avg_conf,
            COALESCE(MIN(Confidence), 0.0) as min_conf,
            COALESCE(MAX(Confidence), 0.0) as max_conf,
            SUM(CASE WHEN Confidence < 0.5 THEN 1 ELSE 0 END) as low_count,
            COUNT(DISTINCT Sci_Name) as species,
            COALESCE(MIN(Date), '') as earliest,
            COALESCE(MAX(Date), '') as latest
         FROM detections",
        [],
        |row| {
            Ok(QualitySummary {
                total_detections: row.get(0)?,
                avg_confidence: row.get(1)?,
                min_confidence: row.get(2)?,
                max_confidence: row.get(3)?,
                low_confidence_count: row.get(4)?,
                distinct_species: row.get(5)?,
                earliest_date: row.get(6)?,
                latest_date: row.get(7)?,
            })
        },
    )?;
    Ok(row)
}

/// Species with a high rate of low-confidence detections (potential false positives).
///
/// Returns `(common_name, sci_name, detection_count, avg_confidence)` tuples
/// for species whose average confidence is below `threshold`, sorted by average
/// confidence ascending (worst offenders first).  Only species with at least
/// `min_count` detections are included.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn low_confidence_species(
    conn: &Connection,
    threshold: f64,
    min_count: u32,
) -> Result<Vec<(String, String, i64, f64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Com_Name, Sci_Name, COUNT(*) as cnt, AVG(Confidence) as avg_conf
         FROM detections
         GROUP BY Sci_Name, Com_Name
         HAVING avg_conf < ?1 AND cnt >= ?2
         ORDER BY avg_conf ASC
         LIMIT 20",
    )?;
    let rows = stmt
        .query_map(params![threshold, min_count], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Daily average confidence trend over the last `days` days.
///
/// Returns `(date, avg_confidence)` pairs in chronological order.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn confidence_trend(conn: &Connection, days: u32) -> Result<Vec<(String, f64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, AVG(Confidence) as avg_conf
         FROM detections
         WHERE Date >= DATE('now', '-' || ?1 || ' days')
         GROUP BY Date
         ORDER BY Date ASC",
    )?;
    let rows = stmt
        .query_map(params![days], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Detection count and average confidence broken down by hour-of-day (0–23).
///
/// Returns `(hour, count, avg_confidence)` tuples covering all time.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_quality_by_hour(conn: &Connection) -> Result<Vec<(u8, i64, f64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT CAST(SUBSTR(Time, 1, 2) AS INTEGER) as hour,
                COUNT(*) as cnt,
                AVG(Confidence) as avg_conf
         FROM detections
         GROUP BY hour
         ORDER BY hour ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            let hour: i64 = row.get(0)?;
            Ok((
                u8::try_from(hour.clamp(0, 23)).unwrap_or(0),
                row.get(1)?,
                row.get(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Number of detections in the rolling 60-minute window ending now.
///
/// Concatenates `Date` and `Time` into an ISO-8601 datetime string and compares
/// against `datetime('now', '-1 hour')`, so results are relative to UTC.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn last_hour_count(conn: &Connection) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT COUNT(*) FROM detections
         WHERE datetime(Date || ' ' || Time) >= datetime('now', '-1 hour')",
        [],
        |row| row.get(0),
    )
    .map_err(DbError::Sqlite)
}

/// Per-species, per-hour detection counts for the top `limit` species on `date`.
///
/// Returns `(common_name, hour_0_23, count)` tuples.  Species are ordered by
/// their total detection count for that day (most-detected first); hours are
/// sorted ascending within each species.  Only the top `limit` species appear.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn today_species_hour_heatmap(
    conn: &Connection,
    date: &str,
    limit: u32,
) -> Result<Vec<(String, u8, i64)>, DbError> {
    let mut stmt = conn.prepare(
        "WITH top_sp AS (
            SELECT Com_Name
            FROM detections
            WHERE Date = ?1
            GROUP BY Com_Name
            ORDER BY COUNT(*) DESC
            LIMIT ?2
         )
         SELECT d.Com_Name,
                CAST(SUBSTR(d.Time, 1, 2) AS INTEGER) AS hour,
                COUNT(*) AS cnt
         FROM detections d
         INNER JOIN top_sp ON d.Com_Name = top_sp.Com_Name
         WHERE d.Date = ?1
         GROUP BY d.Com_Name, hour
         ORDER BY d.Com_Name, hour",
    )?;
    let rows = stmt
        .query_map(params![date, i64::from(limit)], |row| {
            let hour: i64 = row.get(1)?;
            Ok((
                row.get::<_, String>(0)?,
                u8::try_from(hour.clamp(0, 23)).unwrap_or(0),
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Most recent detection with full field detail, for the dashboard "Latest" card.
///
/// Returns `None` if the detections table is empty.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn latest_detection_full(
    conn: &Connection,
) -> Result<Option<crate::sqlite::types::DetectionRow>, DbError> {
    use crate::sqlite::types::{DETECTION_COLS, map_detection_row};
    let sql =
        format!("SELECT {DETECTION_COLS} FROM detections ORDER BY Date DESC, Time DESC LIMIT 1");
    let result = conn.query_row(&sql, [], map_detection_row);
    match result {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(DbError::Sqlite(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;
    use rusqlite::params;

    /// Return the ISO-8601 date `n` days before `now`, computed by `SQLite`
    /// so callers and the fixture agree regardless of the host clock.
    fn days_ago(conn: &Connection, n: i64) -> String {
        conn.query_row(&format!("SELECT DATE('now', '-{n} days')"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn temp_db_with_data() -> (tempfile::NamedTempFile, Connection) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        // Dates are computed relative to "now" so the fixture stays inside
        // the 30-day windows used by the analytics queries under test.
        let recent = days_ago(&conn, 6);
        let earlier = days_ago(&conn, 7);
        for (date, time, sci, com, conf) in [
            (
                recent.as_str(),
                "06:30:00",
                "Turdus merula",
                "Eurasian Blackbird",
                0.87,
            ),
            (
                recent.as_str(),
                "06:45:00",
                "Erithacus rubecula",
                "European Robin",
                0.92,
            ),
            (
                recent.as_str(),
                "07:00:00",
                "Turdus merula",
                "Eurasian Blackbird",
                0.75,
            ),
            (
                earlier.as_str(),
                "18:00:00",
                "Parus major",
                "Great Tit",
                0.80,
            ),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES (?1,?2,?3,?4,?5)",
                params![date, time, sci, com, conf],
            ).unwrap();
        }
        (tmp, conn)
    }

    #[test]
    fn hourly_groups_by_hour() {
        let (_tmp, conn) = temp_db_with_data();
        let recent = days_ago(&conn, 6);
        let hours = hourly_activity(&conn, &recent).unwrap();
        assert_eq!(hours.len(), 2);
        assert_eq!(hours[0].hour, "06");
        assert_eq!(hours[0].count, 2);
    }

    #[test]
    fn daily_counts_chronological() {
        let (_tmp, conn) = temp_db_with_data();
        let days = daily_counts(&conn, 365).unwrap();
        assert!(days.len() >= 2);
        if days.len() >= 2 {
            assert!(days[0].date <= days[1].date);
        }
    }

    #[test]
    fn latest_detection_returns_most_recent() {
        let (_tmp, conn) = temp_db_with_data();
        let recent = days_ago(&conn, 6);
        let (date, time, name) = latest_detection(&conn).unwrap().unwrap();
        assert_eq!(date, recent);
        assert_eq!(time, "07:00:00");
        assert_eq!(name, "Eurasian Blackbird");
    }

    #[test]
    fn latest_detection_empty() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        assert!(latest_detection(&conn).unwrap().is_none());
    }

    #[test]
    fn confidence_distribution_buckets() {
        let (_tmp, conn) = temp_db_with_data();
        let buckets = confidence_distribution(&conn).unwrap();
        // 0.75 → bucket 3, 0.80/0.87 → bucket 4, 0.92 → bucket 5
        assert_eq!(buckets[3], 1);
        assert_eq!(buckets[4], 2);
        assert_eq!(buckets[5], 1);
    }

    #[test]
    fn quality_summary_returns_stats() {
        let (_tmp, conn) = temp_db_with_data();
        let qs = quality_summary(&conn).unwrap();
        assert_eq!(qs.total_detections, 4);
        assert!(qs.avg_confidence > 0.0);
        assert!(qs.avg_confidence <= 1.0);
    }

    #[test]
    fn low_confidence_species_empty_when_none_low() {
        let (_tmp, conn) = temp_db_with_data();
        // all detections have conf >= 0.75; threshold 0.5 means none qualify
        let low = low_confidence_species(&conn, 0.5, 5).unwrap();
        assert!(low.is_empty());
    }

    #[test]
    fn confidence_trend_covers_last_30_days() {
        let (_tmp, conn) = temp_db_with_data();
        let trend = confidence_trend(&conn, 30).unwrap();
        // Fixture inserts rows 6–7 days ago, well inside the 30-day window.
        assert!(!trend.is_empty());
        for (_, avg_conf) in &trend {
            assert!(*avg_conf >= 0.0 && *avg_conf <= 1.0);
        }
    }

    #[test]
    fn detection_quality_by_hour_totals_match() {
        let (_tmp, conn) = temp_db_with_data();
        let by_hour = detection_quality_by_hour(&conn).unwrap();
        let total: i64 = by_hour.iter().map(|(_, cnt, _)| cnt).sum();
        // 4 total detections
        assert_eq!(total, 4);
    }

    #[test]
    fn last_hour_count_returns_non_negative() {
        // Historical seed data won't match 'now'; tests the query compiles and runs.
        let (_tmp, conn) = temp_db_with_data();
        let count = last_hour_count(&conn).unwrap();
        assert!(count >= 0);
    }

    #[test]
    fn today_species_hour_heatmap_returns_cells_for_date() {
        let (_tmp, conn) = temp_db_with_data();
        let recent = days_ago(&conn, 6);
        let cells = today_species_hour_heatmap(&conn, &recent, 10).unwrap();
        // Seed: Blackbird at 06 & 07, Robin at 06 → 3 (species, hour) pairs
        assert!(!cells.is_empty());
        for (_, hour, cnt) in &cells {
            assert!(*hour < 24, "hour must be 0–23");
            assert!(*cnt > 0, "count must be positive");
        }
    }

    #[test]
    fn today_species_hour_heatmap_respects_limit() {
        let (_tmp, conn) = temp_db_with_data();
        let recent = days_ago(&conn, 6);
        let cells_limit1 = today_species_hour_heatmap(&conn, &recent, 1).unwrap();
        // Only 1 species (most-detected) × at most 24 hours
        let species: std::collections::HashSet<_> = cells_limit1
            .iter()
            .map(|(name, _, _)| name.clone())
            .collect();
        assert_eq!(
            species.len(),
            1,
            "limit=1 should return exactly one species"
        );
    }

    #[test]
    fn latest_detection_full_returns_most_recent() {
        let (_tmp, conn) = temp_db_with_data();
        let recent = days_ago(&conn, 6);
        let det = latest_detection_full(&conn).unwrap().unwrap();
        assert_eq!(det.date, recent);
        assert_eq!(det.time, "07:00:00");
        assert_eq!(det.com_name, "Eurasian Blackbird");
        assert!(det.confidence > 0.0 && det.confidence <= 1.0);
    }

    #[test]
    fn latest_detection_full_empty_table() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        assert!(latest_detection_full(&conn).unwrap().is_none());
    }
}
