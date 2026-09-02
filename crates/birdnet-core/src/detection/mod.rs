//! Detection types and pipeline.
//!
//! Defines the core detection data structures and the watch -> analyze -> report pipeline.

pub mod corroboration;
pub mod daemon;
pub mod nocturnal;
pub mod noise;
pub mod pipeline;
pub mod privacy;
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

impl ChunkFilters {
    /// Apply every filter in order: privacy, noise, then corroboration.
    ///
    /// `starts[i]` is the start time in seconds of the chunk whose predictions
    /// are `predictions[i]`; it is only read by the corroboration stage, which
    /// needs to know which chunks are near each other.
    #[must_use]
    pub fn apply(
        &self,
        starts: &[f32],
        predictions: &[Vec<types::Detection>],
    ) -> Vec<Vec<types::Detection>> {
        let after_privacy = self.privacy.filter_predictions(predictions);
        let after_noise = self.noise.filter_predictions(&after_privacy);
        corroboration::corroborate(self.confirmation, starts, &after_noise)
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
        ChunkFilters, corroboration::ConfirmationLevel, noise::NoiseFilter, privacy::PrivacyFilter,
        types::Detection,
    };

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
        let both_present = vec![
            d("Homo sapiens", "Human", 0.80),
            d("Dog", "Dog", 0.91),
            d("Turdus merula", "Eurasian Blackbird", 0.82),
        ];
        // The neighbours must carry no human label of their own, or they are
        // flagged on their own merit and the gate passes whatever the order —
        // which is exactly how the first version of this test was wrong.
        let neighbour = vec![d("Parus major", "Great Tit", 0.9)];

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
        let bark = vec![d("Dog", "Dog", 0.91), d("Turdus merula", "Blackbird", 0.82)];
        let voice = vec![d("Homo sapiens", "Human", 0.8)];
        let quiet = vec![d("Parus major", "Great Tit", 0.9)];

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
        let out = privacy_only.apply(&[0.0, 3.0], &[bark, quiet]);
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
                vec![d("Dog", "Dog", 0.91), d("Turdus merula", "Blackbird", 0.82)],
                vec![d("Turdus merula", "Blackbird", 0.82)],
                vec![d("Turdus merula", "Blackbird", 0.82)],
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
