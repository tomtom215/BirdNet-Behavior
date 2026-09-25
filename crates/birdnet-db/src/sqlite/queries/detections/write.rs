//! Detection write queries: insert, delete, and relabel.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;
use crate::sqlite::types::DetectionRecord;

/// Insert a detection record into the database, returning its rowid.
///
/// The rowid is what a follow-up write that happens after the insert — the
/// `BirdWeather` soundscape id, known only once the upload has been answered
/// — keys on; the natural key is five columns of local wall clock and a path.
///
/// # Errors
///
/// Returns `DbError` on insert failure.
pub fn insert_detection(conn: &Connection, record: &DetectionRecord<'_>) -> Result<i64, DbError> {
    // Explicit column list — `VALUES (?1, …, ?12)` without one was a
    // schema-vs-insert drift waiting to happen and broke in production
    // when migration 7 added `is_locked` as a 13th column. Naming the
    // columns means new columns with a DEFAULT (like `is_locked`) keep
    // this write path working unchanged.
    conn.execute(
        "INSERT INTO detections \
         (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, chunk_offset_secs, correlation_id, Source, Duration_Secs, detected_at_utc, run_id, clip_offset_secs, detection_secs, model_id, model_agreement) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
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
            // NULL is not a hole: migration 32's trigger then converts the wall
            // clock through the host's tz database, which is the right answer
            // for any row whose real instant nobody recorded. A live caller
            // passes `Some` because it knows the offset that was in force —
            // see `birdnet_core::civil::unix_secs_from_local`.
            record.detected_at_utc,
            record.run_id,
            record.clip_offset_secs,
            record.detection_secs,
            record.model_id,
            record.model_agreement,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Record the `BirdWeather` soundscape id a detection was posted with
/// (migration 45). Returns `false` if no row has that rowid.
///
/// # Errors
///
/// Returns `DbError` on update failure.
pub fn set_birdweather_soundscape(
    conn: &Connection,
    rowid: i64,
    soundscape_id: u64,
) -> Result<bool, DbError> {
    let id = i64::try_from(soundscape_id).unwrap_or(i64::MAX);
    let changed = conn.execute(
        "UPDATE detections SET birdweather_soundscape_id = ?2 WHERE rowid = ?1",
        params![rowid, id],
    )?;
    Ok(changed > 0)
}

/// One detection row, named precisely enough to tell it from its neighbours.
///
/// `(Date, Time, Sci_Name)` is what most of the UI has carried, and it is not
/// a key. The table is unique on `(Date, Time, Sci_Name,
/// COALESCE(File_Name, ''), chunk_offset_secs)`: two microphones that hear the
/// same bird in the same second write two rows that share the triple and
/// differ only in the clip they point at. A write keyed on the triple reached
/// both, so deleting the one on screen also deleted the other — including a
/// row its owner had locked.
///
/// Adding the clip name separates them. Since migration 24 folded the chunk
/// offset into `Time`, two chunks of one recording land on different seconds,
/// so the four columns here name one row on every path this station writes.
/// `file_name` is compared the way the unique index compares it —
/// `COALESCE(File_Name, '')` — so `None` names a row with no clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectionKey<'a> {
    /// Detection date (`YYYY-MM-DD`).
    pub date: &'a str,
    /// Detection time (`HH:MM:SS`).
    pub time: &'a str,
    /// Scientific name.
    pub sci_name: &'a str,
    /// The row's `File_Name`, as read back; `None` for a row with no clip.
    pub file_name: Option<&'a str>,
}

/// The `WHERE` clause a [`DetectionKey`] binds as `?1`–`?4`.
pub(super) const KEY_WHERE: &str = "Date = ?1 AND Time = ?2 AND Sci_Name = ?3 \
     AND COALESCE(File_Name, '') = COALESCE(?4, '')";

/// Delete the one detection `key` names.
///
/// Deliberately does not consult the lock: the operator is looking at the
/// row they named (see `birdnet-web`'s `admin/species/manage.rs`). What the
/// lock protects against is a delete that reaches rows nobody named, and a
/// precise key is what stops that.
///
/// Returns `true` if a row was deleted, `false` if no row matched.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn delete_detection_at(conn: &Connection, key: &DetectionKey<'_>) -> Result<bool, DbError> {
    let changed = conn.execute(
        &format!("DELETE FROM detections WHERE {KEY_WHERE}"),
        params![key.date, key.time, key.sci_name, key.file_name],
    )?;
    Ok(changed > 0)
}

/// Re-label the one detection `key` names.
///
/// Returns `true` if a row was updated, `false` if no row matched.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn relabel_detection_at(
    conn: &Connection,
    key: &DetectionKey<'_>,
    new_sci_name: &str,
    new_com_name: &str,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        &format!("UPDATE detections SET Sci_Name = ?5, Com_Name = ?6 WHERE {KEY_WHERE}"),
        params![
            key.date,
            key.time,
            key.sci_name,
            key.file_name,
            new_sci_name,
            new_com_name
        ],
    )?;
    Ok(changed > 0)
}

/// Delete a detection by date, time, and scientific name.
///
/// **Superseded by [`delete_detection_at`]**, which names one row. The triple
/// is not unique — two sources hearing one bird in the same second share it —
/// so this can match more than the row the caller meant. When it does, rows
/// that are locked are kept: a lock is the operator saying "not this one",
/// and a delete aimed at a different row is exactly what it is for. When the
/// triple names a single row it is deleted whether locked or not, as before —
/// then the caller did name it.
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
        "DELETE FROM detections WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3 \
           AND (COALESCE(is_locked, 0) = 0 \
                OR (SELECT COUNT(*) FROM detections \
                     WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3) = 1)",
        params![date, time, sci_name],
    )?;
    Ok(changed > 0)
}

/// Re-label a detection by changing its species identification.
///
/// **Superseded by [`relabel_detection_at`]**: the triple can match a second
/// source's row of the same second, and this re-labels both.
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

    /// DD-32: the id lands on the row the rowid names, and only there.
    #[test]
    fn the_soundscape_id_is_written_by_rowid() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let record = DetectionRecord {
            model_id: None,
            model_agreement: None,
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
            file_name: "a.wav",
            chunk_offset_secs: Some(0.0),
            correlation_id: None,
            source: None,
            duration_secs: None,
            detected_at_utc: None,
            run_id: None,
            clip_offset_secs: None,
            detection_secs: None,
        };
        let first = insert_detection(&conn, &record).unwrap();
        let second = insert_detection(
            &conn,
            &DetectionRecord {
                model_id: None,
                model_agreement: None,
                time: "08:31:00",
                ..record.clone()
            },
        )
        .unwrap();
        assert_ne!(first, second);
        assert!(set_birdweather_soundscape(&conn, second, 4242).unwrap());
        assert!(!set_birdweather_soundscape(&conn, second + 100, 1).unwrap());
        let ids: Vec<Option<i64>> = conn
            .prepare("SELECT birdweather_soundscape_id FROM detections ORDER BY rowid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(ids, vec![None, Some(4242)]);
    }

    #[test]
    fn insert_and_count() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let record = DetectionRecord {
            model_id: None,
            model_agreement: None,
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
            detected_at_utc: None,
            run_id: None,
            clip_offset_secs: None,
            detection_secs: None,
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
            model_id: None,
            model_agreement: None,
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
            detected_at_utc: None,
            run_id: None,
            clip_offset_secs: None,
            detection_secs: None,
        };
        // A second row at a different second with no source = the historical
        // shape (e.g. an imported BirdNET-Pi row).
        let untagged = DetectionRecord {
            model_id: None,
            model_agreement: None,
            time: "06:00:01",
            source: None,
            duration_secs: None,
            detected_at_utc: None,
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

    /// Two sources, one bird, one second: the rows share `(Date, Time,
    /// Sci_Name)` and differ only in their clip. Returns the connection with
    /// `cam1`'s row locked.
    fn two_sources_one_second() -> (tempfile::NamedTempFile, Connection) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        for (file, source) in [("cam1.wav", "cam1"), ("cam2.wav", "cam2")] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, Source)
                 VALUES ('2026-05-01', '06:00:00', 'Turdus merula', 'Blackbird', 0.9, ?1, ?2)",
                params![file, source],
            )
            .unwrap();
        }
        conn.execute(
            "UPDATE detections SET is_locked = 1 WHERE File_Name = 'cam1.wav'",
            [],
        )
        .unwrap();
        (tmp, conn)
    }

    fn files_left(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT File_Name FROM detections ORDER BY File_Name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    const CAM2: DetectionKey<'static> = DetectionKey {
        date: "2026-05-01",
        time: "06:00:00",
        sci_name: "Turdus merula",
        file_name: Some("cam2.wav"),
    };

    /// Finding 1: deleting one source's row must not delete the other's.
    #[test]
    fn a_keyed_delete_removes_only_the_row_it_names() {
        let (_tmp, conn) = two_sources_one_second();
        assert!(delete_detection_at(&conn, &CAM2).unwrap());
        assert_eq!(files_left(&conn), vec!["cam1.wav".to_string()]);
        // Counterpart: a key that names nothing deletes nothing.
        assert!(!delete_detection_at(&conn, &CAM2).unwrap());
        assert_eq!(files_left(&conn), vec!["cam1.wav".to_string()]);
    }

    /// Finding 1: the legacy triple delete, aimed at the unlocked row, must
    /// keep the locked sibling it cannot tell apart.
    #[test]
    fn a_triple_delete_keeps_a_locked_sibling() {
        let (_tmp, conn) = two_sources_one_second();
        assert!(delete_detection(&conn, "2026-05-01", "06:00:00", "Turdus merula").unwrap());
        assert_eq!(files_left(&conn), vec!["cam1.wav".to_string()]);
    }

    /// Counterpart: when the triple names exactly one row, the operator named
    /// it, and a lock does not stop them (the documented single-row rule).
    #[test]
    fn a_triple_delete_of_a_lone_locked_row_still_deletes_it() {
        let (_tmp, conn) = two_sources_one_second();
        assert!(delete_detection_at(&conn, &CAM2).unwrap());
        assert!(delete_detection(&conn, "2026-05-01", "06:00:00", "Turdus merula").unwrap());
        assert!(files_left(&conn).is_empty());
    }

    /// Finding 1: re-labelling one source's row leaves the other's species.
    #[test]
    fn a_keyed_relabel_changes_only_the_row_it_names() {
        let (_tmp, conn) = two_sources_one_second();
        assert!(relabel_detection_at(&conn, &CAM2, "Turdus philomelos", "Song Thrush").unwrap());
        let species: Vec<(String, String)> = conn
            .prepare("SELECT File_Name, Sci_Name FROM detections ORDER BY File_Name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            species,
            vec![
                ("cam1.wav".to_string(), "Turdus merula".to_string()),
                ("cam2.wav".to_string(), "Turdus philomelos".to_string()),
            ]
        );
    }

    /// A clip-less row is named with `file_name: None`, as the unique index
    /// names it, and not confused with a row that has a clip.
    #[test]
    fn a_key_without_a_clip_names_the_clipless_row() {
        let (_tmp, conn) = two_sources_one_second();
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-05-01', '06:00:00', 'Turdus merula', 'Blackbird', 0.9)",
            [],
        )
        .unwrap();
        let clipless = DetectionKey {
            file_name: None,
            ..CAM2
        };
        assert!(delete_detection_at(&conn, &clipless).unwrap());
        assert_eq!(
            files_left(&conn),
            vec!["cam1.wav".to_string(), "cam2.wav".to_string()]
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
            model_id: None,
            model_agreement: None,
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
            detected_at_utc: None,
            run_id: None,
            clip_offset_secs: None,
            detection_secs: None,
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
            model_id: None,
            model_agreement: None,
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
            detected_at_utc: None,
            run_id: None,
            clip_offset_secs: None,
            detection_secs: None,
        };
        insert_detection(&conn, &base).unwrap();
        let chunk2 = DetectionRecord {
            model_id: None,
            model_agreement: None,
            chunk_offset_secs: Some(4.5),
            ..base.clone()
        };
        insert_detection(&conn, &chunk2).unwrap();
        let chunk3 = DetectionRecord {
            model_id: None,
            model_agreement: None,
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
            model_id: None,
            model_agreement: None,
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
            detected_at_utc: None,
            run_id: None,
            clip_offset_secs: None,
            detection_secs: None,
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
                model_id: None,
                model_agreement: None,
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
                detected_at_utc: None,
                run_id: None,
                clip_offset_secs: None,
                detection_secs: None,
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
            model_id: None,
            model_agreement: None,
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
            detected_at_utc: None,
            run_id: None,
            clip_offset_secs: None,
            detection_secs: None,
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
