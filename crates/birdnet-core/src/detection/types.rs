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

    /// Species identifier: "`Scientific_Common`".
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

impl RecordingFile {
    /// Parse a recording filename into its components.
    ///
    /// Supports two formats:
    /// - `YYYY-MM-DD-birdnet-HH:MM:SS.wav` (local mic)
    /// - `YYYY-MM-DD-birdnet-RTSP_ID-HH:MM:SS.wav` (RTSP stream)
    pub fn parse(path: &str) -> Option<Self> {
        // Extract just the filename (without directory or extension)
        let filename = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .strip_suffix(".wav")
            .or_else(|| path.rsplit('/').next().unwrap_or(path).strip_suffix(".flac"))
            .or_else(|| path.rsplit('/').next().unwrap_or(path).strip_suffix(".mp3"))?;

        let parts: Vec<&str> = filename.splitn(5, '-').collect();

        // Minimum: YYYY-MM-DD-birdnet-HH:MM:SS (5 parts with date taking 3)
        if parts.len() < 5 {
            return None;
        }

        // First 3 parts are the date
        let date = format!("{}-{}-{}", parts[0], parts[1], parts[2]);

        // Validate date format (basic check)
        if parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
            return None;
        }

        // After "YYYY-MM-DD-" we expect "birdnet-..."
        let remainder = parts[3..].join("-");

        let (rtsp_id, time) = if let Some(rest) = remainder.strip_prefix("birdnet-") {
            // Check if there's an RTSP ID: "RTSP_ID-HH:MM:SS" vs "HH:MM:SS"
            if rest.contains('-') {
                // Could be RTSP_ID-HH:MM:SS
                if let Some(last_dash) = rest.rfind('-') {
                    let potential_time = &rest[last_dash + 1..];
                    let potential_id = &rest[..last_dash];
                    if potential_time.len() == 8 && potential_time.contains(':') {
                        (Some(potential_id.to_string()), potential_time.to_string())
                    } else {
                        // No valid time after last dash, treat entire rest as time
                        (None, rest.to_string())
                    }
                } else {
                    (None, rest.to_string())
                }
            } else {
                (None, rest.to_string())
            }
        } else {
            return None;
        };

        // Validate time format (HH:MM:SS)
        if time.len() != 8 || time.chars().filter(|c| *c == ':').count() != 2 {
            return None;
        }

        let iso8601 = format!("{date}T{time}");

        Some(Self {
            path: path.to_string(),
            date,
            time,
            iso8601,
            rtsp_id,
        })
    }
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

    #[test]
    fn parse_local_mic_filename() {
        let rf = RecordingFile::parse("2026-03-11-birdnet-08:30:00.wav").unwrap();
        assert_eq!(rf.date, "2026-03-11");
        assert_eq!(rf.time, "08:30:00");
        assert_eq!(rf.iso8601, "2026-03-11T08:30:00");
        assert!(rf.rtsp_id.is_none());
    }

    #[test]
    fn parse_rtsp_filename() {
        let rf = RecordingFile::parse("/data/StreamData/2026-03-11-birdnet-cam1-08:30:00.wav")
            .unwrap();
        assert_eq!(rf.date, "2026-03-11");
        assert_eq!(rf.time, "08:30:00");
        assert_eq!(rf.rtsp_id.as_deref(), Some("cam1"));
    }

    #[test]
    fn parse_flac_extension() {
        let rf = RecordingFile::parse("2026-03-11-birdnet-08:30:00.flac").unwrap();
        assert_eq!(rf.date, "2026-03-11");
    }

    #[test]
    fn parse_invalid_filename_returns_none() {
        assert!(RecordingFile::parse("not-a-valid-file.wav").is_none());
        assert!(RecordingFile::parse("random.txt").is_none());
        assert!(RecordingFile::parse("").is_none());
    }

    #[test]
    fn detection_display() {
        let det = Detection {
            date: "2026-03-11".into(),
            time: "08:30:00".into(),
            scientific_name: "Turdus merula".into(),
            common_name: "Eurasian Blackbird".into(),
            confidence: 0.87,
            start: 3.0,
            stop: 6.0,
            week: 10,
            file_name_extr: None,
        };
        let display = format!("{det}");
        assert!(display.contains("Eurasian Blackbird"));
        assert!(display.contains("87%"));
    }

    #[test]
    fn detection_species_id() {
        let det = Detection {
            date: "2026-03-11".into(),
            time: "08:30:00".into(),
            scientific_name: "Turdus merula".into(),
            common_name: "Eurasian Blackbird".into(),
            confidence: 0.87,
            start: 3.0,
            stop: 6.0,
            week: 10,
            file_name_extr: None,
        };
        assert_eq!(det.species(), "Turdus merula_Eurasian Blackbird");
    }
}
