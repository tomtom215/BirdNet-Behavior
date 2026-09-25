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

/// How far a detection's saved clip reaches either side of the detection's
/// own window, in seconds: the audio the clip exposes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ClipReach {
    /// Seconds of clip before the detection's start.
    pub before: f32,
    /// Seconds of clip after the detection's end.
    pub after: f32,
}

/// Privacy filter that suppresses detections when human voice is detected.
#[derive(Debug, Clone)]
pub struct PrivacyFilter {
    /// The human score at or above which a chunk is suppressed.
    /// `0.0` disables the filter.
    threshold: f32,
    /// How far a saved clip reaches beyond its detection.
    reach: ClipReach,
    /// Length of one analysed chunk, in seconds.
    chunk_secs: f32,
}

impl PrivacyFilter {
    /// Create a new privacy filter with the given threshold.
    ///
    /// A threshold of 0.0 disables the filter entirely.
    pub const fn new(threshold: f32) -> Self {
        Self {
            threshold,
            reach: ClipReach {
                before: 0.0,
                after: 0.0,
            },
            chunk_secs: 3.0,
        }
    }

    /// The same filter, told how far a saved clip reaches (see
    /// [`Self::filter_timed`]).
    #[must_use]
    pub const fn with_clip_reach(mut self, reach: ClipReach) -> Self {
        self.reach = reach;
        self
    }

    /// The same filter, for chunks `chunk_secs` long (default 3 s).
    #[must_use]
    pub const fn with_chunk_secs(mut self, chunk_secs: f32) -> Self {
        self.chunk_secs = chunk_secs;
        self
    }

    /// Length of one analysed chunk, in seconds, as this filter measures a
    /// flagged chunk's span.
    #[must_use]
    pub const fn chunk_secs(&self) -> f32 {
        self.chunk_secs
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

impl PrivacyFilter {
    /// [`Self::filter_predictions`], plus: a detection whose saved clip would
    /// reach into a flagged chunk is suppressed too (`PIPE7`).
    ///
    /// The neighbour rule works by index; a clip reaches by time. With a long
    /// extraction, a pre-capture lead-in, or overlapping chunks, a detection
    /// two or more chunks from the speech has a clip that spans it. So each
    /// surviving detection's clip window — its own span widened by the
    /// configured [`ClipReach`] — is checked against every flagged chunk's
    /// span. The window is the segment's own timeline: clips on a
    /// privacy-filtered station are not extended into neighbouring segments,
    /// whose speech this filter never saw.
    ///
    /// `starts[i]` is chunk `i`'s start in seconds.
    pub fn filter_timed(&self, starts: &[f32], chunks: &[ChunkPrediction]) -> Vec<Vec<Detection>> {
        let chunk_secs = self.chunk_secs;
        let by_index = self.filter_predictions(chunks);
        if !self.is_enabled() {
            return by_index;
        }
        let speech: Vec<(f32, f32)> = chunks
            .iter()
            .zip(starts)
            .filter(|(chunk, _)| self.flags(chunk))
            .map(|(_, &start)| (start, start + chunk_secs))
            .collect();
        if speech.is_empty() {
            return by_index;
        }
        by_index
            .into_iter()
            .map(|list| {
                list.into_iter()
                    .filter(|d| {
                        let from = d.start - self.reach.before;
                        let to = d.stop + self.reach.after;
                        let reaches = speech.iter().any(|&(s, e)| from < e && s < to);
                        if reaches {
                            tracing::debug!(
                                species = %d.common_name,
                                "privacy filter: suppressing a detection whose clip would reach speech"
                            );
                        }
                        !reaches
                    })
                    .collect()
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
            loudest_noise: None,
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

    /// Chunks `0, step, 2·step, …`, each holding one blackbird detection
    /// over its own span, with speech in the chunks listed.
    fn timed(n: usize, step: f32, speech: &[usize]) -> (Vec<f32>, Vec<ChunkPrediction>) {
        #[allow(clippy::cast_precision_loss)]
        let starts: Vec<f32> = (0..n).map(|i| i as f32 * step).collect();
        let chunks = starts
            .iter()
            .enumerate()
            .map(|(i, &s)| ChunkPrediction {
                detections: vec![Detection {
                    start: s,
                    stop: s + 3.0,
                    ..make_detection("Turdus merula", "Blackbird", 0.9)
                }],
                human_score: if speech.contains(&i) { 0.8 } else { 0.0 },
                loudest_noise: None,
            })
            .collect();
        (starts, chunks)
    }

    /// PIPE7: a clip must not carry the speech its chunk was cleared of.
    ///
    /// The filter emptied the flagged chunk and its two neighbours by index,
    /// but a saved clip reaches by time: with a 12-second extraction a
    /// detection two chunks from the speech has a clip spanning it. The same
    /// happens with chunk overlap, where "two chunks away" is 3 seconds.
    #[test]
    fn a_clip_that_would_reach_the_speech_is_suppressed() {
        let wide = PrivacyFilter::new(0.03).with_clip_reach(ClipReach {
            before: 4.5,
            after: 4.5,
        });
        // Speech in chunk 0 (0–3 s). Chunk 2's clip is 1.5–13.5 s.
        let (starts, chunks) = timed(5, 3.0, &[0]);
        let kept = wide.filter_timed(&starts, &chunks);
        assert!(
            kept[2].is_empty(),
            "chunk 2's clip reaches back into the speech"
        );
        // Counterpart: chunk 3's clip, 4.5–16.5 s, does not.
        assert!(!kept[3].is_empty(), "chunk 3's clip is clear of it");
        // And forwards: speech in chunk 4 (12–15 s) is inside chunk 2's clip.
        let (starts, chunks) = timed(5, 3.0, &[4]);
        assert!(wide.filter_timed(&starts, &chunks)[2].is_empty());

        // Overlapping chunks at the default 6-second extraction: speech at
        // 0–3 s, and chunk 2 (3–6 s) has a clip from 1.5 s.
        let default = PrivacyFilter::new(0.03).with_clip_reach(ClipReach {
            before: 1.5,
            after: 1.5,
        });
        let (starts, chunks) = timed(6, 1.5, &[0]);
        let kept = default.filter_timed(&starts, &chunks);
        assert!(kept[2].is_empty(), "overlapping chunks leaked the speech");
        assert!(!kept[4].is_empty(), "chunk 4's clip starts at 4.5 s");
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
            loudest_noise: None,
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
