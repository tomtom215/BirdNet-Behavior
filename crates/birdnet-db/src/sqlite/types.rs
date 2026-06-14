//! Row types shared across `SQLite` query modules.

/// A detection record for database insertion.
///
/// Numeric fields are `Option<f64>` / `Option<i64>` so missing values
/// land in SQLite as `NULL` rather than the empty-string TEXT that
/// would silently corrupt the column type (the schema declares these
/// columns as `REAL` / `INTEGER`). Storing them as TEXT used to make
/// every subsequent read fail with `Invalid column type Text at index N`.
#[derive(Debug, Clone)]
pub struct DetectionRecord<'a> {
    /// Detection date (YYYY-MM-DD).
    pub date: &'a str,
    /// Detection time (HH:MM:SS).
    pub time: &'a str,
    /// Scientific name.
    pub sci_name: &'a str,
    /// Common name.
    pub com_name: &'a str,
    /// Confidence score.
    pub confidence: f64,
    /// Latitude (decimal degrees) — `None` when no station coords configured.
    pub lat: Option<f64>,
    /// Longitude (decimal degrees) — `None` when no station coords configured.
    pub lon: Option<f64>,
    /// Confidence cutoff threshold applied at inference time.
    pub cutoff: Option<f64>,
    /// ISO week number (1–53).
    pub week: Option<i64>,
    /// Sensitivity setting (typically 0.5–1.5).
    pub sensitivity: Option<f64>,
    /// Overlap setting (typically 0.0–2.9 seconds).
    pub overlap: Option<f64>,
    /// Extracted audio filename, relative to the recordings dir.
    pub file_name: &'a str,
    /// Start offset of the chunk within the source file, in seconds. `None`
    /// when the row pre-dates migration 11 (e.g. imported BirdNET-Pi data).
    ///
    /// Together with `(Date, Time, Sci_Name, File_Name)` this gives a unique
    /// key that survives chunked recordings — see migration 11.
    pub chunk_offset_secs: Option<f64>,
    /// Correlation id stamped on every event for one audio file.
    ///
    /// `None` for rows that pre-date migration 12 (historical, imported
    /// from BirdNET-Pi, or written by the quarantine-approve path that
    /// doesn't have a daemon-generated id). When present, an operator
    /// can grep the daemon log for this exact string to see every
    /// decode/infer/notify line that produced this row.
    pub correlation_id: Option<&'a str>,
    /// Audio source/stream label this detection came from — an RTSP stream id
    /// (e.g. `cam1`) or `local` for the on-board microphone.
    ///
    /// `None` for rows that pre-date migration 18 (historical / imported
    /// BirdNET-Pi data, where the source is unknown). Tagging every detection is
    /// non-destructive — it lets multi-stream stations attribute detections to a
    /// source and (later, opt-in) collapse cross-stream duplicates.
    pub source: Option<&'a str>,
}

/// A detection row read from the database.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectionRow {
    /// Detection date.
    pub date: String,
    /// Detection time.
    pub time: String,
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Confidence score.
    pub confidence: f64,
    /// Latitude.
    pub lat: Option<f64>,
    /// Longitude.
    pub lon: Option<f64>,
    /// Cutoff threshold.
    pub cutoff: Option<f64>,
    /// ISO week number.
    pub week: Option<i32>,
    /// Sensitivity setting.
    pub sens: Option<f64>,
    /// Overlap setting.
    pub overlap: Option<f64>,
    /// Extracted audio filename.
    pub file_name: Option<String>,
    /// Correlation id of the daemon event that produced this row.
    ///
    /// `None` for rows that pre-date migration 12. When present, it
    /// uniquely identifies the per-file log slice in the daemon's
    /// `tracing` stream so an operator can click the row in the web
    /// UI and run `journalctl -u birdnet | grep <id>` to see the
    /// decode/infer/notify lines that produced this detection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Audio source/stream label (RTSP stream id like `cam1`, or `local`).
    /// `None` for rows that pre-date migration 18.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// A concurrent detection of the same species from a *different* audio source.
///
/// Powers the multi-stream "also heard by" corroboration display: when several
/// streams hear the same bird at nearly the same time, that agreement is a
/// stronger signal the detection is real. Read-only and non-destructive — it
/// never merges or hides rows.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConcurrentDetection {
    /// Source/stream label that also heard this species (e.g. `cam2`).
    pub source: String,
    /// Time of that detection (HH:MM:SS).
    pub time: String,
    /// Confidence of that detection.
    pub confidence: f64,
}

/// Species with detection count and average confidence.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeciesCount {
    /// Common name.
    pub com_name: String,
    /// Scientific name.
    pub sci_name: String,
    /// Total detection count.
    pub count: i64,
    /// Average confidence score.
    pub avg_confidence: f64,
}

/// Hourly detection count.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HourlyCount {
    /// Hour string (00-23).
    pub hour: String,
    /// Number of detections.
    pub count: i64,
}

/// Daily detection count.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyCount {
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Number of detections on this date.
    pub count: i64,
}

/// Species summary with statistics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeciesSummary {
    /// Common name.
    pub com_name: String,
    /// Scientific name.
    pub sci_name: String,
    /// Total detection count.
    pub count: i64,
    /// Average confidence score.
    pub avg_confidence: f64,
    /// First detection date (YYYY-MM-DD).
    pub first_seen: String,
    /// Last detection date (YYYY-MM-DD).
    pub last_seen: String,
}

/// Per-source detection activity for a single day.
///
/// Powers the Station Health per-source panel. It is an honest **activity**
/// signal — the web process has no live handle on the capture supervisor's
/// per-stream state (live / backing-off / stalled), so Health reports what the
/// data actually shows: how many detections each source contributed today and
/// how recently the last one landed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceActivity {
    /// Source/stream label (the `detections.Source` value). `None` for rows
    /// that pre-date multi-stream tagging (migration 18) — shown as "unlabelled".
    pub source: Option<String>,
    /// Detections attributed to this source on the queried date.
    pub count: i64,
    /// The most recent detection time (HH:MM:SS) from this source that day.
    pub last_time: Option<String>,
}

/// Helper: map a `rusqlite::Row` to `DetectionRow`.
pub(super) fn map_detection_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DetectionRow> {
    Ok(DetectionRow {
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
        correlation_id: row.get(12)?,
        source: row.get(13)?,
    })
}

/// Source-of-truth list of every column projected by detection queries.
///
/// Paired with [`DETECTION_COLS`] (the joined-string SQL projection) and
/// [`map_detection_row`] (the row-to-struct mapper). The
/// `detection_cols_*` drift-gate tests below assert all three stay in
/// lockstep — a column added here without matching updates in the
/// drift-prone surfaces is caught by a failing unit test rather than a
/// silent runtime error.
///
/// Order matters: indexes match the `row.get(N)` calls in
/// [`map_detection_row`]. PR #35 shipped a column-list / row-mapper
/// drift bug; the names list is the structural guarantee that the
/// same shape of drift can't recur silently.
#[cfg(test)]
pub(super) const DETECTION_COL_NAMES: &[&str] = &[
    "Date",
    "Time",
    "Sci_Name",
    "Com_Name",
    "Confidence",
    "Lat",
    "Lon",
    "Cutoff",
    "Week",
    "Sens",
    "Overlap",
    "File_Name",
    "correlation_id",
    "Source",
];

/// Columns selected in all full-row detection queries.
///
/// Must equal `DETECTION_COL_NAMES.join(", ")` — the
/// `detection_cols_matches_names` test pins the invariant.
pub(super) const DETECTION_COLS: &str = "Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, correlation_id, Source";

#[cfg(test)]
mod drift_gate_tests {
    //! Drift-gate tests for the `DETECTION_COLS` / `map_detection_row` /
    //! `DETECTION_COL_NAMES` triple.
    //!
    //! Migration 12 (`correlation_id`) needed three coordinated edits:
    //! 1. column added in the migration SQL,
    //! 2. field added in `DetectionRow`,
    //! 3. column added in `DETECTION_COLS` AND in `map_detection_row`
    //!    (at a matching index).
    //!
    //! Steps 1–2 fail at compile time when missed. Step 3 used to fail
    //! at runtime with the cryptic *"Invalid column type Text at index N"*
    //! that PR #35 spent half a day chasing. These tests turn that into
    //! a unit-test failure with a directly-actionable message.
    use super::{DETECTION_COL_NAMES, DETECTION_COLS};
    use rusqlite::Connection;

    #[test]
    fn detection_cols_matches_names() {
        // DETECTION_COLS is the joined form of DETECTION_COL_NAMES — adding
        // a column to one without the other surfaces here, not in production.
        assert_eq!(DETECTION_COLS, DETECTION_COL_NAMES.join(", "));
    }

    #[test]
    fn detection_cols_count_matches_select_projection() {
        // Build a real SELECT against the migrated schema and confirm the
        // prepared statement's column count matches the names list. A new
        // migration that adds a column without updating DETECTION_COL_NAMES
        // (or vice versa) breaks this assertion before it can ship.
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        let sql = format!("SELECT {DETECTION_COLS} FROM detections LIMIT 0");
        let stmt = conn.prepare(&sql).unwrap();
        assert_eq!(
            stmt.column_count(),
            DETECTION_COL_NAMES.len(),
            "DETECTION_COLS projection count must match DETECTION_COL_NAMES.len()"
        );
    }

    #[test]
    fn detection_cols_names_are_resolvable_against_migrated_schema() {
        // Every name in DETECTION_COL_NAMES must resolve against the
        // migrated `detections` table — a typo or a column dropped by
        // a future migration without updating the names list lights up
        // here. We rely on SQLite raising "no such column" for each
        // missing name rather than building one giant SELECT, so the
        // error message points at the actual offender.
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        for name in DETECTION_COL_NAMES {
            let sql = format!("SELECT \"{name}\" FROM detections LIMIT 0");
            conn.prepare(&sql).unwrap_or_else(|e| {
                panic!("column {name:?} not present in migrated detections schema: {e}")
            });
        }
    }

    #[test]
    fn map_detection_row_reads_every_column_indexed() {
        // Insert a row with every column set to a distinct, type-correct
        // value, then read it back via the canonical SELECT + mapper.
        // If `map_detection_row` ever stops reading any of the indices
        // covered by DETECTION_COL_NAMES, the assertion below fires.
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        let record = super::DetectionRecord {
            date: "2026-05-19",
            time: "09:00:00",
            sci_name: "Pica pica",
            com_name: "Eurasian Magpie",
            confidence: 0.95,
            lat: Some(51.0),
            lon: Some(-0.1),
            cutoff: Some(0.5),
            week: Some(20),
            sensitivity: Some(1.0),
            overlap: Some(0.0),
            file_name: "/tmp/x.wav",
            chunk_offset_secs: Some(3.0),
            correlation_id: Some("abc123"),
            source: Some("cam1"),
        };
        crate::sqlite::queries::detections::insert_detection(&conn, &record).unwrap();

        let sql = format!("SELECT {DETECTION_COLS} FROM detections LIMIT 1");
        let row = conn.query_row(&sql, [], super::map_detection_row).unwrap();

        // Each assertion below pins one column read in `map_detection_row`;
        // a `row.get(N)` regression on any single column lights up the
        // matching assertion with the column's name in the failure.
        assert_eq!(row.date, "2026-05-19");
        assert_eq!(row.time, "09:00:00");
        assert_eq!(row.sci_name, "Pica pica");
        assert_eq!(row.com_name, "Eurasian Magpie");
        assert!((row.confidence - 0.95).abs() < 1e-9);
        assert_eq!(row.lat, Some(51.0));
        assert_eq!(row.lon, Some(-0.1));
        assert_eq!(row.cutoff, Some(0.5));
        assert_eq!(row.week, Some(20));
        assert_eq!(row.sens, Some(1.0));
        assert_eq!(row.overlap, Some(0.0));
        assert_eq!(row.file_name.as_deref(), Some("/tmp/x.wav"));
        assert_eq!(row.correlation_id.as_deref(), Some("abc123"));
        assert_eq!(row.source.as_deref(), Some("cam1"));
    }
}
