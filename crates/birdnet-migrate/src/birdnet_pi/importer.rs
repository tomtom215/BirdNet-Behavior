//! BirdNET-Pi → BirdNet-Behavior data importer.
//!
//! Reads all detections from a BirdNET-Pi `BirdDB.txt` `SQLite` database and
//! inserts them (batch by batch) into the destination `birds.db`.
//!
//! The source file is opened **read-only** and is never modified.
//! Duplicate rows are silently skipped via `INSERT OR IGNORE`.

use rusqlite::{Connection, params};
use std::path::Path;

use crate::error::MigrateError;
use crate::progress::{MigrationProgress, MigrationStage, ProgressHandle};
use crate::schema::{open_source_readonly, row_count};
use crate::traits::{MigrationSummary, Migrator};

/// How many rows to read/write per batch (balances memory and transaction overhead).
const BATCH_SIZE: usize = 500;

/// Intermediate row representation used during transfer.
struct DetectionRow {
    date: String,
    time: String,
    sci_name: String,
    com_name: String,
    confidence: f64,
    lat: Option<f64>,
    lon: Option<f64>,
    cutoff: Option<f64>,
    week: Option<i64>,
    sens: Option<f64>,
    overlap: Option<f64>,
    file_name: Option<String>,
}

/// Migrates BirdNET-Pi detections into a BirdNet-Behavior database.
#[derive(Debug, Clone, Default)]
pub struct BirdNetPiImporter;

impl Migrator for BirdNetPiImporter {
    fn migrate(
        &self,
        source_path: &Path,
        dest_path: &Path,
        progress: &ProgressHandle,
    ) -> Result<MigrationSummary, MigrateError> {
        progress.set_stage(MigrationStage::Importing, "Opening source database");

        let src_conn = open_source_readonly(source_path)?;
        let total = row_count(&src_conn, "detections")?;

        progress.update(MigrationProgress {
            stage: MigrationStage::Importing,
            rows_imported: 0,
            rows_total: total,
            message: format!("Importing {total} detections from BirdNET-Pi"),
            error: None,
        });

        // Open or create the destination database.
        let mut dst_conn = open_or_create_destination(dest_path)?;

        let (imported, skipped) = import_batched(&src_conn, &mut dst_conn, total, progress)?;

        progress.update(MigrationProgress {
            stage: MigrationStage::Complete,
            rows_imported: imported,
            rows_total: total,
            message: format!("Import complete: {imported} rows imported, {skipped} skipped"),
            error: None,
        });

        tracing::info!(
            source = %source_path.display(),
            dest = %dest_path.display(),
            imported,
            skipped,
            "BirdNET-Pi migration complete"
        );

        Ok(MigrationSummary {
            source_rows: total,
            imported_rows: imported,
            skipped_rows: skipped,
            schema_name: "BirdNET-Pi".to_string(),
            source_path: source_path.display().to_string(),
        })
    }
}

/// Open (or create) the destination BirdNet-Behavior database.
fn open_or_create_destination(path: &Path) -> Result<Connection, MigrateError> {
    birdnet_db::sqlite::open_or_create(path).map_err(|e| {
        MigrateError::DestinationOpen(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
            Some(e.to_string()),
        ))
    })
}

/// Perform the batched read-from-source / write-to-dest loop.
///
/// Returns `(imported, skipped)`.
fn import_batched(
    src: &Connection,
    dst: &mut Connection,
    total: u64,
    progress: &ProgressHandle,
) -> Result<(u64, u64), MigrateError> {
    let mut imported = 0_u64;
    let mut skipped = 0_u64;
    let mut offset = 0_u64;

    loop {
        let batch = fetch_batch(src, offset, BATCH_SIZE)?;
        if batch.is_empty() {
            break;
        }

        let batch_len = batch.len() as u64;
        let (ins, sk) = insert_batch(dst, &batch)?;
        imported += ins;
        skipped += sk;
        offset += batch_len;

        progress.update(MigrationProgress {
            stage: MigrationStage::Importing,
            rows_imported: imported,
            rows_total: total,
            message: format!("Imported {imported} / {total} rows"),
            error: None,
        });

        if batch_len < BATCH_SIZE as u64 {
            break; // last batch
        }
    }

    Ok((imported, skipped))
}

/// Read a float column from a BirdNET-Pi row, tolerating dirty data.
///
/// SQLite columns are dynamically typed and upstream BirdNET-Pi wrote
/// whatever Python had on hand, so real-world databases carry TEXT values
/// (usually `""`, sometimes a stringified number) in REAL columns — the
/// same "empty-string poisoning" our own schema migration 11 had to clean
/// up. A strict `row.get::<_, Option<f64>>` aborts the whole import with
/// `InvalidColumnType` on the first such cell; this reader degrades the
/// single cell to `None` (or parses it when it is a stringified number)
/// so one dirty value cannot torpedo a multi-year migration.
fn lenient_f64(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<f64>> {
    use rusqlite::types::ValueRef;
    Ok(match row.get_ref(idx)? {
        ValueRef::Real(f) => Some(f),
        // Coordinates/settings stored as whole numbers; exact for any
        // plausible magnitude (precision only degrades beyond 2^53).
        #[allow(clippy::cast_precision_loss)]
        ValueRef::Integer(i) => Some(i as f64),
        ValueRef::Text(t) => std::str::from_utf8(t)
            .ok()
            .and_then(|s| s.trim().parse().ok()),
        ValueRef::Null | ValueRef::Blob(_) => None,
    })
}

/// Read an integer column from a BirdNET-Pi row, tolerating dirty data.
/// Same rationale as [`lenient_f64`].
fn lenient_i64(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<i64>> {
    use rusqlite::types::ValueRef;
    Ok(match row.get_ref(idx)? {
        ValueRef::Integer(i) => Some(i),
        // A week number stored as REAL (e.g. 23.0) truncates exactly.
        #[allow(clippy::cast_possible_truncation)]
        ValueRef::Real(f) if f.is_finite() => Some(f as i64),
        ValueRef::Text(t) => std::str::from_utf8(t)
            .ok()
            .and_then(|s| s.trim().parse().ok()),
        ValueRef::Real(_) | ValueRef::Null | ValueRef::Blob(_) => None,
    })
}

/// Fetch a page of rows from the source.
fn fetch_batch(
    conn: &Connection,
    offset: u64,
    limit: usize,
) -> Result<Vec<DetectionRow>, MigrateError> {
    let mut stmt = conn
        .prepare(
            "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff,
                    Week, Sens, Overlap, File_Name
             FROM detections
             ORDER BY Date, Time
             LIMIT ?1 OFFSET ?2",
        )
        .map_err(MigrateError::DataTransfer)?;

    let rows = stmt
        .query_map(
            params![
                i64::try_from(limit).unwrap_or(i64::MAX),
                i64::try_from(offset).unwrap_or(i64::MAX)
            ],
            |row| {
                Ok(DetectionRow {
                    date: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    time: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    sci_name: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    com_name: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    confidence: lenient_f64(row, 4)?.unwrap_or(0.0).clamp(0.0, 1.0),
                    lat: lenient_f64(row, 5)?,
                    lon: lenient_f64(row, 6)?,
                    cutoff: lenient_f64(row, 7)?,
                    week: lenient_i64(row, 8)?,
                    sens: lenient_f64(row, 9)?,
                    overlap: lenient_f64(row, 10)?,
                    file_name: row.get(11)?,
                })
            },
        )
        .map_err(MigrateError::DataTransfer)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(MigrateError::DataTransfer)?;

    Ok(rows)
}

/// Insert a batch into the destination inside a single transaction.
///
/// Uses `INSERT OR IGNORE` so duplicate rows are silently skipped.
/// Returns `(inserted, skipped)`.
fn insert_batch(conn: &mut Connection, rows: &[DetectionRow]) -> Result<(u64, u64), MigrateError> {
    let tx = conn.transaction().map_err(MigrateError::DataTransfer)?;

    let mut inserted = 0_u64;

    for row in rows {
        let changes = tx
            .execute(
                "INSERT OR IGNORE INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon,
                  Cutoff, Week, Sens, Overlap, File_Name)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    row.date,
                    row.time,
                    row.sci_name,
                    row.com_name,
                    row.confidence,
                    row.lat,
                    row.lon,
                    row.cutoff,
                    row.week,
                    row.sens,
                    row.overlap,
                    row.file_name,
                ],
            )
            .map_err(MigrateError::DataTransfer)?;
        inserted += changes as u64;
    }

    tx.commit().map_err(MigrateError::DataTransfer)?;

    let batch_len = rows.len() as u64;
    let skipped = batch_len.saturating_sub(inserted);
    Ok((inserted, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    fn make_source(n: usize) -> NamedTempFile {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
        )
        .unwrap();
        for i in 0..n {
            conn.execute(
                "INSERT INTO detections
                   (Date, Time, Sci_Name, Com_Name, Confidence,
                    Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name)
                 VALUES
                    (?1,'06:00:00','Turdus merula','Blackbird',
                     0.9,51.5,-0.1,0.7,1,1.0,0.0,'rec.wav')",
                params![format!("2026-01-{:02}", (i % 28) + 1)],
            )
            .unwrap();
        }
        drop(conn);
        tmp
    }

    #[test]
    fn imports_all_rows() {
        let src = make_source(10);
        let dst = NamedTempFile::new().unwrap();

        let importer = BirdNetPiImporter;
        let handle = ProgressHandle::new();

        let summary = importer.migrate(src.path(), dst.path(), &handle).unwrap();
        assert_eq!(summary.source_rows, 10);
        assert_eq!(summary.imported_rows, 10);
        assert_eq!(summary.skipped_rows, 0);
    }

    #[test]
    fn idempotent_second_import_skips_duplicates() {
        let src = make_source(5);
        let dst = NamedTempFile::new().unwrap();
        let handle = ProgressHandle::new();
        let importer = BirdNetPiImporter;

        // First import
        importer.migrate(src.path(), dst.path(), &handle).unwrap();

        // Second import of the same source → all rows skipped
        let summary2 = importer.migrate(src.path(), dst.path(), &handle).unwrap();
        assert_eq!(summary2.skipped_rows, 5);
        assert_eq!(summary2.imported_rows, 0);
    }

    #[test]
    fn import_empty_source() {
        let src = make_source(0);
        let dst = NamedTempFile::new().unwrap();
        let handle = ProgressHandle::new();
        let importer = BirdNetPiImporter;

        let summary = importer.migrate(src.path(), dst.path(), &handle).unwrap();
        assert_eq!(summary.source_rows, 0);
        assert_eq!(summary.imported_rows, 0);
    }

    /// Real-world BirdNET-Pi databases carry TEXT garbage in numeric columns
    /// (SQLite is dynamically typed; upstream wrote `""` and stringified
    /// numbers into REAL columns). One dirty cell must degrade to NULL, not
    /// abort the whole import with `InvalidColumnType`.
    #[test]
    fn import_tolerates_text_poisoned_numeric_columns() {
        let src = make_source(2);
        {
            let conn = Connection::open(src.path()).unwrap();
            conn.execute(
                "INSERT INTO detections
                   (Date, Time, Sci_Name, Com_Name, Confidence,
                    Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name)
                 VALUES
                    ('2026-02-01','07:00:00','Pica pica','Eurasian Magpie',
                     '0.85', '', ' -0.12 ', NULL, 'garbage', 23.0, '', 'dirty.wav')",
                [],
            )
            .unwrap();
        }
        let dst = NamedTempFile::new().unwrap();
        let handle = ProgressHandle::new();
        let importer = BirdNetPiImporter;

        let summary = importer.migrate(src.path(), dst.path(), &handle).unwrap();
        assert_eq!(summary.source_rows, 3);
        assert_eq!(summary.imported_rows, 3);

        let conn = Connection::open(dst.path()).unwrap();
        let (confidence, lat, lon, week, sens): (
            f64,
            Option<f64>,
            Option<f64>,
            Option<i64>,
            Option<f64>,
        ) = conn
            .query_row(
                "SELECT Confidence, Lat, Lon, Week, Sens
                 FROM detections WHERE File_Name = 'dirty.wav'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        // Stringified numbers parse; empty/garbage strings become NULL; a
        // whole-number REAL week truncates exactly.
        assert!((confidence - 0.85).abs() < f64::EPSILON);
        assert_eq!(lat, None);
        assert_eq!(lon, Some(-0.12));
        assert_eq!(week, None);
        assert_eq!(sens, Some(23.0));
    }
}
