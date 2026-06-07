//! Detection read queries: counts, listings, pagination, today's feed, and
//! the multi-stream corroboration lookup.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;
use crate::sqlite::types::{ConcurrentDetection, DETECTION_COLS, DetectionRow, map_detection_row};

use super::search::{SearchTerm, parse_search_term};

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

/// Detections of the same species from **other** audio sources within
/// `window_secs` of the given detection's instant.
///
/// Powers the multi-stream "also heard by" corroboration display — purely
/// read-only, never collapses or hides anything. Only rows with a non-NULL
/// `Source` different from `exclude_source` are returned, ordered by closeness
/// in time. A ±1-day `Date` bound keeps the scan tight (and correct across a
/// midnight-straddling window) so a very common species stays cheap.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn concurrent_detections_from_other_sources(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
    exclude_source: &str,
    window_secs: f64,
    limit: u32,
) -> Result<Vec<ConcurrentDetection>, DbError> {
    let sql = "SELECT Source, Time, Confidence FROM detections \
               WHERE Sci_Name = ?1 \
                 AND Source IS NOT NULL AND Source <> ?2 \
                 AND Date >= date(?3, '-1 day') AND Date <= date(?3, '+1 day') \
                 AND ABS((julianday(Date || ' ' || Time) - julianday(?3 || ' ' || ?4)) * 86400.0) <= ?5 \
               ORDER BY ABS(julianday(Date || ' ' || Time) - julianday(?3 || ' ' || ?4)) ASC, Source \
               LIMIT ?6";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(
            params![sci_name, exclude_source, date, time, window_secs, limit],
            |row| {
                Ok(ConcurrentDetection {
                    source: row.get(0)?,
                    time: row.get(1)?,
                    confidence: row.get(2)?,
                })
            },
        )?
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
    max_rows: u32,
) -> Result<(Vec<DetectionRow>, bool), DbError> {
    // Bound peak memory: the (public, unauthenticated) export endpoints load
    // the whole result set into a `Vec` and build the entire CSV string from
    // it, so an unfiltered export on a long-running station (millions of rows)
    // would OOM a small Pi. Fetch one row past `max_rows` to detect truncation
    // without a separate COUNT; callers surface an error and ask the user to
    // narrow the date range. `max_rows + 1` is a typed integer interpolated
    // into `LIMIT` — no injection surface.
    let fetch = u64::from(max_rows).saturating_add(1);
    let (where_sql, param_values): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match (from, to) {
        (Some(f), Some(t)) => (
            "WHERE Date >= ?1 AND Date <= ?2",
            vec![Box::new(f.to_string()), Box::new(t.to_string())],
        ),
        (Some(f), None) => ("WHERE Date >= ?1", vec![Box::new(f.to_string())]),
        (None, Some(t)) => ("WHERE Date <= ?1", vec![Box::new(t.to_string())]),
        (None, None) => ("", vec![]),
    };
    let sql = format!(
        "SELECT {DETECTION_COLS} FROM detections {where_sql} \
         ORDER BY Date DESC, Time DESC LIMIT {fetch}"
    );

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt
        .query_map(params_ref.as_slice(), map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let truncated = u64::try_from(rows.len()).unwrap_or(u64::MAX) > u64::from(max_rows);
    if truncated {
        rows.truncate(max_rows as usize);
    }
    Ok((rows, truncated))
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
    use crate::sqlite::queries::detections::insert_detection;
    use crate::sqlite::queries::detections::test_support::temp_db_with_data;
    use crate::sqlite::types::DetectionRecord;

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
    fn concurrent_detections_finds_other_sources_within_window() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let insert = |time: &str, sci: &str, src: Option<&str>, conf: f64| {
            let r = DetectionRecord {
                date: "2026-05-19",
                time,
                sci_name: sci,
                com_name: "X",
                confidence: conf,
                lat: None,
                lon: None,
                cutoff: None,
                week: None,
                sensitivity: None,
                overlap: None,
                file_name: "f.wav",
                chunk_offset_secs: Some(0.0),
                correlation_id: None,
                source: src,
            };
            insert_detection(&conn, &r).unwrap();
        };
        // "This" detection on cam1, plus: a concurrent cam2 (within window),
        // a far-off cam3 (outside window), a same-source cam1 (excluded), and a
        // different species on cam2 (excluded).
        insert("06:00:00", "Pica pica", Some("cam1"), 0.90);
        insert("06:00:05", "Pica pica", Some("cam2"), 0.85);
        insert("06:02:00", "Pica pica", Some("cam3"), 0.80);
        insert("06:00:03", "Pica pica", Some("cam1"), 0.70);
        insert("06:00:04", "Erithacus rubecula", Some("cam2"), 0.95);

        let hits = concurrent_detections_from_other_sources(
            &conn,
            "2026-05-19",
            "06:00:00",
            "Pica pica",
            "cam1",
            30.0,
            8,
        )
        .unwrap();
        assert_eq!(
            hits.len(),
            1,
            "only cam2 qualifies (same species, within 30 s, other source)"
        );
        assert_eq!(hits[0].source, "cam2");
        assert_eq!(hits[0].time, "06:00:05");
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
        let (rows, truncated) = all_detections(&conn, None, None, 10_000).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(!truncated);
    }

    #[test]
    fn all_detections_date_range() {
        let (_tmp, conn) = temp_db_with_data();
        let (rows, _) =
            all_detections(&conn, Some("2026-03-11"), Some("2026-03-11"), 10_000).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn all_detections_from_only() {
        let (_tmp, conn) = temp_db_with_data();
        let (rows, _) = all_detections(&conn, Some("2026-03-11"), None, 10_000).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn all_detections_to_only() {
        let (_tmp, conn) = temp_db_with_data();
        let (rows, _) = all_detections(&conn, None, Some("2026-03-10"), 10_000).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn all_detections_truncates_at_max_rows() {
        let (_tmp, conn) = temp_db_with_data();
        // 4 rows exist; cap at 2 → truncated, exactly 2 returned.
        let (rows, truncated) = all_detections(&conn, None, None, 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(truncated);
        // Cap exactly at the row count → not truncated.
        let (rows, truncated) = all_detections(&conn, None, None, 4).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(!truncated);
    }

    #[test]
    fn detections_by_species_filters() {
        let (_tmp, conn) = temp_db_with_data();
        let rows = detections_by_species(&conn, "Eurasian Blackbird", 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|d| d.com_name == "Eurasian Blackbird"));
    }

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
    fn recent_detections_page_pagination_terminates() {
        // Pin that the page beyond the data is empty (boundary case
        // that mutation testing on `LIMIT ?1 OFFSET ?2` would otherwise
        // flip).
        let (_tmp, conn) = temp_db_with_data();
        let beyond = recent_detections_page(&conn, 10, 100).unwrap();
        assert!(beyond.is_empty());
    }
}
