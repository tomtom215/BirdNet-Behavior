//! Human voice privacy filter.
//!
//! Suppresses every chunk in which the model heard a human (speech, whistling,
//! other human sounds) and the chunks either side of it, so a conversation
//! near the microphone is not kept in the recordings a station retains.
//!
//! The judgement is made on the chunk's *human score* — the highest confidence
//! the model gave any human class, read from its output before the detection
//! threshold and top-N cut ([`ChunkPrediction::human_score`]). It is not made
//! on the detection list. A list only carries a human label when speech scored
//! above the *detection* threshold, so a filter that scanned the list was tuned
//! by that threshold, and its own setting never bound: the BirdNET-Pi rule this
//! port inherited, `human_cutoff = max(10, 6000 × threshold / 100)`, looked at
//! the top ten of a list that was at most ten long.

use crate::detection::types::{ChunkPrediction, Detection};

/// Privacy filter that suppresses detections when human voice is detected.
#[derive(Debug, Clone)]
pub struct PrivacyFilter {
    /// The human score at or above which a chunk is suppressed.
    /// `0.0` disables the filter.
    threshold: f32,
}

impl PrivacyFilter {
    /// Create a new privacy filter with the given threshold.
    ///
    /// A threshold of 0.0 disables the filter entirely.
    pub const fn new(threshold: f32) -> Self {
        Self { threshold }
    }

    /// Whether the privacy filter is enabled (threshold > 0).
    pub fn is_enabled(&self) -> bool {
        self.threshold > 0.0
    }

    /// Get the current threshold.
    pub const fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Whether one chunk, on its own evidence, is to be suppressed.
    fn flags(&self, chunk: &ChunkPrediction) -> bool {
        chunk.human_score >= self.threshold
    }

    /// Suppress every chunk whose human score reaches the threshold, and the
    /// chunks adjacent to it.
    ///
    /// Returns one detection list per input chunk, in order, with suppressed
    /// chunks emptied. Only the human score is consulted; the detection lists
    /// pass through untouched or emptied whole.
    pub fn filter_predictions(&self, chunks: &[ChunkPrediction]) -> Vec<Vec<Detection>> {
        let detections = || chunks.iter().map(|chunk| chunk.detections.clone());
        if !self.is_enabled() || chunks.is_empty() {
            return detections().collect();
        }

        // First pass: which chunks contain a human. Second: their neighbours.
        let flagged: Vec<bool> = chunks.iter().map(|chunk| self.flags(chunk)).collect();
        let flagged = expand_adjacent(&flagged);

        detections()
            .zip(flagged)
            .map(|(chunk, flagged)| {
                if flagged {
                    tracing::debug!("privacy filter: suppressing chunk with human voice");
                    Vec::new()
                } else {
                    chunk
                }
            })
            .collect()
    }
}

/// Whether a label names a human class.
///
/// A substring match on either name, case-insensitively: the V2.4 label set
/// spells its three human classes `Human_Human vocal`, `Human non-vocal_Human
/// non-vocal` and `Human whistling_Human whistling`, and the V3 CSV keeps the
/// same words. The rule is deliberately not one an operator can extend — see
/// the noise filter for why a substring rule cannot be handed out.
#[must_use]
pub fn names_a_human(scientific_name: &str, common_name: &str) -> bool {
    let sci = scientific_name.to_lowercase();
    let com = common_name.to_lowercase();
    sci.contains("human") || com.contains("human")
}

/// Expand boolean flags to include adjacent indices (i-1 and i+1).
fn expand_adjacent(flags: &[bool]) -> Vec<bool> {
    let mut expanded = flags.to_vec();
    for i in 0..flags.len() {
        if flags[i] {
            if i > 0 {
                expanded[i - 1] = true;
            }
            if i + 1 < flags.len() {
                expanded[i + 1] = true;
            }
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_detection(sci_name: &str, common_name: &str, confidence: f32) -> Detection {
        Detection {
            date: "2026-03-14".into(),
            time: "08:00:00".into(),
            scientific_name: sci_name.into(),
            common_name: common_name.into(),
            confidence,
            start: 0.0,
            stop: 3.0,
            week: 11,
            file_name_extr: None,
            model_id: None,
            agreeing_models: None,
        }
    }

    /// A chunk with one blackbird detection and the given human score.
    fn chunk(human_score: f32) -> ChunkPrediction {
        ChunkPrediction {
            detections: vec![make_detection("Turdus merula", "Eurasian Blackbird", 0.9)],
            human_score,
        }
    }

    #[test]
    fn disabled_filter_passes_everything() {
        let filter = PrivacyFilter::new(0.0);
        assert!(!filter.is_enabled());
        let result = filter.filter_predictions(&[chunk(0.9)]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 1);
    }

    #[test]
    fn enabled_filter_suppresses_human_chunks_and_their_neighbours() {
        let filter = PrivacyFilter::new(0.03);
        assert!(filter.is_enabled());

        let result = filter.filter_predictions(&[chunk(0.0), chunk(0.8), chunk(0.0)]);
        assert_eq!(result.len(), 3);
        assert!(result[0].is_empty(), "the chunk before the voice");
        assert!(result[1].is_empty(), "the voice itself");
        assert!(result[2].is_empty(), "the chunk after the voice");
    }

    #[test]
    fn non_adjacent_chunks_not_affected() {
        let filter = PrivacyFilter::new(0.03);
        let result = filter.filter_predictions(&[
            chunk(0.0),
            chunk(0.0),
            chunk(0.8),
            chunk(0.0),
            chunk(0.0),
        ]);
        assert_eq!(result.len(), 5);
        assert!(!result[0].is_empty());
        assert!(result[1].is_empty());
        assert!(result[2].is_empty());
        assert!(result[3].is_empty());
        assert!(!result[4].is_empty());
    }

    /// The row this filter answers to (S-5): the threshold must bind.
    ///
    /// The same chunk, whose detection list carries no human label at all —
    /// speech scored 0.02, below any detection threshold — is suppressed at a
    /// privacy threshold of 0.01 and kept at 0.05. Under the inherited rule the
    /// verdict was a scan of that list, so both settings kept it.
    #[test]
    fn the_privacy_threshold_decides_and_nothing_in_the_detection_list_does() {
        let quiet_speech = || vec![chunk(0.02)];

        assert!(
            PrivacyFilter::new(0.01).filter_predictions(&quiet_speech())[0].is_empty(),
            "speech at 0.02 must be suppressed by a threshold of 0.01"
        );
        assert!(
            !PrivacyFilter::new(0.05).filter_predictions(&quiet_speech())[0].is_empty(),
            "speech at 0.02 must be kept by a threshold of 0.05"
        );

        // The counterpart: what the detection list holds is not evidence. A
        // human *detection* in the list with a score below the threshold is
        // the detection threshold's business, not this filter's — otherwise
        // lowering the detection threshold would tighten privacy, which is
        // the coupling the row is about.
        let human_in_list = vec![ChunkPrediction {
            detections: vec![make_detection("Homo sapiens", "Human", 0.02)],
            human_score: 0.02,
        }];
        assert!(
            !PrivacyFilter::new(0.05).filter_predictions(&human_in_list)[0].is_empty(),
            "a human label in the detection list must not override the threshold"
        );
    }

    #[test]
    fn the_threshold_is_inclusive() {
        let at = vec![chunk(0.03)];
        assert!(PrivacyFilter::new(0.03).filter_predictions(&at)[0].is_empty());
        let below = vec![chunk(0.029_999)];
        assert!(!PrivacyFilter::new(0.03).filter_predictions(&below)[0].is_empty());
    }

    #[test]
    fn empty_predictions_returns_empty() {
        let filter = PrivacyFilter::new(0.03);
        let result = filter.filter_predictions(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn names_a_human_is_case_insensitive_and_matches_every_human_class() {
        assert!(names_a_human("Homo sapiens", "Human"));
        assert!(names_a_human("homo sapiens", "human voice"));
        assert!(names_a_human("Human", "Human vocal"));
        assert!(names_a_human("Human non-vocal", "Human non-vocal"));
        assert!(names_a_human("Human whistling", "Human whistling"));
        assert!(!names_a_human("Turdus merula", "Eurasian Blackbird"));
    }

    #[test]
    fn expand_adjacent_works_correctly() {
        let flags = vec![false, false, true, false, false];
        let expanded = expand_adjacent(&flags);
        assert_eq!(expanded, vec![false, true, true, true, false]);
    }

    #[test]
    fn expand_adjacent_at_boundaries() {
        let flags = vec![true, false, false];
        let expanded = expand_adjacent(&flags);
        assert_eq!(expanded, vec![true, true, false]);

        let flags2 = vec![false, false, true];
        let expanded2 = expand_adjacent(&flags2);
        assert_eq!(expanded2, vec![false, true, true]);
    }
}
