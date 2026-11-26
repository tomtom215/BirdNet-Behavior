//! What the station's models are doing that its owner did not set it up for:
//! the one decision every health surface reads (MDL-03, MDL-06).
//!
//! # Why this exists
//!
//! On 2026-11-25 a real station was broken one model file at a time. A
//! detector that did not start left the header badge green, the Health page
//! "All systems healthy", the Today page silent and `/api/v2/health` at 200; a
//! second classifier that was skipped was one INFO line in the journal; an
//! occurrence filter with no coordinates was "on". Since then the start-up
//! publishes [`ModelsState`], one state each for the detector, every
//! configured classifier and the occurrence filter. This turns that into the
//! conditions a person must hear about, each with the words that say what to
//! do, and nothing else decides it: the header badge and the Station Health
//! banner (through `routes::pages::station_issues`), the Today page's notices,
//! `/api/v2/health` and the station-health notifier all call [`conditions`].
//!
//! # What is a condition
//!
//! Every state that is not what the configuration asks for:
//!
//! * the detector **failed**, **stopped**, or is **not configured** — nothing
//!   is being identified;
//! * a configured classifier that is **left out** (its file missing or
//!   damaged, its labels missing, unreadable or the wrong length, too big for
//!   the memory the station allows), or loaded and **unrouted**, judging
//!   nothing;
//! * the occurrence filter **off** for a reason other than "none is
//!   configured": no coordinates, its model or labels missing or unusable.
//!
//! Three states are deliberately not conditions:
//!
//! * the detector **starting** (the first seconds of a start) and **web
//!   only** (`--web-only`, chosen; the Capture panel says it);
//! * a classifier left out, or a filter off, **because the detector is not
//!   running**. That is the detector's condition, said once, with the cause to
//!   act on; repeating it per classifier would bury that cause under its
//!   consequences;
//! * a filter that is **not configured**: a station may run without one.

use birdnet_core::inference::model_state::{
    ClassifierFault, ClassifierStatus, DetectorState, FilterOff, FilterState, ModelsState,
};
use serde::Serialize;

/// The episode key of the detector's condition.
pub const DETECTOR_KEY: &str = "models:detector";

/// The episode key of the occurrence filter's condition.
pub const FILTER_KEY: &str = "models:occurrence-filter";

/// The prefix every key [`conditions`] returns starts with, so a check that
/// could not look can hold the whole family.
pub const KEY_PREFIX: &str = "models:";

/// The episode key of one classifier's condition.
#[must_use]
pub fn classifier_key(id: &str) -> String {
    format!("{KEY_PREFIX}classifier:{id}")
}

/// One thing the station's models are doing that the owner did not ask for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelCondition {
    /// Stable identity: [`DETECTOR_KEY`], [`classifier_key`] or
    /// [`FILTER_KEY`].
    pub key: String,
    /// The state's own code ([`DetectorState::code`] and its siblings), for a
    /// monitor to key on.
    pub state: &'static str,
    /// Two or three words, for the header badge.
    pub label: &'static str,
    /// A short title, for a notification.
    pub title: String,
    /// What is wrong and what to do about it, as a clause that starts in
    /// lower case and reads after "Needs attention:".
    pub sentence: String,
    /// Whether nothing is being identified: the detector is not running.
    pub stops_detection: bool,
}

impl ModelCondition {
    /// The sentence as text that stands on its own: capitalised, with its
    /// full stop. Not escaped.
    #[must_use]
    pub fn body(&self) -> String {
        format!("{}.", capitalize_first(&self.sentence))
    }
}

/// Upper-case the first character.
#[must_use]
pub fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Every condition in `models`, the detector's first, then the classifiers in
/// the order the configuration declares them, then the filter.
#[must_use]
pub fn conditions(models: &ModelsState) -> Vec<ModelCondition> {
    let mut out: Vec<ModelCondition> = detector_condition(&models.detector).into_iter().collect();
    out.extend(models.classifiers.iter().filter_map(|c| {
        let (label, title) = match &c.status {
            ClassifierStatus::Judging { .. }
            | ClassifierStatus::LeftOut {
                fault: ClassifierFault::DetectorNotRunning,
                ..
            } => return None,
            // The primary never runs without the detector: its fault is the
            // detector's condition, which names it.
            ClassifierStatus::LeftOut { .. } if c.primary => return None,
            ClassifierStatus::LeftOut { .. } => (
                "Classifier left out",
                format!("Classifier \"{}\" is not running", c.id),
            ),
            ClassifierStatus::Unrouted => (
                "Classifier unused",
                format!("Classifier \"{}\" judges nothing", c.id),
            ),
        };
        let effect = if matches!(c.status, ClassifierStatus::LeftOut { .. }) {
            "; the other classifiers go on detecting without it"
        } else {
            ""
        };
        Some(ModelCondition {
            key: classifier_key(&c.id),
            state: c.status.code(),
            label,
            title,
            sentence: format!("{}{effect}", c.message()),
            stops_detection: false,
        })
    }));
    out.extend(filter_condition(&models.occurrence_filter));
    out
}

/// The detector's condition, when it has one.
fn detector_condition(detector: &DetectorState) -> Option<ModelCondition> {
    let (label, title) = match detector {
        DetectorState::Starting | DetectorState::Running | DetectorState::WebOnly { .. } => {
            return None;
        }
        DetectorState::NotConfigured { .. } | DetectorState::Failed { .. } => {
            ("Not detecting", "The detector is not running")
        }
        DetectorState::Stopped => ("Detector stopped", "The detector has stopped"),
    };
    Some(ModelCondition {
        key: DETECTOR_KEY.to_owned(),
        state: detector.code(),
        label,
        title: title.to_owned(),
        sentence: format!(
            "nothing is being identified, because {}",
            detector.message()
        ),
        stops_detection: true,
    })
}

/// The occurrence filter's condition, when it has one.
fn filter_condition(filter: &FilterState) -> Option<ModelCondition> {
    match filter {
        FilterState::Active { .. }
        | FilterState::Off {
            reason: FilterOff::NotConfigured | FilterOff::DetectorNotRunning,
            ..
        } => None,
        FilterState::Off { .. } => Some(ModelCondition {
            key: FILTER_KEY.to_owned(),
            state: filter.code(),
            label: "Species filter off",
            title: "The species occurrence filter is off".to_owned(),
            sentence: filter.message(),
            stops_detection: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use birdnet_core::inference::model_state::ClassifierState;

    use super::*;

    fn classifier(id: &str, primary: bool, status: ClassifierStatus) -> ClassifierState {
        ClassifierState {
            id: id.to_owned(),
            primary,
            model_path: PathBuf::from(format!("/m/{id}.onnx")),
            labels_path: Some(PathBuf::from(format!("/m/{id}.csv"))),
            status,
        }
    }

    fn judging() -> ClassifierStatus {
        ClassifierStatus::Judging {
            sources: Vec::new(),
            unrouted_sources: true,
        }
    }

    /// The station its owner set up: the detector running, every classifier
    /// judging, the filter filtering.
    fn healthy() -> ModelsState {
        ModelsState {
            detector: DetectorState::Running,
            classifiers: vec![
                classifier("birdnet", true, judging()),
                classifier(
                    "perch",
                    false,
                    ClassifierStatus::Judging {
                        sources: vec!["RTSP_1".to_owned()],
                        unrouted_sources: false,
                    },
                ),
            ],
            occurrence_filter: FilterState::Active {
                candidates: Some(140),
            },
        }
    }

    /// The counterpart of every test below: a station doing what it was set
    /// up for has no condition, and nor do the states that are not faults.
    #[test]
    fn a_station_doing_what_it_was_set_up_for_has_no_condition() {
        assert_eq!(conditions(&healthy()), []);
        for detector in [
            DetectorState::Starting,
            DetectorState::WebOnly {
                why: "--web-only".to_owned(),
            },
        ] {
            let state = ModelsState {
                detector,
                ..healthy()
            };
            assert_eq!(conditions(&state), [], "{:?}", state.detector);
        }
        let unfiltered = ModelsState {
            occurrence_filter: FilterState::off(FilterOff::NotConfigured, "none configured"),
            ..healthy()
        };
        assert_eq!(conditions(&unfiltered), []);
    }

    /// Each way the detector can be not running is one condition, which says
    /// nothing is identified and carries the reason.
    #[test]
    fn a_detector_that_is_not_running_is_one_condition_with_its_reason() {
        for (detector, label, reason) in [
            (
                DetectorState::Failed {
                    why: "its labels file names 12".to_owned(),
                },
                "Not detecting",
                "its labels file names 12",
            ),
            (
                DetectorState::NotConfigured {
                    why: "no MODEL_PATH".to_owned(),
                },
                "Not detecting",
                "no MODEL_PATH",
            ),
            (
                DetectorState::Stopped,
                "Detector stopped",
                "restart the station",
            ),
        ] {
            let code = detector.code();
            let mut state = healthy();
            state.set_detector(detector);
            let found = conditions(&state);
            assert_eq!(found.len(), 1, "{found:?}");
            let c = &found[0];
            assert_eq!(
                (c.key.as_str(), c.state, c.label, c.stops_detection),
                (DETECTOR_KEY, code, label, true)
            );
            assert!(
                c.sentence.starts_with("nothing is being identified")
                    && c.sentence.contains(reason),
                "{}",
                c.sentence
            );
        }
    }

    /// A second classifier left out is its own condition, naming it, its
    /// reason and that the others go on; a primary left out is the
    /// detector's.
    #[test]
    fn a_left_out_second_classifier_is_named_with_its_reason() {
        let mut state = healthy();
        state.classifiers[1].status = ClassifierStatus::left_out(
            ClassifierFault::WillNotLoad,
            "its model file /m/perch.onnx will not load; install it again",
        );
        let found = conditions(&state);
        assert_eq!(found.len(), 1, "{found:?}");
        let c = &found[0];
        assert_eq!(
            (c.key.as_str(), c.state, c.label),
            (
                "models:classifier:perch",
                "left_out_will_not_load",
                "Classifier left out"
            )
        );
        assert!(
            c.sentence.contains("`perch` is not running")
                && c.sentence.contains("install it again")
                && c.sentence.contains("other classifiers go on detecting"),
            "{}",
            c.sentence
        );
        assert!(!c.stops_detection);
    }

    /// Every fault a classifier can be left out for is a condition, except
    /// the detector's own.
    #[test]
    fn every_left_out_fault_but_the_detectors_is_a_condition() {
        for code in ClassifierStatus::CODES {
            let Some(fault) = code.strip_prefix("left_out_") else {
                continue;
            };
            let fault = [
                ClassifierFault::FileMissing,
                ClassifierFault::WillNotLoad,
                ClassifierFault::DoesNotFit,
                ClassifierFault::MemoryUnknown,
                ClassifierFault::LabelsNotSet,
                ClassifierFault::LabelsMissing,
                ClassifierFault::LabelsUnreadable,
                ClassifierFault::LabelsWrongLength,
                ClassifierFault::DetectorNotRunning,
            ]
            .into_iter()
            .find(|f| f.code() == fault)
            .expect("every code has its fault");
            let mut state = healthy();
            state.classifiers[1].status = ClassifierStatus::left_out(fault, "why");
            let found = conditions(&state);
            assert_eq!(
                found.len(),
                usize::from(fault != ClassifierFault::DetectorNotRunning),
                "{code}: {found:?}"
            );
        }
    }

    /// A loaded classifier no route reaches is a condition, with the route
    /// that would reach it (MDL-06, MDL-07).
    #[test]
    fn an_unrouted_classifier_is_a_condition_with_the_route_to_add() {
        let mut state = healthy();
        state.classifiers[1].status = ClassifierStatus::Unrouted;
        let found = conditions(&state);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(
            (found[0].state, found[0].label),
            ("unrouted", "Classifier unused")
        );
        assert!(
            found[0].sentence.contains("local:<primary>+perch"),
            "{}",
            found[0].sentence
        );
    }

    /// Each reason a configured filter is off is a condition carrying it.
    #[test]
    fn a_configured_filter_that_is_off_is_a_condition_with_its_reason() {
        for reason in [
            FilterOff::NoCoordinates,
            FilterOff::ModelMissing,
            FilterOff::ModelUnusable,
            FilterOff::LabelsMissing,
            FilterOff::LabelsUnreadable,
        ] {
            let state = ModelsState {
                occurrence_filter: FilterState::off(reason, "set LATITUDE and LONGITUDE"),
                ..healthy()
            };
            let found = conditions(&state);
            assert_eq!(found.len(), 1, "{reason:?}: {found:?}");
            assert_eq!(found[0].key, FILTER_KEY);
            assert_eq!(found[0].state, state.occurrence_filter.code());
            assert!(
                found[0].sentence.contains("set LATITUDE and LONGITUDE"),
                "{}",
                found[0].sentence
            );
        }
    }

    /// A detector that is not running takes its classifiers and filter with
    /// it; the owner hears the cause once, and a second classifier's own,
    /// different fault as well.
    #[test]
    fn a_stopped_detector_is_said_once_and_a_second_fault_beside_it() {
        let mut state = healthy();
        state.classifiers[1].status =
            ClassifierStatus::left_out(ClassifierFault::LabelsMissing, "no labels");
        state.set_detector(DetectorState::Failed {
            why: "the primary will not load".to_owned(),
        });
        let keys: Vec<String> = conditions(&state).into_iter().map(|c| c.key).collect();
        assert_eq!(keys, [DETECTOR_KEY, "models:classifier:perch"]);
    }

    /// Every key belongs to the family a check that could not look holds.
    #[test]
    fn every_key_is_in_the_models_family() {
        let mut state = healthy();
        state.classifiers[1].status = ClassifierStatus::Unrouted;
        state.occurrence_filter = FilterState::off(FilterOff::NoCoordinates, "x");
        state.set_detector(DetectorState::Stopped);
        state.classifiers[1].status = ClassifierStatus::Unrouted;
        state.occurrence_filter = FilterState::off(FilterOff::NoCoordinates, "x");
        let found = conditions(&state);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.iter().all(|c| c.key.starts_with(KEY_PREFIX)));
    }
}
