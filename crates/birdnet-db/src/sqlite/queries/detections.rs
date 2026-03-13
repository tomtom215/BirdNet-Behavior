//! Detection CRUD queries.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;
use crate::sqlite::types::{DETECTION_COLS, DetectionRecord, DetectionRow, map_detection_row};

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

/// Query detections for a specific date, ordered by time descending.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detections_by_date(conn: &Connection, date: &str) -> Result<Vec<DetectionRow>, DbError> {
    let sql = format!(
        "SELECT {DETECTION_COLS} FROM detections WHERE Date = ?1 ORDER BY Time DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![date], map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Query the most recent detections up to `limit`.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn recent_detections(conn: &Connection, limit: u32) -> Result<Vec<DetectionRow>, DbError> {
    let sql = format!(
        "SELECT {DETECTION_COLS} FROM detections ORDER BY Date DESC, Time DESC LIMIT ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![limit], map_detection_row)?
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
) -> Result<Vec<DetectionRow>, DbError> {
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match (from, to) {
        (Some(f), Some(t)) => (
            format!("SELECT {DETECTION_COLS} FROM detections WHERE Date >= ?1 AND Date <= ?2 ORDER BY Date DESC, Time DESC"),
            vec![Box::new(f.to_string()), Box::new(t.to_string())],
        ),
        (Some(f), None) => (
            format!("SELECT {DETECTION_COLS} FROM detections WHERE Date >= ?1 ORDER BY Date DESC, Time DESC"),
            vec![Box::new(f.to_string())],
        ),
        (None, Some(t)) => (
            format!("SELECT {DETECTION_COLS} FROM detections WHERE Date <= ?1 ORDER BY Date DESC, Time DESC"),
            vec![Box::new(t.to_string())],
        ),
        (None, None) => (
            format!("SELECT {DETECTION_COLS} FROM detections ORDER BY Date DESC, Time DESC"),
            vec![],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;

    fn temp_db_with_data() -> (tempfile::NamedTempFile, Connection) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        for (date, time, sci, com, conf) in [
            ("2026-03-11", "06:30:00", "Turdus merula", "Eurasian Blackbird", 0.87),
            ("2026-03-11", "06:45:00", "Erithacus rubecula", "European Robin", 0.92),
            ("2026-03-11", "07:00:00", "Turdus merula", "Eurasian Blackbird", 0.75),
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
    fn insert_and_count() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let record = DetectionRecord {
            date: "2026-03-11", time: "08:30:00",
            sci_name: "Turdus merula", com_name: "Eurasian Blackbird",
            confidence: 0.87, lat: "42.36", lon: "-71.06",
            cutoff: "0.7", week: "10", sensitivity: "1.25",
            overlap: "0.0", file_name: "test.wav",
        };
        insert_detection(&conn, &record).unwrap();
        assert_eq!(detection_count(&conn).unwrap(), 1);
    }

    #[test]
    fn detections_by_date_ordered() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = detections_by_date(&conn, "2026-03-11").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].time, "07:00:00");
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
        let rows = all_detections(&conn, None, None).unwrap();
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn all_detections_date_range() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = all_detections(&conn, Some("2026-03-11"), Some("2026-03-11")).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn all_detections_from_only() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = all_detections(&conn, Some("2026-03-11"), None).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn all_detections_to_only() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = all_detections(&conn, None, Some("2026-03-10")).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn detections_by_species_filters() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = detections_by_species(&conn, "Eurasian Blackbird", 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|d| d.com_name == "Eurasian Blackbird"));
    }
}
