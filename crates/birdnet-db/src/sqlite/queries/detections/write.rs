//! Detection write queries: insert, delete, and relabel.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;
use crate::sqlite::types::DetectionRecord;

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
         (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, chunk_offset_secs, correlation_id, Source, Duration_Secs) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
            record.duration_secs,
        ],
    )?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;
    use crate::sqlite::queries::detections::test_support::temp_db_with_data;
    // Read queries the write tests assert through.
    use crate::sqlite::queries::detections::{
        detection_count, detections_by_species, recent_detections,
    };

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
            duration_secs: None,
        };
        insert_detection(&conn, &record).unwrap();
        assert_eq!(detection_count(&conn).unwrap(), 1);
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
            duration_secs: None,
        };
        // A second row at a different second with no source = the historical
        // shape (e.g. an imported BirdNET-Pi row).
        let untagged = DetectionRecord {
            time: "06:00:01",
            source: None,
            duration_secs: None,
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
            duration_secs: None,
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
            duration_secs: None,
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
            duration_secs: None,
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
                duration_secs: None,
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

    #[test]
    fn duration_secs_round_trips_through_insert_and_read() {
        // Migration 20: the saved clip's length is persisted and read back
        // exactly, so the Recordings grid can show a real duration.
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
            correlation_id: None,
            source: None,
            duration_secs: Some(15.0),
        };
        insert_detection(&conn, &record).unwrap();
        let rows = recent_detections(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].duration_secs, Some(15.0));
    }

    #[test]
    fn duration_secs_null_when_record_omits_it() {
        // Historical / imported / quarantine-approve rows have no clip length
        // to record and stay NULL — never a faked value (migration 20).
        let (_tmp, conn) = temp_db_with_data();
        let rows = recent_detections(&conn, 10).unwrap();
        assert!(rows.iter().all(|r| r.duration_secs.is_none()));
    }
}
