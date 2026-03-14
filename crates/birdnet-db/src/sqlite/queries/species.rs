//! Species-level aggregation queries.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;
use crate::sqlite::types::{
    DETECTION_COLS, DailyCount, DetectionRow, HourlyCount, SpeciesCount, SpeciesSummary,
    map_detection_row,
};

/// Get the number of unique species (by scientific name).
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

/// Search species by name (case-insensitive substring match on common or scientific name).
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
    let mut stmt = conn.prepare(
        "SELECT Com_Name, Sci_Name, COUNT(*) as count, AVG(Confidence) as avg_conf
         FROM detections
         WHERE Com_Name LIKE ?1 COLLATE NOCASE OR Sci_Name LIKE ?1 COLLATE NOCASE
         GROUP BY Com_Name, Sci_Name ORDER BY count DESC LIMIT ?2",
    )?;
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
    rows.reverse(); // chronological order
    Ok(rows)
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
        "SELECT {DETECTION_COLS} FROM detections \
         WHERE Com_Name = ?1 ORDER BY Date DESC, Time DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![com_name, limit], map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get the first-seen date for each species (by scientific name).
///
/// Returns a map from scientific name to its first detection date.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_first_seen(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, String>, DbError> {
    let mut stmt = conn.prepare("SELECT Sci_Name, MIN(Date) FROM detections GROUP BY Sci_Name")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<std::collections::HashMap<String, String>, _>>()?;
    Ok(rows)
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
