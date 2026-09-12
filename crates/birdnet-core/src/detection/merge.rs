//! Combining what several classifiers said about one chunk (`G-10` Stage 3).
//!
//! Stage 2 made a station able to run more than one classifier. This decides
//! what it means when they disagree, which is most of the time: two models
//! trained on different data, with different label sets and different
//! calibration, will not produce the same list.
//!
//! # The policy: union, with each model judged against its own threshold
//!
//! A species is reported if **any** routed classifier reported it above *that
//! classifier's* threshold. Not intersection: a bat classifier and BirdNET
//! share almost no labels, so requiring both to agree would report nothing at
//! all. Not one global threshold either — two models are not calibrated alike,
//! and forcing one number on both means the stricter model is effectively off
//! or the looser one floods the log.
//!
//! # Agreement is counted, never folded into the confidence
//!
//! When two classifiers independently report the same species in the same
//! chunk, that is much stronger evidence than one being confident — it is the
//! honest attack on the false-positive problem that `corroboration.rs` can
//! only approach through repetition. So it is recorded, as a count, on the
//! detection.
//!
//! It is **not** used to adjust the confidence, and the temptation to is worth
//! naming. A combined confidence — an average, a maximum with a bonus, a
//! noisy-or — would be a number with no calibration behind it, presented on
//! the same scale and in the same column as a model's actual output. Every
//! threshold an operator has set, every historical comparison, and every
//! export would silently change meaning. The reported confidence stays the
//! winning classifier's real output; the corroboration is a separate fact in a
//! separate field, and a reader can weigh it themselves.
//!
//! # What a single-classifier station gets
//!
//! Every station today runs one classifier, and for those this is a pass-
//! through: the same detections, in the same order, with `model_id` set to the
//! one classifier and `agreeing_models` set to one. Provenance worth having
//! even then — a station whose model was swapped last March can tell which of
//! its detections came from which.

use std::collections::HashMap;

use crate::detection::types::Detection;

/// What one classifier said about one chunk.
#[derive(Debug, Clone)]
pub struct ModelVerdict {
    /// The classifier's id, as configured.
    pub model_id: String,
    /// What it reported, already filtered by its own threshold.
    pub detections: Vec<Detection>,
}

/// Combine the verdicts of every classifier that judged one chunk.
///
/// Returns one detection per species, carrying the highest confidence any
/// classifier gave it, the id of the classifier that gave that confidence, and
/// how many classifiers reported the species at all.
///
/// The output is sorted by confidence descending, then by scientific name, so
/// it is deterministic — two classifiers finishing in a different order must
/// not reorder a station's detections.
#[must_use]
pub fn merge_verdicts(verdicts: &[ModelVerdict]) -> Vec<Detection> {
    // Species → (best detection so far, which model gave it, how many models
    // reported this species).
    let mut best: HashMap<String, (Detection, String, u8)> = HashMap::new();

    for verdict in verdicts {
        // One classifier reporting the same species twice in a chunk — a
        // label file with a duplicate row, say — must not count as agreement
        // with itself. Agreement means *independent* classifiers.
        let mut seen_here: Vec<&str> = Vec::new();

        for detection in &verdict.detections {
            let key = detection.scientific_name.clone();
            let first_from_this_model = !seen_here.contains(&detection.scientific_name.as_str());
            if first_from_this_model {
                seen_here.push(&detection.scientific_name);
            }

            match best.get_mut(&key) {
                None => {
                    best.insert(key, (detection.clone(), verdict.model_id.clone(), 1));
                }
                Some((existing, winner, count)) => {
                    if first_from_this_model {
                        *count = count.saturating_add(1);
                    }
                    if detection.confidence > existing.confidence {
                        *existing = detection.clone();
                        winner.clone_from(&verdict.model_id);
                    }
                }
            }
        }
    }

    let mut merged: Vec<Detection> = best
        .into_values()
        .map(|(mut detection, model_id, count)| {
            detection.model_id = Some(model_id);
            detection.agreeing_models = Some(count);
            detection
        })
        .collect();

    // Deterministic: confidence first, then name to break ties. Without the
    // second key a HashMap's iteration order would leak into the output and
    // two equally-confident species could swap places between runs.
    merged.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.scientific_name.cmp(&b.scientific_name))
    });
    merged
}

#[cfg(test)]
mod tests {
    use super::{ModelVerdict, merge_verdicts};
    use crate::detection::types::Detection;

    fn det(sci: &str, confidence: f32) -> Detection {
        Detection {
            date: "2026-09-12".to_owned(),
            time: "06:00:00".to_owned(),
            scientific_name: sci.to_owned(),
            common_name: format!("Common {sci}"),
            confidence,
            start: 0.0,
            stop: 3.0,
            week: 37,
            file_name_extr: None,
            model_id: None,
            agreeing_models: None,
        }
    }

    fn verdict(id: &str, dets: Vec<Detection>) -> ModelVerdict {
        ModelVerdict {
            model_id: id.to_owned(),
            detections: dets,
        }
    }

    /// **Every station today runs one classifier**, and for those this must be
    /// a pass-through: same species, same confidences, plus provenance.
    ///
    /// Observed failing with the `agreeing_models` assignment removed: the
    /// field stayed `None`, so a single-model station could not be told apart
    /// from a row written before Stage 3 existed.
    #[test]
    fn one_classifier_passes_through_with_its_provenance() {
        let out = merge_verdicts(&[verdict(
            "birdnet",
            vec![det("Turdus merula", 0.9), det("Pica pica", 0.7)],
        )]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].scientific_name, "Turdus merula");
        assert!((out[0].confidence - 0.9).abs() < 1e-6);
        for d in &out {
            assert_eq!(d.model_id.as_deref(), Some("birdnet"));
            assert_eq!(d.agreeing_models, Some(1), "one model asked, one answered");
        }
    }

    /// **The point of the whole stage.** Two classifiers reporting the same
    /// species in the same chunk is independent corroboration, and it is
    /// counted.
    ///
    /// Observed failing with the `*count` increment removed: agreement stayed
    /// at 1 and the corroboration was indistinguishable from one model
    /// speaking alone.
    #[test]
    fn two_classifiers_agreeing_on_a_species_is_recorded_as_agreement() {
        let out = merge_verdicts(&[
            verdict("birdnet", vec![det("Turdus merula", 0.8)]),
            verdict("perch", vec![det("Turdus merula", 0.6)]),
        ]);
        assert_eq!(out.len(), 1, "one species, one row");
        assert_eq!(out[0].agreeing_models, Some(2));
    }

    /// **Confidence is never invented.** The reported number is the winning
    /// classifier's actual output — not an average, not a maximum with a
    /// bonus. A combined number would have no calibration behind it while
    /// sitting in the same column as a real one.
    ///
    /// Observed failing with the merge averaging the two confidences: 0.8 and
    /// 0.6 became 0.7, a number neither model ever produced.
    #[test]
    fn agreement_does_not_change_the_confidence() {
        let out = merge_verdicts(&[
            verdict("birdnet", vec![det("Turdus merula", 0.8)]),
            verdict("perch", vec![det("Turdus merula", 0.6)]),
        ]);
        assert!(
            (out[0].confidence - 0.8).abs() < 1e-6,
            "expected the winning model's own 0.8, got {}",
            out[0].confidence
        );
        assert_eq!(
            out[0].model_id.as_deref(),
            Some("birdnet"),
            "the id must name the classifier whose confidence is reported"
        );
    }

    /// A species only one classifier heard is still reported — union, not
    /// intersection. A bat classifier and BirdNET share almost no labels, so
    /// requiring agreement would report nothing at all.
    ///
    /// Observed failing with the merge keeping only species seen by every
    /// verdict: both single-model species vanished.
    #[test]
    fn a_species_only_one_classifier_heard_is_still_reported() {
        let out = merge_verdicts(&[
            verdict("birdnet", vec![det("Turdus merula", 0.9)]),
            verdict("bats", vec![det("Pipistrellus pipistrellus", 0.7)]),
        ]);
        assert_eq!(out.len(), 2, "union, not intersection: {out:?}");
        for d in &out {
            assert_eq!(d.agreeing_models, Some(1));
        }
        assert_eq!(
            out[0].scientific_name, "Turdus merula",
            "sorted by confidence"
        );
    }

    /// The lower-confidence model can still be the one that names the
    /// detection, if it is the confident one — the winner is by confidence,
    /// not by load order.
    #[test]
    fn the_most_confident_classifier_names_the_detection() {
        let out = merge_verdicts(&[
            verdict("birdnet", vec![det("Turdus merula", 0.4)]),
            verdict("perch", vec![det("Turdus merula", 0.95)]),
        ]);
        assert_eq!(out[0].model_id.as_deref(), Some("perch"));
        assert!((out[0].confidence - 0.95).abs() < 1e-6);
        assert_eq!(out[0].agreeing_models, Some(2));
    }

    /// One classifier reporting a species twice is not agreement with itself.
    /// A duplicated row in a label file must not manufacture corroboration.
    ///
    /// Observed failing with the `seen_here` guard removed: a single model
    /// listing the species twice reported an agreement of 2, which would read
    /// downstream as two independent models concurring.
    #[test]
    fn one_classifier_repeating_itself_is_not_corroboration() {
        let out = merge_verdicts(&[verdict(
            "birdnet",
            vec![det("Turdus merula", 0.8), det("Turdus merula", 0.6)],
        )]);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].agreeing_models,
            Some(1),
            "a model agreeing with itself is one opinion, not two"
        );
        assert!((out[0].confidence - 0.8).abs() < 1e-6);
    }

    /// Three classifiers, mixed agreement — the count is per species, not per
    /// chunk.
    #[test]
    fn agreement_is_counted_per_species() {
        let out = merge_verdicts(&[
            verdict("a", vec![det("Turdus merula", 0.8), det("Pica pica", 0.5)]),
            verdict("b", vec![det("Turdus merula", 0.7)]),
            verdict("c", vec![det("Turdus merula", 0.6), det("Pica pica", 0.4)]),
        ]);
        let merula = out
            .iter()
            .find(|d| d.scientific_name == "Turdus merula")
            .expect("m");
        let pica = out
            .iter()
            .find(|d| d.scientific_name == "Pica pica")
            .expect("p");
        assert_eq!(merula.agreeing_models, Some(3));
        assert_eq!(pica.agreeing_models, Some(2));
    }

    /// No classifier said anything: an empty chunk, not a panic.
    #[test]
    fn nothing_reported_merges_to_nothing() {
        assert!(merge_verdicts(&[]).is_empty());
        assert!(merge_verdicts(&[verdict("birdnet", vec![])]).is_empty());
    }

    /// The order out is deterministic, so two classifiers finishing in a
    /// different order cannot reorder a station's detections.
    #[test]
    fn equally_confident_species_come_out_in_a_stable_order() {
        let forward =
            merge_verdicts(&[verdict("a", vec![det("Bbb bbb", 0.5), det("Aaa aaa", 0.5)])]);
        let backward =
            merge_verdicts(&[verdict("a", vec![det("Aaa aaa", 0.5), det("Bbb bbb", 0.5)])]);
        let names = |v: &[Detection]| -> Vec<String> {
            v.iter().map(|d| d.scientific_name.clone()).collect()
        };
        assert_eq!(names(&forward), names(&backward));
        assert_eq!(names(&forward), vec!["Aaa aaa", "Bbb bbb"]);
    }
}
