//! Core detection data types.
//!
//! Rust equivalents of Python's `Detection` and `ParseFileName` classes
//! from `scripts/utils/classes.py`.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A single bird detection from ML inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    /// Detection date (YYYY-MM-DD).
    pub date: String,
    /// Detection time (HH:MM:SS).
    pub time: String,
    /// Scientific name (e.g., "Turdus merula").
    pub scientific_name: String,
    /// Common name (e.g., "Eurasian Blackbird").
    pub common_name: String,
    /// Confidence score [0.0, 1.0].
    pub confidence: f32,
    /// Start time in seconds within the recording.
    pub start: f32,
    /// End time in seconds within the recording.
    pub stop: f32,
    /// ISO 8601 week number.
    pub week: u32,
    /// Path to the extracted audio clip (set after extraction).
    pub file_name_extr: Option<String>,
}

impl Detection {
    /// Confidence as integer percentage (0-100).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn confidence_pct(&self) -> u32 {
        (self.confidence * 100.0).round() as u32
    }

    /// Common name with spaces replaced by underscores (for filenames).
    pub fn common_name_safe(&self) -> String {
        self.common_name.replace(' ', "_")
    }

    /// Species identifier: "Scientific_Common".
    pub fn species(&self) -> String {
        format!("{}_{}", self.scientific_name, self.common_name)
    }
}

impl fmt::Display for Detection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}) {:.0}% at {}-{}s",
            self.common_name,
            self.scientific_name,
            self.confidence * 100.0,
            self.start,
            self.stop
        )
    }
}

/// Parsed components from a BirdNET-Pi recording filename.
///
/// Filenames follow the pattern: `YYYY-MM-DD-birdnet-HH:MM:SS.wav`
/// or with RTSP ID: `YYYY-MM-DD-birdnet-RTSP_ID-HH:MM:SS.wav`
#[derive(Debug, Clone)]
pub struct RecordingFile {
    /// Full path to the recording file.
    pub path: String,
    /// Recording date (YYYY-MM-DD).
    pub date: String,
    /// Recording time (HH:MM:SS).
    pub time: String,
    /// ISO 8601 timestamp.
    pub iso8601: String,
    /// RTSP stream identifier (None for local microphone).
    pub rtsp_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_confidence_pct() {
        let det = Detection {
            date: "2026-03-11".into(),
            time: "08:30:00".into(),
            scientific_name: "Turdus merula".into(),
            common_name: "Eurasian Blackbird".into(),
            confidence: 0.8765,
            start: 3.0,
            stop: 6.0,
            week: 10,
            file_name_extr: None,
        };
        assert_eq!(det.confidence_pct(), 88);
        assert_eq!(det.common_name_safe(), "Eurasian_Blackbird");
    }
}
