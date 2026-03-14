//! Human voice privacy filter.
//!
//! Detects human voice/speech in the inference results and masks those chunks
//! (and adjacent chunks) to protect privacy. When enabled, any chunk whose
//! top-N predictions include a "Human" label above the configured cutoff rank
//! is suppressed, along with its neighboring chunks.

use crate::detection::types::Detection;

/// Privacy filter that suppresses detections when human voice is detected.
#[derive(Debug, Clone)]
pub struct PrivacyFilter {
    /// Privacy threshold: 0.0 = disabled, 0.01-0.03 typical.
    /// Used to compute the human cutoff rank.
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

    /// Compute the human cutoff rank.
    ///
    /// If a "Human" label appears within the top `cutoff` predictions
    /// for a chunk, that chunk is flagged for suppression.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn human_cutoff(&self) -> usize {
        // Match BirdNET-Pi: human_cutoff = max(10, (6000 * threshold / 100.0) as usize)
        let computed = (6000.0 * f64::from(self.threshold) / 100.0) as usize;
        computed.max(10)
    }

    /// Filter predictions by suppressing chunks containing human voice.
    ///
    /// Takes a slice of chunks (each chunk is a `Vec<Detection>` sorted by
    /// confidence descending). For each chunk, if any detection within the
    /// top `human_cutoff` results contains "Human" in the scientific or
    /// common name, that chunk and its immediate neighbors are masked
    /// (replaced with empty vectors).
    ///
    /// Returns a new vector of chunks with flagged chunks emptied.
    pub fn filter_predictions(&self, predictions: &[Vec<Detection>]) -> Vec<Vec<Detection>> {
        if !self.is_enabled() || predictions.is_empty() {
            return predictions.to_vec();
        }

        let cutoff = self.human_cutoff();

        // First pass: identify which chunks contain human voice
        let mut human_flags: Vec<bool> = predictions
            .iter()
            .map(|chunk| chunk_contains_human(chunk, cutoff))
            .collect();

        // Second pass: expand flags to adjacent chunks
        let expanded = expand_adjacent(&human_flags);
        human_flags = expanded;

        // Third pass: mask flagged chunks
        predictions
            .iter()
            .zip(human_flags.iter())
            .map(|(chunk, &flagged)| {
                if flagged {
                    tracing::debug!("privacy filter: suppressing chunk with human voice");
                    Vec::new()
                } else {
                    chunk.clone()
                }
            })
            .collect()
    }
}

/// Check if a chunk's top-N predictions contain a human label.
fn chunk_contains_human(detections: &[Detection], cutoff: usize) -> bool {
    let check_count = detections.len().min(cutoff);
    detections[..check_count].iter().any(is_human_label)
}

/// Check if a detection is a human voice label.
fn is_human_label(detection: &Detection) -> bool {
    let sci = detection.scientific_name.to_lowercase();
    let com = detection.common_name.to_lowercase();
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
        }
    }

    #[test]
    fn disabled_filter_passes_everything() {
        let filter = PrivacyFilter::new(0.0);
        assert!(!filter.is_enabled());
        let chunks = vec![vec![make_detection("Homo sapiens", "Human", 0.9)]];
        let result = filter.filter_predictions(&chunks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 1);
    }

    #[test]
    fn enabled_filter_suppresses_human_chunks() {
        let filter = PrivacyFilter::new(0.03);
        assert!(filter.is_enabled());

        let chunks = vec![
            vec![make_detection("Turdus merula", "Eurasian Blackbird", 0.9)],
            vec![
                make_detection("Homo sapiens", "Human", 0.8),
                make_detection("Turdus merula", "Eurasian Blackbird", 0.3),
            ],
            vec![make_detection("Parus major", "Great Tit", 0.7)],
        ];

        let result = filter.filter_predictions(&chunks);
        assert_eq!(result.len(), 3);
        // Chunk 0 is adjacent to chunk 1 (human), so it should be suppressed
        assert!(result[0].is_empty());
        // Chunk 1 contains human, should be suppressed
        assert!(result[1].is_empty());
        // Chunk 2 is adjacent to chunk 1, should be suppressed
        assert!(result[2].is_empty());
    }

    #[test]
    fn non_adjacent_chunks_not_affected() {
        let filter = PrivacyFilter::new(0.03);

        let chunks = vec![
            vec![make_detection("Turdus merula", "Eurasian Blackbird", 0.9)],
            vec![make_detection("Parus major", "Great Tit", 0.7)],
            vec![make_detection("Homo sapiens", "Human", 0.8)],
            vec![make_detection("Erithacus rubecula", "European Robin", 0.6)],
            vec![make_detection("Cyanistes caeruleus", "Blue Tit", 0.5)],
        ];

        let result = filter.filter_predictions(&chunks);
        assert_eq!(result.len(), 5);
        // Chunk 0: not adjacent to human (chunk 2), should pass
        assert!(!result[0].is_empty());
        // Chunk 1: adjacent to chunk 2 (human), should be suppressed
        assert!(result[1].is_empty());
        // Chunk 2: human, suppressed
        assert!(result[2].is_empty());
        // Chunk 3: adjacent to chunk 2, suppressed
        assert!(result[3].is_empty());
        // Chunk 4: not adjacent to human, should pass
        assert!(!result[4].is_empty());
    }

    #[test]
    fn human_cutoff_calculation() {
        let filter = PrivacyFilter::new(0.03);
        // 6000 * 0.03 / 100.0 = 1.8 -> 1, but max(10, 1) = 10
        assert_eq!(filter.human_cutoff(), 10);

        let filter2 = PrivacyFilter::new(1.0);
        // 6000 * 1.0 / 100.0 = 60
        assert_eq!(filter2.human_cutoff(), 60);
    }

    #[test]
    fn empty_predictions_returns_empty() {
        let filter = PrivacyFilter::new(0.03);
        let result = filter.filter_predictions(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn is_human_label_case_insensitive() {
        assert!(is_human_label(&make_detection(
            "Homo sapiens",
            "Human",
            0.9
        )));
        assert!(is_human_label(&make_detection(
            "homo sapiens",
            "human voice",
            0.9
        )));
        assert!(!is_human_label(&make_detection(
            "Turdus merula",
            "Eurasian Blackbird",
            0.9
        )));
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
