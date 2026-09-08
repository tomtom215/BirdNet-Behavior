//! SQLite → DuckDB synchronisation and basic detection mutations.
//!
//! Provides `sync_from_sqlite` for bulk incremental sync and
//! `insert_detection` for real-time single-row writes, keeping both
//! databases in step without requiring `DuckDB`'s `sqlite_scanner` extension
//! (which needs network access, critical for air-gapped Pi deployments).

use duckdb::params;

use super::{AnalyticsDb, AnalyticsError};
use crate::queries;

/// Columns copied from `SQLite` into the `DuckDB` detections table, in the
/// order the appender expects them.
const SYNC_COLS: &str = "Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, \
                         Cutoff, Week, Sens, Overlap, File_Name";

/// How many columns [`SYNC_COLS`] names. The optional columns are appended
/// after these, so this is where their read indices start.
const SYNC_COL_COUNT: usize = 12;

/// Provenance column, appended to [`SYNC_COLS`] when the source has it.
///
/// Detected rather than assumed. In the product `migrate()` always runs before
/// any sync, so the column is always present — but `sync_from_sqlite` is public
/// API on a library crate, and a caller handing it a connection whose schema
/// predates migration 25 should get a sync without provenance rather than a
/// hard failure that empties every analytics dashboard. The cost of asking is
/// one `PRAGMA` per sync.
const PROVENANCE_COL: &str = "import_batch_id";

/// Reviewer-verdict column, appended when the source has it. Detected for the
/// same reason as [`PROVENANCE_COL`].
const VERDICT_COL: &str = "review_verdict";

/// Monotonic-instant column (migration 32), appended when the source has it.
/// Detected for the same reason as [`PROVENANCE_COL`].
const INSTANT_COL: &str = "detected_at_utc";

/// Analysis-run column (migration 43): which daemon start, and so which model
/// bytes, produced the row. Detected for the same reason as
/// [`PROVENANCE_COL`].
const RUN_COL: &str = "run_id";

/// Whether `detections` in this `SQLite` database carries `column`.
fn has_column(conn: &rusqlite::Connection, column: &str) -> bool {
    conn.prepare("SELECT 1 FROM pragma_table_info('detections') WHERE name = ?1")
        .and_then(|mut stmt| stmt.exists([column]))
        .unwrap_or(false)
}

/// How many rows are appended before the appender is flushed.
///
/// The sync used to read the entire `SQLite` detections table into a
/// `Vec<SyncRow>` before appending a single row, so peak memory grew with the
/// station's whole history rather than with the work in flight. Measured on
/// x86_64: **1 000 000 rows → 541 MiB, 2 000 000 rows → 967 MiB** resident,
/// against the `MemoryMax=1G` the systemd unit sets — so a station stopped
/// being able to start at roughly 2.1 M detections, and `Restart=always` turned
/// that into a restart loop. A multi-year BirdNET-Pi database, which is exactly
/// what `birdnet-migrate` imports, is that size on arrival.
///
/// Streaming instead makes peak memory a function of this batch, not of the
/// row count. 10 000 rows is a few MiB in the appender's buffer while still
/// amortising the flush over a useful chunk.
const APPEND_BATCH_ROWS: u64 = 10_000;

/// A detection as written by the live path.
///
/// Carries the same columns the bulk sync copies, so a row written
/// live and the same row rebuilt by a resync are identical. They were not:
/// the live insert wrote six columns and left `Lat`, `Lon`, `Cutoff`,
/// `Week`, `Sens` and `Overlap` NULL, so "the same detection" meant
/// different things depending on how it got into the store — and the drift
/// rebuild added in this cycle would silently *change* those columns on any
/// station that triggered it.
///
/// Nothing read the six today, which is why it was latent rather than
/// broken. It is fixed rather than documented because the next analytic
/// that wants `Week` or `Lat` would have found a column that is populated
/// or not depending on station history, which is the hardest kind of bug to
/// see.
#[derive(Debug, Clone, Copy, Default)]
pub struct LiveDetection<'a> {
    /// Local civil date, `YYYY-MM-DD`.
    pub date: &'a str,
    /// Local civil time, `HH:MM:SS`.
    pub time: &'a str,
    /// Scientific name.
    pub sci_name: &'a str,
    /// Common name.
    pub com_name: &'a str,
    /// Model confidence.
    pub confidence: f64,
    /// Station latitude at the time of recording.
    pub lat: Option<f64>,
    /// Station longitude at the time of recording.
    pub lon: Option<f64>,
    /// Confidence cutoff applied at inference.
    pub cutoff: Option<f64>,
    /// ISO week number.
    pub week: Option<i32>,
    /// Sensitivity setting.
    pub sens: Option<f64>,
    /// Chunk overlap in seconds.
    pub overlap: Option<f64>,
    /// Saved clip path, or the source segment.
    pub file_name: &'a str,
    /// The instant the detection happened, seconds since the Unix epoch.
    ///
    /// Not optional the way the other analytics columns are, and not left to a
    /// resync to fill in: `detection_instant` is derived from it, and every
    /// analytic that measures elapsed time or order now reads that. A live row
    /// mirrored here without it would be present in the store, correct in every
    /// displayed field, and **absent from sessionize, funnel, retention,
    /// next-species and every gap query** until the next full rebuild happened
    /// to fold it in — which on a healthy station is never.
    ///
    /// `None` only for a row whose local wall clock names no point in time.
    pub detected_at_utc: Option<i64>,
    /// The `analysis_runs` row of the daemon start that produced it (migration
    /// 43). The live path always has one; `None` is a row no run made.
    pub run_id: Option<i64>,
}

/// Which `SQLite` rows a stream reads.
#[derive(Debug, Clone, Copy)]
enum RowFilter<'a> {
    /// Every row.
    All,
    /// Rows at or after a `"YYYY-MM-DD HH:MM:SS"` cutoff (the incremental sync).
    AtOrAfter(&'a str),
    /// Rows on one `YYYY-MM-DD` date (a per-day repair).
    OnDate(&'a str),
}

/// What one store holds for one day, reduced to a number that changes when
/// any row does (DD-23).
///
/// `digest` is an order-independent sum of per-row FNV-1a hashes over the
/// columns both stores carry and every analytic reads — `Time`, `Sci_Name`,
/// `Confidence`, `review_verdict`, `detected_at_utc` — so a delete paired with
/// a back-dated insert, which leaves every count the startup check compares
/// exactly where it was, moves the digest of the day it happened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DayFingerprint {
    /// Rows on the day.
    pub rows: u64,
    /// Order-independent digest of the rows.
    pub digest: u64,
}

impl DayFingerprint {
    /// Fold one row in.
    fn add(
        &mut self,
        time: &str,
        sci_name: &str,
        confidence: f64,
        verdict: Option<&str>,
        instant: Option<i64>,
    ) {
        let mut h = Fnv1a::new();
        h.write(time.as_bytes());
        h.write(b"\x1f");
        h.write(sci_name.as_bytes());
        h.write(b"\x1f");
        h.write(&confidence.to_bits().to_le_bytes());
        h.write(b"\x1f");
        h.write(verdict.unwrap_or("\0").as_bytes());
        h.write(b"\x1f");
        h.write(&instant.map_or([0xff; 8], i64::to_le_bytes));
        self.rows += 1;
        self.digest = self.digest.wrapping_add(h.finish());
    }
}

/// Per-day fingerprints of a store, keyed by `Date`.
pub type DayFingerprints = std::collections::BTreeMap<String, DayFingerprint>;

/// 64-bit FNV-1a: small, dependency-free, and only ever compared against
/// itself on the same machine.
struct Fnv1a(u64);

impl Fnv1a {
    const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    const fn finish(&self) -> u64 {
        self.0
    }
}

/// Above this many differing days a full rebuild is cheaper than a per-day
/// repair, and more likely to be what happened (an import, a wholesale edit).
const FULL_REBUILD_ABOVE_DAYS: usize = 60;

impl AnalyticsDb {
    /// Per-day fingerprints of the `SQLite` side, read from the source of
    /// truth. Columns the source predates (see [`VERDICT_COL`],
    /// [`INSTANT_COL`]) fold in as absent, which is what the copy holds for
    /// them too.
    ///
    /// # Errors
    ///
    /// Returns an error if the read fails.
    pub fn sqlite_day_fingerprints(
        sqlite_conn: &rusqlite::Connection,
    ) -> Result<DayFingerprints, AnalyticsError> {
        let read_err =
            |e: rusqlite::Error| AnalyticsError::InvalidData(format!("SQLite read error: {e}"));
        let verdict_col = if has_column(sqlite_conn, VERDICT_COL) {
            VERDICT_COL
        } else {
            "NULL"
        };
        let instant_col = if has_column(sqlite_conn, INSTANT_COL) {
            INSTANT_COL
        } else {
            "NULL"
        };
        let sql = format!(
            "SELECT Date, Time, Sci_Name, Confidence, {verdict_col}, {instant_col} FROM detections"
        );
        let mut stmt = sqlite_conn.prepare(&sql).map_err(read_err)?;
        let mut rows = stmt.query([]).map_err(read_err)?;
        let mut out = DayFingerprints::new();
        while let Some(row) = rows.next().map_err(read_err)? {
            let date: String = row.get(0).map_err(read_err)?;
            let time: String = row.get(1).map_err(read_err)?;
            let sci: String = row.get(2).map_err(read_err)?;
            let confidence: f64 = row.get(3).map_err(read_err)?;
            let verdict: Option<String> = row.get(4).map_err(read_err)?;
            let instant: Option<i64> = row.get(5).map_err(read_err)?;
            out.entry(date)
                .or_default()
                .add(&time, &sci, confidence, verdict.as_deref(), instant);
        }
        Ok(out)
    }

    /// Per-day fingerprints of this copy.
    ///
    /// # Errors
    ///
    /// Returns an error if the read fails.
    pub fn day_fingerprints(&self) -> Result<DayFingerprints, AnalyticsError> {
        let mut stmt = self.conn.prepare(
            "SELECT Date, Time, Sci_Name, Confidence, review_verdict, detected_at_utc \
             FROM detections",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = DayFingerprints::new();
        while let Some(row) = rows.next()? {
            let date: String = row.get(0)?;
            let time: String = row.get(1)?;
            let sci: String = row.get(2)?;
            let confidence: f64 = row.get(3)?;
            let verdict: Option<String> = row.get(4)?;
            let instant: Option<i64> = row.get(5)?;
            out.entry(date)
                .or_default()
                .add(&time, &sci, confidence, verdict.as_deref(), instant);
        }
        Ok(out)
    }

    /// The days on which this copy and `SQLite` hold different rows, sorted.
    ///
    /// A day present on one side only is out of step; so is a day whose row
    /// count agrees and whose digest does not — the net-zero drift that the
    /// count-based startup check can never see.
    ///
    /// # Errors
    ///
    /// Returns an error if either read fails.
    pub fn days_out_of_step(
        &self,
        sqlite_conn: &rusqlite::Connection,
    ) -> Result<Vec<String>, AnalyticsError> {
        let truth = Self::sqlite_day_fingerprints(sqlite_conn)?;
        let copy = self.day_fingerprints()?;
        let mut days: Vec<String> = truth
            .keys()
            .chain(copy.keys())
            .filter(|d| truth.get(*d) != copy.get(*d))
            .cloned()
            .collect();
        days.sort();
        days.dedup();
        Ok(days)
    }

    /// Rebuild the named days of this copy from `SQLite`: each day's rows are
    /// deleted and re-read from the source of truth. Returns the rows written.
    ///
    /// # Errors
    ///
    /// Returns an error if a delete, read or append fails; days already
    /// repaired stay repaired.
    pub fn repair_days(
        &self,
        sqlite_conn: &rusqlite::Connection,
        days: &[String],
    ) -> Result<u64, AnalyticsError> {
        let mut written = 0_u64;
        for day in days {
            self.conn
                .execute("DELETE FROM detections WHERE Date = ?", params![day])?;
            written +=
                self.stream_sqlite_into(sqlite_conn, "detections", RowFilter::OnDate(day))?;
        }
        if !days.is_empty() {
            self.refresh_view_from(sqlite_conn)?;
        }
        Ok(written)
    }

    /// Find every day on which this copy disagrees with `SQLite` and repair
    /// it: day by day when few days differ, by a full rebuild when many do.
    /// Returns the days that were out of step, sorted; empty means the two
    /// stores agreed.
    ///
    /// # Errors
    ///
    /// Returns an error if the comparison or the repair fails.
    pub fn repair_drift(
        &self,
        sqlite_conn: &rusqlite::Connection,
    ) -> Result<Vec<String>, AnalyticsError> {
        let days = self.days_out_of_step(sqlite_conn)?;
        if days.is_empty() {
            return Ok(days);
        }
        if days.len() > FULL_REBUILD_ABOVE_DAYS {
            let rows = self.full_resync_from_sqlite(sqlite_conn)?;
            tracing::info!(
                days = days.len(),
                rows,
                "analytics copy disagreed with the database on many days; rebuilt in full"
            );
        } else {
            let rows = self.repair_days(sqlite_conn, &days)?;
            tracing::info!(
                days = ?days,
                rows,
                "analytics copy disagreed with the database on some days; those days rebuilt"
            );
        }
        Ok(days)
    }

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

        let count = self.stream_sqlite_into(
            sqlite_conn,
            "detections",
            cutoff
                .as_deref()
                .map_or(RowFilter::All, RowFilter::AtOrAfter),
        )?;

        if count > 0 {
            self.refresh_view_from(sqlite_conn)?;
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
        let count = self.stream_sqlite_into(sqlite_conn, "detections_staging", RowFilter::All)?;

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
        self.refresh_view_from(sqlite_conn)?;
        tracing::info!(
            rows = count,
            "rebuilt DuckDB detections from SQLite (full resync)"
        );

        Ok(count)
    }

    /// Stream detections from `SQLite` straight into the named `DuckDB` table,
    /// flushing every [`APPEND_BATCH_ROWS`].
    ///
    /// `table` is an internal, hard-coded identifier (`detections` for the
    /// incremental path, `detections_staging` for the atomic full rebuild) —
    /// never untrusted input. `after` filters to rows at or after a
    /// `"YYYY-MM-DD HH:MM:SS"` cutoff; `None` reads the whole table.
    ///
    /// Rows are appended as they are read rather than collected first, so peak
    /// memory tracks the batch rather than the station's entire history — see
    /// [`APPEND_BATCH_ROWS`] for the numbers that made that necessary.
    ///
    /// A failure part-way through leaves the batches already flushed in place.
    /// For the incremental path that is strictly better than the previous
    /// all-or-nothing behaviour: the next sync recomputes its cutoff from what
    /// `DuckDB` actually holds and resumes from there, so a station that dies
    /// mid-sync makes progress instead of starting over. The full rebuild is
    /// unaffected either way — it streams into a staging table that is only
    /// swapped in once complete.
    ///
    /// Returns the number of rows appended.
    fn stream_sqlite_into(
        &self,
        sqlite_conn: &rusqlite::Connection,
        table: &str,
        filter: RowFilter<'_>,
    ) -> Result<u64, AnalyticsError> {
        let read_err =
            |e: rusqlite::Error| AnalyticsError::InvalidData(format!("SQLite read error: {e}"));

        // `>=` (not `>`): the caller deletes the cutoff second from DuckDB
        // first, then re-reads it whole from SQLite here, so rows that tie the
        // latest synced second aren't permanently skipped (see sync_from_sqlite).
        // Optional columns, in the order they are appended to the projection —
        // which is also the order the appender writes them, and therefore the
        // order of the DuckDB table's trailing columns. Kept as one list so the
        // read indices below are derived rather than hand-counted; the previous
        // shape (`row.get(if provenance { 13 } else { 12 })`) had one more term
        // to get wrong with every column added.
        let optional: Vec<&str> = [PROVENANCE_COL, VERDICT_COL, INSTANT_COL, RUN_COL]
            .into_iter()
            .filter(|c| has_column(sqlite_conn, c))
            .collect();
        let index_of = |col: &str| {
            optional
                .iter()
                .position(|c| *c == col)
                .map(|i| SYNC_COL_COUNT + i)
        };
        let (provenance, verdict, instant, run) = (
            index_of(PROVENANCE_COL),
            index_of(VERDICT_COL),
            index_of(INSTANT_COL),
            index_of(RUN_COL),
        );
        let mut cols = SYNC_COLS.to_owned();
        for col in &optional {
            cols.push_str(", ");
            cols.push_str(col);
        }
        let sql = match filter {
            RowFilter::All => format!("SELECT {cols} FROM detections ORDER BY Date, Time"),
            RowFilter::AtOrAfter(_) => format!(
                "SELECT {cols} FROM detections \
                 WHERE (Date || ' ' || Time) >= ? ORDER BY Date, Time"
            ),
            RowFilter::OnDate(_) => {
                format!("SELECT {cols} FROM detections WHERE Date = ? ORDER BY Date, Time")
            }
        };

        let mut stmt = sqlite_conn.prepare(&sql).map_err(read_err)?;
        let mut rows = match filter {
            RowFilter::AtOrAfter(ts) | RowFilter::OnDate(ts) => {
                stmt.query(rusqlite::params![ts]).map_err(read_err)?
            }
            RowFilter::All => stmt.query([]).map_err(read_err)?,
        };

        let mut appender = self.conn.appender(table)?;
        let mut total = 0_u64;
        let mut in_batch = 0_u64;

        while let Some(row) = rows.next().map_err(read_err)? {
            // Bound to this iteration: each value is moved into the appender
            // and dropped before the next row is read.
            let date: String = row.get(0).map_err(read_err)?;
            let time: String = row.get(1).map_err(read_err)?;
            let sci_name: String = row.get(2).map_err(read_err)?;
            let com_name: String = row.get(3).map_err(read_err)?;
            let confidence: f64 = row.get(4).map_err(read_err)?;
            let lat: Option<f64> = row.get(5).map_err(read_err)?;
            let lon: Option<f64> = row.get(6).map_err(read_err)?;
            let cutoff: Option<f64> = row.get(7).map_err(read_err)?;
            let week: Option<i32> = row.get(8).map_err(read_err)?;
            let sens: Option<f64> = row.get(9).map_err(read_err)?;
            let overlap: Option<f64> = row.get(10).map_err(read_err)?;
            let file_name: Option<String> = row.get(11).map_err(read_err)?;
            // Provenance (migration 25). NULL means "this station recorded it",
            // which is the answer for every row on a station that has never
            // imported anything — and the reason a merged history stays
            // separable in the analytics rather than only in SQLite. A source
            // predating the migration has no such column; NULL is then the
            // truthful value for every one of its rows.
            let import_batch_id: Option<i64> = match provenance {
                Some(i) => row.get(i).map_err(read_err)?,
                None => None,
            };
            // The reviewer's verdict (migration 26). NULL is "unreviewed",
            // which is the honest value for a source that has no such column.
            let review_verdict: Option<String> = match verdict {
                Some(i) => row.get(i).map_err(read_err)?,
                None => None,
            };
            // The instant (migration 32). NULL for a source predating it, and
            // for rows whose wall clock names no point in time — the analytics
            // view keeps those out of every ordering rather than guessing.
            let detected_at_utc: Option<i64> = match instant {
                Some(i) => row.get(i).map_err(read_err)?,
                None => None,
            };
            // The run (migration 43). NULL for a source predating it and for
            // rows this station did not analyse.
            let run_id: Option<i64> = match run {
                Some(i) => row.get(i).map_err(read_err)?,
                None => None,
            };

            appender.append_row(params![
                date,
                time,
                sci_name,
                com_name,
                confidence,
                lat,
                lon,
                cutoff,
                week,
                sens,
                overlap,
                file_name,
                import_batch_id,
                review_verdict,
                detected_at_utc,
                run_id,
            ])?;

            total += 1;
            in_batch += 1;
            if in_batch >= APPEND_BATCH_ROWS {
                appender.flush()?;
                in_batch = 0;
            }
        }

        // Always flush, even at zero rows: an unflushed appender is dropped
        // silently, so relying on the loop's flush would lose a final partial
        // batch.
        appender.flush()?;
        Ok(total)
    }

    /// Insert a single detection record directly.
    ///
    /// Use for real-time insertion alongside `SQLite` writes. `import_batch_id`
    /// and `review_verdict` are deliberately absent from [`LiveDetection`]:
    /// a live detection is by definition this station's own and unreviewed, so
    /// both are NULL, and offering them as parameters would invite a caller to
    /// say otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn insert_detection(&self, d: &LiveDetection<'_>) -> Result<(), AnalyticsError> {
        self.conn.execute(
            "INSERT INTO detections
                (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon,
                 Cutoff, Week, Sens, Overlap, File_Name, detected_at_utc, run_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                d.date,
                d.time,
                d.sci_name,
                d.com_name,
                d.confidence,
                d.lat,
                d.lon,
                d.cutoff,
                d.week,
                d.sens,
                d.overlap,
                d.file_name,
                d.detected_at_utc,
                d.run_id
            ],
        )?;
        Ok(())
    }

    /// Delete a detection from the OLAP copy, mirroring `SQLite`'s
    /// `delete_detection`.
    ///
    /// Returns the number of rows removed.
    ///
    /// # Why this exists
    ///
    /// The incremental sync can only ever *add* rows newer than the ones it
    /// already holds — it has no way to notice a removal. Without a mirror an
    /// operator's deleted false positive stayed in every behavioural and
    /// time-series dashboard permanently, because nothing else ever revisits a
    /// row once it is synced.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete fails.
    pub fn delete_detection(
        &self,
        date: &str,
        time: &str,
        sci_name: &str,
    ) -> Result<u64, AnalyticsError> {
        let n = self.conn.execute(
            "DELETE FROM detections WHERE Date = ? AND Time = ? AND Sci_Name = ?",
            params![date, time, sci_name],
        )?;
        Ok(n as u64)
    }

    /// Rebuild `detections_ts` to match the provenance rule `SQLite` is
    /// currently applying.
    ///
    /// Read from the SQLite connection rather than from a parameter because
    /// SQLite is where the setting lives, and because the alternative — passing
    /// a flag down every sync path — is how the two stores drift apart. A store
    /// that has just been synced from a database is exactly the moment to
    /// re-read the rule that database is using.
    ///
    /// A settings table that cannot be read yields "include", which is the
    /// documented default and the behaviour every station had before the setting
    /// existed.
    ///
    /// # Errors
    ///
    /// Returns an error if the view cannot be recreated.
    pub fn refresh_view_from(
        &self,
        sqlite_conn: &rusqlite::Connection,
    ) -> Result<(), AnalyticsError> {
        let exclude = sqlite_conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                rusqlite::params![queries::EXCLUDE_IMPORTS_SETTING],
                |r| r.get::<_, String>(0),
            )
            .is_ok_and(|v| v == "true");
        self.set_exclude_imports(exclude)
    }

    /// Rebuild `detections_ts` with imported detections included or excluded.
    ///
    /// Called when the operator flips the setting, so the change takes effect
    /// without waiting for the next sync or restart.
    ///
    /// # Errors
    ///
    /// Returns an error if the view cannot be recreated.
    pub fn set_exclude_imports(&self, exclude: bool) -> Result<(), AnalyticsError> {
        self.conn
            .execute_batch(&queries::detections_ts_view_sql(exclude))?;
        Ok(())
    }

    /// Remove every row an import brought in, mirroring `SQLite`'s
    /// `delete_import_batch`.
    ///
    /// Returns the number of rows removed. Without this mirror an undone import
    /// would vanish from the species lists and the heat map — which read
    /// `SQLite` — and stay in sessionize, funnel, retention, next-species,
    /// phenology and every time-series query, which read this copy. Two stores
    /// answering with two different histories is the failure `sync_from_sqlite`
    /// exists to prevent, and an incremental sync cannot notice a removal.
    ///
    /// `import_batch_id` is NULL for every locally recorded detection, so this
    /// cannot reach one.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete fails.
    pub fn delete_import_batch(&self, batch_id: i64) -> Result<u64, AnalyticsError> {
        let n = self.conn.execute(
            "DELETE FROM detections WHERE import_batch_id = ?",
            params![batch_id],
        )?;
        Ok(n as u64)
    }

    /// Re-label a detection in the OLAP copy, mirroring `SQLite`'s
    /// `relabel_detection`.
    ///
    /// Returns the number of rows updated. See [`Self::delete_detection`] for
    /// why the mirror is needed at all.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn relabel_detection(
        &self,
        date: &str,
        time: &str,
        old_sci_name: &str,
        new_sci_name: &str,
        new_com_name: &str,
    ) -> Result<u64, AnalyticsError> {
        let n = self.conn.execute(
            "UPDATE detections SET Sci_Name = ?, Com_Name = ? \
             WHERE Date = ? AND Time = ? AND Sci_Name = ?",
            params![new_sci_name, new_com_name, date, time, old_sci_name],
        )?;
        Ok(n as u64)
    }

    /// Empty the OLAP detections copy, mirroring the admin "clear detections"
    /// control.
    ///
    /// Returns the number of rows removed. See [`Self::delete_detection`] for
    /// why the mirror is needed at all; this is the case where its absence was
    /// most visible, since the station's own dashboard reported zero detections
    /// while the analytics dashboards beside it still rendered a full history.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete fails.
    pub fn clear_detections(&self) -> Result<u64, AnalyticsError> {
        let n = self.conn.execute("DELETE FROM detections", params![])?;
        Ok(n as u64)
    }

    /// Copy the station's recording-effort table into the OLAP store.
    ///
    /// Small by construction — one row per (day, source), so a four-year
    /// three-microphone station has about 4 400 of them — so this is a full
    /// replace rather than an incremental sync. Effort rows are also *mutable*
    /// (today's row is incremented every five minutes), which an
    /// append-only incremental sync could not track.
    ///
    /// Without this the effort-corrected abundance query has no denominator in
    /// the store it runs in, and silently returns NULL rates — the failure mode
    /// that made the whole phenology module look optional.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `SQLite` or writing to `DuckDB` fails.
    pub fn sync_recording_effort(
        &self,
        sqlite_conn: &rusqlite::Connection,
    ) -> Result<u64, AnalyticsError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS recording_effort (
                date VARCHAR NOT NULL,
                source VARCHAR NOT NULL,
                seconds DOUBLE NOT NULL
            );",
        )?;

        let read_err =
            |e: rusqlite::Error| AnalyticsError::InvalidData(format!("SQLite read error: {e}"));
        let mut stmt = sqlite_conn
            .prepare("SELECT date, source, seconds FROM recording_effort")
            .map_err(read_err)?;
        let rows: Vec<(String, String, f64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(read_err)?
            .filter_map(Result::ok)
            .collect();

        self.conn.execute_batch("DELETE FROM recording_effort;")?;
        let mut appender = self.conn.appender("recording_effort")?;
        for (date, source, seconds) in &rows {
            appender.append_row(params![date, source, seconds])?;
        }
        appender.flush()?;
        Ok(rows.len() as u64)
    }

    /// Mirror a reviewer's verdict onto the OLAP copy.
    ///
    /// `verdict` is `Some("confirmed")` / `Some("rejected")`, or `None` to
    /// return the detection to unreviewed. `detections_ts` — the view every
    /// analytic reads — filters on this column, so without the mirror a
    /// rejection changes the SQLite aggregates and leaves every behavioural and
    /// time-series dashboard still counting the reject.
    ///
    /// Returns the number of rows updated.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn set_review_verdict(
        &self,
        date: &str,
        time: &str,
        sci_name: &str,
        verdict: Option<&str>,
    ) -> Result<u64, AnalyticsError> {
        let n = self.conn.execute(
            "UPDATE detections SET review_verdict = ? \
             WHERE Date = ? AND Time = ? AND Sci_Name = ?",
            params![verdict, date, time, sci_name],
        )?;
        Ok(n as u64)
    }

    /// Count detections carrying a `rejected` verdict.
    ///
    /// Used by the startup drift check. Row counts alone cannot see verdict
    /// drift — a rejection changes no row count in either store — so a station
    /// whose verdicts diverged would never self-heal without this.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn rejected_detection_count(&self) -> Result<u64, AnalyticsError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM detections WHERE review_verdict = 'rejected'",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Count detections carrying no monotonic instant.
    ///
    /// The third signal the startup drift check needs, and the reason it needs
    /// a third: `detected_at_utc` (migration 32) adds a *column*, not rows, so a
    /// store synced before it exists agrees with SQLite on both the row count
    /// and the rejected count and disagrees on every value in this one.
    ///
    /// Left unnoticed that is not a cosmetic difference. `detection_instant` is
    /// NULL for those rows, and every analytic that measures elapsed time or
    /// order now reads it — so a station upgrading with a populated analytics
    /// store would find sessionize, funnel, retention, next-species and every
    /// gap query silently returning nothing, with both stores answering every
    /// query they were asked. That is precisely the failure this whole check
    /// was written for, in a new disguise.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn unstamped_detection_count(&self) -> Result<u64, AnalyticsError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM detections WHERE detected_at_utc IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(n).unwrap_or(0))
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

    /// Count synced detections whose `Date`/`Time` names no point in time.
    ///
    /// These rows are present in the OLAP copy and in `SELECT COUNT(*)`, but
    /// carry a NULL `detection_timestamp` (see
    /// [`CREATE_DETECTIONS_TS_VIEW`](crate::queries::CREATE_DETECTIONS_TS_VIEW))
    /// and so are absent from every time-bucketed analytic. A non-zero count is
    /// the reason a dashboard total can sit below the station's raw detection
    /// count, and is worth surfacing rather than leaving for an operator to
    /// discover by arithmetic.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn unplaceable_detection_count(&self) -> Result<u64, AnalyticsError> {
        let count: i64 = self
            .conn
            .query_row(queries::COUNT_UNPLACEABLE_DETECTIONS, [], |row| row.get(0))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_db() -> (AnalyticsDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).unwrap();
        (db, dir)
    }

    /// One unparseable `Date`/`Time` must not take every analytics query down
    /// with it.
    ///
    /// The BirdNET-Pi importer maps a NULL `Date` to `""` and passes malformed
    /// values through verbatim, and `Date TEXT NOT NULL` constrains neither —
    /// so a real station's history can carry rows no calendar can place. With a
    /// plain `CAST` in `detections_ts` those rows did not degrade the analytics,
    /// they *aborted* it: DuckDB raises `Conversion Error` for the whole query,
    /// so one bad row in a multi-year history emptied every behavioural and
    /// time-series dashboard while the rest of the app — served from SQLite —
    /// looked perfectly healthy.
    ///
    /// `COUNT(*)` over the view is deliberately asserted too: it keeps working
    /// either way, because DuckDB never evaluates the projected columns, which
    /// is why the health checks stayed green throughout.
    #[test]
    fn one_unparseable_date_does_not_abort_every_analytics_query() {
        let (db, _tmp) = make_db();
        let sqlite_dir = TempDir::new().unwrap();
        let sc = rusqlite::Connection::open(sqlite_dir.path().join("b.db")).unwrap();
        sc.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
             Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections VALUES ('2026-03-12','06:30:00','Turdus merula','Blackbird',0.87,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
             INSERT INTO detections VALUES ('','','Parus major','Great Tit',0.75,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
             INSERT INTO detections VALUES ('not-a-date','25:99:99','Corvus corax','Raven',0.60,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
             INSERT INTO detections VALUES ('2026-03-13','07:00:00','Parus major','Great Tit',0.75,NULL,NULL,NULL,NULL,NULL,NULL,NULL);",
        ).unwrap();

        // The bad rows sync without complaint — nothing upstream rejects them.
        assert_eq!(db.sync_from_sqlite(&sc).unwrap(), 4);
        assert_eq!(db.detection_count().unwrap(), 4);

        // Never evaluated the cast columns, so this passed even when broken.
        let total: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM detections_ts", [], |r| r.get(0))
            .expect("COUNT(*) over the view");
        assert_eq!(total, 4);

        // The two queries that actually broke: both touch a cast column.
        let by_timestamp: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM detections_ts WHERE detection_timestamp > '2000-01-01'",
                [],
                |r| r.get(0),
            )
            .expect("filtering on detection_timestamp must not raise");
        assert_eq!(by_timestamp, 2, "only the two placeable rows are counted");

        let distinct_days: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT detection_date) FROM detections_ts",
                [],
                |r| r.get(0),
            )
            .expect("grouping by detection_date must not raise");
        assert_eq!(
            distinct_days, 2,
            "12th and 13th; the unplaceable rows drop out"
        );

        // Unplaceable rows are excluded rather than coerced to some epoch date,
        // which would invent detections on 1970-01-01.
        let null_ts: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM detections_ts WHERE detection_timestamp IS NULL",
                [],
                |r| r.get(0),
            )
            .expect("counting unplaceable rows must not raise");
        assert_eq!(null_ts, 2);

        // The exclusion is reportable, not silent.
        assert_eq!(db.unplaceable_detection_count().unwrap(), 2);
        assert_eq!(
            db.detection_count().unwrap() - db.unplaceable_detection_count().unwrap(),
            2,
            "raw count minus unplaceable is what the dashboards can actually show"
        );
    }

    /// A minimal live detection for tests.
    ///
    /// The optional columns are left `None` here because these tests are about
    /// insert/count behaviour; `a_live_row_and_a_resynced_row_carry_the_same_columns`
    /// in `tests/analytics_divergence.rs` is what holds the twelve-column
    /// contract.
    fn live<'a>(
        date: &'a str,
        time: &'a str,
        sci_name: &'a str,
        com_name: &'a str,
        confidence: f64,
        file_name: &'a str,
    ) -> LiveDetection<'a> {
        LiveDetection {
            date,
            time,
            sci_name,
            com_name,
            confidence,
            file_name,
            ..LiveDetection::default()
        }
    }

    #[test]
    fn insert_and_count() {
        let (db, _tmp) = make_db();
        db.insert_detection(&live(
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.87,
            "t.wav",
        ))
        .unwrap();
        db.insert_detection(&live(
            "2026-03-12",
            "06:35:00",
            "Erithacus rubecula",
            "European Robin",
            0.92,
            "t.wav",
        ))
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
        db.insert_detection(&live(
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Blackbird",
            0.87,
            "t.wav",
        ))
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
        db.insert_detection(&live(
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Blackbird",
            0.87,
            "t.wav",
        ))
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
        db.insert_detection(&live(
            "2026-06-05",
            "10:00:00",
            "Parus major",
            "Great Tit",
            0.90,
            "now.wav",
        ))
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
        db.insert_detection(&live(
            "2026-06-05",
            "10:00:00",
            "Parus major",
            "Great Tit",
            0.9,
            "x.wav",
        ))
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

    /// The gate for DD-23: a delete paired with a back-dated insert leaves
    /// every count the startup check compares unchanged, and used to leave the
    /// copy permanently wrong. The day fingerprint sees it, and only that day
    /// is rebuilt.
    #[test]
    fn net_zero_drift_is_found_by_the_day_fingerprint_and_only_that_day_is_rebuilt() {
        let (db, _tmp) = make_db();
        let sc = rusqlite::Connection::open_in_memory().unwrap();
        sc.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT, \
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER, Sens REAL, \
             Overlap REAL, File_Name TEXT, review_verdict TEXT, detected_at_utc INTEGER);",
        )
        .unwrap();
        for (date, time, sci, com) in [
            ("2026-03-11", "06:00:00", "Turdus merula", "Blackbird"),
            ("2026-03-11", "06:30:00", "Parus major", "Great Tit"),
            ("2026-03-12", "06:00:00", "Turdus merula", "Blackbird"),
            ("2026-03-12", "07:15:00", "Erithacus rubecula", "Robin"),
            ("2026-03-13", "06:00:00", "Turdus merula", "Blackbird"),
            ("2026-03-13", "06:30:00", "Parus major", "Great Tit"),
        ] {
            sc.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
                 VALUES (?1, ?2, ?3, ?4, 0.85)",
                rusqlite::params![date, time, sci, com],
            )
            .unwrap();
        }
        assert_eq!(db.full_resync_from_sqlite(&sc).unwrap(), 6);
        assert!(db.days_out_of_step(&sc).unwrap().is_empty(), "fixture");

        // The drift: the robin is deleted and an owl is written back-dated
        // onto the same day, in SQLite only.
        sc.execute(
            "DELETE FROM detections WHERE Date = '2026-03-12' AND Sci_Name = 'Erithacus rubecula'",
            [],
        )
        .unwrap();
        sc.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
             VALUES ('2026-03-12', '03:10:00', 'Strix aluco', 'Tawny Owl', 0.91)",
            [],
        )
        .unwrap();

        // Vacuity guard: after the incremental sync (which re-reads only the
        // latest second, on another day) every signal the count-based check
        // compares agrees.
        db.sync_from_sqlite(&sc).unwrap();
        let sqlite_rows: i64 = sc
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            u64::try_from(sqlite_rows).unwrap(),
            db.detection_count().unwrap()
        );
        assert_eq!(db.rejected_detection_count().unwrap(), 0);
        let owl_before: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Sci_Name = 'Strix aluco'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(owl_before, 0, "the copy is wrong, as it would be");

        assert_eq!(
            db.days_out_of_step(&sc).unwrap(),
            vec!["2026-03-12".to_string()],
            "the day fingerprint must name exactly the day that drifted"
        );
        assert_eq!(
            db.repair_drift(&sc).unwrap(),
            vec!["2026-03-12".to_string()]
        );

        let count = |sci: &str| -> i64 {
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM detections WHERE Sci_Name = ?",
                    params![sci],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(
            count("Strix aluco"),
            1,
            "the back-dated owl reached the copy"
        );
        assert_eq!(count("Erithacus rubecula"), 0, "the deleted robin left it");
        assert_eq!(db.detection_count().unwrap(), 6);
        assert!(db.days_out_of_step(&sc).unwrap().is_empty(), "repaired");

        // Counterpart: the other days were not touched — their rows are the
        // same, and a repair of nothing writes nothing.
        assert_eq!(db.repair_days(&sc, &[]).unwrap(), 0);
        let per_day: Vec<(String, i64)> = {
            let mut st = db
                .conn
                .prepare("SELECT Date, COUNT(*) FROM detections GROUP BY Date ORDER BY Date")
                .unwrap();
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert_eq!(
            per_day,
            vec![
                ("2026-03-11".to_string(), 2),
                ("2026-03-12".to_string(), 2),
                ("2026-03-13".to_string(), 2)
            ]
        );
    }

    /// A verdict is part of the fingerprint: rejecting a row in `SQLite` alone
    /// moves that day, and no other.
    #[test]
    fn a_changed_verdict_moves_only_its_day() {
        let (db, _tmp) = make_db();
        let sc = rusqlite::Connection::open_in_memory().unwrap();
        sc.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT, \
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER, Sens REAL, \
             Overlap REAL, File_Name TEXT, review_verdict TEXT, detected_at_utc INTEGER);",
        )
        .unwrap();
        for date in ["2026-03-11", "2026-03-12"] {
            sc.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
                 VALUES (?1, '06:00:00', 'Turdus merula', 'Blackbird', 0.85)",
                rusqlite::params![date],
            )
            .unwrap();
        }
        db.full_resync_from_sqlite(&sc).unwrap();
        sc.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Date = '2026-03-12'",
            [],
        )
        .unwrap();
        assert_eq!(
            db.days_out_of_step(&sc).unwrap(),
            vec!["2026-03-12".to_string()]
        );
        db.repair_drift(&sc).unwrap();
        assert_eq!(db.rejected_detection_count().unwrap(), 1);
    }
}
