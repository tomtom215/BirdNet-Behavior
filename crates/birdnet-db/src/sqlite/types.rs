//! Row types shared across SQLite query modules.

/// A detection record for database insertion.
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
    /// Latitude.
    pub lat: &'a str,
    /// Longitude.
    pub lon: &'a str,
    /// Confidence cutoff threshold.
    pub cutoff: &'a str,
    /// ISO week number.
    pub week: &'a str,
    /// Sensitivity setting.
    pub sensitivity: &'a str,
    /// Overlap setting.
    pub overlap: &'a str,
    /// Extracted audio filename.
    pub file_name: &'a str,
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
    })
}

/// Columns selected in all full-row detection queries.
pub(super) const DETECTION_COLS: &str =
    "Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name";
