//! Human voice privacy filter.
//!
//! Suppresses every chunk in which the model heard a human (speech, whistling,
//! other human sounds) and the chunks either side of it, so a conversation
//! near the microphone is not kept in the recordings a station retains.
//!
//! Two rules, either of which flags a chunk, each read from the model's output
//! before the detection threshold and the top-N cut:
//!
//! * the *rank rule* is BirdNET-Pi's: a human class among the model's `N`
//!   highest-scoring classes ([`ChunkPrediction::human_rank`]; `N = 10` is
//!   BirdNET-Pi's own default, and what a station gets unless it sets
//!   `PRIVACY_RANK`);
//! * the *score rule* is this project's addition: a human class whose
//!   confidence reaches a threshold ([`ChunkPrediction::human_score`]; off
//!   unless `PRIVACY_THRESHOLD` is set).
//!
//! The judgement is not made on the detection list. A list only carries a human label when speech scored
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
    /// `0.0` switches the score rule off.
    threshold: f32,
    /// A chunk is suppressed when a human class ranks within this many of the
    /// model's highest-scoring classes. `0` switches the rank rule off.
    rank_cutoff: usize,
    /// How far a saved clip reaches beyond its detection.
    reach: ClipReach,
    /// Length of one analysed chunk, in seconds.
    chunk_secs: f32,
    /// Rules for named classifiers, on their own scales (review F09). A
    /// classifier not named here is judged by the station's rule.
    per_classifier: Vec<(String, ClassifierPrivacyRule)>,
}

/// One classifier's own privacy rule, read on that classifier's own scale
/// (review F09). Each half left `None` is the station's.
///
/// Classifiers do not share a score scale or a class count, so one station
/// threshold or rank cutoff means a different thing for each: a human class
/// ranked tenth among Perch's ~15 000 classes is not the claim the same rank
/// makes among BirdNET's 6 000.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ClassifierPrivacyRule {
    /// Its score threshold (`0.0` switches its score rule off); `None` is the
    /// station's.
    pub threshold: Option<f32>,
    /// Its rank cutoff (`0` switches its rank rule off); `None` is the
    /// station's.
    pub rank_cutoff: Option<usize>,
}

impl PrivacyFilter {
    /// Create a privacy filter with the given score threshold and the rank
    /// rule off.
    ///
    /// A threshold of 0.0 disables the filter entirely, until
    /// [`Self::with_rank_cutoff`] turns the other rule on.
    pub const fn new(threshold: f32) -> Self {
        Self {
            threshold,
            rank_cutoff: 0,
            reach: ClipReach {
                before: 0.0,
                after: 0.0,
            },
            chunk_secs: 3.0,
            per_classifier: Vec::new(),
        }
    }

    /// The same filter with BirdNET-Pi's rank rule: a chunk is suppressed when
    /// a human class is among the model's `rank_cutoff` highest-scoring
    /// classes. `0` switches the rule off.
    #[must_use]
    pub const fn with_rank_cutoff(mut self, rank_cutoff: usize) -> Self {
        self.rank_cutoff = rank_cutoff;
        self
    }

    /// The same filter, told how far a saved clip reaches (see
    /// [`Self::filter_timed`]).
    #[must_use]
    pub const fn with_clip_reach(mut self, reach: ClipReach) -> Self {
        self.reach = reach;
        self
    }

    /// The same filter, with `model_id` judged by its own `rule` rather than
    /// the station's (review F09). Naming a classifier twice keeps the last
    /// rule. A rule only binds on a chunk that carries per-classifier evidence
    /// ([`ChunkPrediction::by_classifier`]); with none set, the filter decides
    /// every chunk exactly as it did before rules existed.
    #[must_use]
    pub fn with_classifier_rule(
        mut self,
        model_id: impl Into<String>,
        rule: ClassifierPrivacyRule,
    ) -> Self {
        let model_id = model_id.into();
        self.per_classifier.retain(|(id, _)| *id != model_id);
        self.per_classifier.push((model_id, rule));
        self
    }

    /// The score threshold and rank cutoff `model_id` is judged by: its own
    /// rule where one is set, else the station's.
    #[must_use]
    pub fn rule_for(&self, model_id: &str) -> (f32, usize) {
        let own = self
            .per_classifier
            .iter()
            .find(|(id, _)| id == model_id)
            .map_or_else(ClassifierPrivacyRule::default, |(_, rule)| *rule);
        (
            own.threshold.unwrap_or(self.threshold),
            own.rank_cutoff.unwrap_or(self.rank_cutoff),
        )
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

    /// Whether the privacy filter is enabled: either of the station's rules
    /// is on, or a classifier's own rule turns one on for it.
    pub fn is_enabled(&self) -> bool {
        let on = |threshold: f32, cutoff: usize| threshold > 0.0 || cutoff > 0;
        on(self.threshold, self.rank_cutoff)
            || self
                .per_classifier
                .iter()
                .any(|(id, _)| on(self.rule_for(id).0, self.rule_for(id).1))
    }

    /// Get the current score threshold (`0.0` when the score rule is off).
    pub const fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Get the current rank cutoff (`0` when the rank rule is off).
    pub const fn rank_cutoff(&self) -> usize {
        self.rank_cutoff
    }

    /// Whether one chunk, on its own evidence, is to be suppressed.
    ///
    /// With per-classifier evidence ([`ChunkPrediction::by_classifier`]) each
    /// classifier is judged by its own rule ([`Self::rule_for`]) on its own
    /// score and rank, and the chunk is flagged when any of them fires (review
    /// F09). Without it, the chunk's summaries are judged by the station's
    /// rule. Both readings give the same verdict when no classifier has a rule
    /// of its own.
    fn flags(&self, chunk: &ChunkPrediction) -> bool {
        let fires = |threshold: f32, cutoff: usize, score: f32, rank: Option<usize>| {
            let by_score = threshold > 0.0 && score >= threshold;
            let by_rank = rank.is_some_and(|rank| rank < cutoff);
            by_score || by_rank
        };
        let heard = if chunk.by_classifier.is_empty() {
            fires(
                self.threshold,
                self.rank_cutoff,
                chunk.human_score,
                chunk.human_rank,
            )
        } else {
            chunk.by_classifier.iter().any(|e| {
                let (threshold, cutoff) = self.rule_for(&e.model_id);
                fires(threshold, cutoff, e.human_score, e.human_rank)
            })
        };
        heard || chunk.privacy_unassessed
    }

    /// Which chunks this filter masks: every chunk a human was heard in, and
    /// its neighbours. All `false` when the filter is off.
    ///
    /// The windows [`Self::filter_predictions`] empties, whether or not they
    /// held a detection: what is counted as audio masked for a voice (ACC-02).
    #[must_use]
    pub fn masked_windows(&self, chunks: &[ChunkPrediction]) -> Vec<bool> {
        if !self.is_enabled() {
            return vec![false; chunks.len()];
        }
        // First pass: which chunks contain a human. Second: their neighbours.
        let flagged: Vec<bool> = chunks.iter().map(|chunk| self.flags(chunk)).collect();
        expand_adjacent(&flagged)
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

        let flagged = self.masked_windows(chunks);

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

/// Perch v2's human classes whose names do not contain the word "human"
/// (MMP-05).
///
/// Every class of its vocabulary that the `AudioSet` ontology places
/// under *Human sounds* (`github.com/audioset/ontology`, `ontology.json`, read
/// 2026-11-23, SHA-256 `9c685f44…d2f15d`), and *Homo sapiens*. Perch spells
/// them as `AudioSet` does, with `_` for spaces and `_and_` for commas.
///
/// The breadth follows V2.4's, whose three human classes are vocal, non-vocal
/// and whistling: a cough, footsteps or clapping are a person at the
/// microphone as surely as speech is. `Human_voice` and `Human_group_actions`
/// are matched by the word rule and are not repeated here.
pub const PERCH_HUMAN_CLASSES: &[&str] = &[
    "Applause",
    "Breathing",
    "Burping_and_eructation",
    "Chatter",
    "Cheering",
    "Chewing_and_mastication",
    "Child_speech_and_kid_speaking",
    "Chuckle_and_chortle",
    "Clapping",
    "Conversation",
    "Cough",
    "Crowd",
    "Crying_and_sobbing",
    "Fart",
    "Female_singing",
    "Female_speech_and_woman_speaking",
    "Finger_snapping",
    "Gasp",
    "Giggle",
    "Hands",
    "Homo sapiens",
    "Laughter",
    "Male_singing",
    "Male_speech_and_man_speaking",
    "Respiratory_sounds",
    "Run",
    "Screaming",
    "Shout",
    "Sigh",
    "Singing",
    "Sneeze",
    "Speech",
    "Speech_synthesizer",
    "Walk_and_footsteps",
    "Whispering",
    "Yell",
];

/// Whether a label names a human class.
///
/// Two rules, case-insensitively. Either name has the whole word "human"
/// (split on anything that is not a letter): V2.4's `Human vocal`, `Human
/// non-vocal` and `Human whistle`, V3.0's `Homo sapiens;human` and `Homo
/// Sapiens;Human vocal`, Perch's `Human_voice`. A whole word and not a
/// substring, because Perch carries the cicada *Pacarina schumanni* (MMP-23).
/// Or the scientific name is one of [`PERCH_HUMAN_CLASSES`], Perch's speech,
/// singing and other human sounds, which never say "human". The rule is
/// deliberately not one an operator can extend — see the noise filter for why
/// a substring rule cannot be handed out.
#[must_use]
pub fn names_a_human(scientific_name: &str, common_name: &str) -> bool {
    let has_word = |name: &str| {
        name.split(|c: char| !c.is_alphabetic())
            .any(|word| word.eq_ignore_ascii_case("human"))
    };
    let sci = scientific_name.trim();
    has_word(sci)
        || has_word(common_name)
        || PERCH_HUMAN_CLASSES
            .iter()
            .any(|class| class.eq_ignore_ascii_case(sci))
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
            model_scores: Vec::new(),
        }
    }

    /// A chunk with one blackbird detection and the given human score.
    fn chunk(human_score: f32) -> ChunkPrediction {
        ChunkPrediction {
            detections: vec![make_detection("Turdus merula", "Eurasian Blackbird", 0.9)],
            human_score,
            human_rank: None,
            privacy_unassessed: false,
            loudest_noise: None,
            by_classifier: Vec::new(),
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
                human_rank: None,
                privacy_unassessed: false,
                loudest_noise: None,
                by_classifier: Vec::new(),
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
            human_rank: None,
            privacy_unassessed: false,
            loudest_noise: None,
            by_classifier: Vec::new(),
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

    /// MMP-23: "human" inside another word is not a human class. Observed
    /// failing with the substring rule, which read this cicada as speech.
    #[test]
    fn a_name_containing_human_inside_a_word_is_not_human() {
        assert!(!names_a_human("Pacarina schumanni", "Pacarina schumanni"));
        assert!(names_a_human("Human_voice", "Human_voice"));
        assert!(names_a_human("speech", "Speech"));
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

    /// A chunk with one blackbird detection, no score on the confidence scale,
    /// and a human class at `rank` among the model's classes.
    fn ranked(rank: Option<usize>) -> ChunkPrediction {
        ChunkPrediction {
            human_rank: rank,
            ..chunk(0.0)
        }
    }

    /// BirdNET-Pi's rule at its default: a human class within the model's
    /// ten highest-scoring classes. "Within the first ten" is ranks 0..=9,
    /// so rank 9 masks and rank 10 does not — the boundary BirdNET-Pi's
    /// `prediction[:human_cutoff]` draws, and the one a `<=` slip moves.
    #[test]
    fn the_rank_rule_masks_a_human_in_the_top_ten_and_not_the_eleventh() {
        let filter = PrivacyFilter::new(0.0).with_rank_cutoff(10);
        assert!(
            filter.is_enabled(),
            "the rank rule alone enables the filter"
        );

        let kept = |rank| !filter.filter_predictions(&[ranked(rank)])[0].is_empty();
        assert!(!kept(Some(0)), "the top answer");
        assert!(
            !kept(Some(9)),
            "the tenth class is inside BirdNET-Pi's cutoff"
        );
        assert!(kept(Some(10)), "the eleventh class is outside it");
        assert!(
            kept(Some(500)),
            "a human sound far down the list is not speech"
        );
        assert!(kept(None), "a model with no human class never masks");
    }

    /// The neighbour rule is BirdNET-Pi's too: the windows either side of a
    /// flagged one go with it, whichever rule flagged it.
    #[test]
    fn the_rank_rule_masks_the_neighbours_of_the_window_too() {
        let filter = PrivacyFilter::new(0.0).with_rank_cutoff(10);
        let result = filter.filter_predictions(&[
            ranked(Some(40)),
            ranked(Some(3)),
            ranked(Some(40)),
            ranked(Some(40)),
        ]);
        assert!(result[0].is_empty() && result[1].is_empty() && result[2].is_empty());
        assert!(!result[3].is_empty(), "two windows away is clear");
    }

    /// `PRIVACY_RANK=0` is the off switch, and it must not be the same thing
    /// as "rank below zero": with the rule off a human at the very top of the
    /// list masks nothing, as the all-off filter does.
    #[test]
    fn a_rank_cutoff_of_zero_switches_the_rank_rule_off() {
        let filter = PrivacyFilter::new(0.0).with_rank_cutoff(0);
        assert!(!filter.is_enabled());
        assert!(!filter.filter_predictions(&[ranked(Some(0))])[0].is_empty());
    }

    /// The two rules are independent and either one suppresses: the rank
    /// rule does not turn the score threshold off, and a score threshold of
    /// zero does not turn the rank rule off.
    #[test]
    fn either_rule_alone_suppresses_a_window() {
        let both = PrivacyFilter::new(0.03).with_rank_cutoff(10);
        let by_score = ChunkPrediction {
            human_rank: Some(200),
            ..chunk(0.5)
        };
        let by_rank = ChunkPrediction {
            human_rank: Some(2),
            ..chunk(0.0)
        };
        let neither = ChunkPrediction {
            human_rank: Some(200),
            ..chunk(0.0)
        };
        assert!(both.filter_predictions(&[by_score])[0].is_empty());
        assert!(both.filter_predictions(&[by_rank])[0].is_empty());
        assert!(!both.filter_predictions(&[neither])[0].is_empty());
        // Score rule off, rank rule on: a loud score alone masks nothing.
        let rank_only = PrivacyFilter::new(0.0).with_rank_cutoff(10);
        let loud_but_low_ranked = ChunkPrediction {
            human_rank: Some(200),
            ..chunk(0.9)
        };
        assert!(!rank_only.filter_predictions(&[loud_but_low_ranked])[0].is_empty());
    }

    /// The timed filter (clips that reach a flagged window) honours the rank
    /// rule, not only the score rule: its own flag test is the same one.
    #[test]
    fn a_clip_that_would_reach_a_rank_flagged_window_is_suppressed() {
        let filter = PrivacyFilter::new(0.0)
            .with_rank_cutoff(10)
            .with_clip_reach(ClipReach {
                before: 4.5,
                after: 4.5,
            });
        let (starts, mut chunks) = timed(5, 3.0, &[]);
        chunks[0].human_rank = Some(1);
        let kept = filter.filter_timed(&starts, &chunks);
        assert!(
            kept[2].is_empty(),
            "chunk 2's clip reaches back into chunk 0"
        );
        assert!(!kept[3].is_empty(), "chunk 3's clip does not");
    }

    /// One classifier's evidence about people in a chunk.
    fn heard_by(
        id: &str,
        score: f32,
        rank: Option<usize>,
    ) -> crate::detection::types::ClassifierEvidence {
        crate::detection::types::ClassifierEvidence {
            model_id: id.to_owned(),
            human_score: score,
            human_rank: rank,
            loudest_noise: None,
        }
    }

    /// A chunk two classifiers answered, with the summaries the daemon writes
    /// beside the per-classifier evidence: the highest score and best rank.
    fn heard_by_both(
        a: crate::detection::types::ClassifierEvidence,
        b: crate::detection::types::ClassifierEvidence,
    ) -> ChunkPrediction {
        ChunkPrediction {
            human_score: a.human_score.max(b.human_score),
            human_rank: match (a.human_rank, b.human_rank) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (x, y) => x.or(y),
            },
            by_classifier: vec![a, b],
            ..chunk(0.0)
        }
    }

    /// Review F09: a classifier is judged against its own rule, on its own
    /// scale. Perch's 0.30 clears the 0.25 its rule names, though the station's
    /// 0.80 is far above every score; and a rule naming BirdNET, which heard
    /// 0.10, does not borrow Perch's 0.30 to fire. The second is what a
    /// "highest score against the lowest threshold" reading would get wrong.
    #[test]
    fn each_classifier_is_judged_by_its_own_privacy_rule() {
        let chunk = heard_by_both(
            heard_by("birdnet", 0.10, Some(300)),
            heard_by("perch", 0.30, Some(40)),
        );
        let own = |id: &str, rule| {
            PrivacyFilter::new(0.80)
                .with_classifier_rule(id, rule)
                .masked_windows(std::slice::from_ref(&chunk))[0]
        };
        let score = |t| ClassifierPrivacyRule {
            threshold: Some(t),
            rank_cutoff: None,
        };
        assert!(
            own("perch", score(0.25)),
            "perch's own threshold was not applied"
        );
        assert!(
            !own("birdnet", score(0.25)),
            "birdnet's rule fired on another classifier's score"
        );
        // The rank rule likewise: Perch's human class is 40th, inside a cutoff
        // of 50 named for it; BirdNET's is 300th, outside the same cutoff.
        let rank = |c| ClassifierPrivacyRule {
            threshold: None,
            rank_cutoff: Some(c),
        };
        let station = PrivacyFilter::new(0.0).with_rank_cutoff(10);
        let masks = |f: PrivacyFilter| f.masked_windows(std::slice::from_ref(&chunk))[0];
        assert!(
            !masks(station.clone()),
            "precondition: the station's rule does not fire"
        );
        assert!(masks(
            station.clone().with_classifier_rule("perch", rank(50))
        ));
        assert!(!masks(station.with_classifier_rule("birdnet", rank(50))));
        // A rule can switch a classifier's score rule off.
        let off = PrivacyFilter::new(0.20).with_classifier_rule("perch", score(0.0));
        assert!(!off.masked_windows(std::slice::from_ref(&chunk))[0]);
    }

    /// The default is unchanged: with no classifier rule, judging each
    /// classifier on the station's rule decides every chunk as the summaries
    /// (highest score, best rank) did, across every combination tried.
    #[test]
    fn with_no_classifier_rule_the_verdict_is_the_summary_s() {
        let scores = [0.0, 0.05, 0.3, 0.5, 0.9, f32::NAN];
        let ranks = [None, Some(0), Some(9), Some(10), Some(400)];
        let mut tried = 0;
        for threshold in [0.0, 0.03, 0.3, 0.5] {
            for cutoff in [0, 10, 50] {
                let filter = PrivacyFilter::new(threshold).with_rank_cutoff(cutoff);
                for &sa in &scores {
                    for &sb in &scores {
                        for &ra in &ranks {
                            for &rb in &ranks {
                                let each =
                                    heard_by_both(heard_by("a", sa, ra), heard_by("b", sb, rb));
                                let summary = ChunkPrediction {
                                    by_classifier: Vec::new(),
                                    ..each.clone()
                                };
                                assert_eq!(
                                    filter.masked_windows(std::slice::from_ref(&each)),
                                    filter.masked_windows(std::slice::from_ref(&summary)),
                                    "t={threshold} c={cutoff} a=({sa},{ra:?}) b=({sb},{rb:?})"
                                );
                                tried += 1;
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(tried, 4 * 3 * 36 * 25);
    }
}
