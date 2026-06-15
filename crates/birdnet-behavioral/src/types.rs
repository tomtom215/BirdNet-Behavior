//! Behavioral analytics result types.
//!
//! These types represent the output of duckdb-behavioral queries
//! applied to bird detection data.

use serde::Serialize;

/// A bird activity session (output of `sessionize`).
///
/// Groups continuous bird activity into sessions where a gap
/// greater than the threshold creates a new session.
#[derive(Debug, Clone, Serialize)]
pub struct ActivitySession {
    /// Species common name.
    pub species: String,
    /// Session identifier.
    pub session_id: u64,
    /// Number of detections in this session.
    pub detection_count: u32,
    /// Session start timestamp (ISO 8601).
    pub start_time: String,
    /// Session end timestamp (ISO 8601).
    pub end_time: String,
    /// Session duration in seconds.
    pub duration_secs: u64,
}

/// Species retention data (output of `retention`).
///
/// Tracks how many species return after their first detection,
/// measured at various day intervals.
#[derive(Debug, Clone, Serialize)]
pub struct SpeciesRetention {
    /// Species common name.
    pub species: String,
    /// Retention rates at specified intervals.
    /// Key: day interval (e.g., 1, 7, 30), Value: retention rate (0.0-1.0).
    pub retention_rates: Vec<RetentionRate>,
    /// Classification based on retention pattern.
    pub classification: ResidencyType,
}

/// A single retention rate measurement.
#[derive(Debug, Clone, Serialize)]
pub struct RetentionRate {
    /// Days after first detection.
    pub days: u32,
    /// Proportion of occurrences that returned (0.0 - 1.0).
    pub rate: f64,
}

/// Species residency classification derived from retention patterns.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum ResidencyType {
    /// High retention (> 0.7 at day 30) -- present most days.
    Resident,
    /// Medium retention (0.3 - 0.7 at day 30) -- seasonal visitor.
    Regular,
    /// Low retention (< 0.3 at day 30) -- passing through.
    Migrant,
    /// Single-day event (retention drops to 0 after day 1).
    Rarity,
}

impl ResidencyType {
    /// Classify a species based on its long-term retention rate.
    ///
    /// - Resident: > 0.7 (present most days)
    /// - Regular: 0.3 - 0.7 (seasonal visitor)
    /// - Migrant: 0.01 - 0.3 (passing through)
    /// - Rarity: < 0.01 (single-day event)
    pub fn from_retention_rate(rate: f64) -> Self {
        if rate > 0.7 {
            Self::Resident
        } else if rate > 0.3 {
            Self::Regular
        } else if rate > 0.01 {
            Self::Migrant
        } else {
            Self::Rarity
        }
    }
}

/// Dawn chorus funnel result (output of `window_funnel`).
///
/// Tracks how many "steps" of an expected species sequence occur.
#[derive(Debug, Clone, Serialize)]
pub struct ChorusFunnel {
    /// Date of the dawn chorus observation.
    pub date: String,
    /// Number of funnel steps completed (0 = none matched).
    pub steps_completed: u32,
    /// Total steps in the funnel definition.
    pub total_steps: u32,
    /// Species sequence that was matched.
    pub matched_species: Vec<String>,
}

/// Dawn chorus funnel step timings (output of `window_funnel_events`, v0.8.0).
///
/// Like [`ChorusFunnel`] but reports *when* each completed step was satisfied,
/// so the UI can show the actual progression (e.g. Robin 05:42 → Blackbird
/// 05:51 → Wren 06:03) rather than just how many steps were reached.
#[derive(Debug, Clone, Serialize)]
pub struct ChorusFunnelEvents {
    /// Date of the dawn chorus observation.
    pub date: String,
    /// Timestamp each completed step fired, in funnel order (ISO 8601).
    /// Its length equals the number of steps completed that day.
    pub step_times: Vec<String>,
    /// The expected species sequence; pair `species_sequence[i]` with
    /// `step_times[i]` for the completed steps.
    pub species_sequence: Vec<String>,
}

/// Sequence pattern match result (output of `sequence_match`).
#[derive(Debug, Clone, Serialize)]
pub struct PatternMatch {
    /// Date the pattern was observed.
    pub date: String,
    /// Whether the full pattern was matched.
    pub matched: bool,
    /// Species involved in the pattern.
    pub species_sequence: Vec<String>,
}

/// Sequence pattern occurrence count (output of `sequence_count`, v0.8.0).
///
/// Like [`PatternMatch`] but reports *how many* non-overlapping times the
/// ordered sequence occurred that day, not merely whether it happened at all.
#[derive(Debug, Clone, Serialize)]
pub struct PatternCount {
    /// Date the pattern was observed.
    pub date: String,
    /// Number of non-overlapping occurrences of the ordered sequence.
    pub count: u64,
    /// Species involved in the pattern.
    pub species_sequence: Vec<String>,
}

/// Sequence pattern *match timings* (output of `sequence_match_events`, v0.8.0).
///
/// Like [`PatternMatch`] but reports *when* each step of the ordered sequence
/// fired (ISO 8601). On a day that doesn't complete the sequence, `step_times`
/// holds the timestamps of the longest in-order prefix reached (e.g. two of
/// three steps), mirroring `window_funnel_events`; a full match has one
/// timestamp per step, so `step_times.len() == species_sequence.len()`
/// identifies the days [`PatternCount`] also counts.
#[derive(Debug, Clone, Serialize)]
pub struct PatternMatchEvents {
    /// Date the pattern was evaluated.
    pub date: String,
    /// Timestamp each matched step fired, in sequence order (ISO 8601). Its
    /// length is the longest in-order prefix reached that day. Pair
    /// `species_sequence[i]` with `step_times[i]`.
    pub step_times: Vec<String>,
    /// Species involved in the pattern.
    pub species_sequence: Vec<String>,
}

/// Next species prediction (output of `sequence_next_node`).
#[derive(Debug, Clone, Serialize)]
pub struct NextSpeciesPrediction {
    /// The trigger species.
    pub after_species: String,
    /// Predicted next species.
    pub predicted_species: String,
    /// Number of times this sequence was observed.
    pub frequency: u64,
    /// Proportion of times this species followed (0.0 - 1.0).
    pub probability: f64,
}

/// Parameters for a sessionize query.
#[derive(Debug, Clone)]
pub struct SessionizeParams {
    /// Species to analyze (None = all species).
    pub species: Option<String>,
    /// Gap threshold that defines a new session.
    pub gap_minutes: u32,
    /// Maximum number of sessions to return.
    pub limit: u32,
}

impl Default for SessionizeParams {
    fn default() -> Self {
        Self {
            species: None,
            gap_minutes: 30,
            limit: 100,
        }
    }
}

/// Parameters for a retention query.
#[derive(Debug, Clone)]
pub struct RetentionParams {
    /// Day intervals to measure retention at.
    pub intervals: Vec<u32>,
    /// Minimum number of total detections to include a species.
    pub min_detections: u32,
}

impl Default for RetentionParams {
    fn default() -> Self {
        Self {
            intervals: vec![1, 2, 3, 7, 14, 30],
            min_detections: 5,
        }
    }
}

/// Parameters for a funnel query.
#[derive(Debug, Clone)]
pub struct FunnelParams {
    /// Ordered list of species expected in the funnel.
    pub species_sequence: Vec<String>,
    /// Time window for the funnel (in minutes).
    pub window_minutes: u32,
    /// Hours of day to analyze (e.g., 4-8 for dawn).
    pub hour_start: u32,
    /// End hour.
    pub hour_end: u32,
}

impl Default for FunnelParams {
    fn default() -> Self {
        Self {
            species_sequence: vec![
                "European Robin".into(),
                "Eurasian Blackbird".into(),
                "Song Thrush".into(),
                "Eurasian Wren".into(),
                "Great Tit".into(),
            ],
            window_minutes: 120,
            hour_start: 4,
            hour_end: 8,
        }
    }
}

/// Parameters for a sequence-pattern match query (`sequence_match`).
#[derive(Debug, Clone)]
pub struct PatternParams {
    /// Ordered species the pattern must match, in sequence.
    pub species_sequence: Vec<String>,
    /// Optional maximum gap between consecutive steps. `None` = any gap.
    pub max_gap_minutes: Option<u32>,
    /// First hour of day to consider (e.g. 4 for dawn).
    pub hour_start: u32,
    /// Last hour of day to consider (inclusive).
    pub hour_end: u32,
}

impl Default for PatternParams {
    fn default() -> Self {
        Self {
            species_sequence: vec![
                "European Robin".into(),
                "Eurasian Blackbird".into(),
                "Eurasian Wren".into(),
            ],
            max_gap_minutes: None,
            hour_start: 4,
            hour_end: 8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sessionize_params() {
        let params = SessionizeParams::default();
        assert_eq!(params.gap_minutes, 30);
        assert_eq!(params.limit, 100);
        assert!(params.species.is_none());
    }

    #[test]
    fn default_retention_params() {
        let params = RetentionParams::default();
        assert_eq!(params.intervals, vec![1, 2, 3, 7, 14, 30]);
        assert_eq!(params.min_detections, 5);
    }

    #[test]
    fn default_funnel_params() {
        let params = FunnelParams::default();
        assert_eq!(params.species_sequence.len(), 5);
        assert_eq!(params.species_sequence[0], "European Robin");
        assert_eq!(params.window_minutes, 120);
    }

    #[test]
    fn residency_classification() {
        assert_eq!(ResidencyType::Resident, ResidencyType::Resident);
        assert_ne!(ResidencyType::Migrant, ResidencyType::Rarity);
    }
}
