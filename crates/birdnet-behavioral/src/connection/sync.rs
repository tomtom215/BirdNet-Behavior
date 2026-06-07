//! SQLite → DuckDB synchronisation and basic detection mutations.
//!
//! Provides `sync_from_sqlite` for bulk incremental sync and
//! `insert_detection` for real-time single-row writes, keeping both
//! databases in step without requiring `DuckDB`'s `sqlite_scanner` extension
//! (which needs network access, critical for air-gapped Pi deployments).

use duckdb::params;

use super::{AnalyticsDb, AnalyticsError};
use crate::queries;

impl AnalyticsDb {
    /// Sync detections from a `SQLite` connection into `DuckDB`.
    ///
    /// Performs an incremental sync — only rows newer than the latest
    /// detection already in `DuckDB` are inserted.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `SQLite` or writing to `DuckDB` fails.
    pub fn sync_from_sqlite(
        &self,
        sqlite_conn: &rusqlite::Connection,
    ) -> Result<u64, AnalyticsError> {
        let has_data: bool =
            self.conn
                .query_row("SELECT COUNT(*) > 0 FROM detections", [], |row| row.get(0))?;

        let cutoff: Option<String> = if has_data {
            Some(self.conn.query_row(
                "SELECT Date || ' ' || Time FROM detections \
                 ORDER BY Date DESC, Time DESC LIMIT 1",
                [],
                |row| row.get(0),
            )?)
        } else {
            None
        };

        // Make the cutoff second whole rather than skipping it. A single second
        // can hold many detections (multiple chunks of one recording, or
        // simultaneous hits from different audio sources), so a strict
        // `> cutoff` read permanently dropped any SQLite row that *tied* the
        // latest synced second but wasn't yet in DuckDB. Delete that second from
        // DuckDB and let `read_sqlite_detections` re-read it (with `>=`) from
        // SQLite — the source of truth — so it's rebuilt exactly, no duplicates.
        if let Some(ref ts) = cutoff {
            self.conn.execute(
                "DELETE FROM detections WHERE (Date || ' ' || Time) = ?",
                params![ts],
            )?;
        }

        let rows = read_sqlite_detections(sqlite_conn, cutoff.as_deref())
            .map_err(|e| AnalyticsError::InvalidData(format!("SQLite read error: {e}")))?;

        let count = u64::try_from(rows.len()).unwrap_or(0);

        if count > 0 {
            self.append_rows_to("detections", &rows)?;
            self.conn
                .execute_batch(queries::CREATE_DETECTIONS_TS_VIEW)?;
            tracing::info!(rows = count, "synced detections from SQLite to DuckDB");
        }

        Ok(count)
    }

    /// Rebuild the `DuckDB` detections copy from `SQLite` in full.
    ///
    /// Unlike [`Self::sync_from_sqlite`], which only pulls rows newer than the latest
    /// detection already in `DuckDB`, this truncates the OLAP copy and
    /// re-appends every `SQLite` row. A bulk historical import (the BirdNET-Pi
    /// migration) writes *back-dated* detections that the incremental cutoff
    /// would skip, so after such an import the OLAP copy must be rebuilt for the
    /// imported history to appear — with its original timestamps — in the
    /// behavioural and time-series analytics.
    ///
    /// Returns the number of rows loaded.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `SQLite` or writing to `DuckDB` fails.
    pub fn full_resync_from_sqlite(
        &self,
        sqlite_conn: &rusqlite::Connection,
    ) -> Result<u64, AnalyticsError> {
        let rows = read_sqlite_detections(sqlite_conn, None)
            .map_err(|e| AnalyticsError::InvalidData(format!("SQLite read error: {e}")))?;
        let count = u64::try_from(rows.len()).unwrap_or(0);

        // Truncate first: a full rebuild must not be filtered by the incremental
        // cutoff (which would drop back-dated imports) and must not duplicate
        // rows already present from the startup sync.
        //
        // Build the new copy in a staging table and swap it in atomically,
        // rather than `DELETE`-then-append in place. A failed append (or a crash
        // mid-rebuild) previously left the live OLAP copy *empty* — recoverable
        // only by re-running, and silently under-reporting until then. The
        // appender writes to the staging table outside any transaction (avoiding
        // DuckDB's appender/transaction interaction); the swap is plain
        // transactional SQL with an explicit rollback so a failure leaves both
        // the live table and the connection usable.
        self.conn.execute_batch(
            "CREATE OR REPLACE TABLE detections_staging AS SELECT * FROM detections WHERE false;",
        )?;
        self.append_rows_to("detections_staging", &rows)?;

        self.conn.execute_batch("BEGIN TRANSACTION;")?;
        let swap = self.conn.execute_batch(
            "DELETE FROM detections;
             INSERT INTO detections SELECT * FROM detections_staging;",
        );
        match swap {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(e) => {
                // Roll the swap back and drop the staging table so the live copy
                // and the connection are left in a clean, usable state.
                let _ = self.conn.execute_batch("ROLLBACK;");
                let _ = self
                    .conn
                    .execute_batch("DROP TABLE IF EXISTS detections_staging;");
                return Err(AnalyticsError::from(e));
            }
        }
        self.conn
            .execute_batch("DROP TABLE IF EXISTS detections_staging;")?;

        // Refresh the view unconditionally so it exists even after a rebuild
        // that loaded zero rows.
        self.conn
            .execute_batch(queries::CREATE_DETECTIONS_TS_VIEW)?;
        tracing::info!(
            rows = count,
            "rebuilt DuckDB detections from SQLite (full resync)"
        );

        Ok(count)
    }

    /// Bulk-append already-read `SQLite` rows into the named `DuckDB` table via
    /// the appender. `table` is an internal, hard-coded identifier (`detections`
    /// for the incremental path, `detections_staging` for the atomic full
    /// rebuild) — never untrusted input.
    ///
    /// Shared by the incremental and full sync paths; callers refresh the
    /// timestamp view and emit their own log line.
    fn append_rows_to(&self, table: &str, rows: &[SyncRow]) -> Result<(), AnalyticsError> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut appender = self.conn.appender(table)?;
        for row in rows {
            appender.append_row(params![
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
            ])?;
        }
        appender.flush()?;
        Ok(())
    }

    /// Insert a single detection record directly.
    ///
    /// Use for real-time insertion alongside `SQLite` writes.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn insert_detection(
        &self,
        date: &str,
        time: &str,
        sci_name: &str,
        com_name: &str,
        confidence: f64,
        file_name: &str,
    ) -> Result<(), AnalyticsError> {
        self.conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![date, time, sci_name, com_name, confidence, file_name],
        )?;
        Ok(())
    }

    /// Count total detections in `DuckDB`.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn detection_count(&self) -> Result<u64, AnalyticsError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM detections", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// Count unique species (by common name) in `DuckDB`.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn species_count(&self) -> Result<u64, AnalyticsError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT Com_Name) FROM detections",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}

/// An intermediate row read from `SQLite` for syncing to `DuckDB`.
#[derive(Debug)]
struct SyncRow {
    date: String,
    time: String,
    sci_name: String,
    com_name: String,
    confidence: f64,
    lat: Option<f64>,
    lon: Option<f64>,
    cutoff: Option<f64>,
    week: Option<i32>,
    sens: Option<f64>,
    overlap: Option<f64>,
    file_name: Option<String>,
}

/// Read detections from `SQLite`, optionally filtering by a timestamp cutoff.
fn read_sqlite_detections(
    conn: &rusqlite::Connection,
    after: Option<&str>,
) -> Result<Vec<SyncRow>, rusqlite::Error> {
    const COLS: &str = "Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, \
                        Cutoff, Week, Sens, Overlap, File_Name";

    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(SyncRow {
            date: row.get(0)?,
            time: row.get(1)?,
            sci_name: row.get(2)?,
            com_name: row.get(3)?,
            confidence: row.get(4)?,
            lat: row.get(5)?,
            lon: row.get(6)?,
            cutoff: row.get(7)?,
            week: row.get(8)?,
            sens: row.get(9)?,
            overlap: row.get(10)?,
            file_name: row.get(11)?,
        })
    };

    after.map_or_else(
        || {
            let sql = format!("SELECT {COLS} FROM detections ORDER BY Date, Time");
            conn.prepare(&sql)?.query_map([], map_row)?.collect()
        },
        |ts| {
            // `>=` (not `>`): the caller deletes the cutoff second from DuckDB
            // first, then re-reads it whole from SQLite here, so rows that tie
            // the cutoff second aren't permanently skipped (see sync_from_sqlite).
            let sql = format!(
                "SELECT {COLS} FROM detections \
                 WHERE (Date || ' ' || Time) >= ? ORDER BY Date, Time"
            );
            conn.prepare(&sql)?.query_map([ts], map_row)?.collect()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_db() -> (AnalyticsDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).unwrap();
        (db, dir)
    }

    #[test]
    fn insert_and_count() {
        let (db, _tmp) = make_db();
        db.insert_detection(
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.87,
            "t.wav",
        )
        .unwrap();
        db.insert_detection(
            "2026-03-12",
            "06:35:00",
            "Erithacus rubecula",
            "European Robin",
            0.92,
            "t.wav",
        )
        .unwrap();
        assert_eq!(db.detection_count().unwrap(), 2);
        assert_eq!(db.species_count().unwrap(), 2);
    }

    #[test]
    fn sync_from_sqlite_full() {
        let (db, _tmp) = make_db();
        let sqlite_dir = TempDir::new().unwrap();
        let sc = rusqlite::Connection::open(sqlite_dir.path().join("b.db")).unwrap();
        sc.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
             Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections VALUES ('2026-03-12','06:30:00','Turdus merula','Blackbird',0.87,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
             INSERT INTO detections VALUES ('2026-03-12','07:00:00','Parus major','Great Tit',0.75,NULL,NULL,NULL,NULL,NULL,NULL,NULL);",
        ).unwrap();
        assert_eq!(db.sync_from_sqlite(&sc).unwrap(), 2);
        assert_eq!(db.detection_count().unwrap(), 2);
    }

    #[test]
    fn sync_from_sqlite_incremental() {
        let (db, _tmp) = make_db();
        db.insert_detection(
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Blackbird",
            0.87,
            "t.wav",
        )
        .unwrap();

        let sqlite_dir = TempDir::new().unwrap();
        let sc = rusqlite::Connection::open(sqlite_dir.path().join("b.db")).unwrap();
        sc.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
             Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections VALUES ('2026-03-12','06:30:00','Turdus merula','Blackbird',0.87,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
             INSERT INTO detections VALUES ('2026-03-12','07:00:00','Parus major','Great Tit',0.75,NULL,NULL,NULL,NULL,NULL,NULL,NULL);",
        ).unwrap();
        // Sync deletes the cutoff second (`06:30:00`) from DuckDB and re-reads
        // it from SQLite along with the new `07:00:00` row, so the return count
        // is 2 (one boundary re-read + one strictly new). End state still 2.
        assert_eq!(db.sync_from_sqlite(&sc).unwrap(), 2);
        assert_eq!(db.detection_count().unwrap(), 2);
    }

    #[test]
    fn sync_from_sqlite_includes_same_second_ties() {
        // Regression: a single second can hold many detections (multiple chunks
        // of one recording, or simultaneous hits from different audio sources).
        // The old strict `> cutoff` read permanently dropped any SQLite row that
        // tied the latest already-synced second — the analytics copy then
        // under-counted forever (recoverable only by a full resync).
        //
        // The fix deletes the cutoff second from DuckDB and re-reads it with
        // `>=` from SQLite (the source of truth), so the boundary is rebuilt
        // exactly and incremental sync is lossless across ties.
        let (db, _tmp) = make_db();

        // DuckDB already holds *one* of two same-second detections.
        db.insert_detection(
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Blackbird",
            0.87,
            "t.wav",
        )
        .unwrap();

        // SQLite holds both same-second detections plus a later row.
        let sqlite_dir = TempDir::new().unwrap();
        let sc = rusqlite::Connection::open(sqlite_dir.path().join("b.db")).unwrap();
        sc.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
             Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections VALUES ('2026-03-12','06:30:00','Turdus merula','Blackbird',0.87,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
             INSERT INTO detections VALUES ('2026-03-12','06:30:00','Parus major','Great Tit',0.91,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
             INSERT INTO detections VALUES ('2026-03-12','07:00:00','Erithacus rubecula','Robin',0.83,NULL,NULL,NULL,NULL,NULL,NULL,NULL);",
        ).unwrap();

        // Sync must rebuild the boundary second (so the tied row appears) plus
        // the later row, and must NOT duplicate the already-present Blackbird.
        db.sync_from_sqlite(&sc).unwrap();
        assert_eq!(
            db.detection_count().unwrap(),
            3,
            "all three SQLite rows should be in DuckDB; the same-second tie at \
             06:30 must not be dropped, and the already-present row must not be \
             duplicated"
        );

        // Idempotent: running sync again is a no-op (no duplicates).
        db.sync_from_sqlite(&sc).unwrap();
        assert_eq!(db.detection_count().unwrap(), 3);
    }

    #[test]
    fn counts_empty() {
        let (db, _tmp) = make_db();
        assert_eq!(db.detection_count().unwrap(), 0);
        assert_eq!(db.species_count().unwrap(), 0);
    }

    #[test]
    fn full_resync_includes_backdated_imports() {
        // Reproduces the import bug: DuckDB already holds a recent detection (as
        // if the live daemon had been running), then a bulk historical import
        // writes *back-dated* rows into SQLite. The incremental sync skips them
        // because they predate the cutoff; the full resync must include them.
        let (db, _tmp) = make_db();
        db.insert_detection(
            "2026-06-05",
            "10:00:00",
            "Parus major",
            "Great Tit",
            0.90,
            "now.wav",
        )
        .unwrap();

        let sqlite_dir = TempDir::new().unwrap();
        let sc = rusqlite::Connection::open(sqlite_dir.path().join("b.db")).unwrap();
        sc.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
             Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections VALUES ('2023-01-01','06:30:00','Turdus merula','Blackbird',0.80,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
             INSERT INTO detections VALUES ('2026-06-05','10:00:00','Parus major','Great Tit',0.90,NULL,NULL,NULL,NULL,NULL,NULL,NULL);",
        ).unwrap();

        // Incremental sync skips the back-dated 2023 row (older than the 2026
        // cutoff). The cutoff second is deleted from DuckDB and re-read from
        // SQLite, which still yields one row (the same 2026-06-05 10:00:00
        // detection rebuilt exactly) — so the return count is 1, not 0. The
        // back-dated history remains invisible until `full_resync_from_sqlite`.
        assert_eq!(db.sync_from_sqlite(&sc).unwrap(), 1);

        // Full resync rebuilds from scratch and includes the back-dated history.
        assert_eq!(db.full_resync_from_sqlite(&sc).unwrap(), 2);
        assert_eq!(db.detection_count().unwrap(), 2);
        let backdated: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Date = '2023-01-01'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(backdated, 1, "imported 2023 history must be present");
    }

    #[test]
    fn full_resync_on_empty_sqlite_clears_and_keeps_view() {
        // A rebuild against an empty source truncates the OLAP copy and leaves a
        // working timestamp view (no rows, but queryable).
        let (db, _tmp) = make_db();
        db.insert_detection(
            "2026-06-05",
            "10:00:00",
            "Parus major",
            "Great Tit",
            0.9,
            "x.wav",
        )
        .unwrap();

        let sqlite_dir = TempDir::new().unwrap();
        let sc = rusqlite::Connection::open(sqlite_dir.path().join("b.db")).unwrap();
        sc.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
             Sens REAL, Overlap REAL, File_Name TEXT);",
        )
        .unwrap();

        assert_eq!(db.full_resync_from_sqlite(&sc).unwrap(), 0);
        assert_eq!(db.detection_count().unwrap(), 0);
        let view_rows: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM detections_ts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(view_rows, 0);
    }

    #[test]
    fn full_resync_is_atomic_and_repeatable() {
        // The atomic rebuild builds into `detections_staging` and swaps it in.
        // Verify the staging table doesn't leak after a successful rebuild and
        // that running the rebuild repeatedly is idempotent (no duplicates, no
        // residual staging table from a prior run).
        let (db, _tmp) = make_db();
        let sqlite_dir = TempDir::new().unwrap();
        let sc = rusqlite::Connection::open(sqlite_dir.path().join("b.db")).unwrap();
        sc.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
             Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections VALUES ('2026-06-05','10:00:00','Parus major','Great Tit',0.9,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
             INSERT INTO detections VALUES ('2026-06-05','10:00:01','Turdus merula','Blackbird',0.8,NULL,NULL,NULL,NULL,NULL,NULL,NULL);",
        ).unwrap();

        // Helper: does a table exist in this DuckDB?
        let staging_exists = |db: &AnalyticsDb| -> bool {
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM information_schema.tables \
                     WHERE table_name = 'detections_staging'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap()
                > 0
        };

        assert_eq!(db.full_resync_from_sqlite(&sc).unwrap(), 2);
        assert_eq!(db.detection_count().unwrap(), 2);
        assert!(
            !staging_exists(&db),
            "staging table must be dropped after swap"
        );

        // Repeat: still 2 rows (not 4), staging still gone.
        assert_eq!(db.full_resync_from_sqlite(&sc).unwrap(), 2);
        assert_eq!(db.detection_count().unwrap(), 2);
        assert!(!staging_exists(&db));
    }
}
