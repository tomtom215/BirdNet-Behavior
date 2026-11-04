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
    /// BirdNET week number, 1-48 (four per month; not the ISO week, which
    /// runs to 53). The value the occurrence filter is asked about.
    pub week: u32,
    /// Path to the extracted audio clip (set after extraction).
    pub file_name_extr: Option<String>,
    /// Which classifier reported this (`G-10` Stage 3).
    ///
    /// `None` on a row written before this existed. On a station running one
    /// classifier it is that classifier's id, which is provenance worth having
    /// even then: a station whose model was swapped last March can tell which
    /// of its detections came from which.
    #[serde(default)]
    pub model_id: Option<String>,
    /// How many classifiers independently reported this species in this chunk.
    ///
    /// `1` when only one ran or only one agreed; `2` or more means independent
    /// corroboration, which is a different and much stronger claim than one
    /// model being confident. `None` on a row written before this existed —
    /// distinct from `Some(1)`, which says one model was asked and one
    /// answered.
    #[serde(default)]
    pub agreeing_models: Option<u8>,
    /// What every classifier that listened to this chunk said about this
    /// species, each on its own scale (migrations 57 and 58).
    ///
    /// Empty when nothing recorded it: a detection built outside the daemon, a
    /// row older than the feature, or a chunk only one classifier listened to
    /// (a comparison needs two; with one, the detection row is the whole
    /// record). One entry per classifier that ran, so a classifier absent from
    /// the list was not asked, which is not the same as one that was asked and
    /// could not judge ([`AnswerState::NoLabel`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_scores: Vec<ModelAnswer>,
}

/// What one classifier could say about one species in one chunk.
///
/// Four states rather than a score and a flag: scoring a species low, having no
/// class for it, and failing the window are different facts, and a reader that
/// rounds them all to zero reports a model's silence as its opinion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerState {
    /// Scored at or above its own bar.
    Heard,
    /// Scored, and did not clear its own bar.
    BelowBar,
    /// Has no class for this species, so it cannot judge.
    NoLabel,
    /// Its window failed, or gave no usable number for this species.
    Failed,
}

/// One classifier's own answer about one species in one chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelAnswer {
    /// The classifier's id, as configured.
    pub model_id: String,
    /// What it could say.
    pub state: AnswerState,
    /// Its own score, 0 to 1, on its own scale. `Some` exactly for
    /// [`AnswerState::Heard`] and [`AnswerState::BelowBar`].
    pub score: Option<f32>,
    /// The bar that score was judged against: the classifier's own threshold,
    /// else the station's, as loaded (not the lowered floor the daemon runs it
    /// at). `Some` exactly when `score` is.
    pub bar: Option<f32>,
    /// Where in the recording the classifier's window began, seconds.
    pub window_start_secs: f32,
    /// Where it ended. Classifiers' windows differ in length, so "the same
    /// chunk" is not "the same instants".
    pub window_end_secs: f32,
}

/// What one inference pass says about one chunk of audio.
///
/// The detections are the species that cleared the confidence threshold and
/// the top-N cut. The human score is read from the same model output *before*
/// either cut, so it is available whether or not any human class was confident
/// enough to become a detection — a detection list contains a human label only
/// when speech scored above the *detection* threshold, and a privacy filter
/// that reads the list is therefore tuned by the detection threshold, whatever
/// its own setting says.
#[derive(Debug, Clone, Default)]
pub struct ChunkPrediction {
    /// Species detections, sorted by confidence descending, at most top-N.
    pub detections: Vec<Detection>,
    /// The highest confidence the model gave any human class (speech,
    /// whistling, other human sounds) for this chunk, on the same scale as
    /// [`Detection::confidence`]. `0.0` when the label set has no human class.
    pub human_score: f32,
    /// Where the best-scoring human class stands among *all* the model's
    /// classes for this chunk, counting from `0` for the highest score: the
    /// number of classes that outscored it. `None` when the label set has no
    /// human class.
    ///
    /// This is the quantity BirdNET-Pi's privacy rule reads (`filter_humans`
    /// masks a window when a human class is within the first
    /// `max(10, 6000 × PRIVACY_THRESHOLD / 100)` classes of the sorted
    /// output), taken, like [`Self::human_score`], before the detection
    /// threshold and the top-N cut. A non-finite human score ranks `0`.
    pub human_rank: Option<usize>,
    /// The highest-scoring watched noise class (see
    /// [`crate::inference::model::BirdNetModel::watch_noise_classes`]) in this
    /// chunk, read like [`Self::human_score`] from the model output *before*
    /// the detection threshold and the top-N cut. `None` when no noise class
    /// is watched or the label set has none.
    ///
    /// The noise filter used to read only [`Self::detections`], so a
    /// `NOISE_THRESHOLD` below the detection threshold could never fire: the
    /// bark it was set to catch had already been cut from the list.
    pub loudest_noise: Option<Detection>,
}

impl Detection {
    /// Confidence as integer percentage (0-100).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn confidence_pct(&self) -> u32 {
        // Clamp to [0, 1] before the cast so the result is always a sane 0-100:
        // confidence is bounded upstream, but a stray out-of-range value would
        // otherwise exceed 100, and this pins the invariant at the boundary.
        (self.confidence.clamp(0.0, 1.0) * 100.0).round() as u32
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
        let basename = path.rsplit('/').next().unwrap_or(path);
        // Case-insensitive, like the watcher's `is_audio_file`: a `.WAV` it
        // admitted and this refused failed on every pass, for ever.
        let (filename, ext) = basename.rsplit_once('.')?;
        if !["wav", "flac", "mp3"]
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
        {
            return None;
        }

        let parts: Vec<&str> = filename.splitn(5, '-').collect();

        // Minimum: YYYY-MM-DD-birdnet-HH:MM:SS (5 parts with date taking 3)
        if parts.len() < 5 {
            return None;
        }

        // First 3 parts are the date
        let date = format!("{}-{}-{}", parts[0], parts[1], parts[2]);

        // Validate date format: the three segments must be 4/2/2 ASCII digits.
        // Checking lengths alone let a bogus name like `abcd-XY-ZW-birdnet-…`
        // through, and its non-date string then flowed into the extraction
        // output path (`By_Date/<date>/…`) and DB rows.
        if parts[0].len() != 4
            || parts[1].len() != 2
            || parts[2].len() != 2
            || !parts[0].bytes().all(|b| b.is_ascii_digit())
            || !parts[1].bytes().all(|b| b.is_ascii_digit())
            || !parts[2].bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }

        // After "YYYY-MM-DD-" we expect "birdnet-..."
        let remainder = parts[3..].join("-");

        let rest = remainder.strip_prefix("birdnet-")?;

        // Check if there's an RTSP ID: "RTSP_ID-HH:MM:SS" vs "HH:MM:SS"
        let (rtsp_id, time) = rest.rfind('-').map_or_else(
            || (None, rest.to_string()),
            |last_dash| {
                let potential_time = &rest[last_dash + 1..];
                let potential_id = &rest[..last_dash];
                if potential_time.len() == 8 && potential_time.contains(':') {
                    (Some(potential_id.to_string()), potential_time.to_string())
                } else {
                    (None, rest.to_string())
                }
            },
        );

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

    /// The watcher admits `.WAV` (its `is_audio_file` ignores case), so the
    /// parser must too: a card recorder writing upper-case names had every
    /// file picked up and then refused, for ever, as unparseable.
    #[test]
    fn upper_case_extensions_parse_like_lower_case_ones() {
        for ext in ["WAV", "Wav", "FLAC", "MP3"] {
            let path = format!("/recs/2026-05-19-birdnet-09:00:00.{ext}");
            let rec = RecordingFile::parse(&path).unwrap_or_else(|| panic!("{path} did not parse"));
            assert_eq!(rec.date, "2026-05-19");
            assert_eq!(rec.time, "09:00:00");
        }
        // Counterpart: something that is not audio still is not.
        assert!(RecordingFile::parse("/recs/2026-05-19-birdnet-09:00:00.txt").is_none());
    }

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
            model_id: None,
            agreeing_models: None,
            model_scores: Vec::new(),
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
        let rf =
            RecordingFile::parse("/data/StreamData/2026-03-11-birdnet-cam1-08:30:00.wav").unwrap();
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
            model_id: None,
            agreeing_models: None,
            model_scores: Vec::new(),
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
            model_id: None,
            agreeing_models: None,
            model_scores: Vec::new(),
        };
        assert_eq!(det.species(), "Turdus merula_Eurasian Blackbird");
    }
}
