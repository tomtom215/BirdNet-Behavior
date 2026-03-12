//! Result types returned by time-series analytics queries.
//!
//! All types derive `Serialize` for direct JSON serialisation by the
//! axum handlers, and `Debug + Clone` for ergonomic usage.

use serde::Serialize;

/// A single tumbling or hopping window row.
#[derive(Debug, Clone, Serialize)]
pub struct WindowRow {
    /// Start of the window (ISO-8601 timestamp or date).
    pub window_start: String,
    /// End of the window (ISO-8601 timestamp or date).
    pub window_end: String,
    /// Total detections in this window.
    pub detection_count: i64,
    /// Distinct species count in this window.
    pub species_count: i64,
    /// Average confidence score (0.0 – 1.0).
    pub avg_confidence: Option<f64>,
}

/// A single day in a sliding/moving-average result.
#[derive(Debug, Clone, Serialize)]
pub struct TrendRow {
    /// The date this row represents (ISO-8601).
    pub date: String,
    /// Raw detection count for this day.
    pub daily_detections: i64,
    /// Species richness for this day.
    pub species_richness: i64,
    /// Smoothed moving average of detection count.
    pub moving_avg_detections: Option<f64>,
    /// Smoothed moving average of species richness.
    pub moving_avg_species: Option<f64>,
}

/// A single session window (gap-based grouping).
#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    /// Monotonically increasing session identifier within the result set.
    pub session_id: i64,
    /// Calendar date of the session.
    pub date: String,
    /// Session start timestamp (ISO-8601).
    pub session_start: String,
    /// Session end timestamp (ISO-8601).
    pub session_end: String,
    /// Number of detections in the session.
    pub detection_count: i64,
    /// Number of distinct species in the session.
    pub species_count: i64,
    /// Duration of the session in minutes.
    pub duration_minutes: i64,
    /// Largest internal gap within the session (minutes).
    pub max_internal_gap_minutes: Option<i64>,
}

/// Hourly activity heatmap row (average detections by hour-of-day).
#[derive(Debug, Clone, Serialize)]
pub struct HourlyHeatmapRow {
    /// Hour of day (0 – 23).
    pub hour_of_day: i64,
    /// Total detections at this hour across all included days.
    pub total_detections: i64,
    /// Number of days that had at least one detection at this hour.
    pub active_days: i64,
    /// Average detections per active day.
    pub avg_detections_per_day: f64,
    /// Unique species seen at this hour.
    pub unique_species: i64,
}

/// Species diversity row (daily richness + Shannon entropy).
#[derive(Debug, Clone, Serialize)]
pub struct DiversityRow {
    /// The date (ISO-8601).
    pub date: String,
    /// Number of distinct species observed.
    pub species_richness: i64,
    /// Total detections for this day.
    pub total_detections: i64,
    /// Shannon diversity index H′ (may be absent if only one species).
    pub shannon_h: Option<f64>,
    /// Pielou's evenness (0 = one dominant species; 1 = perfectly even).
    pub pielou_evenness: Option<f64>,
}

/// Species accumulation curve row.
#[derive(Debug, Clone, Serialize)]
pub struct AccumulationRow {
    /// The date new species were first observed (ISO-8601).
    pub date: String,
    /// How many new species were first seen on this date.
    pub new_species_today: i64,
    /// Running total of distinct species seen up to and including this date.
    pub cumulative_species: i64,
}

/// Anomaly detection row.
#[derive(Debug, Clone, Serialize)]
pub struct AnomalyRow {
    /// Date (ISO-8601).
    pub date: String,
    /// Raw detection count.
    pub detections: i64,
    /// Rolling mean at this date.
    pub rolling_mean: Option<f64>,
    /// Rolling standard deviation.
    pub rolling_stddev: Option<f64>,
    /// Z-score: how many SDs above/below the mean.
    pub z_score: Option<f64>,
    /// `"high"`, `"low"`, or `"normal"`.
    pub anomaly_flag: String,
}

/// Year-over-year comparison row.
#[derive(Debug, Clone, Serialize)]
pub struct YearOverYearRow {
    /// ISO week start date.
    pub week_start: String,
    /// Detection count this year.
    pub current_year_count: i64,
    /// Detection count same week last year.
    pub prior_year_count: Option<i64>,
    /// Absolute delta (current − prior).
    pub yoy_delta: i64,
    /// Species count this year.
    pub current_year_species: i64,
    /// Species count prior year.
    pub prior_year_species: Option<i64>,
}

/// Inactivity gap within a day.
#[derive(Debug, Clone, Serialize)]
pub struct GapRow {
    /// When the silence started (end of last detection before the gap).
    pub gap_start: Option<String>,
    /// When activity resumed.
    pub gap_end: String,
    /// Duration of the gap in minutes.
    pub gap_minutes: i64,
}

/// Peak activity window row.
#[derive(Debug, Clone, Serialize)]
pub struct PeakWindowRow {
    /// Window start (ISO-8601 timestamp).
    pub window_start: String,
    /// Window end (ISO-8601 timestamp).
    pub window_end: String,
    /// Detection count in this window.
    pub detection_count: i64,
    /// Species count in this window.
    pub species_count: i64,
    /// Peak confidence in this window.
    pub peak_confidence: Option<f64>,
}
