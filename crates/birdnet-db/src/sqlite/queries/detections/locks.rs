//! Detection lock/unlock queries (protect a clip from the disk purge).

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

/// Lock a detection (protect it from disk purge).
///
/// Returns `true` if a row was updated.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn lock_detection(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "UPDATE detections SET is_locked = 1 WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3",
        params![date, time, sci_name],
    )?;
    Ok(changed > 0)
}

/// Unlock a detection (allow disk purge again).
///
/// Returns `true` if a row was updated.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn unlock_detection(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "UPDATE detections SET is_locked = 0 WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3",
        params![date, time, sci_name],
    )?;
    Ok(changed > 0)
}

/// Get all file names that are locked (for purge protection).
///
/// Returns distinct non-null file names for locked detections.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn locked_file_names(conn: &Connection) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT File_Name FROM detections \
         WHERE is_locked = 1 AND File_Name IS NOT NULL",
    )?;
    let rows = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// Check if a detection is locked.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn is_detection_locked(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<bool, DbError> {
    let locked: i64 = conn.query_row(
        "SELECT COALESCE(is_locked, 0) FROM detections \
         WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3",
        params![date, time, sci_name],
        |row| row.get(0),
    )?;
    Ok(locked != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;
    use crate::sqlite::queries::detections::test_support::temp_db_with_data;

    #[test]
    fn lock_unlock_detection_flips_is_locked() {
        let (_tmp, conn) = temp_db_with_data();
        assert!(!is_detection_locked(&conn, "2026-03-10", "18:00:00", "Parus major").unwrap());
        assert!(lock_detection(&conn, "2026-03-10", "18:00:00", "Parus major").unwrap());
        assert!(is_detection_locked(&conn, "2026-03-10", "18:00:00", "Parus major").unwrap());
        assert!(unlock_detection(&conn, "2026-03-10", "18:00:00", "Parus major").unwrap());
        assert!(!is_detection_locked(&conn, "2026-03-10", "18:00:00", "Parus major").unwrap());
    }

    #[test]
    fn lock_detection_returns_false_when_no_match() {
        let (_tmp, conn) = temp_db_with_data();
        assert!(!lock_detection(&conn, "1900-01-01", "00:00:00", "Parus major").unwrap());
    }

    #[test]
    fn unlock_detection_returns_false_when_no_match() {
        let (_tmp, conn) = temp_db_with_data();
        assert!(!unlock_detection(&conn, "1900-01-01", "00:00:00", "Parus major").unwrap());
    }

    #[test]
    fn locked_file_names_lists_distinct_locked_files() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        for (date, time, sci, com, conf, file) in [
            (
                "2026-03-11",
                "06:30:00",
                "Turdus merula",
                "Eurasian Blackbird",
                0.87,
                "a.wav",
            ),
            (
                "2026-03-11",
                "06:45:00",
                "Erithacus rubecula",
                "European Robin",
                0.92,
                "a.wav",
            ),
            (
                "2026-03-10",
                "18:00:00",
                "Parus major",
                "Great Tit",
                0.80,
                "b.wav",
            ),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name) VALUES (?1,?2,?3,?4,?5,?6)",
                params![date, time, sci, com, conf, file],
            ).unwrap();
        }
        // Lock two rows on `a.wav` and one on `b.wav` — locked_file_names
        // must return both distinct file names, not duplicate `a.wav`.
        lock_detection(&conn, "2026-03-11", "06:30:00", "Turdus merula").unwrap();
        lock_detection(&conn, "2026-03-11", "06:45:00", "Erithacus rubecula").unwrap();
        lock_detection(&conn, "2026-03-10", "18:00:00", "Parus major").unwrap();
        let mut names = locked_file_names(&conn).unwrap();
        names.sort();
        assert_eq!(names, vec!["a.wav".to_string(), "b.wav".to_string()]);
    }

    #[test]
    fn locked_file_names_omits_unlocked_rows() {
        let (_tmp, conn) = temp_db_with_data();
        // None are locked by default.
        assert!(locked_file_names(&conn).unwrap().is_empty());
    }
}
