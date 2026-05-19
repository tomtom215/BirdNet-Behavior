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
    })
}

/// Columns selected in all full-row detection queries.
///
/// Ordering matches [`map_detection_row`] — never reorder one without the
/// other, never drop a column from this list while the row mapper still
/// reads at that index. PR #35 shipped a column-list / row-mapper drift
/// bug; the explicit `correlation_id` at the end of both is the same
/// pattern.
pub(super) const DETECTION_COLS: &str = "Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, correlation_id";
