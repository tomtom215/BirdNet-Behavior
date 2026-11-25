//! Detection lock/unlock queries (protect a clip from the disk purge).

use rusqlite::{Connection, params};

use super::write::{DetectionKey, KEY_WHERE};
use crate::sqlite::connection::DbError;

/// Lock or unlock the one detection `key` names.
fn set_locked_at(conn: &Connection, key: &DetectionKey<'_>, locked: bool) -> Result<bool, DbError> {
    let changed = conn.execute(
        &format!("UPDATE detections SET is_locked = ?5 WHERE {KEY_WHERE}"),
        params![
            key.date,
            key.time,
            key.sci_name,
            key.file_name,
            i64::from(locked)
        ],
    )?;
    Ok(changed > 0)
}

/// Lock the one detection `key` names (protect it from the disk purge and
/// from bulk deletes).
///
/// Returns `true` if a row was updated.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn lock_detection_at(conn: &Connection, key: &DetectionKey<'_>) -> Result<bool, DbError> {
    set_locked_at(conn, key, true)
}

/// Unlock the one detection `key` names.
///
/// The triple-keyed [`unlock_detection`] also unlocked a second source's row
/// of the same second — silently removing a protection its owner had set.
///
/// Returns `true` if a row was updated.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn unlock_detection_at(conn: &Connection, key: &DetectionKey<'_>) -> Result<bool, DbError> {
    set_locked_at(conn, key, false)
}

/// Whether the one detection `key` names is locked. `Ok(false)` when no row
/// matches.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn is_detection_locked_at(conn: &Connection, key: &DetectionKey<'_>) -> Result<bool, DbError> {
    let locked: Option<i64> = conn.query_row(
        &format!("SELECT MAX(COALESCE(is_locked, 0)) FROM detections WHERE {KEY_WHERE}"),
        params![key.date, key.time, key.sci_name, key.file_name],
        |row| row.get(0),
    )?;
    Ok(locked.unwrap_or(0) != 0)
}

/// Lock a detection (protect it from disk purge).
///
/// **Superseded by [`lock_detection_at`]**: the triple can match a second
/// source's row of the same second.
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
/// **Superseded by [`unlock_detection_at`]**: the triple can match a second
/// source's row of the same second, and this unlocks it too.
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

/// Whether a detection's clip is kept back from automatic deletion, as an SQL
/// condition over `detections AS d`: the clip retention, the per-species cap
/// and the disk-full purge all read it, so the three agree.
///
/// A locked clip is always kept back. With `retain_reviewed` (the
/// `RETAIN_REVIEWED_AUDIO` setting, on unless turned off) so is the clip of a
/// detection somebody judged by listening: one with a review verdict, either
/// way, or a comment (RES-10). That clip is the evidence a second reviewer
/// needs to check the judgement; without it a confirmed record cannot be
/// verified again by anyone. A comment whose clip could not be told
/// (`file_name` NULL, written before comments named a clip) counts for every
/// detection of its second, as it is listed on each of them.
///
/// The text is a constant chosen by the flag and binds no parameter, so a
/// caller can place it in a query whatever that query's own parameters are.
#[must_use]
pub const fn clip_kept_back_sql(retain_reviewed: bool) -> &'static str {
    if retain_reviewed {
        "(COALESCE(d.is_locked, 0) = 1
          OR d.review_verdict IS NOT NULL
          OR EXISTS (SELECT 1 FROM detection_comments c
                      WHERE c.date = d.Date AND c.time = d.Time
                        AND c.sci_name = d.Sci_Name
                        AND (c.file_name IS NULL
                             OR c.file_name = COALESCE(d.File_Name, ''))))"
    } else {
        "(COALESCE(d.is_locked, 0) = 1)"
    }
}

/// The clips the disk-full purge must not delete: every clip
/// [`clip_kept_back_sql`] keeps back. With `retain_reviewed` off this is
/// exactly [`locked_file_names`].
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn kept_back_file_names(
    conn: &Connection,
    retain_reviewed: bool,
) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT d.File_Name FROM detections d
          WHERE d.File_Name IS NOT NULL AND TRIM(d.File_Name) <> ''
            AND {}",
        clip_kept_back_sql(retain_reviewed)
    ))?;
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

    /// Finding 1: unlocking one source's row must not unlock the other's.
    #[test]
    fn a_keyed_unlock_leaves_the_other_sources_lock() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        for file in ["cam1.wav", "cam2.wav"] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES ('2026-05-01', '06:00:00', 'Turdus merula', 'Blackbird', 0.9, ?1)",
                params![file],
            )
            .unwrap();
        }
        let key = |file| DetectionKey {
            date: "2026-05-01",
            time: "06:00:00",
            sci_name: "Turdus merula",
            file_name: Some(file),
        };
        assert!(lock_detection_at(&conn, &key("cam1.wav")).unwrap());
        assert!(lock_detection_at(&conn, &key("cam2.wav")).unwrap());
        assert!(unlock_detection_at(&conn, &key("cam2.wav")).unwrap());
        assert!(is_detection_locked_at(&conn, &key("cam1.wav")).unwrap());
        assert!(!is_detection_locked_at(&conn, &key("cam2.wav")).unwrap());
        assert!(!is_detection_locked_at(&conn, &key("nope.wav")).unwrap());
        // Counterpart: a key that names no row changes nothing and says so.
        assert!(!lock_detection_at(&conn, &key("nope.wav")).unwrap());
        assert!(!unlock_detection_at(&conn, &key("nope.wav")).unwrap());
    }

    /// Which clips are kept back, by kind (RES-10): locked always; a verdict
    /// either way, a comment on the clip and a comment written before
    /// comments named a clip only with `retain_reviewed`; nothing else.
    #[test]
    fn kept_back_clips_are_the_locked_and_the_judged() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        for (time, file) in [
            ("06:00:00", "locked.wav"),
            ("06:00:01", "confirmed.wav"),
            ("06:00:02", "rejected.wav"),
            ("06:00:03", "commented.wav"),
            ("06:00:04", "legacy-comment.wav"),
            ("06:00:05", "nothing.wav"),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES ('2026-05-01', ?1, 'Turdus merula', 'Blackbird', 0.9, ?2)",
                params![time, file],
            )
            .unwrap();
        }
        conn.execute_batch(
            "UPDATE detections SET is_locked = 1 WHERE File_Name = 'locked.wav';
             UPDATE detections SET review_verdict = 'confirmed' WHERE File_Name = 'confirmed.wav';
             UPDATE detections SET review_verdict = 'rejected' WHERE File_Name = 'rejected.wav';
             INSERT INTO detection_comments (date, time, sci_name, file_name, author, body)
                  VALUES ('2026-05-01', '06:00:03', 'Turdus merula', 'commented.wav', 'ada', 'x');
             INSERT INTO detection_comments (date, time, sci_name, file_name, author, body)
                  VALUES ('2026-05-01', '06:00:04', 'Turdus merula', NULL, 'ada', 'y');",
        )
        .unwrap();
        let sorted = |retain| {
            let mut v = kept_back_file_names(&conn, retain).unwrap();
            v.sort();
            v
        };
        assert_eq!(
            sorted(true),
            [
                "commented.wav",
                "confirmed.wav",
                "legacy-comment.wav",
                "locked.wav",
                "rejected.wav"
            ]
        );
        assert_eq!(sorted(false), ["locked.wav"]);
        assert_eq!(sorted(false), locked_file_names(&conn).unwrap());
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
