//! Opt-in per-window scores, including windows that produced no detection
//! (RES-04, decision 4 of 2026-11-25, tier 2).
//!
//! Off: no setting turns it on yet, and the analyser does not call it yet
//! (the hook is reported with the change). When on, every analysed window of every
//! classifier is one row: when it started, its source and recording, the run
//! and classifier that analysed it, and every label scored at or above a floor,
//! packed. A window with nothing at the floor is still a row (`n = 0`): it was
//! analysed and found nothing that high, which is the record `detections`
//! cannot hold. Its scores are the model's outputs, not probabilities, and a
//! window with no score above the floor is not an absence.
//!
//! # Where it lives, and what it costs
//!
//! In its own database file, never the station's: so it stays out of the
//! fourteen-copy backup ring, and dropping it loses nothing else. Rows older
//! than the retention are pruned by [`WindowScoreStore::prune`]. The cost per
//! window is the key (about 60 bytes on disk with `SQLite`'s overhead) plus six
//! bytes per label at or above the floor; [`estimate_bytes_per_day`] states the
//! arithmetic, and `the_cost_of_a_day_is_measured_not_guessed` in this module
//! measures a synthetic day on disk: one source at defaults (28,800 windows),
//! two labels per window, 4,481,024 bytes (about 4.5 MB a day, 134 MB at the
//! default 30-day retention, 1.6 GB a year if kept). Neither is a measurement on real audio:
//! how many labels cross a given floor per window depends on the soundscape.
//!
//! # Privacy
//!
//! Labels in [`WindowScoresConfig::excluded_labels`] (the human and sound
//! classes, which the caller names) are never written, whatever their score:
//! a per-window human score is a record of speech to the second.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

/// The store's schema version, in its `store_meta`.
pub const WINDOW_SCORES_SCHEMA: &str = "window-scores-1";

/// Bytes one packed label takes: a `u16` label index and an `f32` score.
pub const PACKED_LABEL_BYTES: usize = 6;

/// How the store is set up.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowScoresConfig {
    /// Scores below this are not kept. A lower floor keeps more and costs more.
    pub floor: f32,
    /// Days a window is kept.
    pub retention_days: u32,
    /// Label indices never written.
    pub excluded_labels: BTreeSet<u16>,
}

/// The floor a store keeps scores from when none is given.
pub const DEFAULT_FLOOR: f32 = 0.1;

/// Days a window is kept when no retention is given.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

/// The longest retention a store accepts, in days.
pub const MAX_RETENTION_DAYS: u32 = 3650;

impl WindowScoresConfig {
    /// A configuration within bounds: a floor outside 0 to 1 (or not a
    /// number) takes [`DEFAULT_FLOOR`], and a retention outside 1 to
    /// [`MAX_RETENTION_DAYS`] takes [`DEFAULT_RETENTION_DAYS`].
    ///
    /// The store has no setting of its own yet: there is nothing that turns it
    /// on, so a station records nothing here until the opt-in is wired.
    #[must_use]
    pub fn bounded(floor: f32, retention_days: u32) -> Self {
        Self {
            floor: if floor.is_finite() && (0.0..=1.0).contains(&floor) {
                floor
            } else {
                DEFAULT_FLOOR
            },
            retention_days: if (1..=MAX_RETENTION_DAYS).contains(&retention_days) {
                retention_days
            } else {
                DEFAULT_RETENTION_DAYS
            },
            excluded_labels: BTreeSet::new(),
        }
    }
}

/// One analysed window, as the analyser hands it over.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowRecord<'a> {
    /// When the window's first sample was captured, Unix milliseconds UTC.
    pub start_utc_ms: i64,
    /// The source.
    pub source: &'a str,
    /// The recording's file name.
    pub recording: &'a str,
    /// The window's start within the recording, seconds.
    pub offset_secs: f64,
    /// How much of the window was audio (not padding), seconds.
    pub audio_secs: f64,
    /// `analysis_runs.id`.
    pub run_id: i64,
    /// The classifier.
    pub model_id: &'a str,
    /// Every label's score, by label index (the model's full output, or any
    /// superset of what crosses the floor).
    pub scores: &'a [(u16, f32)],
}

/// A window read back.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredWindow {
    /// Unix milliseconds UTC.
    pub start_utc_ms: i64,
    /// The source.
    pub source: String,
    /// The recording.
    pub recording: String,
    /// Offset in it.
    pub offset_secs: f64,
    /// Audio in the window.
    pub audio_secs: f64,
    /// The run.
    pub run_id: i64,
    /// The classifier.
    pub model_id: String,
    /// The floor it was recorded at.
    pub floor: f64,
    /// Labels at or above the floor, by index, in index order.
    pub scores: Vec<(u16, f32)>,
}

/// The opt-in store.
#[derive(Debug)]
pub struct WindowScoreStore {
    conn: Connection,
    config: WindowScoresConfig,
}

fn pack(scores: &[(u16, f32)], floor: f32, excluded: &BTreeSet<u16>) -> Vec<u8> {
    let mut kept: Vec<(u16, f32)> = scores
        .iter()
        .copied()
        .filter(|(i, s)| s.is_finite() && *s >= floor && !excluded.contains(i))
        .collect();
    kept.sort_by_key(|(i, _)| *i);
    kept.dedup_by_key(|(i, _)| *i);
    let mut out = Vec::with_capacity(kept.len() * PACKED_LABEL_BYTES);
    for (i, s) in kept {
        out.extend_from_slice(&i.to_le_bytes());
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

fn unpack(blob: &[u8]) -> Vec<(u16, f32)> {
    blob.as_chunks::<PACKED_LABEL_BYTES>()
        .0
        .iter()
        .map(|c| {
            (
                u16::from_le_bytes([c[0], c[1]]),
                f32::from_le_bytes([c[2], c[3], c[4], c[5]]),
            )
        })
        .collect()
}

impl WindowScoreStore {
    /// Open (or create) the store at `path`.
    ///
    /// # Errors
    ///
    /// `DbError` when the file cannot be opened or its schema written.
    pub fn open(path: &Path, config: WindowScoresConfig) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS store_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS window_scores (
                 start_utc_ms INTEGER NOT NULL,
                 source       TEXT NOT NULL,
                 model_id     TEXT NOT NULL,
                 recording    TEXT NOT NULL,
                 offset_secs  REAL NOT NULL,
                 audio_secs   REAL NOT NULL,
                 run_id       INTEGER NOT NULL,
                 floor        REAL NOT NULL,
                 n            INTEGER NOT NULL CHECK (n >= 0),
                 scores       BLOB NOT NULL,
                 PRIMARY KEY (source, model_id, start_utc_ms, recording, offset_secs)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS window_scores_time ON window_scores (start_utc_ms);",
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO store_meta (key, value) VALUES ('schema', ?1)",
            [WINDOW_SCORES_SCHEMA],
        )?;
        Ok(Self { conn, config })
    }

    /// Record one window. A window already recorded (same source, classifier,
    /// start, recording and offset) is left as it was: a retry is not a second
    /// window.
    ///
    /// # Errors
    ///
    /// `DbError` on insert failure.
    pub fn record(&self, w: &WindowRecord<'_>) -> Result<(), DbError> {
        let blob = pack(w.scores, self.config.floor, &self.config.excluded_labels);
        let n = i64::try_from(blob.len() / PACKED_LABEL_BYTES).unwrap_or(i64::MAX);
        self.conn.execute(
            "INSERT OR IGNORE INTO window_scores
                 (start_utc_ms, source, model_id, recording, offset_secs, audio_secs, run_id,
                  floor, n, scores)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                w.start_utc_ms,
                w.source,
                w.model_id,
                w.recording,
                w.offset_secs,
                w.audio_secs,
                w.run_id,
                f64::from(self.config.floor),
                n,
                blob
            ],
        )?;
        Ok(())
    }

    /// Delete windows older than the retention, relative to `now_ms`; how many.
    ///
    /// # Errors
    ///
    /// `DbError` on delete failure.
    pub fn prune(&self, now_ms: i64) -> Result<u64, DbError> {
        let keep_ms = i64::from(self.config.retention_days) * 86_400_000;
        let n = self.conn.execute(
            "DELETE FROM window_scores WHERE start_utc_ms < ?1",
            [now_ms.saturating_sub(keep_ms)],
        )?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Windows starting in `[from_ms, to_ms)`, in time order.
    ///
    /// # Errors
    ///
    /// `DbError` on query failure.
    pub fn windows(&self, from_ms: i64, to_ms: i64) -> Result<Vec<StoredWindow>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT start_utc_ms, source, recording, offset_secs, audio_secs, run_id, model_id,
                    floor, scores
               FROM window_scores WHERE start_utc_ms >= ?1 AND start_utc_ms < ?2
              ORDER BY start_utc_ms, source, model_id, recording, offset_secs",
        )?;
        let rows = stmt
            .query_map([from_ms, to_ms], |r| {
                Ok(StoredWindow {
                    start_utc_ms: r.get(0)?,
                    source: r.get(1)?,
                    recording: r.get(2)?,
                    offset_secs: r.get(3)?,
                    audio_secs: r.get(4)?,
                    run_id: r.get(5)?,
                    model_id: r.get(6)?,
                    floor: r.get(7)?,
                    scores: unpack(&r.get::<_, Vec<u8>>(8)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// Bytes a day of windows costs, by arithmetic: `windows_per_day` windows each
/// with `labels_per_window` labels at or above the floor, at
/// `key_bytes` per row. A planning figure, not a measurement.
#[must_use]
pub const fn estimate_bytes_per_day(
    windows_per_day: u64,
    labels_per_window: u64,
    key_bytes: u64,
) -> u64 {
    windows_per_day * (key_bytes + labels_per_window * PACKED_LABEL_BYTES as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> WindowScoresConfig {
        WindowScoresConfig {
            floor: 0.1,
            retention_days: 30,
            excluded_labels: BTreeSet::from([7]),
        }
    }

    fn window(start: i64, scores: &[(u16, f32)]) -> WindowRecord<'_> {
        WindowRecord {
            start_utc_ms: start,
            source: "mic",
            recording: "a.wav",
            offset_secs: 0.0,
            audio_secs: 3.0,
            run_id: 1,
            model_id: "birdnet",
            scores,
        }
    }

    /// The decision's acceptance gate: a window whose best score is 0.4,
    /// against a 0.75 detection threshold, is recorded with that score.
    #[test]
    fn a_window_below_the_threshold_is_recorded_with_its_score() {
        let dir = tempfile::tempdir().unwrap();
        let store = WindowScoreStore::open(&dir.path().join("w.db"), config()).unwrap();
        store
            .record(&window(1_000, &[(3, 0.4), (4, 0.05), (7, 0.9)]))
            .unwrap();
        store.record(&window(4_000, &[(3, 0.02)])).unwrap();
        let w = store.windows(0, 10_000).unwrap();
        assert_eq!(
            w.len(),
            2,
            "a window with nothing at the floor is still a row"
        );
        assert_eq!(
            w[0].scores,
            vec![(3, 0.4)],
            "floor applied, excluded label dropped"
        );
        assert!(w[1].scores.is_empty());
    }

    #[test]
    fn a_retried_window_is_not_a_second_window_and_old_ones_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let store = WindowScoreStore::open(&dir.path().join("w.db"), config()).unwrap();
        store.record(&window(1_000, &[(1, 0.5)])).unwrap();
        store.record(&window(1_000, &[(1, 0.9)])).unwrap();
        assert_eq!(store.windows(0, 2_000).unwrap()[0].scores, vec![(1, 0.5)]);
        let day = 86_400_000;
        store.record(&window(40 * day, &[])).unwrap();
        assert_eq!(store.prune(40 * day + 1).unwrap(), 1, "the 30-day-old one");
        assert_eq!(store.windows(0, i64::MAX).unwrap().len(), 1);
    }

    #[test]
    fn a_configuration_out_of_bounds_takes_the_default() {
        let c = WindowScoresConfig::bounded(7.0, 0);
        assert!((c.floor - DEFAULT_FLOOR).abs() < f32::EPSILON);
        assert_eq!(c.retention_days, DEFAULT_RETENTION_DAYS);
        let c = WindowScoresConfig::bounded(0.25, 7);
        assert!((c.floor - 0.25).abs() < f32::EPSILON);
        assert_eq!(c.retention_days, 7);
    }

    /// What a day costs on disk, for the documentation: one source at
    /// defaults (3 s windows, no overlap, 28,800 a day), two labels at the
    /// floor per window. Synthetic scores; real audio decides the label count.
    #[test]
    fn the_cost_of_a_day_is_measured_not_guessed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.db");
        let store = WindowScoreStore::open(&path, config()).unwrap();
        store.conn.execute_batch("BEGIN").unwrap();
        let rec = "20260501-060000.wav";
        for i in 0..28_800_i64 {
            let scores = [(u16::try_from(i % 6000).unwrap(), 0.2), (11, 0.15)];
            store
                .record(&WindowRecord {
                    start_utc_ms: i * 3_000,
                    recording: rec,
                    #[allow(clippy::cast_precision_loss)]
                    offset_secs: (i % 20) as f64 * 3.0,
                    ..window(0, &scores)
                })
                .unwrap();
        }
        store.conn.execute_batch("COMMIT").unwrap();
        store
            .conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let bytes = std::fs::metadata(&path).unwrap().len();
        eprintln!("window_scores: one synthetic day = {bytes} bytes");
        assert!(bytes > 28_800 * 12, "{bytes}");
        assert!(bytes < 28_800 * 200, "{bytes}");
    }
}
