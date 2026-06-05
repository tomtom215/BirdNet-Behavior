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

        let rows = read_sqlite_detections(sqlite_conn, cutoff.as_deref())
            .map_err(|e| AnalyticsError::InvalidData(format!("SQLite read error: {e}")))?;

        let count = u64::try_from(rows.len()).unwrap_or(0);

        if count > 0 {
            self.append_rows(&rows)?;
            self.conn
                .execute_batch(queries::CREATE_DETECTIONS_TS_VIEW)?;
            tracing::info!(rows = count, "synced detections from SQLite to DuckDB");
        }

        Ok(count)
    }

    /// Rebuild the `DuckDB` detections copy from `SQLite` in full.
    ///
    /// Unlike [`sync_from_sqlite`], which only pulls rows newer than the latest
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
        self.conn.execute_batch("DELETE FROM detections;")?;
        self.append_rows(&rows)?;
        // Refresh the view unconditionally so it exists even after a rebuild
        // that loaded zero rows.
        self.conn
            .execute_batch(queries::CREATE_DETECTIONS_TS_VIEW)?;
        tracing::info!(rows = count, "rebuilt DuckDB detections from SQLite (full resync)");

        Ok(count)
    }

    /// Bulk-append already-read `SQLite` rows into the `DuckDB` `detections`
    /// table via the appender. Shared by the incremental and full sync paths;
    /// callers refresh the timestamp view and emit their own log line.
    fn append_rows(&self, rows: &[SyncRow]) -> Result<(), AnalyticsError> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut appender = self.conn.appender("detections")?;
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
            let sql = format!(
                "SELECT {COLS} FROM detections \
                 WHERE (Date || ' ' || Time) > ? ORDER BY Date, Time"
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
        assert_eq!(db.sync_from_sqlite(&sc).unwrap(), 1);
        assert_eq!(db.detection_count().unwrap(), 2);
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

        // Incremental sync skips the 2023 row (older than the 2026 cutoff) and
        // the 2026 row (equal to, not after, the cutoff): nothing new.
        assert_eq!(db.sync_from_sqlite(&sc).unwrap(), 0);

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
        db.insert_detection("2026-06-05", "10:00:00", "Parus major", "Great Tit", 0.9, "x.wav")
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
}
