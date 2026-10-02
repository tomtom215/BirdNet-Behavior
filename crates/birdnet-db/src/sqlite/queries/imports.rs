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

use rusqlite::{Connection, params};

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

/// Remove an import batch and every unlocked detection it brought in.
///
/// # Why an import has to be reversible
///
/// Until this existed, importing another station's history was a one-way door.
/// `provenance.rs` profiles the source, compares its modal coordinate to this
/// station's and warns past [`DIFFERENT_SITE_KM`] — and then the operator's only
/// options were to accept the merge forever or to wipe the whole database. Its
/// own module doc states the reason that matters: the damage is not detectable
/// after the fact, so a dataset that merged two sites cannot be repaired, only
/// discarded. That argument is only bearable if "discarded" means *this import*
/// rather than *everything*.
///
/// # What is removed, and what is deliberately not
///
/// Every unlocked `detections` row tagged with `batch_id`, then — once none
/// remain — the `import_batches` row itself. The `species_summary` triggers fire on the delete, so the maintained
/// rollup follows without a rebuild — checked rather than assumed, in
/// `import_undo.rs`.
///
/// Rows recorded *locally* are never touched: the `WHERE` is on
/// `import_batch_id`, which is NULL for every detection this station heard
/// itself. That is the whole safety property, and the reason the column was
/// added in migration 25 rather than the import being tracked in a side table.
///
/// **Locked rows are kept.** Removing an import is a bulk action, and every
/// bulk action in this station skips what the operator locked (see
/// `birdnet-web`'s `admin/species/manage.rs`): a lock on an imported record
/// is the operator saying "keep this one" about a row nobody is looking at
/// when they press "remove import". While any remain the `import_batches`
/// row stays too — they reference it, and they are still attributable to it.
///
/// Verdicts in `detection_reviews` on the removed rows go with them. They key
/// on `(date, time, sci_name)`, so a verdict left behind judged nothing and
/// still listed on the review page, and a re-import would not bring it back
/// into force: `review_verdict` is written by the review, not by the import.
/// A verdict whose triple still names a detection that stays is kept.
/// `detection_comments` are append-only records of what someone said, whose
/// deletion is audited one by one; they are left for the operator to remove.
///
/// The `DuckDB` analytics copy is a separate store and is **not** reached from
/// here; the caller mirrors the delete with
/// `AnalyticsDb::delete_import_batch`, exactly as it already does for a single
/// deleted detection.
///
/// # Errors
///
/// Returns `DbError` if the transaction cannot be opened or a statement
/// fails. The whole removal is one transaction, so a failure leaves the import
/// intact rather than half-removed.
pub fn remove_import_batch(conn: &Connection, batch_id: i64) -> Result<ImportRemoval, DbError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS import_removal_keys (
            date TEXT NOT NULL, time TEXT NOT NULL, sci_name TEXT NOT NULL);
         DELETE FROM import_removal_keys;",
    )?;
    tx.execute(
        "INSERT INTO import_removal_keys
         SELECT DISTINCT Date, Time, Sci_Name FROM detections
          WHERE import_batch_id = ?1 AND COALESCE(is_locked, 0) = 0",
        params![batch_id],
    )?;
    let deleted = tx.execute(
        "DELETE FROM detections WHERE import_batch_id = ?1 AND COALESCE(is_locked, 0) = 0",
        params![batch_id],
    )?;
    tx.execute(
        "DELETE FROM detection_reviews
          WHERE EXISTS (SELECT 1 FROM import_removal_keys k
                         WHERE k.date = detection_reviews.date
                           AND k.time = detection_reviews.time
                           AND k.sci_name = detection_reviews.sci_name)
            AND NOT EXISTS (SELECT 1 FROM detections d
                             WHERE d.Date = detection_reviews.date
                               AND d.Time = detection_reviews.time
                               AND d.Sci_Name = detection_reviews.sci_name)",
        [],
    )?;
    tx.execute_batch("DROP TABLE import_removal_keys;")?;
    let kept_locked: i64 = tx.query_row(
        "SELECT COUNT(*) FROM detections WHERE import_batch_id = ?1",
        params![batch_id],
        |row| row.get(0),
    )?;
    if kept_locked == 0 {
        tx.execute(
            "DELETE FROM import_batches WHERE id = ?1",
            params![batch_id],
        )?;
    }
    tx.commit()?;
    Ok(ImportRemoval {
        deleted: u64::try_from(deleted).unwrap_or(0),
        kept_locked: u64::try_from(kept_locked).unwrap_or(0),
    })
}

/// What [`remove_import_batch`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportRemoval {
    /// Detections removed.
    pub deleted: u64,
    /// Detections of the batch kept because they are locked. Non-zero means
    /// the batch record was kept too, and the operator should be told why the
    /// import is still listed.
    pub kept_locked: u64,
}

/// Remove an import batch, returning how many detections were removed.
///
/// [`remove_import_batch`] with only the deleted count — kept for existing
/// callers; a caller that reports the result should use that one, so it can
/// say how many locked rows were kept.
///
/// # Errors
///
/// As [`remove_import_batch`].
pub fn delete_import_batch(conn: &Connection, batch_id: i64) -> Result<u64, DbError> {
    remove_import_batch(conn, batch_id).map(|r| r.deleted)
}

/// How many detections a batch currently accounts for.
///
/// Counted live rather than read from `import_batches.row_count`, for the same
/// reason as [`imported_detection_count`]: the recorded count is what was
/// written once, and the question a confirmation dialog has to answer is how
/// much is about to disappear.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn import_batch_row_count(conn: &Connection, batch_id: i64) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE import_batch_id = ?1",
        params![batch_id],
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
