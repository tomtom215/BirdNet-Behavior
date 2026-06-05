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
    // Explicit column list — `VALUES (?1, …, ?12)` without one was a
    // schema-vs-insert drift waiting to happen and broke in production
    // when migration 7 added `is_locked` as a 13th column. Naming the
    // columns means new columns with a DEFAULT (like `is_locked`) keep
    // this write path working unchanged.
    conn.execute(
        "INSERT INTO detections \
         (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, chunk_offset_secs, correlation_id, Source) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
            record.chunk_offset_secs,
            record.correlation_id,
            record.source,
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
        "SELECT COUNT(*) FROM detections WHERE Date = ?1 AND Sci_Name = ?2",
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
        "SELECT {DETECTION_COLS} FROM detections \
         WHERE Date = ?1 AND File_Name IS NOT NULL AND TRIM(File_Name) <> '' \
         ORDER BY Confidence DESC, Time DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![date, limit], map_detection_row)?
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
            format!(
                "SELECT {DETECTION_COLS} FROM detections WHERE Date >= ?1 AND Date <= ?2 ORDER BY Date DESC, Time DESC"
            ),
            vec![Box::new(f.to_string()), Box::new(t.to_string())],
        ),
        (Some(f), None) => (
            format!(
                "SELECT {DETECTION_COLS} FROM detections WHERE Date >= ?1 ORDER BY Date DESC, Time DESC"
            ),
            vec![Box::new(f.to_string())],
        ),
        (None, Some(t)) => (
            format!(
                "SELECT {DETECTION_COLS} FROM detections WHERE Date <= ?1 ORDER BY Date DESC, Time DESC"
            ),
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

/// Delete a detection by date, time, and scientific name.
///
/// Returns `true` if a row was deleted, `false` if no match was found.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn delete_detection(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "DELETE FROM detections WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3",
        params![date, time, sci_name],
    )?;
    Ok(changed > 0)
}

/// Re-label a detection by changing its species identification.
///
/// Returns `true` if a row was updated, `false` if no match was found.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn relabel_detection(
    conn: &Connection,
    date: &str,
    time: &str,
    old_sci_name: &str,
    new_sci_name: &str,
    new_com_name: &str,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "UPDATE detections SET Sci_Name = ?4, Com_Name = ?5 \
         WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3",
        params![date, time, old_sci_name, new_sci_name, new_com_name],
    )?;
    Ok(changed > 0)
}

/// What the operator meant by a free-text search term.
///
/// Public for unit-test access. `Exclude` carries the rest of the term
/// (post-`"NOT "` prefix, trimmed) so the caller can format it directly
/// into a SQL LIKE pattern.
#[derive(Debug, PartialEq, Eq)]
pub enum SearchTerm {
    /// The whole term is a `Com_Name LIKE %term% OR Sci_Name LIKE %term%`
    /// inclusion pattern.
    Include(String),
    /// The term begins case-insensitively with `"NOT "` and has at least
    /// one non-whitespace character after it. The carried string is the
    /// content after the prefix, trimmed; the caller wraps it as
    /// `Com_Name NOT LIKE %term%`.
    Exclude(String),
}

/// Parse an operator-supplied search box value into [`SearchTerm`].
///
/// `None` if the input is `None`, empty, or whitespace-only — the caller
/// should drop the WHERE clause entirely in that case.
///
/// The `"NOT "` prefix is the legacy BirdNET-Pi exclusion syntax. We use
/// `str::strip_prefix_ignore_ascii_case` rather than `s.len() > 4 &&
/// s[..4].eq_ignore_ascii_case("NOT ")` because the second form has an
/// equivalent mutant on the length comparison: with the calling code's
/// up-front `.trim()`, a 4-char input ending in space is unreachable, so
/// `> 4` and `>= 4` produce identical observable behaviour. Eliminating
/// the explicit length comparison eliminates the mutant. Tracked in the
/// `parse_search_term_*` tests below.
#[must_use]
pub fn parse_search_term(raw: Option<&str>) -> Option<SearchTerm> {
    let trimmed = raw.map(str::trim).filter(|s| !s.is_empty())?;
    if let Some(rest) = strip_not_prefix(trimmed) {
        let rest = rest.trim();
        if !rest.is_empty() {
            return Some(SearchTerm::Exclude(rest.to_string()));
        }
    }
    Some(SearchTerm::Include(trimmed.to_string()))
}

/// Strip a case-insensitive `"NOT "` prefix.
///
/// Returns `Some(&s[4..])` if `s` is at least 4 bytes long and the first
/// four bytes are ASCII-equal-ignore-case to `"NOT "`. Otherwise `None`.
///
/// This uses `s.get(..4)` rather than a length comparison so cargo-mutants
/// has no `>` / `>=` boundary to flip — the existence check is implicit
/// in the `Option` return. The unit test pins every cell of the case-
/// insensitive prefix match table.
fn strip_not_prefix(s: &str) -> Option<&str> {
    let head = s.get(..4)?;
    if head.eq_ignore_ascii_case("NOT ") {
        Some(&s[4..])
    } else {
        None
    }
}

/// Search today's detections with optional text filter, limit, and offset.
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
    limit: u32,
    offset: u32,
) -> Result<Vec<DetectionRow>, DbError> {
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        match parse_search_term(search) {
            Some(SearchTerm::Exclude(rest)) => {
                let pattern = format!("%{rest}%");
                (
                    format!(
                        "SELECT {DETECTION_COLS} FROM detections \
                         WHERE Date = ?1 AND Com_Name NOT LIKE ?2 \
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
                         WHERE Date = ?1 AND (Com_Name LIKE ?2 OR Sci_Name LIKE ?2) \
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
                     WHERE Date = ?1 ORDER BY Time DESC LIMIT ?2 OFFSET ?3"
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

/// Count today's detections with an optional text filter.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn todays_detection_count(
    conn: &Connection,
    date: &str,
    search: Option<&str>,
) -> Result<i64, DbError> {
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        match parse_search_term(search) {
            Some(SearchTerm::Exclude(rest)) => {
                let pattern = format!("%{rest}%");
                (
                    "SELECT COUNT(*) FROM detections WHERE Date = ?1 AND Com_Name NOT LIKE ?2"
                        .to_string(),
                    vec![Box::new(date.to_string()), Box::new(pattern)],
                )
            }
            Some(SearchTerm::Include(term)) => {
                let pattern = format!("%{term}%");
                (
                    "SELECT COUNT(*) FROM detections WHERE Date = ?1 AND (Com_Name LIKE ?2 OR Sci_Name LIKE ?2)"
                        .to_string(),
                    vec![Box::new(date.to_string()), Box::new(pattern)],
                )
            }
            None => (
                "SELECT COUNT(*) FROM detections WHERE Date = ?1".to_string(),
                vec![Box::new(date.to_string())],
            ),
        };

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();
    let count: i64 = conn.query_row(&sql, params_ref.as_slice(), |row| row.get(0))?;
    Ok(count)
}

/// Get a list of distinct dates that have detections, ordered descending.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_dates(conn: &Connection, limit: u32) -> Result<Vec<String>, DbError> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT Date FROM detections ORDER BY Date DESC LIMIT ?1")?;
    let rows = stmt
        .query_map(params![limit], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

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
        "SELECT Com_Name, Sci_Name, COUNT(*) as cnt FROM detections \
         WHERE Date = ?1 GROUP BY Com_Name, Sci_Name ORDER BY cnt DESC",
    )?;
    let rows = stmt
        .query_map(params![date], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
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
    fn insert_and_count() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let record = DetectionRecord {
            date: "2026-03-11",
            time: "08:30:00",
            sci_name: "Turdus merula",
            com_name: "Eurasian Blackbird",
            confidence: 0.87,
            lat: Some(42.36),
            lon: Some(-71.06),
            cutoff: Some(0.7),
            week: Some(10),
            sensitivity: Some(1.25),
            overlap: Some(0.0),
            file_name: "test.wav",
            chunk_offset_secs: Some(0.0),
            correlation_id: None,
            source: None,
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
    fn source_column_tags_streams_and_leaves_historical_null() {
        // Stage 1 contract: a new detection is tagged with its stream/source
        // label; a row written without a source (historical / imported) stays
        // NULL and reads back as None — non-destructive.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let tagged = DetectionRecord {
            date: "2026-05-19",
            time: "06:00:00",
            sci_name: "Pica pica",
            com_name: "Eurasian Magpie",
            confidence: 0.9,
            lat: None,
            lon: None,
            cutoff: None,
            week: None,
            sensitivity: None,
            overlap: None,
            file_name: "2026-05-19-birdnet-cam1-06:00:00.wav",
            chunk_offset_secs: Some(0.0),
            correlation_id: None,
            source: Some("cam1"),
        };
        // A second row at a different second with no source = the historical
        // shape (e.g. an imported BirdNET-Pi row).
        let untagged = DetectionRecord {
            time: "06:00:01",
            source: None,
            ..tagged.clone()
        };
        insert_detection(&conn, &tagged).unwrap();
        insert_detection(&conn, &untagged).unwrap();

        let by_time: std::collections::HashMap<String, Option<String>> =
            recent_detections(&conn, 10)
                .unwrap()
                .into_iter()
                .map(|r| (r.time, r.source))
                .collect();
        assert_eq!(by_time["06:00:00"].as_deref(), Some("cam1"));
        assert_eq!(
            by_time["06:00:01"], None,
            "untagged row must read back NULL"
        );
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

    // ── coverage carryover from PR #49 ─────────────────────────────────
    //
    // These tests cover the remaining helpers in this module so the
    // file can be brought under cargo-mutants at `missed = 0`. Each
    // adds one assertion per branch the surface is supposed to honour.

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
    fn delete_detection_removes_matching_row_and_returns_true() {
        let (_tmp, conn) = temp_db_with_data();
        assert!(delete_detection(&conn, "2026-03-10", "18:00:00", "Parus major").unwrap());
        assert_eq!(detection_count(&conn).unwrap(), 3);
    }

    #[test]
    fn delete_detection_returns_false_when_no_match() {
        let (_tmp, conn) = temp_db_with_data();
        assert!(!delete_detection(&conn, "2026-03-10", "00:00:00", "Parus major").unwrap());
        assert_eq!(detection_count(&conn).unwrap(), 4);
    }

    #[test]
    fn relabel_detection_updates_both_names_and_returns_true() {
        let (_tmp, conn) = temp_db_with_data();
        let updated = relabel_detection(
            &conn,
            "2026-03-10",
            "18:00:00",
            "Parus major",
            "Cyanistes caeruleus",
            "Eurasian Blue Tit",
        )
        .unwrap();
        assert!(updated);
        let rows = detections_by_species(&conn, "Eurasian Blue Tit", 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sci_name, "Cyanistes caeruleus");
    }

    #[test]
    fn relabel_detection_returns_false_when_no_match() {
        let (_tmp, conn) = temp_db_with_data();
        let updated = relabel_detection(
            &conn,
            "1900-01-01",
            "00:00:00",
            "Parus major",
            "Cyanistes caeruleus",
            "Eurasian Blue Tit",
        )
        .unwrap();
        assert!(!updated);
    }

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
        let rows = todays_detections(&conn, "2026-03-11", None, 10, 0).unwrap();
        assert_eq!(rows.len(), 3);

        // Include pattern (Com_Name LIKE).
        let rows = todays_detections(&conn, "2026-03-11", Some("Robin"), 10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].com_name, "European Robin");

        // Exclusion pattern (NOT LIKE).
        let rows = todays_detections(&conn, "2026-03-11", Some("NOT Robin"), 10, 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.com_name != "European Robin"));
    }

    #[test]
    fn todays_detections_pagination() {
        let (_tmp, conn) = temp_db_with_data();
        let page1 = todays_detections(&conn, "2026-03-11", None, 2, 0).unwrap();
        let page2 = todays_detections(&conn, "2026-03-11", None, 2, 2).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 1);
        assert_ne!(page1[0].time, page2[0].time);
    }

    #[test]
    fn todays_detections_whitespace_search_treated_as_none() {
        let (_tmp, conn) = temp_db_with_data();
        // A blank search term should not collapse the result set.
        let rows = todays_detections(&conn, "2026-03-11", Some("   "), 10, 0).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn todays_detections_search_matches_sci_name_too() {
        // The inclusion path matches either Com_Name or Sci_Name LIKE.
        let (_tmp, conn) = temp_db_with_data();
        let rows = todays_detections(&conn, "2026-03-11", Some("Erithacus"), 10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sci_name, "Erithacus rubecula");
    }

    #[test]
    fn todays_detection_count_filters_match_query_path() {
        let (_tmp, conn) = temp_db_with_data();
        assert_eq!(
            todays_detection_count(&conn, "2026-03-11", None).unwrap(),
            3
        );
        assert_eq!(
            todays_detection_count(&conn, "2026-03-11", Some("Robin")).unwrap(),
            1
        );
        assert_eq!(
            todays_detection_count(&conn, "2026-03-11", Some("NOT Robin")).unwrap(),
            2
        );
        assert_eq!(
            todays_detection_count(&conn, "2026-03-11", Some("   ")).unwrap(),
            3
        );
    }

    #[test]
    #[allow(clippy::items_after_statements)] // the type aliases tighten the asserter ergonomics
    fn insert_detection_with_null_optional_fields_stores_nulls() {
        // The DetectionRecord struct carries Option<f64>/Option<i64> so
        // missing values become SQLite NULLs — this contract is the
        // entire reason migration 11 + DetectionRecord exists. Pin it
        // against the columns that are nullable (lat, lon, cutoff,
        // week, sens, overlap). `chunk_offset_secs` is `NOT NULL
        // DEFAULT 0.0` since migration 11, so we pass Some(0.0).
        type OptF = Option<f64>;
        type OptI = Option<i64>;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let record = DetectionRecord {
            date: "2026-03-11",
            time: "08:30:00",
            sci_name: "Turdus merula",
            com_name: "Eurasian Blackbird",
            confidence: 0.87,
            lat: None,
            lon: None,
            cutoff: None,
            week: None,
            sensitivity: None,
            overlap: None,
            file_name: "test.wav",
            chunk_offset_secs: Some(0.0),
            correlation_id: None,
            source: None,
        };

        insert_detection(&conn, &record).unwrap();

        // Read back: the optional fields should be SQL NULL, not the
        // empty string that the pre-migration-11 daemon used to
        // produce and that poisoned every typed read.
        let cols: (OptF, OptF, OptF, OptI, OptF, OptF) = conn
            .query_row(
                "SELECT Lat, Lon, Cutoff, Week, Sens, Overlap FROM detections WHERE Sci_Name = ?1",
                params!["Turdus merula"],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(cols, (None, None, None, None, None, None));
    }

    #[test]
    fn insert_detection_chunk_offset_is_stored_in_unique_key() {
        // Migration 11 added `chunk_offset_secs` to the UNIQUE
        // constraint so a Magpie that calls in five chunks of one file
        // doesn't collapse to a single row. Two rows with identical
        // (Date, Time, Sci_Name, File_Name) but different chunk offsets
        // must both succeed.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let base = DetectionRecord {
            date: "2026-05-19",
            time: "09:00:00",
            sci_name: "Pica pica",
            com_name: "Eurasian Magpie",
            confidence: 0.93,
            lat: None,
            lon: None,
            cutoff: None,
            week: Some(20),
            sensitivity: None,
            overlap: None,
            file_name: "magpie.wav",
            chunk_offset_secs: Some(0.0),
            correlation_id: None,
            source: None,
        };
        insert_detection(&conn, &base).unwrap();
        let chunk2 = DetectionRecord {
            chunk_offset_secs: Some(4.5),
            ..base.clone()
        };
        insert_detection(&conn, &chunk2).unwrap();
        let chunk3 = DetectionRecord {
            chunk_offset_secs: Some(9.0),
            ..base.clone()
        };
        insert_detection(&conn, &chunk3).unwrap();

        assert_eq!(detection_count(&conn).unwrap(), 3);
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

    // ── correlation_id round trip (migration 12) ───────────────────────

    #[test]
    fn correlation_id_round_trips_through_insert_and_read() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let record = DetectionRecord {
            date: "2026-05-19",
            time: "09:00:00",
            sci_name: "Pica pica",
            com_name: "Eurasian Magpie",
            confidence: 0.92,
            lat: None,
            lon: None,
            cutoff: None,
            week: Some(20),
            sensitivity: None,
            overlap: None,
            file_name: "magpie.wav",
            chunk_offset_secs: Some(0.0),
            correlation_id: Some("e-20260519-abc123"),
            source: Some("local"),
        };
        insert_detection(&conn, &record).unwrap();
        let rows = recent_detections(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].correlation_id.as_deref(), Some("e-20260519-abc123"));
        assert_eq!(rows[0].source.as_deref(), Some("local"));
    }

    #[test]
    fn correlation_id_null_when_record_omits_it() {
        // Quarantine-approve / BirdNET-Pi-import paths write None;
        // migration 12 keeps the column NULL so the daemon's id-shape
        // contract isn't forced on every code path.
        let (_tmp, conn) = temp_db_with_data();
        let rows = recent_detections(&conn, 10).unwrap();
        assert!(rows.iter().all(|r| r.correlation_id.is_none()));
    }

    // ── parse_search_term ──────────────────────────────────────────────
    //
    // Direct unit tests for the helper. The whole point of extracting it
    // was to make every NOT-prefix-recognition cell observable without
    // round-tripping through SQL — the cargo-mutants report on
    // detections.rs flagged the in-line `s.len() > 4 && s[..4] ...`
    // form as carrying an equivalent mutant (with the up-front
    // `.trim()` the length-4 boundary was unreachable). The helper now
    // uses `s.get(..4)`, which has no `>` / `>=` for cargo-mutants to
    // flip.

    #[test]
    fn parse_search_term_none_or_empty_returns_none() {
        assert_eq!(parse_search_term(None), None);
        assert_eq!(parse_search_term(Some("")), None);
        assert_eq!(parse_search_term(Some("   ")), None);
        assert_eq!(parse_search_term(Some("\t\n")), None);
    }

    #[test]
    fn parse_search_term_plain_word_is_include() {
        assert_eq!(
            parse_search_term(Some("Robin")),
            Some(SearchTerm::Include("Robin".into()))
        );
        // Leading / trailing whitespace is trimmed before the dispatch.
        assert_eq!(
            parse_search_term(Some("  Robin  ")),
            Some(SearchTerm::Include("Robin".into()))
        );
    }

    #[test]
    fn parse_search_term_not_prefix_is_exclude() {
        assert_eq!(
            parse_search_term(Some("NOT Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
        // Case-insensitive prefix match: "not", "Not", "NoT" all work.
        assert_eq!(
            parse_search_term(Some("not Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
        assert_eq!(
            parse_search_term(Some("Not Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
        assert_eq!(
            parse_search_term(Some("nOt Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
    }

    #[test]
    fn parse_search_term_not_prefix_trims_remainder() {
        // The remainder is trimmed too — "NOT   Robin   " excludes
        // "Robin", not "  Robin  ".
        assert_eq!(
            parse_search_term(Some("NOT   Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
    }

    #[test]
    fn parse_search_term_lone_not_degrades_to_include() {
        // The 3-char "NOT" doesn't have the trailing space so doesn't
        // match the prefix → include "NOT".
        assert_eq!(
            parse_search_term(Some("NOT")),
            Some(SearchTerm::Include("NOT".into()))
        );
        // "NOT " with the literal trailing space SHOULD be unreachable
        // in practice because the function trims its input. But even if
        // a caller bypasses the trim, an empty remainder degrades to an
        // inclusion of the original-trimmed string ("NOT") rather than
        // collapsing to an exclude-everything that would return 0
        // rows. The helper assumes its caller has already trimmed, so
        // we pass "NOT" (3 chars) here — the trim invariant is what
        // makes "NOT " (with trailing space) unreachable from the
        // public-API surface.
        assert_eq!(
            parse_search_term(Some("NOT ")),
            // After the helper's own trim, "NOT" — the strip prefix
            // requires at least 4 bytes, so this falls through to
            // include "NOT".
            Some(SearchTerm::Include("NOT".into()))
        );
    }

    #[test]
    fn parse_search_term_short_strings_are_include() {
        // Any string shorter than 4 bytes can't have a "NOT " prefix.
        assert_eq!(
            parse_search_term(Some("a")),
            Some(SearchTerm::Include("a".into()))
        );
        assert_eq!(
            parse_search_term(Some("NOT")),
            Some(SearchTerm::Include("NOT".into()))
        );
        assert_eq!(
            parse_search_term(Some("not")),
            Some(SearchTerm::Include("not".into()))
        );
    }

    #[test]
    fn parse_search_term_notx_is_include_not_exclude() {
        // 4 chars but no trailing space — first 4 chars are "NOTX",
        // which doesn't equal "NOT " ignoring case → include path.
        assert_eq!(
            parse_search_term(Some("NOTX")),
            Some(SearchTerm::Include("NOTX".into()))
        );
        // Same with 5 chars where the 4th is non-space.
        assert_eq!(
            parse_search_term(Some("NOTOK")),
            Some(SearchTerm::Include("NOTOK".into()))
        );
    }

    #[test]
    fn parse_search_term_multibyte_input_does_not_panic() {
        // The helper uses `s.get(..4)` which never panics on a
        // non-char-boundary slice — it just returns None. Pin the
        // contract so a future refactor can't reintroduce the
        // pre-helper `s[..4]` slice that would panic on a 2-byte
        // emoji.
        assert_eq!(
            parse_search_term(Some("∅Owl")), // 4 bytes (∅) + 3 chars = 6 bytes
            Some(SearchTerm::Include("∅Owl".into()))
        );
        // A pure-multibyte string shorter than 4 bytes.
        assert_eq!(
            parse_search_term(Some("ω")), // 2 bytes
            Some(SearchTerm::Include("ω".into()))
        );
    }

    #[test]
    fn correlation_id_can_be_used_to_pull_one_files_rows() {
        // The operator-facing usage pattern: "given the id from one
        // detection's log slice, give me every row from the same
        // file". This must round-trip exactly.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let cid_a = "e-A";
        let cid_b = "e-B";
        for (cid, offset) in [
            (Some(cid_a), 0.0_f64),
            (Some(cid_a), 4.5),
            (Some(cid_a), 9.0),
            (Some(cid_b), 0.0),
        ] {
            let r = DetectionRecord {
                date: "2026-05-19",
                time: "09:00:00",
                sci_name: "Pica pica",
                com_name: "Eurasian Magpie",
                confidence: 0.9,
                lat: None,
                lon: None,
                cutoff: None,
                week: Some(20),
                sensitivity: None,
                overlap: None,
                file_name: if cid == Some("e-A") { "a.wav" } else { "b.wav" },
                chunk_offset_secs: Some(offset),
                correlation_id: cid,
                source: None,
            };
            insert_detection(&conn, &r).unwrap();
        }

        // The dedicated index from migration 12 lets a future endpoint
        // pull by correlation_id efficiently.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE correlation_id = ?1",
                params![cid_a],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }
}
