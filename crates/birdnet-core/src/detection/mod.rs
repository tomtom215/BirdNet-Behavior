//! Detection types and pipeline.
//!
//! Defines the core detection data structures and the watch -> analyze -> report pipeline.

pub mod answers;
pub mod corroboration;
pub mod daemon;
pub mod dynamic_threshold;
pub mod merge;
pub mod nocturnal;
pub mod noise;
pub mod pipeline;
pub mod privacy;
pub mod sound_class;
pub mod types;

/// The two filters that judge a whole chunk rather than one detection.
///
/// Bundled because they sit at the same point in the chain, are configured the
/// same way, and are always passed together — `process_and_infer_filtered` was
/// already over clippy's argument limit before the second one existed.
///
/// Order matters and is fixed here rather than at each call site: privacy
/// first. A recording containing both a voice and a bark must be suppressed
/// for the voice whatever the noise filter is set to, and the privacy filter
/// spreads to neighbouring chunks while the noise filter does not — running it
/// second would let the noise filter's narrower verdict be mistaken for the
/// whole answer.
#[derive(Debug, Clone)]
pub struct ChunkFilters {
    /// Suppresses chunks containing human speech, and their neighbours.
    pub privacy: privacy::PrivacyFilter,
    /// Suppresses chunks containing a watched non-bird noise class.
    pub noise: noise::NoiseFilter,
    /// Requires a species to be heard in enough nearby windows before it is
    /// recorded. Runs last, and has to: the other two remove whole chunks, and
    /// counting corroboration across chunks a filter has already emptied would
    /// credit a species with evidence that was thrown away.
    pub confirmation: corroboration::ConfirmationLevel,
}

/// What the filters discarded from one recording, by reason (ACC-02).
///
/// Each discard is counted once, under the first stage that removed it, in
/// the order the daemon applies them: privacy, noise, corroboration, then the
/// species occurrence filter and the operator's lists. *Windows* are analysis
/// windows a filter masked whole, whether or not they held a detection: the
/// audio the station listened to and did not judge. *Detections* are the
/// species the classifier reported at the floor it runs at (the lowest
/// per-species threshold, so some sit under the station's own bar and would
/// have been dropped later for confidence); sounds and human classes, which
/// are never stored, are not counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterTally {
    /// The recording's date, `YYYY-MM-DD`, as its file name gives it. Empty
    /// until the daemon sets it.
    pub date: String,
    /// Analysis windows in the recording.
    pub windows: u64,
    /// Windows masked for a human voice, the voiced ones and their neighbours.
    pub privacy_windows: u64,
    /// Windows masked for a watched noise class, not already masked for a voice.
    pub noise_windows: u64,
    /// Species detections the privacy filter removed.
    pub privacy_detections: u64,
    /// Species detections the noise filter removed.
    pub noise_detections: u64,
    /// Species detections corroboration removed.
    pub corroboration_detections: u64,
    /// Species detections the occurrence model rejected here this week, or
    /// that it does not score (ACC-01).
    pub occurrence_detections: u64,
    /// Species detections the operator's include or exclude list removed.
    pub species_list_detections: u64,
    /// Windows no routed classifier answered (review F18): not listening, and
    /// not counted as effort.
    pub failed_windows: u64,
    /// Milliseconds of audio that decoded, to the end of the last window
    /// (review F10); `0` when not known. A truncated file's header can claim
    /// more: a WAV cut short by a crash says 15 s and holds 1 s, and ffmpeg's
    /// placeholder header says nothing at all.
    pub decoded_ms: u64,
}

impl FilterTally {
    /// Whether anything was discarded.
    #[must_use]
    pub const fn discarded_anything(&self) -> bool {
        self.privacy_windows
            + self.noise_windows
            + self.privacy_detections
            + self.noise_detections
            + self.corroboration_detections
            + self.occurrence_detections
            + self.species_list_detections
            > 0
    }

    /// One row per reason label: `(reason, windows masked, detections
    /// dropped)`. Only `privacy` and `noise` mask windows.
    #[must_use]
    pub const fn by_reason(&self) -> [(&'static str, u64, u64); 5] {
        [
            ("privacy", self.privacy_windows, self.privacy_detections),
            ("noise", self.noise_windows, self.noise_detections),
            ("corroboration", 0, self.corroboration_detections),
            ("occurrence", 0, self.occurrence_detections),
            ("species_list", 0, self.species_list_detections),
        ]
    }
}

/// Species detections in some chunks' lists: not the sounds and human classes.
fn species_in(lists: &[Vec<types::Detection>]) -> u64 {
    lists
        .iter()
        .flatten()
        .filter(|d| !sound_class::is_sound_class(&d.scientific_name, &d.common_name))
        .count() as u64
}

impl ChunkFilters {
    /// [`Self::apply`], also counting what each filter discarded (ACC-02).
    #[must_use]
    pub fn apply_tallied(
        &self,
        starts: &[f32],
        chunks: &[types::ChunkPrediction],
    ) -> (Vec<Vec<types::Detection>>, FilterTally) {
        let before: Vec<Vec<types::Detection>> =
            chunks.iter().map(|c| c.detections.clone()).collect();
        let after_privacy = self.privacy.filter_timed(starts, chunks);
        let privacy_masked = self.privacy.masked_windows(chunks);
        let loudest: Vec<Option<types::Detection>> =
            chunks.iter().map(|c| c.loudest_noise.clone()).collect();
        let noise_windows = after_privacy
            .iter()
            .zip(&loudest)
            .zip(&privacy_masked)
            .filter(|((list, loud), masked)| {
                !**masked && self.noise.firing(list, loud.as_ref()).is_some()
            })
            .count() as u64;
        let after_noise = self
            .noise
            .filter_with_evidence(starts, &after_privacy, &loudest);
        let kept = corroboration::corroborate(self.confirmation, starts, &after_noise);

        let (n0, n1, n2, n3) = (
            species_in(&before),
            species_in(&after_privacy),
            species_in(&after_noise),
            species_in(&kept),
        );
        let tally = FilterTally {
            windows: chunks.len() as u64,
            privacy_windows: privacy_masked.iter().filter(|&&m| m).count() as u64,
            noise_windows,
            privacy_detections: n0.saturating_sub(n1),
            noise_detections: n1.saturating_sub(n2),
            corroboration_detections: n2.saturating_sub(n3),
            ..FilterTally::default()
        };
        (kept, tally)
    }

    /// Apply every filter in order: privacy, noise, then corroboration.
    ///
    /// `starts[i]` is the start time in seconds of the chunk whose prediction
    /// is `chunks[i]`. The corroboration stage reads it to know which chunks
    /// are near each other, and the privacy stage to check each clip's reach
    /// against the speech in time. The privacy
    /// stage reads each chunk's human score; the later stages read only its
    /// detections.
    #[must_use]
    pub fn apply(
        &self,
        starts: &[f32],
        chunks: &[types::ChunkPrediction],
    ) -> Vec<Vec<types::Detection>> {
        self.apply_tallied(starts, chunks).0
    }

    /// Whether any filter will suppress anything.
    #[must_use]
    pub fn any_enabled(&self) -> bool {
        self.privacy.is_enabled() || self.noise.is_enabled() || self.confirmation.enabled()
    }
}

#[cfg(test)]
mod chunk_filter_tests {
    use super::{
        ChunkFilters,
        corroboration::ConfirmationLevel,
        noise::NoiseFilter,
        privacy::PrivacyFilter,
        types::{ChunkPrediction, Detection},
    };

    /// A chunk the model heard no human in.
    fn quiet(detections: Vec<Detection>) -> ChunkPrediction {
        ChunkPrediction {
            detections,
            human_score: 0.0,
            human_rank: None,
            privacy_unassessed: false,
            loudest_noise: None,
        }
    }

    /// A chunk the model heard a human in, clearly.
    fn voiced(detections: Vec<Detection>) -> ChunkPrediction {
        ChunkPrediction {
            detections,
            human_score: 0.8,
            human_rank: None,
            privacy_unassessed: false,
            loudest_noise: None,
        }
    }

    /// A detection with the given names and confidence.
    fn d(sci: &str, com: &str, confidence: f32) -> Detection {
        Detection {
            date: "2026-03-14".into(),
            time: "19:40:00".into(),
            scientific_name: sci.into(),
            common_name: com.into(),
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

    /// Both filters on.
    fn both() -> ChunkFilters {
        ChunkFilters {
            privacy: PrivacyFilter::new(0.03),
            noise: NoiseFilter::with_default_classes(0.5),
            confirmation: ConfirmationLevel::Off,
        }
    }

    /// A bark scored below the *detection* threshold still reaches a noise
    /// threshold set below it. The filter read only the detection list,
    /// which never holds anything under the detection threshold, so
    /// `NOISE_THRESHOLD=0.3` under `CONFIDENCE=0.7` could never fire.
    #[test]
    fn a_bark_below_the_detection_threshold_still_trips_the_noise_filter() {
        let filters = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise: NoiseFilter::with_default_classes(0.3),
            confirmation: ConfirmationLevel::Off,
        };
        let mut barked = quiet(vec![d("Turdus merula", "Eurasian Blackbird", 0.8)]);
        barked.loudest_noise = Some(d("Dog", "Dog", 0.5));
        let out = filters.apply(&[0.0], &[barked]);
        assert!(
            out[0].is_empty(),
            "the bark at 0.5 cleared the 0.3 noise bar"
        );

        // Counterpart: a quieter bark, under the noise bar, leaves the chunk.
        let mut faint = quiet(vec![d("Turdus merula", "Eurasian Blackbird", 0.8)]);
        faint.loudest_noise = Some(d("Dog", "Dog", 0.2));
        let out = filters.apply(&[0.0], &[faint]);
        assert_eq!(out[0].len(), 1);
    }

    #[test]
    fn privacy_runs_first_so_a_bark_cannot_swallow_the_evidence_of_a_voice() {
        // The ordering is fixed in `apply`, so this is what fixes it.
        //
        // A chunk holding both a voice and a bark: privacy first flags it and
        // spreads to the neighbours, which is the guarantee the privacy filter
        // makes. Noise first empties the chunk, and the privacy filter then
        // scans an empty chunk, finds no human, and leaves the neighbours
        // alone — so a recording of someone talking survives, in the chunks
        // either side, because a dog happened to bark over the middle of it.
        let both_present = voiced(vec![
            d("Dog", "Dog", 0.91),
            d("Turdus merula", "Eurasian Blackbird", 0.82),
        ]);
        // The neighbours must carry no human score of their own, or they are
        // flagged on their own merit and the gate passes whatever the order —
        // which is exactly how the first version of this test was wrong.
        let neighbour = quiet(vec![d("Parus major", "Great Tit", 0.9)]);

        let out = both().apply(
            &[0.0, 3.0, 6.0],
            &[neighbour.clone(), both_present, neighbour],
        );
        assert!(out[1].is_empty(), "the chunk itself must be suppressed");
        assert!(
            out[0].is_empty() && out[2].is_empty(),
            "the privacy spread was lost: a bark over speech left the neighbouring \
             chunks of that speech recorded"
        );
    }

    #[test]
    fn each_filter_still_acts_alone() {
        // Counterpart: the ordering gate above is satisfied by any arrangement
        // that suppresses everything, so pin that each filter is actually
        // consulted and that neither one's verdict is the other's.
        let bark = quiet(vec![
            d("Dog", "Dog", 0.91),
            d("Turdus merula", "Blackbird", 0.82),
        ]);
        let voice = voiced(vec![d("Parus major", "Great Tit", 0.8)]);
        let tit = quiet(vec![d("Parus major", "Great Tit", 0.9)]);

        // Noise only: the bark chunk goes, the voice chunk stays.
        let noise_only = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise: NoiseFilter::with_default_classes(0.5),
            confirmation: ConfirmationLevel::Off,
        };
        let out = noise_only.apply(&[0.0, 3.0], &[bark.clone(), voice]);
        assert!(out[0].is_empty());
        assert_eq!(out[1].len(), 1, "the privacy filter acted while disabled");

        // Privacy only: the voice chunk goes, the bark chunk stays.
        let privacy_only = ChunkFilters {
            privacy: PrivacyFilter::new(0.03),
            noise: NoiseFilter::with_default_classes(0.0),
            confirmation: ConfirmationLevel::Off,
        };
        let out = privacy_only.apply(&[0.0, 3.0], &[bark, tit]);
        assert_eq!(out[0].len(), 2, "the noise filter acted while disabled");
    }

    #[test]
    fn corroboration_runs_after_the_chunk_filters_and_not_before() {
        // The ordering `apply` fixes, in the direction that can actually go
        // wrong. Corroboration counts how many nearby chunks carry a species;
        // if it runs before the filters that empty whole chunks, it counts
        // evidence that is about to be thrown away.
        //
        // Three chunks at 3 s steps. A blackbird sings through all three; a dog
        // barks over the first. `Strict` needs 70% of the neighbourhood, and
        // the middle chunk's neighbourhood is all three, so it needs all three.
        //
        //   filters then corroborate (correct): the bark empties chunk 0, the
        //     blackbird is left with 2 of 3, and the middle chunk drops it.
        //   corroborate then filters (wrong): the blackbird has 3 of 3 and
        //     survives — and the bark, being alone, is itself corroborated away
        //     before the noise filter ever sees it.
        let filters = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise: NoiseFilter::with_default_classes(0.5),
            confirmation: ConfirmationLevel::Strict,
        };
        let out = filters.apply(
            &[0.0, 3.0, 6.0],
            &[
                quiet(vec![
                    d("Dog", "Dog", 0.91),
                    d("Turdus merula", "Blackbird", 0.82),
                ]),
                quiet(vec![d("Turdus merula", "Blackbird", 0.82)]),
                quiet(vec![d("Turdus merula", "Blackbird", 0.82)]),
            ],
        );
        assert!(
            out[1].is_empty(),
            "corroboration credited the blackbird with a chunk the noise filter \
             had already emptied: {out:?}"
        );
        // Counterpart, so the gate is not satisfied by a filter that simply
        // suppresses everything: the last chunk's neighbourhood is only two
        // wide, and both of those carry the blackbird.
        assert_eq!(
            out[2].len(),
            1,
            "the trailing chunk was corroborated by its own neighbourhood and \
             should have survived: {out:?}"
        );
    }

    /// ACC-02's gate: a voice between two birds masks three windows (the
    /// voiced one and both neighbours), and the three birds it removed are
    /// counted against privacy. Nothing counted this before.
    #[test]
    fn a_voice_between_two_birds_is_counted_as_three_masked_windows() {
        let filters = ChunkFilters {
            privacy: PrivacyFilter::new(0.03),
            noise: NoiseFilter::with_default_classes(0.0),
            confirmation: ConfirmationLevel::Off,
        };
        let bird = quiet(vec![d("Turdus merula", "Eurasian Blackbird", 0.8)]);
        let mut voice = voiced(vec![d("Turdus merula", "Eurasian Blackbird", 0.8)]);
        // The human class itself is in the list: it is a sound, not a
        // species, and must not be counted as a bird the filter removed.
        voice.detections.push(d("Human vocal", "Human vocal", 0.9));
        let (out, tally) = filters.apply_tallied(&[0.0, 3.0, 6.0], &[bird.clone(), voice, bird]);
        assert!(out.iter().all(Vec::is_empty));
        assert_eq!(tally.windows, 3);
        assert_eq!(tally.privacy_windows, 3, "{tally:?}");
        assert_eq!(tally.privacy_detections, 3, "{tally:?}");
        assert_eq!(tally.noise_windows, 0);
        assert_eq!(tally.noise_detections + tally.corroboration_detections, 0);
    }

    /// Counterparts: each filter's discards land under its own reason, once,
    /// and a recording nothing touched counts nothing.
    #[test]
    fn each_filter_s_discards_are_counted_under_its_own_reason_once() {
        // A bark masks its window; a voice in the same window is counted
        // under privacy only, because privacy ran first.
        let filters = ChunkFilters {
            privacy: PrivacyFilter::new(0.03),
            noise: NoiseFilter::with_default_classes(0.5),
            confirmation: ConfirmationLevel::Off,
        };
        let mut barked_and_voiced = voiced(vec![d("Parus major", "Great Tit", 0.8)]);
        barked_and_voiced.loudest_noise = Some(d("Dog", "Dog", 0.9));
        let mut barked = quiet(vec![
            d("Dog", "Dog", 0.9),
            d("Parus major", "Great Tit", 0.8),
            d("Pica pica", "Eurasian Magpie", 0.8),
        ]);
        barked.loudest_noise = Some(d("Dog", "Dog", 0.9));
        let clear = quiet(vec![d("Parus major", "Great Tit", 0.8)]);
        // Windows: 0 voiced, 1 neighbour of the voice, 2 clear, 3 barked, 4 clear.
        // Each detection at its own window's times, or the clip-reach rule
        // reads them all as reaching the voice at 0-3 s.
        let mut chunks = vec![
            barked_and_voiced,
            clear.clone(),
            clear.clone(),
            barked,
            clear.clone(),
        ];
        for (chunk, start) in chunks.iter_mut().zip([0.0_f32, 3.0, 6.0, 9.0, 12.0]) {
            for det in &mut chunk.detections {
                det.start = start;
                det.stop = det.start + 3.0;
            }
        }
        let (_, tally) = filters.apply_tallied(&[0.0, 3.0, 6.0, 9.0, 12.0], &chunks);
        assert_eq!(tally.windows, 5);
        assert_eq!(
            (tally.privacy_windows, tally.privacy_detections),
            (2, 2),
            "{tally:?}"
        );
        assert_eq!(
            (tally.noise_windows, tally.noise_detections),
            (1, 2),
            "{tally:?}"
        );
        assert_eq!(tally.corroboration_detections, 0);

        // Corroboration: a lone call in three windows is removed and counted;
        // it masks no window.
        let strict = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise: NoiseFilter::with_default_classes(0.0),
            confirmation: ConfirmationLevel::Strict,
        };
        let (out, tally) = strict.apply_tallied(
            &[0.0, 3.0, 6.0],
            &[clear.clone(), quiet(Vec::new()), quiet(Vec::new())],
        );
        assert!(out[0].is_empty(), "precondition: the lone call is removed");
        assert_eq!(tally.corroboration_detections, 1);
        assert_eq!(tally.privacy_windows + tally.noise_windows, 0);

        // Nothing on, nothing counted; and `apply` is `apply_tallied`'s lists.
        let off = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise: NoiseFilter::with_default_classes(0.0),
            confirmation: ConfirmationLevel::Off,
        };
        let chunks = [clear.clone(), clear];
        let (out, tally) = off.apply_tallied(&[0.0, 3.0], &chunks);
        let names = |lists: &[Vec<Detection>]| -> Vec<Vec<String>> {
            lists
                .iter()
                .map(|l| l.iter().map(|d| d.scientific_name.clone()).collect())
                .collect()
        };
        assert_eq!(names(&out), names(&off.apply(&[0.0, 3.0], &chunks)));
        assert_eq!(
            tally,
            super::FilterTally {
                windows: 2,
                ..super::FilterTally::default()
            }
        );
    }

    #[test]
    fn any_enabled_reports_either_filter() {
        let off = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise: NoiseFilter::with_default_classes(0.0),
            confirmation: ConfirmationLevel::Off,
        };
        assert!(!off.any_enabled());
        assert!(both().any_enabled());
        assert!(
            ChunkFilters {
                privacy: PrivacyFilter::new(0.0),
                noise: NoiseFilter::with_default_classes(0.5),
                confirmation: ConfirmationLevel::Off,
            }
            .any_enabled(),
            "the noise filter alone must count as enabled"
        );
        assert!(
            ChunkFilters {
                privacy: PrivacyFilter::new(0.0),
                noise: NoiseFilter::with_default_classes(0.0),
                confirmation: ConfirmationLevel::Balanced,
            }
            .any_enabled(),
            "corroboration alone must count as enabled"
        );
    }
}
