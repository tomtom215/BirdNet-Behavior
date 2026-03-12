//! `DuckDB` file-based connection management.
//!
//! Provides a durable, file-backed `DuckDB` database for behavioral analytics.
//! Data is synced from the operational `SQLite` database (OLTP) into `DuckDB`
//! (OLAP) for complex analytical queries using the `duckdb-behavioral` extension.
//!
//! File-based storage ensures durability across power losses and restarts,
//! which is critical for field deployments on Raspberry Pi devices.

use duckdb::{Connection, Error as DuckDbError};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::queries;

/// Errors from `DuckDB` operations.
#[derive(Debug)]
pub enum AnalyticsError {
    /// `DuckDB` connection or query error.
    Database(DuckDbError),
    /// Failed to load the behavioral extension.
    ExtensionLoad(String),
    /// Query returned unexpected data.
    InvalidData(String),
}

impl fmt::Display for AnalyticsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(e) => write!(f, "DuckDB error: {e}"),
            Self::ExtensionLoad(msg) => write!(f, "extension load error: {msg}"),
            Self::InvalidData(msg) => write!(f, "invalid data: {msg}"),
        }
    }
}

impl std::error::Error for AnalyticsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            Self::ExtensionLoad(_) | Self::InvalidData(_) => None,
        }
    }
}

impl From<DuckDbError> for AnalyticsError {
    fn from(e: DuckDbError) -> Self {
        Self::Database(e)
    }
}

/// A file-backed `DuckDB` connection for behavioral analytics.
///
/// Wraps a `DuckDB` connection opened against a persistent file,
/// with the detections timestamp view pre-created for query use.
#[derive(Debug)]
pub struct AnalyticsDb {
    conn: Connection,
    path: PathBuf,
    extension_loaded: bool,
}

impl AnalyticsDb {
    /// Open or create a file-based `DuckDB` database.
    ///
    /// Creates the database file if it doesn't exist. Sets up the detections
    /// table schema for analytical queries.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or the schema
    /// cannot be created.
    pub fn open(path: &Path) -> Result<Self, AnalyticsError> {
        let conn = Connection::open(path)?;

        // Create the detections table matching SQLite schema
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS detections (
                Date TEXT NOT NULL,
                Time TEXT NOT NULL,
                Sci_Name TEXT NOT NULL,
                Com_Name TEXT NOT NULL,
                Confidence DOUBLE NOT NULL,
                Lat DOUBLE,
                Lon DOUBLE,
                Cutoff DOUBLE,
                Week INTEGER,
                Sens DOUBLE,
                Overlap DOUBLE,
                File_Name TEXT
            );",
        )?;

        // Create the timestamp view for behavioral queries
        conn.execute_batch(queries::CREATE_DETECTIONS_TS_VIEW)?;

        Ok(Self {
            conn,
            path: path.to_path_buf(),
            extension_loaded: false,
        })
    }

    /// Get the database file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the behavioral extension has been loaded.
    pub const fn extension_loaded(&self) -> bool {
        self.extension_loaded
    }

    /// Load the `duckdb-behavioral` extension from the community repository.
    ///
    /// This requires network access on first run to download the extension.
    /// Once installed, subsequent loads use the cached version.
    ///
    /// # Errors
    ///
    /// Returns an error if the extension cannot be installed or loaded.
    /// This is non-fatal -- the database can still serve basic queries
    /// without the behavioral extension.
    pub fn load_extension(&mut self) -> Result<(), AnalyticsError> {
        self.conn
            .execute_batch(queries::LOAD_BEHAVIORAL)
            .map_err(|e| AnalyticsError::ExtensionLoad(e.to_string()))?;
        self.extension_loaded = true;
        Ok(())
    }

    /// Get a reference to the underlying `DuckDB` connection.
    ///
    /// Use this for executing custom queries or the SQL builders
    /// from [`crate::queries`].
    pub const fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Sync detections from a `SQLite` connection into `DuckDB`.
    ///
    /// Reads directly from the `SQLite` connection (via `rusqlite`) and
    /// inserts into `DuckDB`. This avoids requiring `DuckDB`'s `sqlite_scanner`
    /// extension, which needs network access to download -- critical for
    /// air-gapped field deployments.
    ///
    /// Performs an incremental sync: inserts only rows that are newer than
    /// the latest detection already in `DuckDB`, based on Date and Time.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `SQLite` or writing to `DuckDB` fails.
    pub fn sync_from_sqlite(
        &self,
        sqlite_conn: &rusqlite::Connection,
    ) -> Result<u64, AnalyticsError> {
        // Determine the cutoff for incremental sync
        let has_data: bool =
            self.conn
                .query_row("SELECT COUNT(*) > 0 FROM detections", [], |row| row.get(0))?;

        let cutoff: Option<String> = if has_data {
            Some(self.conn.query_row(
                "SELECT Date || ' ' || Time FROM detections ORDER BY Date DESC, Time DESC LIMIT 1",
                [],
                |row| row.get(0),
            )?)
        } else {
            None
        };

        // Read from SQLite
        let rows = read_sqlite_detections(sqlite_conn, cutoff.as_deref())
            .map_err(|e| AnalyticsError::InvalidData(format!("SQLite read error: {e}")))?;

        let count = u64::try_from(rows.len()).unwrap_or(0);

        // Batch insert into DuckDB
        if !rows.is_empty() {
            let mut appender = self.conn.appender("detections")?;
            for row in &rows {
                appender.append_row(duckdb::params![
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

            // Refresh the timestamp view after new data
            self.conn
                .execute_batch(queries::CREATE_DETECTIONS_TS_VIEW)?;

            tracing::info!(rows = count, "synced detections from SQLite to DuckDB");
        }

        Ok(count)
    }

    /// Insert a single detection record directly.
    ///
    /// Used for real-time insertion alongside `SQLite` writes,
    /// keeping both databases in sync without batch syncs.
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
            duckdb::params![date, time, sci_name, com_name, confidence, file_name],
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

    /// Count unique species in `DuckDB`.
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

    /// Execute a sessionize query and return raw results as JSON-ready rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or the behavioral extension
    /// is not loaded.
    pub fn sessionize(
        &self,
        params: &crate::types::SessionizeParams,
    ) -> Result<Vec<crate::types::ActivitySession>, AnalyticsError> {
        if !self.extension_loaded {
            return Err(AnalyticsError::ExtensionLoad(
                "behavioral extension not loaded".into(),
            ));
        }

        let sql = queries::sessionize_sql(params);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::ActivitySession {
                species: row.get(0)?,
                session_id: row.get(1)?,
                detection_count: row.get(2)?,
                start_time: row.get(3)?,
                end_time: row.get(4)?,
                duration_secs: row.get(5)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Execute a retention query to track species return patterns.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or the behavioral extension
    /// is not loaded.
    pub fn retention(
        &self,
        params: &crate::types::RetentionParams,
    ) -> Result<Vec<crate::types::SpeciesRetention>, AnalyticsError> {
        if !self.extension_loaded {
            return Err(AnalyticsError::ExtensionLoad(
                "behavioral extension not loaded".into(),
            ));
        }

        let sql = queries::retention_sql(params);
        let mut stmt = self.conn.prepare(&sql)?;

        // The retention() function returns an array of booleans/rates
        let rows = stmt.query_map([], |row| {
            let species: String = row.get(0)?;
            let rates_raw: Vec<f64> = row.get(1)?;

            Ok((species, rates_raw))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (species, rates_raw) = row?;

            let retention_rates: Vec<crate::types::RetentionRate> = params
                .intervals
                .iter()
                .zip(rates_raw.iter())
                .map(|(&days, &rate)| crate::types::RetentionRate { days, rate })
                .collect();

            // Classify residency based on 30-day retention (or last available)
            let long_term_rate = retention_rates.last().map_or(0.0, |r| r.rate);
            let classification = crate::types::ResidencyType::from_retention_rate(long_term_rate);

            results.push(crate::types::SpeciesRetention {
                species,
                retention_rates,
                classification,
            });
        }

        Ok(results)
    }

    /// Execute a dawn chorus funnel analysis query.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or the behavioral extension
    /// is not loaded.
    pub fn funnel(
        &self,
        params: &crate::types::FunnelParams,
    ) -> Result<Vec<crate::types::ChorusFunnel>, AnalyticsError> {
        if !self.extension_loaded {
            return Err(AnalyticsError::ExtensionLoad(
                "behavioral extension not loaded".into(),
            ));
        }

        let sql = queries::funnel_sql(params);
        let total_steps = u32::try_from(params.species_sequence.len()).unwrap_or(0);
        let species_sequence = params.species_sequence.clone();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (date, steps_completed) = row?;
            let matched_species: Vec<String> = species_sequence
                .iter()
                .take(steps_completed as usize)
                .cloned()
                .collect();

            results.push(crate::types::ChorusFunnel {
                date,
                steps_completed,
                total_steps,
                matched_species,
            });
        }

        Ok(results)
    }

    /// Execute a next-species prediction query.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or the behavioral extension
    /// is not loaded.
    pub fn next_species(
        &self,
        trigger: &str,
        window_minutes: u32,
        limit: u32,
    ) -> Result<Vec<crate::types::NextSpeciesPrediction>, AnalyticsError> {
        if !self.extension_loaded {
            return Err(AnalyticsError::ExtensionLoad(
                "behavioral extension not loaded".into(),
            ));
        }

        let sql = queries::next_species_sql(trigger, window_minutes, limit);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let frequency: i64 = row.get(1)?;
            Ok(crate::types::NextSpeciesPrediction {
                after_species: trigger.to_string(),
                predicted_species: row.get(0)?,
                frequency: u64::try_from(frequency).unwrap_or(0),
                probability: 0.0, // Computed after collecting all rows
            })
        })?;

        let mut results: Vec<crate::types::NextSpeciesPrediction> = Vec::new();
        for row in rows {
            results.push(row?);
        }

        // Compute probabilities from frequencies
        let total: u64 = results.iter().map(|r| r.frequency).sum();
        if total > 0 {
            for result in &mut results {
                #[allow(clippy::cast_precision_loss)]
                {
                    result.probability = result.frequency as f64 / total as f64;
                }
            }
        }

        Ok(results)
    }
}

/// A detection row read from `SQLite` for syncing to `DuckDB`.
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
///
/// If `after` is `Some("YYYY-MM-DD HH:MM:SS")`, only rows newer than that
/// timestamp are returned. If `None`, all rows are returned.
fn read_sqlite_detections(
    conn: &rusqlite::Connection,
    after: Option<&str>,
) -> Result<Vec<SyncRow>, rusqlite::Error> {
    const COLUMNS: &str = "Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name";

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
            let sql = format!("SELECT {COLUMNS} FROM detections ORDER BY Date, Time");
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map([], map_row)?.collect()
        },
        |ts| {
            let sql = format!(
                "SELECT {COLUMNS} FROM detections WHERE (Date || ' ' || Time) > ? ORDER BY Date, Time"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map([ts], map_row)?.collect()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_db() -> (AnalyticsDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("analytics.duckdb");
        let db = AnalyticsDb::open(&db_path).unwrap();
        (db, dir)
    }

    fn insert_test_data(db: &AnalyticsDb) {
        db.insert_detection(
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.87,
            "test.wav",
        )
        .unwrap();
        db.insert_detection(
            "2026-03-12",
            "06:35:00",
            "Erithacus rubecula",
            "European Robin",
            0.92,
            "test.wav",
        )
        .unwrap();
        db.insert_detection(
            "2026-03-12",
            "07:00:00",
            "Parus major",
            "Great Tit",
            0.75,
            "test2.wav",
        )
        .unwrap();
    }

    #[test]
    fn open_creates_file_database() {
        let (db, _tmp) = create_test_db();
        assert!(db.path().exists());
        assert!(!db.extension_loaded());
    }

    #[test]
    fn insert_and_count() {
        let (db, _tmp) = create_test_db();
        insert_test_data(&db);

        assert_eq!(db.detection_count().unwrap(), 3);
        assert_eq!(db.species_count().unwrap(), 3);
    }

    #[test]
    fn sync_from_sqlite_full() {
        let (db, _tmp) = create_test_db();

        // Create a SQLite database with test data
        let sqlite_dir = TempDir::new().unwrap();
        let sqlite_path = sqlite_dir.path().join("birds.db");
        let sqlite_conn = rusqlite::Connection::open(&sqlite_path).unwrap();
        sqlite_conn
            .execute_batch(
                "CREATE TABLE detections (
                    Date TEXT NOT NULL,
                    Time TEXT NOT NULL,
                    Sci_Name TEXT NOT NULL,
                    Com_Name TEXT NOT NULL,
                    Confidence REAL NOT NULL,
                    Lat REAL,
                    Lon REAL,
                    Cutoff REAL,
                    Week INTEGER,
                    Sens REAL,
                    Overlap REAL,
                    File_Name TEXT
                );
                INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                VALUES ('2026-03-12', '06:30:00', 'Turdus merula', 'Eurasian Blackbird', 0.87, 'test.wav');
                INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                VALUES ('2026-03-12', '07:00:00', 'Parus major', 'Great Tit', 0.75, 'test2.wav');",
            )
            .unwrap();

        let rows = db.sync_from_sqlite(&sqlite_conn).unwrap();
        assert_eq!(rows, 2);
        assert_eq!(db.detection_count().unwrap(), 2);
    }

    #[test]
    fn sync_from_sqlite_incremental() {
        let (db, _tmp) = create_test_db();

        // Pre-populate DuckDB with one row
        db.insert_detection(
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.87,
            "test.wav",
        )
        .unwrap();

        // Create SQLite with the same row plus a newer one
        let sqlite_dir = TempDir::new().unwrap();
        let sqlite_path = sqlite_dir.path().join("birds.db");
        let sqlite_conn = rusqlite::Connection::open(&sqlite_path).unwrap();
        sqlite_conn
            .execute_batch(
                "CREATE TABLE detections (
                    Date TEXT NOT NULL,
                    Time TEXT NOT NULL,
                    Sci_Name TEXT NOT NULL,
                    Com_Name TEXT NOT NULL,
                    Confidence REAL NOT NULL,
                    Lat REAL,
                    Lon REAL,
                    Cutoff REAL,
                    Week INTEGER,
                    Sens REAL,
                    Overlap REAL,
                    File_Name TEXT
                );
                INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                VALUES ('2026-03-12', '06:30:00', 'Turdus merula', 'Eurasian Blackbird', 0.87, 'test.wav');
                INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                VALUES ('2026-03-12', '07:00:00', 'Parus major', 'Great Tit', 0.75, 'test2.wav');",
            )
            .unwrap();

        let rows = db.sync_from_sqlite(&sqlite_conn).unwrap();
        assert_eq!(rows, 1); // Only the newer row
        assert_eq!(db.detection_count().unwrap(), 2);
    }

    #[test]
    fn sessionize_requires_extension() {
        let (db, _tmp) = create_test_db();
        let params = crate::types::SessionizeParams::default();
        let err = db.sessionize(&params).unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    #[test]
    fn next_species_requires_extension() {
        let (db, _tmp) = create_test_db();
        let err = db.next_species("European Robin", 60, 10).unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    #[test]
    fn detection_count_empty() {
        let (db, _tmp) = create_test_db();
        assert_eq!(db.detection_count().unwrap(), 0);
        assert_eq!(db.species_count().unwrap(), 0);
    }
}
