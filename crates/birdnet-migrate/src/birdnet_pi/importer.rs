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
use crate::provenance::{ImportOptions, SourceProfile};
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

impl BirdNetPiImporter {
    /// Import with explicit reconciliation options.
    ///
    /// [`Migrator::migrate`] delegates here with [`ImportOptions::default`],
    /// which shifts nothing and records a batch whose `applied_shift_secs` is 0.
    ///
    /// Two things happen here that plain `migrate` never did:
    ///
    /// * **The clock is reconciled.** BirdNET-Pi stores local wall-clock with no
    ///   offset recorded, so a history from another timezone is otherwise
    ///   re-read as this station's local time. `options.shift_secs` is applied
    ///   once, to every imported timestamp, before the row is written — so the
    ///   hour-of-day analytics see one clock rather than two.
    /// * **Provenance is recorded.** Every imported row is tagged with an
    ///   `import_batches` row carrying the source's coordinates, the station's,
    ///   the distance between them and the shift applied. `import_batch_id IS
    ///   NULL` continues to mean "this station recorded it", so nothing that
    ///   already exists changes meaning.
    ///
    /// # Errors
    ///
    /// Returns `MigrateError` on any database or I/O failure.
    pub fn migrate_with_options(
        &self,
        source_path: &Path,
        dest_path: &Path,
        progress: &ProgressHandle,
        options: &ImportOptions,
        station: (Option<f64>, Option<f64>),
    ) -> Result<MigrationSummary, MigrateError> {
        progress.set_stage(MigrationStage::Importing, "Opening source database");

        let src_conn = open_source_readonly(source_path)?;
        let total = row_count(&src_conn, "detections")?;
        let profile = SourceProfile::from_connection(&src_conn);

        progress.update(MigrationProgress {
            stage: MigrationStage::Importing,
            rows_imported: 0,
            rows_total: total,
            message: format!("Importing {total} detections from BirdNET-Pi"),
            error: None,
        });

        let mut dst_conn = open_or_create_destination(dest_path)?;
        let batch_id = record_import_batch(&dst_conn, source_path, &profile, options, station);

        let (imported, skipped) =
            import_batched_tagged(&src_conn, &mut dst_conn, total, progress, options, batch_id)?;

        if let Some(id) = batch_id {
            let _ = dst_conn.execute(
                "UPDATE import_batches SET row_count = ?1 WHERE id = ?2",
                params![i64::try_from(imported).unwrap_or(i64::MAX), id],
            );
        }

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
            shift_secs = options.shift_secs,
            batch_id,
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

/// Insert the `import_batches` row this import's detections will point at.
///
/// Returns `None` — and imports untagged, exactly as before — when the
/// destination predates migration 25. That is deliberate: an older database is
/// a database whose rows are all local recordings, so "untagged" is the honest
/// answer, and refusing to import into it would be worse than importing without
/// provenance.
fn record_import_batch(
    dst: &Connection,
    source_path: &Path,
    profile: &SourceProfile,
    options: &ImportOptions,
    station: (Option<f64>, Option<f64>),
) -> Option<i64> {
    let (station_lat, station_lon) = station;
    let distance = profile.distance_km_to(station_lat, station_lon);

    let inserted = dst.execute(
        "INSERT INTO import_batches
            (source_kind, source_label, source_path, source_lat, source_lon,
             station_lat, station_lon, distance_km, source_utc_offset_secs,
             applied_shift_secs, row_count, notes)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,?11)",
        params![
            "birdnet-pi-sqlite",
            options.label.as_deref(),
            source_path.display().to_string(),
            profile.modal_lat,
            profile.modal_lon,
            station_lat,
            station_lon,
            distance,
            options.source_utc_offset_secs,
            options.shift_secs,
            options.notes.as_deref(),
        ],
    );

    match inserted {
        Ok(_) => Some(dst.last_insert_rowid()),
        // No `import_batches` table: a pre-migration-25 destination. Importing
        // untagged is the right answer there — every row in such a database is
        // a local recording, so "untagged" is true — and refusing the import
        // would be worse than importing without provenance.
        Err(e) => {
            tracing::warn!(error = %e, "import provenance not recorded (destination predates migration 25)");
            None
        }
    }
}

/// Shift a `(date, time)` pair by `secs`, in place, using SQLite's own date
/// arithmetic.
///
/// SQLite rather than hand-rolled arithmetic because the destination's every
/// other date operation goes through SQLite, and a second implementation of
/// calendar arithmetic is a second thing to get wrong at a month boundary.
///
/// A row whose `Date`/`Time` name no point in time is returned **unchanged**
/// rather than dropped or zeroed. Those rows already exist in real BirdNET-Pi
/// databases (a NULL `Date` arrives as `""`), they are already excluded from
/// every time-bucketed analytic, and silently rewriting them to some epoch
/// would turn "unplaceable" into "placed, wrongly".
fn shift_timestamp(conn: &Connection, date: &str, time: &str, secs: i64) -> (String, String) {
    if secs == 0 {
        return (date.to_owned(), time.to_owned());
    }
    let shifted: Option<(String, String)> = conn
        .query_row(
            "SELECT strftime('%Y-%m-%d', datetime(?1 || ' ' || ?2, ?3)),
                    strftime('%H:%M:%S', datetime(?1 || ' ' || ?2, ?3))",
            params![date, time, format!("{secs} seconds")],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .ok()
        .and_then(|(d, t)| Some((d?, t?)));
    shifted.unwrap_or_else(|| (date.to_owned(), time.to_owned()))
}

/// Convert one source-local timestamp onto *this* station's clock.
///
/// # Why this replaced a flat shift
///
/// The importer used to add a single number of seconds to every row, computed
/// once as `destination_offset_now − source_offset`. Both halves of that are
/// frozen at import time, and a history is years long, so the result is an hour
/// out for however much of it fell under a different daylight-saving regime.
///
/// This converts instead: the source wall clock minus the source's offset is the
/// real UTC instant, and SQLite's `'localtime'` modifier renders that instant in
/// the host's zone **for that instant** — so the destination half is exact on
/// both sides of every transition. Verified against a `Europe/Berlin` host
/// importing a UTC+0 source, six representative timestamps:
///
/// ```text
/// source (UTC+0)        flat shift            per-row            truth
/// 2024-01-15 06:00:00   2024-01-15 08:00 ✗    2024-01-15 07:00   07:00
/// 2024-07-15 06:00:00   2024-07-15 08:00      2024-07-15 08:00   08:00
/// 2024-03-31 00:30:00   2024-03-31 02:30 ✗    2024-03-31 01:30   01:30
/// 2024-03-31 01:30:00   2024-03-31 03:30      2024-03-31 03:30   03:30
/// 2024-10-27 00:30:00   2024-10-27 02:30      2024-10-27 02:30   02:30
/// 2024-10-27 01:30:00   2024-10-27 03:30 ✗    2024-10-27 02:30   02:30
/// ```
///
/// Three of six wrong before, none after. (Migration 32's comment established
/// separately that SQLite's tz modifiers do resolve per-date rather than to
/// today's offset: Europe/Berlin gives +1 h for a January timestamp and +2 h for
/// a July one.)
///
/// # What it still cannot do
///
/// `src_offset_secs` is one number for the whole history, so if the *source*
/// station observed daylight saving, its summer rows carry a real offset this
/// number does not describe and land an hour out. That half needs the source's
/// IANA zone and a time-zone database. The improvement here is that the
/// destination half stopped contributing its own, independent hour of error.
///
/// A row whose `Date`/`Time` name no point in time — BirdNET-Pi's columns are
/// free-form `TEXT` — is returned unchanged rather than dropped, which is what
/// the flat shift did too.
fn to_local_here(
    conn: &Connection,
    date: &str,
    time: &str,
    src_offset_secs: i64,
) -> (String, String) {
    // `strftime('%s', …)` reads its argument as UTC, so subtracting the source's
    // offset turns "wall clock at the source" into the real instant.
    let converted: Option<(String, String)> = conn
        .query_row(
            "SELECT strftime('%Y-%m-%d', datetime(strftime('%s', ?1 || ' ' || ?2) - ?3,
                                                  'unixepoch', 'localtime')),
                    strftime('%H:%M:%S', datetime(strftime('%s', ?1 || ' ' || ?2) - ?3,
                                                  'unixepoch', 'localtime'))",
            params![date, time, src_offset_secs],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .ok()
        .and_then(|(d, t)| Some((d?, t?)));
    converted.unwrap_or_else(|| (date.to_owned(), time.to_owned()))
}

/// The batched loop, applying `options.shift_secs` and tagging with `batch_id`.
fn import_batched_tagged(
    src: &Connection,
    dst: &mut Connection,
    total: u64,
    progress: &ProgressHandle,
    options: &ImportOptions,
    batch_id: Option<i64>,
) -> Result<(u64, u64), MigrateError> {
    let mut imported = 0_u64;
    let mut skipped = 0_u64;
    let mut offset = 0_u64;

    loop {
        let mut batch = fetch_batch(src, offset, BATCH_SIZE)?;
        if batch.is_empty() {
            break;
        }
        if options.shifts_time() {
            for row in &mut batch {
                // The source's offset wins when it is given: it drives a real
                // per-row conversion, where `shift_secs` can only add a constant.
                let (d, t) = options.source_utc_offset_secs.map_or_else(
                    || shift_timestamp(dst, &row.date, &row.time, options.shift_secs),
                    |src| to_local_here(dst, &row.date, &row.time, src),
                );
                row.date = d;
                row.time = t;
            }
        }

        let batch_len = batch.len() as u64;
        let (ins, sk) = insert_batch_tagged(dst, &batch, batch_id)?;
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
            break;
        }
    }

    Ok((imported, skipped))
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
fn insert_batch_tagged(
    conn: &mut Connection,
    rows: &[DetectionRow],
    batch_id: Option<i64>,
) -> Result<(u64, u64), MigrateError> {
    let Some(id) = batch_id else {
        // Pre-migration-25 destination: no column to tag with.
        return insert_batch(conn, rows);
    };
    let tx = conn.transaction().map_err(MigrateError::DataTransfer)?;
    let mut inserted = 0_u64;
    for row in rows {
        let changes = tx
            .execute(
                "INSERT OR IGNORE INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon,
                  Cutoff, Week, Sens, Overlap, File_Name, import_batch_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
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
                    id,
                ],
            )
            .map_err(MigrateError::DataTransfer)?;
        inserted += changes as u64;
    }
    tx.commit().map_err(MigrateError::DataTransfer)?;
    let batch_len = rows.len() as u64;
    Ok((inserted, batch_len.saturating_sub(inserted)))
}

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

    /// A source row whose `Date` names no point in time is imported, not
    /// skipped — and a NULL `Date` arrives as `""`.
    ///
    /// This is the front half of the mechanism that emptied the analytics
    /// dashboards: `Option<String>::unwrap_or_default()` turns a NULL `Date`
    /// into the empty string, our `Date TEXT NOT NULL` accepts it (the column
    /// type forbids NULL, not nonsense), and the OLAP sync copies it onward,
    /// where a plain `CAST` in `detections_ts` used to abort every query that
    /// touched a timestamp. The behaviour is pinned rather than changed:
    /// dropping a station's detections during a migration is worse than
    /// carrying rows the dashboards cannot place, so the fix is that the OLAP
    /// view tolerates them, the validator says so, and the count is
    /// reportable. If this test ever has to change, that trade-off is being
    /// revisited deliberately.
    #[test]
    fn rows_with_unparseable_dates_are_imported_not_skipped() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections VALUES ('2026-01-01','06:00:00','Turdus merula','Blackbird',0.9,NULL,NULL,NULL,NULL,NULL,NULL,'a.wav');
             INSERT INTO detections VALUES (NULL,NULL,'Parus major','Great Tit',0.8,NULL,NULL,NULL,NULL,NULL,NULL,'b.wav');
             INSERT INTO detections VALUES ('','','Erithacus rubecula','Robin',0.7,NULL,NULL,NULL,NULL,NULL,NULL,'c.wav');
             INSERT INTO detections VALUES ('not-a-date','25:99:99','Corvus corax','Raven',0.6,NULL,NULL,NULL,NULL,NULL,NULL,'d.wav');",
        )
        .unwrap();
        drop(conn);

        let dst = NamedTempFile::new().unwrap();
        let handle = ProgressHandle::new();
        let summary = BirdNetPiImporter
            .migrate(tmp.path(), dst.path(), &handle)
            .unwrap();
        assert_eq!(summary.source_rows, 4);
        assert_eq!(
            summary.imported_rows, 4,
            "every row is imported; none is skipped for a bad Date"
        );
        assert_eq!(summary.skipped_rows, 0);

        let dc = Connection::open(dst.path()).unwrap();
        // A NULL Date becomes "" rather than staying NULL.
        let blank: i64 = dc
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Date = '' AND Time = ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            blank, 2,
            "the NULL row and the empty-string row both land as ''"
        );
        let still_null: i64 = dc
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Date IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_null, 0);

        let unparseable: i64 = dc
            .query_row(
                "SELECT COUNT(*) FROM detections
                  WHERE Date NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unparseable, 3);
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

    /// A BirdNET-Pi row with no `File_Name` must still dedupe on re-import.
    ///
    /// It did not: `File_Name` is part of the destination's UNIQUE key and was
    /// nullable, and SQLite treats NULLs as distinct, so `INSERT OR IGNORE`
    /// had nothing to conflict with and silently doubled the row while
    /// reporting `skipped = 0`. Migration 23 made the index NULL-insensitive.
    #[test]
    fn a_null_file_name_still_dedupes() {
        let src = NamedTempFile::new().unwrap();
        let conn = Connection::open(src.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections
               (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-01-01','06:00:00','Turdus merula','Blackbird',0.9,NULL);",
        )
        .unwrap();
        drop(conn);

        let dst = NamedTempFile::new().unwrap();
        let handle = ProgressHandle::new();
        let importer = BirdNetPiImporter;
        importer.migrate(src.path(), dst.path(), &handle).unwrap();
        let second = importer.migrate(src.path(), dst.path(), &handle).unwrap();

        let dconn = Connection::open(dst.path()).unwrap();
        let rows: i64 = dconn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            rows, 1,
            "re-import must not duplicate a row whose File_Name is NULL"
        );
        assert_eq!(second.imported_rows, 0);
        assert_eq!(second.skipped_rows, 1);
    }
}
