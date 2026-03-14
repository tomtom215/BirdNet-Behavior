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
        if let Ok(idx) = usize::try_from(bucket) {
            if let Some(b) = buckets.get_mut(idx) {
                *b = count;
            }
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
        .query_map(params![week_start, week_end, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
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

#[cfg(test)]
mod tests {
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
    fn hourly_groups_by_hour() {
        let (_tmp, conn) = temp_db_with_data();
        let hours = hourly_activity(&conn, "2026-03-11").unwrap();
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
        let (date, time, name) = latest_detection(&conn).unwrap().unwrap();
        assert_eq!(date, "2026-03-11");
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
}
