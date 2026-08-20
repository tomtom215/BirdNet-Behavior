//! Reading the record of what was imported, and from where.
//!
//! # Why this exists
//!
//! Migration 25 gave every imported detection an `import_batch_id` and recorded
//! the batch's origin — coordinates, distance from this station, the clock shift
//! applied. Nothing read any of it. The provenance was written and then never
//! looked at by a single query, page or endpoint, which made it a forensic
//! record for someone willing to open the database by hand rather than
//! something the station could tell you.
//!
//! That matters more here than it would elsewhere. `birdnet-migrate` warns
//! *before* an import that the source is 340 km away and on another clock, and
//! it is right not to block — merging two sites is a legitimate thing to want.
//! But once the operator says yes, every location- and hour-dependent analytic
//! reads the union as one station: solar overlays, "first of year", life-list
//! firsts, species-richness curves, phenology. A chart cannot be looked at and
//! judged, because nothing on it says part of it came from somewhere else.
//!
//! These queries are what let a surface say so.

use rusqlite::Connection;

use crate::sqlite::connection::DbError;

/// One recorded import, as the operator would need to see it.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportBatch {
    /// Row id, and the value carried by `detections.import_batch_id`.
    pub id: i64,
    /// When the import ran (`YYYY-MM-DD HH:MM:SS`, UTC).
    pub imported_at: String,
    /// What kind of source it came from (`birdnet-pi`, `csv`, …).
    pub source_kind: String,
    /// The operator's name for the source station, if they gave one.
    pub source_label: Option<String>,
    /// Great-circle distance from this station, km. `None` when either side
    /// had no coordinates — which is itself worth showing, because it means
    /// nothing could be checked.
    pub distance_km: Option<f64>,
    /// Seconds added to every imported timestamp to put the two histories on
    /// one clock. `0` means none was applied.
    pub applied_shift_secs: i64,
    /// Rows the import wrote.
    pub row_count: i64,
    /// Free-text note stored with the batch.
    pub notes: Option<String>,
}

impl ImportBatch {
    /// Whether this batch came from somewhere far enough away to be a different
    /// site.
    ///
    /// The threshold is `birdnet_migrate::provenance::DIFFERENT_SITE_KM` (5 km)
    /// — not a scientific boundary, but the distance past which habitat,
    /// sunrise time and species pool start to differ enough that merging is a
    /// decision rather than a formality. Repeated here rather than imported
    /// because `birdnet-db` does not depend on `birdnet-migrate`; the constant
    /// is asserted equal in `birdnet-migrate`'s own tests.
    #[must_use]
    pub fn is_different_site(&self) -> bool {
        self.distance_km.is_some_and(|km| km > DIFFERENT_SITE_KM)
    }
}

/// Distance past which an import is reported as a different site, in km.
///
/// Must equal `birdnet_migrate::provenance::DIFFERENT_SITE_KM`.
pub const DIFFERENT_SITE_KM: f64 = 5.0;

/// Every recorded import, newest first.
///
/// Returns an empty vector on a station that has never imported anything, which
/// is the common case and not an error.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn list_import_batches(conn: &Connection) -> Result<Vec<ImportBatch>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, imported_at, source_kind, source_label, distance_km,
                applied_shift_secs, row_count, notes
           FROM import_batches
          ORDER BY imported_at DESC, id DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ImportBatch {
            id: row.get(0)?,
            imported_at: row.get(1)?,
            source_kind: row.get(2)?,
            source_label: row.get(3)?,
            distance_km: row.get(4)?,
            applied_shift_secs: row.get(5)?,
            row_count: row.get(6)?,
            notes: row.get(7)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// How many detections currently in the database came from an import.
///
/// Counted from `detections`, not from `import_batches.row_count`: the two
/// diverge as soon as anything is deleted, and the question a surface needs to
/// answer is "how much of what I am looking at is imported", not "how much was
/// written once".
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn imported_detection_count(conn: &Connection) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE import_batch_id IS NOT NULL",
        [],
        |row| row.get(0),
    )
    .map_err(DbError::Sqlite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_or_create;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_or_create(&dir.path().join("birds.db")).unwrap();
        crate::migration::migrate(&conn).unwrap();
        (dir, conn)
    }

    fn insert_batch(conn: &Connection, label: &str, km: Option<f64>, shift: i64, rows: i64) -> i64 {
        conn.execute(
            "INSERT INTO import_batches
               (imported_at, source_kind, source_label, distance_km, applied_shift_secs, row_count)
             VALUES (datetime('now'), 'birdnet-pi', ?1, ?2, ?3, ?4)",
            rusqlite::params![label, km, shift, rows],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn a_station_that_never_imported_reports_nothing() {
        let (_d, conn) = db();
        assert!(list_import_batches(&conn).unwrap().is_empty());
        assert_eq!(imported_detection_count(&conn).unwrap(), 0);
    }

    #[test]
    fn batches_read_back_with_their_origin() {
        let (_d, conn) = db();
        insert_batch(&conn, "Old garden", Some(0.4), 0, 12);
        insert_batch(&conn, "Coastal site", Some(341.0), -21_600, 900);
        let batches = list_import_batches(&conn).unwrap();
        assert_eq!(batches.len(), 2);
        let coastal = batches
            .iter()
            .find(|b| b.source_label.as_deref() == Some("Coastal site"))
            .expect("coastal batch");
        assert_eq!(coastal.row_count, 900);
        assert_eq!(coastal.applied_shift_secs, -21_600);
        assert!(coastal.is_different_site());

        let garden = batches
            .iter()
            .find(|b| b.source_label.as_deref() == Some("Old garden"))
            .expect("garden batch");
        assert!(
            !garden.is_different_site(),
            "400 m is the same site — a moved GPS fix, not another station"
        );
    }

    /// A batch with no coordinates is not "the same site". Nothing could be
    /// checked, and saying otherwise would be the reassuring answer rather than
    /// the true one.
    #[test]
    fn an_unlocated_batch_is_not_claimed_to_be_the_same_site() {
        let (_d, conn) = db();
        insert_batch(&conn, "CSV export", None, 0, 5);
        let b = &list_import_batches(&conn).unwrap()[0];
        assert_eq!(b.distance_km, None);
        assert!(!b.is_different_site());
    }

    /// The count comes from the rows that are actually there.
    ///
    /// `import_batches.row_count` records what an import *wrote*; the two
    /// diverge the moment anything is deleted, and every surface that asks this
    /// question is asking about what it is currently displaying.
    #[test]
    fn the_count_follows_deletions_rather_than_the_recorded_row_count() {
        let (_d, conn) = db();
        let id = insert_batch(&conn, "Coastal site", Some(341.0), 0, 3);
        for time in ["06:00:00", "07:00:00", "08:00:00"] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, import_batch_id)
                 VALUES ('2026-01-01', ?1, 'Turdus merula', 'Eurasian Blackbird', 0.9, ?2)",
                rusqlite::params![time, id],
            )
            .unwrap();
        }
        // One recorded by this station, which must not be counted as imported.
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-01-01', '09:00:00', 'Parus major', 'Great Tit', 0.9)",
            [],
        )
        .unwrap();
        assert_eq!(imported_detection_count(&conn).unwrap(), 3);

        conn.execute("DELETE FROM detections WHERE Time = '06:00:00'", [])
            .unwrap();
        assert_eq!(
            imported_detection_count(&conn).unwrap(),
            2,
            "the count must describe what is in the database now"
        );
        assert_eq!(
            list_import_batches(&conn).unwrap()[0].row_count,
            3,
            "the batch still records what it wrote"
        );
    }
}
