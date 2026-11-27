//! The multi-model invariants (audit MMP-21), each named and each tested.
//!
//! Several were held only by example tests that fix one input. Where the
//! invariant is a property — of every input, not of one — it is a property test
//! here. The examples that already exist are named beside each invariant, so
//! this file is also the index of where each one is held.
//!
//! | # | Invariant | Held by |
//! |---|-----------|---------|
//! | I1 | A classifier's window is clamped to the recorded audio, and the rest is padding: `supported + padded = window`, `padded = 0` when the audio covers the window | `window_support_is_the_audio_that_exists_and_the_rest_is_padding` (here) |
//! | I2 | Two answers' shared support is symmetric, never negative, and never more than either's support | `shared_support_is_symmetric_and_bounded` (here) |
//! | I3 | The merge reports each species some classifier reported exactly once, under one of the confidences a classifier gave it (never a blend), with an agreement between 1 and the classifiers that reported it | `the_merge_keeps_one_real_score_per_species` (here) |
//! | I4 | When any classifier cleared its own bar for a species, the merged detection is one that cleared its own bar | `a_species_some_classifier_cleared_is_carried_by_one_that_did` (here) |
//! | I5 | The merge does not depend on the order classifiers finished in (distinct scores) | `the_merge_does_not_depend_on_the_order_classifiers_answered` (here) |
//! | I6 | Privacy: a window is masked exactly when it or a neighbour has a classifier whose own rule fires on its own evidence, or a human-capable classifier failed it | `the_privacy_mask_is_each_classifier_s_own_rule_and_the_neighbours` (here) |
//! | I7 | Privacy is monotone: more evidence never unmasks a window, and a rule for a classifier that did not answer changes nothing | `more_evidence_never_unmasks_and_an_absent_classifier_s_rule_is_inert` (here) |
//! | I8 | Noise: a chunk is dropped exactly when a watched class reaches the threshold of the classifier that scored it | `a_chunk_is_dropped_exactly_when_a_watched_class_reaches_its_own_classifier_s_bar` (here) |
//! | I9 | Route isolation: a source routed to one classifier is judged by that one alone | `a_source_routed_to_one_classifier_hears_only_that_one` (here); `a_routed_source_reaches_exactly_its_classifiers` (registry) |
//! | I10 | One classifier failing a window leaves the others' detections, and a window none answered costs that window only | `a_classifier_failing_a_window_does_not_silence_the_others`, `windows_answered_around_a_failed_one_are_kept` (process) |
//! | I11 | A human-capable classifier failing a window masks it under the privacy filter | `a_window_a_classifier_that_hears_people_failed_is_masked` (process); I6 here |
//! | I12 | Two names of one bundled synonym pair are one species to the merge; any two other names are two | `the_two_names_of_a_pair_merge_and_no_others_do` (here) |
//!
//! Not held anywhere, and not claimed: that two real models agree on real
//! audio (no two-real-model run exists in CI), and that agreement predicts a
//! correct identification.

use std::collections::{BTreeMap, BTreeSet};

use birdnet_core::detection::answers::{shared_support_secs, window_support};
use birdnet_core::detection::merge::{ModelVerdict, merge_verdicts, merge_verdicts_with};
use birdnet_core::detection::noise::NoiseFilter;
use birdnet_core::detection::privacy::{ClassifierPrivacyRule, PrivacyFilter};
use birdnet_core::detection::types::{
    AnswerState, ChunkPrediction, ClassifierEvidence, Detection, ModelAnswer,
};
use birdnet_core::inference::synonyms::SynonymTable;
use proptest::prelude::*;

const SPECIES: [&str; 4] = [
    "Turdus merula",
    "Pica pica",
    "Erithacus rubecula",
    "Parus major",
];
const MODELS: [&str; 4] = ["birdnet", "v3", "perch", "bat"];

fn det(sci: &str, confidence: f32, model: Option<&str>) -> Detection {
    Detection {
        date: "2026-09-12".to_owned(),
        time: "06:00:00".to_owned(),
        scientific_name: sci.to_owned(),
        common_name: sci.to_owned(),
        confidence,
        start: 0.0,
        stop: 3.0,
        week: 37,
        file_name_extr: None,
        model_id: model.map(str::to_owned),
        agreeing_models: None,
        model_scores: Vec::new(),
    }
}

fn answer(start: f32, end: f32, padded: f32) -> ModelAnswer {
    ModelAnswer {
        model_id: "m".to_owned(),
        state: AnswerState::Heard,
        score: Some(0.9),
        bar: Some(0.5),
        window_start_secs: start,
        window_end_secs: end,
        padded_secs: padded,
    }
}

/// Verdicts from generated `(threshold, [(species index, confidence)])`, one
/// per model, with confidences made distinct across the whole set so that no
/// tie can make an order-dependence look like none (or the reverse).
fn verdicts_from(raw: &[(f32, Vec<(usize, f32)>)]) -> Vec<ModelVerdict> {
    let mut n = 0_u16;
    raw.iter()
        .enumerate()
        .map(|(m, (threshold, dets))| ModelVerdict {
            model_id: MODELS[m].to_owned(),
            threshold: *threshold,
            detections: dets
                .iter()
                .map(|(s, c)| {
                    n += 1;
                    // Distinct, and still in [0, 1].
                    let c = (c * 0.9).mul_add(1.0, f32::from(n) * 1e-4).min(1.0);
                    det(SPECIES[*s], c, None)
                })
                .collect(),
        })
        .collect()
}

fn verdict_strategy() -> impl Strategy<Value = Vec<(f32, Vec<(usize, f32)>)>> {
    proptest::collection::vec(
        (
            0.0_f32..1.0,
            proptest::collection::vec((0..SPECIES.len(), 0.0_f32..1.0), 0..5),
        ),
        1..=MODELS.len(),
    )
}

/// Each chunk: per-classifier evidence `(model index, score, rank)` and
/// whether a human-capable classifier failed it.
type ChunkSpec = (Vec<(usize, f32, Option<usize>)>, bool);

fn chunks_from(spec: &[ChunkSpec]) -> Vec<ChunkPrediction> {
    spec.iter()
        .map(|(evidence, unassessed)| ChunkPrediction {
            detections: vec![det("Turdus merula", 0.9, Some("birdnet"))],
            human_score: evidence.iter().map(|e| e.1).fold(0.0, f32::max),
            human_rank: evidence.iter().filter_map(|e| e.2).min(),
            privacy_unassessed: *unassessed,
            loudest_noise: None,
            by_classifier: evidence
                .iter()
                .map(|(m, score, rank)| ClassifierEvidence {
                    model_id: MODELS[*m].to_owned(),
                    human_score: *score,
                    human_rank: *rank,
                    loudest_noise: None,
                })
                .collect(),
        })
        .collect()
}

fn chunk_strategy() -> impl Strategy<Value = Vec<ChunkSpec>> {
    proptest::collection::vec(
        (
            proptest::collection::vec(
                (0..2_usize, 0.0_f32..1.0, proptest::option::of(0_usize..30)),
                0..3,
            ),
            proptest::bool::weighted(0.1),
        ),
        0..12,
    )
}

/// The privacy mask written out from its definition, independently of the
/// filter's code.
fn reference_mask(filter: &PrivacyFilter, chunks: &[ChunkPrediction]) -> Vec<bool> {
    if !filter.is_enabled() {
        return vec![false; chunks.len()];
    }
    let fires = |(threshold, cutoff): (f32, usize), score: f32, rank: Option<usize>| {
        (threshold > 0.0 && score >= threshold) || rank.is_some_and(|r| r < cutoff)
    };
    let flagged: Vec<bool> = chunks
        .iter()
        .map(|c| {
            let heard = if c.by_classifier.is_empty() {
                fires(
                    (filter.threshold(), filter.rank_cutoff()),
                    c.human_score,
                    c.human_rank,
                )
            } else {
                c.by_classifier
                    .iter()
                    .any(|e| fires(filter.rule_for(&e.model_id), e.human_score, e.human_rank))
            };
            heard || c.privacy_unassessed
        })
        .collect();
    (0..flagged.len())
        .map(|i| {
            flagged[i] || (i > 0 && flagged[i - 1]) || flagged.get(i + 1).copied().unwrap_or(false)
        })
        .collect()
}

proptest! {
    /// I1.
    #[test]
    fn window_support_is_the_audio_that_exists_and_the_rest_is_padding(
        start in 0.0_f32..600.0,
        window in 0.5_f32..10.0,
        audio_after_start in -5.0_f32..15.0,
    ) {
        let audio_end = start + audio_after_start;
        let (end, padded) = window_support(start, window, audio_end);
        prop_assert!(end >= start, "support starts at the window");
        prop_assert!(end <= start + window + 1e-3);
        prop_assert!(padded >= 0.0);
        prop_assert!(((end - start) + padded - window).abs() < 1e-3,
            "supported {} + padded {padded} = window {window}", end - start);
        if audio_end >= start {
            prop_assert!(end <= audio_end + 1e-3, "never past the recording");
        }
        if audio_end >= start + window {
            prop_assert!(padded.abs() < 1e-6, "a covered window has no padding");
        }
    }

    /// I2.
    #[test]
    fn shared_support_is_symmetric_and_bounded(
        a in (0.0_f32..30.0, 0.0_f32..10.0),
        b in (0.0_f32..30.0, 0.0_f32..10.0),
    ) {
        let x = answer(a.0, a.0 + a.1, 0.0);
        let y = answer(b.0, b.0 + b.1, 0.0);
        let s = shared_support_secs(&x, &y);
        prop_assert!((s - shared_support_secs(&y, &x)).abs() < f32::EPSILON);
        prop_assert!(s >= 0.0);
        prop_assert!(s <= a.1.min(b.1) + 1e-4);
    }

    /// I3.
    #[test]
    fn the_merge_keeps_one_real_score_per_species(raw in verdict_strategy()) {
        let verdicts = verdicts_from(&raw);
        let merged = merge_verdicts(&verdicts);
        let mut reported: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for v in &verdicts {
            for d in &v.detections {
                reported.entry(d.scientific_name.as_str()).or_default().insert(&v.model_id);
            }
        }
        let names: Vec<&str> = merged.iter().map(|d| d.scientific_name.as_str()).collect();
        let unique: BTreeSet<&str> = names.iter().copied().collect();
        prop_assert_eq!(names.len(), unique.len(), "one detection per species");
        prop_assert_eq!(unique, reported.keys().copied().collect::<BTreeSet<_>>());
        for d in &merged {
            let given: Vec<f32> = verdicts
                .iter()
                .flat_map(|v| &v.detections)
                .filter(|x| x.scientific_name == d.scientific_name)
                .map(|x| x.confidence)
                .collect();
            prop_assert!(given.iter().any(|c| c.to_bits() == d.confidence.to_bits()),
                "{} carries {} which no classifier gave ({given:?})",
                d.scientific_name, d.confidence);
            let agreeing = usize::from(d.agreeing_models.unwrap_or(0));
            prop_assert!(agreeing >= 1);
            prop_assert!(agreeing <= reported[d.scientific_name.as_str()].len());
            let winner = d.model_id.as_deref().unwrap_or("");
            prop_assert!(reported[d.scientific_name.as_str()].contains(winner),
                "the named classifier reported the species");
        }
    }

    /// I4.
    #[test]
    fn a_species_some_classifier_cleared_is_carried_by_one_that_did(raw in verdict_strategy()) {
        let verdicts = verdicts_from(&raw);
        let merged = merge_verdicts(&verdicts);
        for d in &merged {
            let cleared: Vec<&ModelVerdict> = verdicts
                .iter()
                .filter(|v| v.detections.iter().any(|x| {
                    let bar = v.threshold;
                    x.scientific_name == d.scientific_name && x.confidence >= bar
                }))
                .collect();
            if cleared.is_empty() {
                continue;
            }
            let winner = verdicts
                .iter()
                .find(|v| Some(v.model_id.as_str()) == d.model_id.as_deref())
                .expect("the winner is a classifier");
            prop_assert!(d.confidence >= winner.threshold,
                "{} won with {} under its own bar {}", winner.model_id, d.confidence, winner.threshold);
            prop_assert_eq!(usize::from(d.agreeing_models.unwrap_or(0)), cleared.len(),
                "agreement counts the classifiers that cleared their own bar");
        }
    }

    /// I5.
    #[test]
    fn the_merge_does_not_depend_on_the_order_classifiers_answered(
        raw in verdict_strategy(),
        rotate in 0_usize..4,
    ) {
        let verdicts = verdicts_from(&raw);
        let mut reordered = verdicts.clone();
        let k = rotate % reordered.len();
        reordered.rotate_left(k);
        reordered.reverse();
        let key = |v: Vec<Detection>| -> Vec<(String, u32, Option<String>, Option<u8>)> {
            v.into_iter()
                .map(|d| (d.scientific_name, d.confidence.to_bits(), d.model_id, d.agreeing_models))
                .collect()
        };
        prop_assert_eq!(key(merge_verdicts(&verdicts)), key(merge_verdicts(&reordered)));
    }

    /// I6.
    #[test]
    fn the_privacy_mask_is_each_classifier_s_own_rule_and_the_neighbours(
        spec in chunk_strategy(),
        station in (0.0_f32..1.0, 0_usize..20),
        own in (proptest::option::of(0.0_f32..1.0), proptest::option::of(0_usize..20)),
    ) {
        let filter = PrivacyFilter::new(station.0)
            .with_rank_cutoff(station.1)
            .with_classifier_rule("v3", ClassifierPrivacyRule { threshold: own.0, rank_cutoff: own.1 });
        let chunks = chunks_from(&spec);
        prop_assert_eq!(filter.masked_windows(&chunks), reference_mask(&filter, &chunks));
        // What is masked is what the filter empties.
        let kept = filter.filter_predictions(&chunks);
        for (masked, k) in filter.masked_windows(&chunks).iter().zip(&kept) {
            prop_assert_eq!(*masked, k.is_empty());
        }
    }

    /// I7.
    #[test]
    fn more_evidence_never_unmasks_and_an_absent_classifier_s_rule_is_inert(
        spec in chunk_strategy(),
        extra in (0_usize..12, 0_usize..2, 0.0_f32..1.0, proptest::option::of(0_usize..30)),
        station in (0.0_f32..1.0, 0_usize..20),
        absent in (0.0_f32..1.0, 0_usize..30),
    ) {
        let filter = PrivacyFilter::new(station.0).with_rank_cutoff(station.1);
        let chunks = chunks_from(&spec);
        let before = filter.masked_windows(&chunks);
        if !chunks.is_empty() {
            let mut more = spec;
            let i = extra.0 % more.len();
            // Adding evidence to a chunk that had some keeps the reading
            // per-classifier, so the comparison is like for like.
            if !more[i].0.is_empty() {
                more[i].0.push((extra.1, extra.2, extra.3));
                let after = filter.masked_windows(&chunks_from(&more));
                for (b, a) in before.iter().zip(&after) {
                    prop_assert!(!*b || *a, "a window was unmasked by more evidence");
                }
            }
        }
        // "bat" never answers in these chunks: its rule cannot move the mask.
        let with_rule = filter.clone().with_classifier_rule(
            "bat",
            ClassifierPrivacyRule { threshold: Some(absent.0), rank_cutoff: Some(absent.1) },
        );
        if filter.is_enabled() {
            prop_assert_eq!(with_rule.masked_windows(&chunks), before);
        }
    }

    /// I8.
    #[test]
    fn a_chunk_is_dropped_exactly_when_a_watched_class_reaches_its_own_classifier_s_bar(
        chunks in proptest::collection::vec(
            proptest::collection::vec((any::<bool>(), 0.0_f32..1.0, 0_usize..2), 0..4),
            0..8,
        ),
        station in 0.0_f32..1.0,
        own in 0.0_f32..1.0,
    ) {
        let filter = NoiseFilter::new(station, vec!["Dog".to_owned()])
            .with_classifier_threshold("v3", own);
        let predictions: Vec<Vec<Detection>> = chunks
            .iter()
            .map(|c| c.iter().map(|(dog, conf, m)| {
                det(if *dog { "Dog" } else { "Pica pica" }, *conf, Some(MODELS[*m]))
            }).collect())
            .collect();
        let out = filter.filter_with_classifier_evidence(&[], &predictions, &[]);
        for (chunk, kept) in predictions.iter().zip(&out) {
            let fires = chunk.iter().any(|d| {
                // Written out, not read from the filter: v3 has its own bar.
                let bar = if d.model_id.as_deref() == Some("v3") { own } else { station };
                d.common_name == "Dog" && bar > 0.0 && d.confidence >= bar
            });
            if fires {
                prop_assert!(kept.is_empty());
            } else {
                prop_assert_eq!(kept.len(), chunk.len());
            }
        }
    }

    /// I12.
    #[test]
    fn the_two_names_of_a_pair_merge_and_no_others_do(
        a_conf in 0.0_f32..1.0,
        b_conf in 0.0_f32..1.0,
        paired in any::<bool>(),
    ) {
        let table = SynonymTable::parse("Old genus\tNew genus\n");
        let other = if paired { "New genus" } else { "Pica pica" };
        let verdicts = [
            ModelVerdict { model_id: "a".into(), threshold: 0.0, detections: vec![det("Old genus", a_conf, None)] },
            ModelVerdict { model_id: "b".into(), threshold: 0.0, detections: vec![det(other, b_conf, None)] },
        ];
        let merged = merge_verdicts_with(&verdicts, &table);
        prop_assert_eq!(merged.len(), if paired { 1 } else { 2 });
        if paired {
            prop_assert_eq!(merged[0].agreeing_models, Some(2));
        }
    }
}

/// I9, end to end through two ONNX sessions: on a station running two
/// classifiers, a source routed to one of them is judged by that one alone,
/// and the other neither answers nor is recorded as listening.
#[test]
fn a_source_routed_to_one_classifier_hears_only_that_one() {
    use birdnet_core::detection::ChunkFilters;
    use birdnet_core::detection::corroboration::ConfirmationLevel;
    use birdnet_core::detection::daemon::process_and_infer_filtered;
    use birdnet_core::detection::pipeline::PipelineConfig;
    use birdnet_core::inference::model::ModelConfig;
    use birdnet_core::inference::registry::{ClassifierRegistry, ModelSpec};
    use birdnet_core::inference::species_filter::{SpeciesFilter, SpeciesFilterConfig};

    const TINY_V24: &[u8] = include_bytes!("../src/testdata/tiny_v24_test.onnx");
    const TINY_V30: &[u8] = include_bytes!("../src/testdata/tiny_v30_test.onnx");
    let tmp = tempfile::tempdir().unwrap();
    let labels = tmp.path().join("labels.txt");
    std::fs::write(
        &labels,
        (0..11)
            .map(|i| format!("Species{i} bird_Bird {i}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let spec = |id: &str, bytes: &[u8]| {
        let path = tmp.path().join(format!("{id}.onnx"));
        std::fs::write(&path, bytes).unwrap();
        ModelSpec {
            id: id.to_owned(),
            model_path: path,
            labels_path: labels.clone(),
            threshold: None,
            sample_rate: Some(48_000),
            output_scale: None,
        }
    };
    let routes = std::collections::HashMap::from([
        ("camera".to_owned(), vec!["v3".to_owned()]),
        ("mic".to_owned(), vec!["birdnet".to_owned()]),
    ]);
    let mut registry = ClassifierRegistry::load(
        &[spec("birdnet", TINY_V24), spec("v3", TINY_V30)],
        &routes,
        &ModelConfig {
            confidence_threshold: 0.0,
            ..ModelConfig::default()
        },
    )
    .expect("both load");
    assert_eq!(registry.route_for(Some("camera")), vec![1]);
    assert_eq!(registry.route_for(Some("mic")), vec![0]);
    assert_eq!(registry.route_for(Some("unnamed")), vec![0], "the primary");

    // Three seconds of low noise at 48 kHz.
    let wav = tmp.path().join("2026-05-19-birdnet-09:00:00.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&wav, spec).unwrap();
    let mut x: u32 = 7;
    for _ in 0..48_000 * 3 {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        w.write_sample(((x >> 16) as i16) / 8).unwrap();
    }
    w.finalize().unwrap();

    let filters = ChunkFilters {
        privacy: PrivacyFilter::new(0.0),
        noise: NoiseFilter::new(0.0, Vec::new()),
        confirmation: ConfirmationLevel::Off,
    };
    let config = PipelineConfig {
        raw_audio_input: true,
        target_sample_rate: 48_000,
        chunk_duration_secs: 3.0,
        ..PipelineConfig::default()
    };
    let mut species = SpeciesFilter::new_passthrough(SpeciesFilterConfig::default());
    let mut run = |route: &[usize]| {
        process_and_infer_filtered(
            &wav,
            &config,
            &mut registry,
            route,
            &filters,
            &mut species,
            None,
            None,
            None,
            "",
        )
        .expect("the file is analysed")
        .events
    };
    for (route, only) in [(vec![1_usize], "v3"), (vec![0], "birdnet")] {
        let events = run(&route);
        assert!(
            !events.is_empty(),
            "threshold 0: {only} reports its species"
        );
        for e in &events {
            assert_eq!(
                e.detection.model_id.as_deref(),
                Some(only),
                "{:?}",
                e.detection
            );
            assert!(
                e.detection.model_scores.iter().all(|a| a.model_id == only),
                "only the routed classifier is recorded as listening: {:?}",
                e.detection.model_scores
            );
        }
    }
    // The counterpart: routed to both, both are heard, so the isolation above
    // is the route's doing and not one classifier's silence.
    let both = run(&[0, 1]);
    let ids: BTreeSet<&str> = both
        .iter()
        .flat_map(|e| e.detection.model_scores.iter().map(|a| a.model_id.as_str()))
        .collect();
    assert_eq!(ids, BTreeSet::from(["birdnet", "v3"]));
}
