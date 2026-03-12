//! `SQLite` operational database.
//!
//! Provides connection management, WAL mode enforcement, and query helpers
//! for the birds.db detection database.

use rusqlite::{Connection, params};
use std::fmt;
use std::path::Path;

/// Database errors.
#[derive(Debug)]
pub enum DbError {
    /// `SQLite` error.
    Sqlite(rusqlite::Error),
    /// Database file not found.
    NotFound(String),
    /// Schema validation failed.
    Schema(String),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::NotFound(path) => write!(f, "database not found: {path}"),
            Self::Schema(msg) => write!(f, "schema error: {msg}"),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::NotFound(_) | Self::Schema(_) => None,
        }
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// Open a `SQLite` connection with WAL mode and recommended PRAGMAs.
///
/// # Errors
///
/// Returns `DbError` if the database cannot be opened or WAL mode cannot be set.
pub fn open_connection(path: &Path) -> Result<Connection, DbError> {
    if !path.exists() {
        return Err(DbError::NotFound(path.display().to_string()));
    }

    let conn = Connection::open(path)?;

    // Enforce WAL mode for crash resilience
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;
         PRAGMA cache_size=-2000;
         PRAGMA foreign_keys=ON;",
    )?;

    Ok(conn)
}

/// Open or create a `SQLite` database with the detections schema.
///
/// # Errors
///
/// Returns `DbError` on connection or schema creation failure.
pub fn open_or_create(path: &Path) -> Result<Connection, DbError> {
    let conn = Connection::open(path)?;

    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;
         PRAGMA cache_size=-2000;
         PRAGMA foreign_keys=ON;",
    )?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS detections (
            Date TEXT NOT NULL,
            Time TEXT NOT NULL,
            Sci_Name TEXT NOT NULL,
            Com_Name TEXT NOT NULL,
            Confidence REAL NOT NULL,
            Lat REAL,
            Lon REAL,
            Cutoff REAL,
            Week INTEGER,
            Sens REAL,
            Overlap REAL,
            File_Name TEXT
        );",
    )?;

    Ok(conn)
}

/// A detection record for database insertion.
#[derive(Debug, Clone)]
pub struct DetectionRecord<'a> {
    /// Detection date (YYYY-MM-DD).
    pub date: &'a str,
    /// Detection time (HH:MM:SS).
    pub time: &'a str,
    /// Scientific name.
    pub sci_name: &'a str,
    /// Common name.
    pub com_name: &'a str,
    /// Confidence score.
    pub confidence: f64,
    /// Latitude.
    pub lat: &'a str,
    /// Longitude.
    pub lon: &'a str,
    /// Confidence cutoff threshold.
    pub cutoff: &'a str,
    /// ISO week number.
    pub week: &'a str,
    /// Sensitivity setting.
    pub sensitivity: &'a str,
    /// Overlap setting.
    pub overlap: &'a str,
    /// Extracted audio filename.
    pub file_name: &'a str,
}

/// Insert a detection record into the database.
///
/// # Errors
///
/// Returns `DbError` on insert failure.
pub fn insert_detection(conn: &Connection, record: &DetectionRecord<'_>) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO detections VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            record.date,
            record.time,
            record.sci_name,
            record.com_name,
            record.confidence,
            record.lat,
            record.lon,
            record.cutoff,
            record.week,
            record.sensitivity,
            record.overlap,
            record.file_name,
        ],
    )?;
    Ok(())
}

/// Get the total number of detections.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_count(conn: &Connection) -> Result<i64, DbError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM detections", [], |row| row.get(0))?;
    Ok(count)
}

/// Get the number of unique species.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_count(conn: &Connection) -> Result<i64, DbError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT Sci_Name) FROM detections",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Query detections for a specific date.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detections_by_date(conn: &Connection, date: &str) -> Result<Vec<DetectionRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name
         FROM detections WHERE Date = ?1 ORDER BY Time DESC",
    )?;

    let rows = stmt
        .query_map(params![date], |row| {
            Ok(DetectionRow {
                date: row.get(0)?,
                time: row.get(1)?,
                sci_name: row.get(2)?,
                com_name: row.get(3)?,
                confidence: row.get(4)?,
                lat: row.get(5)?,
                lon: row.get(6)?,
                cutoff: row.get(7)?,
                week: row.get(8)?,
                sens: row.get(9)?,
                overlap: row.get(10)?,
                file_name: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Query recent detections with a limit.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn recent_detections(conn: &Connection, limit: u32) -> Result<Vec<DetectionRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name
         FROM detections ORDER BY Date DESC, Time DESC LIMIT ?1",
    )?;

    let rows = stmt
        .query_map(params![limit], |row| {
            Ok(DetectionRow {
                date: row.get(0)?,
                time: row.get(1)?,
                sci_name: row.get(2)?,
                com_name: row.get(3)?,
                confidence: row.get(4)?,
                lat: row.get(5)?,
                lon: row.get(6)?,
                cutoff: row.get(7)?,
                week: row.get(8)?,
                sens: row.get(9)?,
                overlap: row.get(10)?,
                file_name: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Get top species by detection count.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn top_species(conn: &Connection, limit: u32) -> Result<Vec<SpeciesCount>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Com_Name, Sci_Name, COUNT(*) as count, AVG(Confidence) as avg_conf
         FROM detections GROUP BY Com_Name, Sci_Name ORDER BY count DESC LIMIT ?1",
    )?;

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

/// Query all detections, optionally filtered by date range.
///
/// When both `from` and `to` are `None`, returns all detections ordered by date/time descending.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn all_detections(
    conn: &Connection,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<DetectionRow>, DbError> {
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match (from, to) {
        (Some(f), Some(t)) => (
            "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name \
             FROM detections WHERE Date >= ?1 AND Date <= ?2 ORDER BY Date DESC, Time DESC"
                .to_string(),
            vec![Box::new(f.to_string()), Box::new(t.to_string())],
        ),
        (Some(f), None) => (
            "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name \
             FROM detections WHERE Date >= ?1 ORDER BY Date DESC, Time DESC"
                .to_string(),
            vec![Box::new(f.to_string())],
        ),
        (None, Some(t)) => (
            "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name \
             FROM detections WHERE Date <= ?1 ORDER BY Date DESC, Time DESC"
                .to_string(),
            vec![Box::new(t.to_string())],
        ),
        (None, None) => (
            "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name \
             FROM detections ORDER BY Date DESC, Time DESC"
                .to_string(),
            vec![],
        ),
    };

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_ref.as_slice(), |row| {
            Ok(DetectionRow {
                date: row.get(0)?,
                time: row.get(1)?,
                sci_name: row.get(2)?,
                com_name: row.get(3)?,
                confidence: row.get(4)?,
                lat: row.get(5)?,
                lon: row.get(6)?,
                cutoff: row.get(7)?,
                week: row.get(8)?,
                sens: row.get(9)?,
                overlap: row.get(10)?,
                file_name: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// A detection row from the database.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectionRow {
    /// Detection date.
    pub date: String,
    /// Detection time.
    pub time: String,
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Confidence score.
    pub confidence: f64,
    /// Latitude.
    pub lat: Option<f64>,
    /// Longitude.
    pub lon: Option<f64>,
    /// Cutoff threshold.
    pub cutoff: Option<f64>,
    /// ISO week number.
    pub week: Option<i32>,
    /// Sensitivity setting.
    pub sens: Option<f64>,
    /// Overlap setting.
    pub overlap: Option<f64>,
    /// Extracted audio filename.
    pub file_name: Option<String>,
}

/// Species with count and average confidence.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeciesCount {
    /// Common name.
    pub com_name: String,
    /// Scientific name.
    pub sci_name: String,
    /// Total detection count.
    pub count: i64,
    /// Average confidence score.
    pub avg_confidence: f64,
}

/// Hourly detection count.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HourlyCount {
    /// Hour string (00-23).
    pub hour: String,
    /// Number of detections.
    pub count: i64,
}

/// Daily detection count.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyCount {
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Number of detections on this date.
    pub count: i64,
}

/// Get daily detection counts for the last N days.
///
/// Returns counts in chronological order (oldest first).
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

/// Get daily detection counts for a specific species.
///
/// Returns up to `days` most recent days with counts for the given species.
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
         FROM detections WHERE Com_Name = ?1
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

    // Reverse so they're in chronological order
    rows.reverse();
    Ok(rows)
}

/// Get recent detections for a specific species.
///
/// Returns detections ordered by date/time descending.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detections_by_species(
    conn: &Connection,
    com_name: &str,
    limit: u32,
) -> Result<Vec<DetectionRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name
         FROM detections WHERE Com_Name = ?1 ORDER BY Date DESC, Time DESC LIMIT ?2",
    )?;

    let rows = stmt
        .query_map(params![com_name, limit], |row| {
            Ok(DetectionRow {
                date: row.get(0)?,
                time: row.get(1)?,
                sci_name: row.get(2)?,
                com_name: row.get(3)?,
                confidence: row.get(4)?,
                lat: row.get(5)?,
                lon: row.get(6)?,
                cutoff: row.get(7)?,
                week: row.get(8)?,
                sens: row.get(9)?,
                overlap: row.get(10)?,
                file_name: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Get species summary (count, avg confidence, first/last seen) by common name.
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
         FROM detections WHERE Com_Name = ?1 GROUP BY Com_Name",
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

/// Species summary with statistics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeciesSummary {
    /// Common name.
    pub com_name: String,
    /// Scientific name.
    pub sci_name: String,
    /// Total detection count.
    pub count: i64,
    /// Average confidence score.
    pub avg_confidence: f64,
    /// First detection date (YYYY-MM-DD).
    pub first_seen: String,
    /// Last detection date (YYYY-MM-DD).
    pub last_seen: String,
}

/// Get hourly activity for a specific species (across all dates).
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_hourly_activity(
    conn: &Connection,
    com_name: &str,
) -> Result<Vec<HourlyCount>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT SUBSTR(Time, 1, 2) as hour, COUNT(*) as count
         FROM detections WHERE Com_Name = ?1
         GROUP BY hour ORDER BY hour",
    )?;

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

/// Get the most recent detection timestamp.
///
/// Returns `(date, time, com_name)` of the latest detection, or `None` if empty.
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

/// Confidence distribution buckets.
///
/// Returns counts of detections in each confidence range:
/// `[0-50%, 50-60%, 60-70%, 70-80%, 80-90%, 90-100%]`.
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
         FROM detections
         GROUP BY bucket
         ORDER BY bucket",
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

/// Run a quick integrity check.
///
/// # Errors
///
/// Returns `DbError` on check failure.
pub fn quick_check(conn: &Connection) -> Result<bool, DbError> {
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db() -> (tempfile::NamedTempFile, Connection) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        (tmp, conn)
    }

    fn insert_sample_records(conn: &Connection) {
        let records = [
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
        ];
        for (date, time, sci, com, conf) in &records {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![date, time, sci, com, conf],
            )
            .unwrap();
        }
    }

    #[test]
    fn create_and_insert() {
        let (_tmp, conn) = temp_db();
        let record = DetectionRecord {
            date: "2026-03-11",
            time: "08:30:00",
            sci_name: "Turdus merula",
            com_name: "Eurasian Blackbird",
            confidence: 0.87,
            lat: "42.36",
            lon: "-71.06",
            cutoff: "0.7",
            week: "10",
            sensitivity: "1.25",
            overlap: "0.0",
            file_name: "test.wav",
        };
        insert_detection(&conn, &record).unwrap();

        assert_eq!(detection_count(&conn).unwrap(), 1);
        assert_eq!(species_count(&conn).unwrap(), 1);
    }

    #[test]
    fn wal_mode_is_set() {
        let (_tmp, conn) = temp_db();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn quick_check_passes() {
        let (_tmp, conn) = temp_db();
        assert!(quick_check(&conn).unwrap());
    }

    #[test]
    fn open_nonexistent_returns_error() {
        let result = open_connection(&PathBuf::from("/nonexistent/birds.db"));
        assert!(matches!(result, Err(DbError::NotFound(_))));
    }

    #[test]
    fn query_detections_by_date() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let rows = detections_by_date(&conn, "2026-03-11").unwrap();
        assert_eq!(rows.len(), 3);
        // Should be sorted by time DESC
        assert_eq!(rows[0].time, "07:00:00");
    }

    #[test]
    fn query_recent_detections() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let rows = recent_detections(&conn, 2).unwrap();
        assert_eq!(rows.len(), 2);
        // Most recent first
        assert_eq!(rows[0].date, "2026-03-11");
    }

    #[test]
    fn query_top_species() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let species = top_species(&conn, 10).unwrap();
        assert_eq!(species.len(), 3);
        // Blackbird has 2 detections, should be first
        assert_eq!(species[0].com_name, "Eurasian Blackbird");
        assert_eq!(species[0].count, 2);
    }

    #[test]
    fn query_all_detections_no_filter() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let rows = all_detections(&conn, None, None).unwrap();
        assert_eq!(rows.len(), 4);
        // Most recent date first
        assert_eq!(rows[0].date, "2026-03-11");
    }

    #[test]
    fn query_all_detections_with_date_range() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let rows = all_detections(&conn, Some("2026-03-11"), Some("2026-03-11")).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.date == "2026-03-11"));
    }

    #[test]
    fn query_all_detections_from_only() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let rows = all_detections(&conn, Some("2026-03-11"), None).unwrap();
        assert_eq!(rows.len(), 3); // only 2026-03-11 records (2026-03-10 excluded)
    }

    #[test]
    fn query_all_detections_to_only() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let rows = all_detections(&conn, None, Some("2026-03-10")).unwrap();
        assert_eq!(rows.len(), 1); // only the 2026-03-10 record
    }

    #[test]
    fn query_hourly_activity() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let hours = hourly_activity(&conn, "2026-03-11").unwrap();
        assert_eq!(hours.len(), 2); // 06 and 07
        assert_eq!(hours[0].hour, "06");
        assert_eq!(hours[0].count, 2);
    }

    #[test]
    fn query_daily_counts() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        // Query with a large window to get all days
        let days = daily_counts(&conn, 365).unwrap();
        // Sample data has 2 dates: 2026-03-10 and 2026-03-11
        assert!(days.len() >= 2);

        // Check they're in chronological order
        if days.len() >= 2 {
            assert!(days[0].date <= days[1].date);
        }

        // Check counts are positive
        for day in &days {
            assert!(day.count > 0);
        }
    }

    #[test]
    fn query_species_summary() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let summary = species_summary(&conn, "Eurasian Blackbird").unwrap();
        assert!(summary.is_some());
        let s = summary.unwrap();
        assert_eq!(s.com_name, "Eurasian Blackbird");
        assert_eq!(s.sci_name, "Turdus merula");
        assert_eq!(s.count, 2);
        // Average of 0.87 and 0.75 = 0.81
        assert!((s.avg_confidence - 0.81).abs() < 0.01);
    }

    #[test]
    fn query_species_summary_not_found() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let summary = species_summary(&conn, "Nonexistent Bird").unwrap();
        assert!(summary.is_none());
    }

    #[test]
    fn query_detections_by_species() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let rows = detections_by_species(&conn, "Eurasian Blackbird", 10).unwrap();
        assert_eq!(rows.len(), 2);
        // All should be blackbird
        assert!(rows.iter().all(|d| d.com_name == "Eurasian Blackbird"));
    }

    #[test]
    fn query_species_daily_counts() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let days = species_daily_counts(&conn, "Eurasian Blackbird", 7).unwrap();
        assert_eq!(days.len(), 1); // Only 1 date with blackbird detections
        assert_eq!(days[0].date, "2026-03-11");
        assert_eq!(days[0].count, 2);
    }

    #[test]
    fn query_species_hourly_activity() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let hours = species_hourly_activity(&conn, "Eurasian Blackbird").unwrap();
        assert_eq!(hours.len(), 2); // 06 and 07
        assert_eq!(hours[0].hour, "06");
        assert_eq!(hours[0].count, 1);
        assert_eq!(hours[1].hour, "07");
        assert_eq!(hours[1].count, 1);
    }

    #[test]
    fn query_latest_detection() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let latest = latest_detection(&conn).unwrap();
        assert!(latest.is_some());
        let (date, time, name) = latest.unwrap();
        assert_eq!(date, "2026-03-11");
        assert_eq!(time, "07:00:00");
        assert_eq!(name, "Eurasian Blackbird");
    }

    #[test]
    fn query_latest_detection_empty() {
        let (_tmp, conn) = temp_db();

        let latest = latest_detection(&conn).unwrap();
        assert!(latest.is_none());
    }

    #[test]
    fn query_confidence_distribution() {
        let (_tmp, conn) = temp_db();
        insert_sample_records(&conn);

        let buckets = confidence_distribution(&conn).unwrap();
        // Sample data: 0.87, 0.92, 0.75, 0.80
        // 0.75 -> bucket 3 (70-80%)
        // 0.80, 0.87 -> bucket 4 (80-90%)
        // 0.92 -> bucket 5 (90-100%)
        assert_eq!(buckets[3], 1); // 70-80%
        assert_eq!(buckets[4], 2); // 80-90%
        assert_eq!(buckets[5], 1); // 90-100%
    }
}
