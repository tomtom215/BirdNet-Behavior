//! What a station did with the model files it has: the one description of its
//! models after start-up (MDL-01 to MDL-16).
//!
//! # Why this exists
//!
//! On 2026-11-25 a real station was started from the built binary with the
//! real files, and broken one file at a time. One bad *optional* file stopped
//! all detection, and in most of the other cases the station did not say what
//! state its models were in: a geomodel with no coordinates was reported "on",
//! a second classifier that was skipped was announced by one INFO log line, and
//! a second classifier loaded beside the only microphone judged nothing and
//! said nothing. The facts existed — in a log line here, a metric there, the
//! configuration file — and no surface could put them together.
//!
//! This is them put together, as types: for the detector, each configured
//! classifier and the occurrence filter, exactly one state, and for every
//! state that is not the one the owner set the station up for, a sentence that
//! says what to do. The application fills it in as start-up proceeds
//! (`src/daemon`), holds it where the web crate reads it
//! (`MetricsRegistry::models_state`), and exports it as three state-set
//! gauges, one series per state with the one in force at 1:
//!
//! * `birdnet_detector_state{state}` — [`DetectorState::code`];
//! * `birdnet_classifier_state{classifier,state}` — [`ClassifierStatus::code`];
//! * `birdnet_occurrence_filter_state{state}` — [`FilterState::code`].
//!
//! The codes are stable identifiers, listed in each type's `CODES`, for alerts
//! and for the dashboard to key on; the `why` text is for a person and may be
//! reworded.
//!
//! # What it is not
//!
//! It is not the *health* of a running classifier (whether its windows are
//! failing) — that is [`crate::inference::health`] — and not the
//! configuration: a classifier the configuration names appears here whether or
//! not it loaded, and that difference is the point.

use std::path::PathBuf;

use serde::Serialize;

use crate::inference::registry::LOCAL_SOURCE;

/// Everything the station's models are doing, after start-up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelsState {
    /// Whether the detector is running, and why not when it is not.
    pub detector: DetectorState,
    /// Every classifier the configuration names, the primary first, in the
    /// order the configuration declares them.
    pub classifiers: Vec<ClassifierState>,
    /// The occurrence (species range) filter.
    pub occurrence_filter: FilterState,
}

impl Default for ModelsState {
    fn default() -> Self {
        Self {
            detector: DetectorState::Starting,
            classifiers: Vec::new(),
            occurrence_filter: FilterState::off(
                FilterOff::DetectorNotRunning,
                "the detector has not started yet",
            ),
        }
    }
}

impl ModelsState {
    /// Set the detector's state.
    ///
    /// A detector that is not running (and not still starting) runs no
    /// classifier and no filter, whatever they were loaded as: each loaded
    /// classifier becomes [`ClassifierFault::DetectorNotRunning`] and the
    /// filter [`FilterOff::DetectorNotRunning`], with the detector's own
    /// message as the reason. A classifier already left out keeps its own
    /// reason, which is the one to act on first.
    pub fn set_detector(&mut self, detector: DetectorState) {
        if !matches!(detector, DetectorState::Running | DetectorState::Starting) {
            let why = detector.message();
            for classifier in &mut self.classifiers {
                if !matches!(classifier.status, ClassifierStatus::LeftOut { .. }) {
                    classifier.status = ClassifierStatus::left_out(
                        ClassifierFault::DetectorNotRunning,
                        why.clone(),
                    );
                }
            }
            self.occurrence_filter = FilterState::off(FilterOff::DetectorNotRunning, why);
        }
        self.detector = detector;
    }
}

/// The detection daemon as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DetectorState {
    /// Start-up has not reached the detector yet.
    Starting,
    /// Running: recordings in the watch folder are being analysed.
    Running,
    /// Not started, by the station's own choice: `--web-only`, or a
    /// configuration with errors and no last good copy to fall back to.
    WebOnly {
        /// Which, in words.
        why: String,
    },
    /// Not started because no classifier, labels file or watch folder is
    /// configured.
    NotConfigured {
        /// What is missing and how to set it.
        why: String,
    },
    /// Asked to start and could not: the primary classifier would not load,
    /// a route names a classifier that does not exist, or the watch folder
    /// could not be watched.
    Failed {
        /// The fault, as the start-up found it.
        why: String,
    },
    /// It was running and its loop has stopped.
    Stopped,
}

impl DetectorState {
    /// Every [`Self::code`], for an exposition that lists each state.
    pub const CODES: &'static [&'static str] = &[
        "starting",
        "running",
        "web_only",
        "not_configured",
        "failed",
        "stopped",
    ];

    /// The stable identifier of this state.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::WebOnly { .. } => "web_only",
            Self::NotConfigured { .. } => "not_configured",
            Self::Failed { .. } => "failed",
            Self::Stopped => "stopped",
        }
    }

    /// Whether recordings are being analysed.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    /// What a person reads.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Starting => "the detector is starting".to_owned(),
            Self::Running => "the detector is running".to_owned(),
            Self::WebOnly { why } => format!("the detector is off: {why}"),
            Self::NotConfigured { why } => format!("the detector is not set up: {why}"),
            Self::Failed { why } => format!("the detector did not start: {why}"),
            Self::Stopped => {
                "the detector stopped; restart the station to resume detection".to_owned()
            }
        }
    }
}

/// One classifier the configuration names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassifierState {
    /// The operator's name for it (`MODEL_ID`, `MODEL_2_ID`, …).
    pub id: String,
    /// Whether it is the primary (`MODEL_PATH`), the one the station cannot
    /// run without.
    pub primary: bool,
    /// Its model file, as configured.
    pub model_path: PathBuf,
    /// Its labels file, as configured; `None` when none is.
    pub labels_path: Option<PathBuf>,
    /// What became of it.
    pub status: ClassifierStatus,
}

/// What became of one configured classifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClassifierStatus {
    /// Loaded, and judging recordings.
    Judging {
        /// The sources a `MODEL_ROUTES` entry sends to it, sorted.
        sources: Vec<String>,
        /// Whether it also judges every source no route names. True of the
        /// primary alone: an unrouted source gets the primary.
        unrouted_sources: bool,
    },
    /// Loaded, holding its memory, and named by no route, so it judges
    /// nothing. Not refused — an operator may load one before routing a
    /// source to it — but never to be mistaken for one that has heard
    /// nothing yet.
    Unrouted {
        /// The primary classifier's id, which the route that would reach
        /// this one starts with.
        primary: String,
    },
    /// Not running.
    LeftOut {
        /// Why, as a kind.
        fault: ClassifierFault,
        /// Why, for a person, naming the file and what to do.
        why: String,
    },
}

/// Why a configured classifier is not running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierFault {
    /// Its model file is not there.
    FileMissing,
    /// Its model file is there and ONNX Runtime will not load it (truncated,
    /// damaged, not a model, a shape the station cannot read).
    WillNotLoad,
    /// It would not fit in the memory the station allows its classifiers.
    DoesNotFit,
    /// The machine does not report its memory, so it cannot be shown to fit.
    MemoryUnknown,
    /// No labels file is configured for it.
    LabelsNotSet,
    /// Its labels file is not there.
    LabelsMissing,
    /// Its labels file is there and cannot be read as labels.
    LabelsUnreadable,
    /// Its labels file names a different number of classes from the number
    /// the model scores, so every species would carry another's name.
    LabelsWrongLength,
    /// It was not loaded because the detector did not start.
    DetectorNotRunning,
}

impl ClassifierFault {
    /// The stable identifier, without the `left_out_` prefix.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::FileMissing => "file_missing",
            Self::WillNotLoad => "will_not_load",
            Self::DoesNotFit => "does_not_fit",
            Self::MemoryUnknown => "memory_unknown",
            Self::LabelsNotSet => "labels_not_set",
            Self::LabelsMissing => "labels_missing",
            Self::LabelsUnreadable => "labels_unreadable",
            Self::LabelsWrongLength => "labels_wrong_length",
            Self::DetectorNotRunning => "detector_not_running",
        }
    }
}

impl ClassifierStatus {
    /// Every [`Self::code`], for an exposition that lists each state.
    pub const CODES: &'static [&'static str] = &[
        "judging",
        "unrouted",
        "left_out_file_missing",
        "left_out_will_not_load",
        "left_out_does_not_fit",
        "left_out_memory_unknown",
        "left_out_labels_not_set",
        "left_out_labels_missing",
        "left_out_labels_unreadable",
        "left_out_labels_wrong_length",
        "left_out_detector_not_running",
    ];

    /// The stable identifier of this state.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Judging { .. } => "judging",
            Self::Unrouted { .. } => "unrouted",
            Self::LeftOut { fault, .. } => match fault {
                ClassifierFault::FileMissing => "left_out_file_missing",
                ClassifierFault::WillNotLoad => "left_out_will_not_load",
                ClassifierFault::DoesNotFit => "left_out_does_not_fit",
                ClassifierFault::MemoryUnknown => "left_out_memory_unknown",
                ClassifierFault::LabelsNotSet => "left_out_labels_not_set",
                ClassifierFault::LabelsMissing => "left_out_labels_missing",
                ClassifierFault::LabelsUnreadable => "left_out_labels_unreadable",
                ClassifierFault::LabelsWrongLength => "left_out_labels_wrong_length",
                ClassifierFault::DetectorNotRunning => "left_out_detector_not_running",
            },
        }
    }

    /// A left-out classifier, with the reason a person reads.
    #[must_use]
    pub fn left_out(fault: ClassifierFault, why: impl Into<String>) -> Self {
        Self::LeftOut {
            fault,
            why: why.into(),
        }
    }
}

impl ClassifierState {
    /// What a person reads about this classifier.
    #[must_use]
    pub fn message(&self) -> String {
        match &self.status {
            ClassifierStatus::Judging {
                sources,
                unrouted_sources,
            } => match (sources.is_empty(), unrouted_sources) {
                (true, true) => format!("classifier `{}` is judging every source", self.id),
                (true, false) => format!("classifier `{}` is judging no source", self.id),
                (false, true) => format!(
                    "classifier `{}` is judging {} and every source no route names",
                    self.id,
                    sources.join(", ")
                ),
                (false, false) => {
                    format!("classifier `{}` is judging {}", self.id, sources.join(", "))
                }
            },
            ClassifierStatus::Unrouted { primary } => format!(
                "classifier `{id}` is loaded and judging nothing: no MODEL_ROUTES entry names \
                 it, and a source without a route gets the primary alone. Add it to a route \
                 (`{LOCAL_SOURCE}:{primary}+{id}` for a microphone set in the configuration \
                 file), or remove it to free its memory",
                id = self.id
            ),
            ClassifierStatus::LeftOut { why, .. } => {
                format!("classifier `{}` is not running: {why}", self.id)
            }
        }
    }
}

/// The occurrence (species range) filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FilterState {
    /// Loaded, the station has a position, and it is filtering.
    Active {
        /// How many species it admitted the last time it ran; `None` until
        /// it has.
        candidates: Option<u64>,
    },
    /// Not filtering: every species the classifiers know is a candidate,
    /// wherever the station is.
    Off {
        /// Why, as a kind.
        reason: FilterOff,
        /// Why, for a person, naming the file or setting and what to do.
        why: String,
    },
}

/// Why the occurrence filter is not filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOff {
    /// No occurrence model is configured.
    NotConfigured,
    /// The model loaded, and the station has no `LATITUDE`/`LONGITUDE`, so it
    /// cannot be asked what occurs here.
    NoCoordinates,
    /// The configured model file is not there.
    ModelMissing,
    /// The model file is there and could not be used (damaged, the wrong
    /// file for the network, or it failed on the last recording).
    ModelUnusable,
    /// The configured labels file is not there.
    LabelsMissing,
    /// The labels file is there and cannot be read as labels.
    LabelsUnreadable,
    /// The detector is not running, so neither is the filter.
    DetectorNotRunning,
}

impl FilterState {
    /// Every [`Self::code`], for an exposition that lists each state.
    pub const CODES: &'static [&'static str] = &[
        "active",
        "off_not_configured",
        "off_no_coordinates",
        "off_model_missing",
        "off_model_unusable",
        "off_labels_missing",
        "off_labels_unreadable",
        "off_detector_not_running",
    ];

    /// An `Off` state.
    #[must_use]
    pub fn off(reason: FilterOff, why: impl Into<String>) -> Self {
        Self::Off {
            reason,
            why: why.into(),
        }
    }

    /// The stable identifier of this state.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Active { .. } => "active",
            Self::Off { reason, .. } => match reason {
                FilterOff::NotConfigured => "off_not_configured",
                FilterOff::NoCoordinates => "off_no_coordinates",
                FilterOff::ModelMissing => "off_model_missing",
                FilterOff::ModelUnusable => "off_model_unusable",
                FilterOff::LabelsMissing => "off_labels_missing",
                FilterOff::LabelsUnreadable => "off_labels_unreadable",
                FilterOff::DetectorNotRunning => "off_detector_not_running",
            },
        }
    }

    /// Whether it is filtering.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    /// How many species it admitted the last time it ran, when it is
    /// filtering and has run.
    #[must_use]
    pub const fn candidates(&self) -> Option<u64> {
        match self {
            Self::Active { candidates } => *candidates,
            Self::Off { .. } => None,
        }
    }

    /// What a person reads.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Active { candidates: None } => {
                "the species occurrence filter is on and has not run yet".to_owned()
            }
            Self::Active {
                candidates: Some(n),
            } => format!("the species occurrence filter is on, admitting {n} species"),
            Self::Off { why, .. } => format!(
                "the species occurrence filter is off, so every species the classifiers know \
                 is a candidate wherever the station is: {why}"
            ),
        }
    }
}

/// One model file the configuration names and the model catalogue knows, and
/// what the station is doing to keep it the file this build pins (MDL-10).
///
/// Published beside [`ModelsState`], not inside it: that is what the detector
/// made of the files at its start, and this is the station looking after the
/// files while it runs. A file damaged or lost is fetched again in the
/// background; once it verifies, a station whose detector, classifier or
/// filter was left without it restarts to load it, and one that is already
/// using it does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelFile {
    /// The catalogue id of the model the file belongs to (`birdnet-v24`).
    pub catalogue_id: String,
    /// The file, as the configuration names it.
    pub path: PathBuf,
    /// What the station is doing about it.
    pub repair: RepairState,
}

/// What the station is doing about one model file ([`ModelFile`]). Times are
/// seconds since the Unix epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RepairState {
    /// Not looked at yet: the first check hashes the whole file.
    Checking,
    /// The bytes are the ones this build pins.
    Verified,
    /// Missing or damaged, and being fetched again now.
    Fetching {
        /// When this attempt started.
        since_unix: u64,
        /// Bytes of the file on the disk so far.
        done: u64,
        /// Bytes it has in all.
        total: u64,
    },
    /// Missing or damaged; the last attempt to fetch it failed, and the
    /// station tries again at `at_unix`.
    WillRetry {
        /// When the next attempt is.
        at_unix: u64,
        /// Attempts made so far, each one round of every origin.
        attempts: u32,
        /// Why the last one failed.
        why: String,
    },
    /// Missing or damaged, and this station has nowhere it may fetch it from
    /// (started `--offline` without `BIRDNET_MODEL_MIRROR`). It looks again
    /// at `looks_again_unix`, for a file put in place by hand.
    NotFetched {
        /// Why, and what to do.
        why: String,
        /// When it looks at the file again.
        looks_again_unix: u64,
    },
    /// It was missing or damaged, and now verifies.
    Repaired {
        /// When it verified.
        at_unix: u64,
        /// What it took to use it.
        restart: RepairRestart,
    },
}

/// What using a repaired file took ([`RepairState::Repaired`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairRestart {
    /// Nothing: whatever reads the file had it loaded already.
    NotNeeded,
    /// The station is restarting to load it.
    Restarting,
    /// It needs a restart, and nothing here can restart this process.
    NeededByHand,
}

impl RepairState {
    /// Every [`Self::code`], for an exposition that lists each state.
    pub const CODES: &'static [&'static str] = &[
        "checking",
        "verified",
        "fetching",
        "will_retry",
        "not_fetched",
        "repaired",
    ];

    /// The stable identifier of this state.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::Verified => "verified",
            Self::Fetching { .. } => "fetching",
            Self::WillRetry { .. } => "will_retry",
            Self::NotFetched { .. } => "not_fetched",
            Self::Repaired { .. } => "repaired",
        }
    }

    /// Whether the file is the one this build pins.
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self, Self::Verified | Self::Repaired { .. })
    }
}

impl ModelFile {
    /// What a person reads about this file. Times are given as the epoch
    /// seconds they are; a page renders them in the station's own clock.
    #[must_use]
    pub fn message(&self) -> String {
        let file = self.path.display();
        match &self.repair {
            RepairState::Checking => format!("{file} has not been checked yet"),
            RepairState::Verified => format!("{file} is the file this build pins"),
            RepairState::Fetching { done, total, .. } => format!(
                "{file} is missing or damaged, and the station is fetching it again \
                 ({done} of {total} bytes)"
            ),
            RepairState::WillRetry {
                at_unix,
                attempts,
                why,
            } => format!(
                "{file} is missing or damaged, and {attempts} attempt(s) to fetch it again \
                 failed ({why}); the station will retry at {at_unix} (Unix time)"
            ),
            RepairState::NotFetched { why, .. } => {
                format!("{file} is missing or damaged, and is not being fetched: {why}")
            }
            RepairState::Repaired { restart, .. } => match restart {
                RepairRestart::NotNeeded => format!("{file} was fetched again and verifies"),
                RepairRestart::Restarting => format!(
                    "{file} was fetched again and verifies; the station is restarting to use it"
                ),
                RepairRestart::NeededByHand => {
                    format!("{file} was fetched again and verifies; restart the station to use it")
                }
            },
        }
    }
}

/// Whether an occurrence model setting turns the filter off on purpose.
///
/// The setting is `METADATA_MODEL_PATH`, `BIRDNET_METADATA_MODEL` or
/// `--metadata-model`, and off is `off` or `none`, in any case. An owner who runs unfiltered says so
/// with it, and the filter is then not configured, not a file to look for.
/// THE predicate for that, for the daemon and the station's defaults alike.
#[must_use]
pub fn occurrence_filter_turned_off(setting: &std::path::Path) -> bool {
    setting.to_str().is_some_and(|s| {
        let s = s.trim();
        s.eq_ignore_ascii_case("off") || s.eq_ignore_ascii_case("none")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each `CODES` list is exactly the codes its type produces, in order and
    /// without repeats: an exposition that walks the list then has one series
    /// per state, and a state added without its code fails here rather than
    /// vanishing from the metrics.
    #[test]
    fn every_state_has_its_code_listed_once() {
        let detector = [
            DetectorState::Starting,
            DetectorState::Running,
            DetectorState::WebOnly { why: String::new() },
            DetectorState::NotConfigured { why: String::new() },
            DetectorState::Failed { why: String::new() },
            DetectorState::Stopped,
        ];
        assert_eq!(
            detector.iter().map(DetectorState::code).collect::<Vec<_>>(),
            DetectorState::CODES
        );

        let faults = [
            ClassifierFault::FileMissing,
            ClassifierFault::WillNotLoad,
            ClassifierFault::DoesNotFit,
            ClassifierFault::MemoryUnknown,
            ClassifierFault::LabelsNotSet,
            ClassifierFault::LabelsMissing,
            ClassifierFault::LabelsUnreadable,
            ClassifierFault::LabelsWrongLength,
            ClassifierFault::DetectorNotRunning,
        ];
        let mut classifier = vec![
            ClassifierStatus::Judging {
                sources: Vec::new(),
                unrouted_sources: true,
            },
            ClassifierStatus::Unrouted {
                primary: String::new(),
            },
        ];
        classifier.extend(faults.iter().map(|f| ClassifierStatus::left_out(*f, "")));
        assert_eq!(
            classifier
                .iter()
                .map(ClassifierStatus::code)
                .collect::<Vec<_>>(),
            ClassifierStatus::CODES
        );
        for fault in faults {
            assert_eq!(
                ClassifierStatus::left_out(fault, "").code(),
                format!("left_out_{}", fault.code())
            );
        }

        let filter = [
            FilterState::Active { candidates: None },
            FilterState::off(FilterOff::NotConfigured, ""),
            FilterState::off(FilterOff::NoCoordinates, ""),
            FilterState::off(FilterOff::ModelMissing, ""),
            FilterState::off(FilterOff::ModelUnusable, ""),
            FilterState::off(FilterOff::LabelsMissing, ""),
            FilterState::off(FilterOff::LabelsUnreadable, ""),
            FilterState::off(FilterOff::DetectorNotRunning, ""),
        ];
        assert_eq!(
            filter.iter().map(FilterState::code).collect::<Vec<_>>(),
            FilterState::CODES
        );
    }

    /// A filter that is off admits no count, so nothing can print "admitting
    /// N" for a filter that is not running; an active one carries its own.
    #[test]
    fn only_an_active_filter_has_candidates() {
        assert_eq!(
            FilterState::Active {
                candidates: Some(42)
            }
            .candidates(),
            Some(42)
        );
        assert!(FilterState::Active { candidates: None }.is_active());
        let off = FilterState::off(FilterOff::NoCoordinates, "set LATITUDE");
        assert_eq!(off.candidates(), None);
        assert!(!off.is_active());
        assert!(off.message().contains("set LATITUDE"), "{}", off.message());
    }

    /// The JSON the next phase's API returns: one `state` key per object, in
    /// the same words as the metric codes.
    #[test]
    fn the_state_serialises_with_the_codes_it_exports() {
        let state = ModelsState {
            detector: DetectorState::Running,
            classifiers: vec![ClassifierState {
                id: "perch".to_owned(),
                primary: false,
                model_path: PathBuf::from("/m/perch.onnx"),
                labels_path: None,
                status: ClassifierStatus::left_out(ClassifierFault::WillNotLoad, "truncated"),
            }],
            occurrence_filter: FilterState::off(FilterOff::NoCoordinates, "no position"),
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["detector"]["state"], "running");
        assert_eq!(json["classifiers"][0]["status"]["state"], "left_out");
        assert_eq!(json["classifiers"][0]["status"]["fault"], "will_not_load");
        assert_eq!(json["occurrence_filter"]["state"], "off");
        assert_eq!(json["occurrence_filter"]["reason"], "no_coordinates");
    }

    /// A detector that stops takes the loaded classifiers and the filter
    /// with it, and a classifier already left out keeps its own reason; a
    /// running one leaves them as they were published.
    #[test]
    fn a_detector_that_is_not_running_runs_no_classifier_and_no_filter() {
        let classifier = |id: &str, status| ClassifierState {
            id: id.to_owned(),
            primary: id == "birdnet",
            model_path: PathBuf::from(format!("/m/{id}.onnx")),
            labels_path: None,
            status,
        };
        let mut state = ModelsState {
            classifiers: vec![
                classifier(
                    "birdnet",
                    ClassifierStatus::Judging {
                        sources: Vec::new(),
                        unrouted_sources: true,
                    },
                ),
                classifier(
                    "perch",
                    ClassifierStatus::left_out(ClassifierFault::LabelsMissing, "no labels"),
                ),
            ],
            occurrence_filter: FilterState::Active {
                candidates: Some(3),
            },
            ..ModelsState::default()
        };
        let running = {
            let mut s = state.clone();
            s.set_detector(DetectorState::Running);
            s
        };
        assert_eq!(running.classifiers, state.classifiers);
        assert!(running.occurrence_filter.is_active());

        state.set_detector(DetectorState::Stopped);
        assert_eq!(
            state.classifiers[0].status.code(),
            "left_out_detector_not_running"
        );
        assert_eq!(
            state.classifiers[1].status.code(),
            "left_out_labels_missing"
        );
        assert_eq!(state.occurrence_filter.code(), "off_detector_not_running");
        assert_eq!(state.detector, DetectorState::Stopped);
    }

    /// An unrouted classifier's message gives the route that would use it,
    /// with the key a one-microphone station needs.
    #[test]
    fn an_unrouted_classifier_is_told_the_route_that_reaches_it() {
        let state = ClassifierState {
            id: "perch".to_owned(),
            primary: false,
            model_path: PathBuf::from("/m/perch.onnx"),
            labels_path: Some(PathBuf::from("/m/perch.csv")),
            status: ClassifierStatus::Unrouted {
                primary: "birdnet".to_owned(),
            },
        };
        assert!(
            state.message().contains("`local:birdnet+perch`"),
            "{}",
            state.message()
        );
        assert!(
            !state.message().contains("<primary>"),
            "{}",
            state.message()
        );
    }

    /// Each repair state has its code listed once, in order, and only the
    /// two states that mean "the file is the pinned one" say so.
    #[test]
    fn every_repair_state_has_its_code_listed_once() {
        let states = [
            RepairState::Checking,
            RepairState::Verified,
            RepairState::Fetching {
                since_unix: 0,
                done: 0,
                total: 0,
            },
            RepairState::WillRetry {
                at_unix: 0,
                attempts: 1,
                why: String::new(),
            },
            RepairState::NotFetched {
                why: String::new(),
                looks_again_unix: 0,
            },
            RepairState::Repaired {
                at_unix: 0,
                restart: RepairRestart::NotNeeded,
            },
        ];
        assert_eq!(
            states.iter().map(RepairState::code).collect::<Vec<_>>(),
            RepairState::CODES
        );
        assert_eq!(
            states
                .iter()
                .filter(|s| s.is_verified())
                .map(RepairState::code)
                .collect::<Vec<_>>(),
            ["verified", "repaired"]
        );
        let file = ModelFile {
            catalogue_id: "birdnet-v24".to_owned(),
            path: PathBuf::from("/m/birdnet-v24.onnx"),
            repair: RepairState::WillRetry {
                at_unix: 1_800_000_000,
                attempts: 2,
                why: "connection refused".to_owned(),
            },
        };
        let json = serde_json::to_value(&file).unwrap();
        assert_eq!(json["repair"]["state"], "will_retry");
        assert_eq!(json["repair"]["at_unix"], 1_800_000_000_u64);
        assert!(
            file.message().contains("retry at 1800000000"),
            "{}",
            file.message()
        );
    }

    /// `off` and `none`, in any case and with spaces around, turn the filter
    /// off; a path, even one ending in either word, does not.
    #[test]
    fn off_and_none_turn_the_filter_off_and_a_path_does_not() {
        use std::path::Path;
        for off in ["off", "OFF", " none ", "None"] {
            assert!(occurrence_filter_turned_off(Path::new(off)), "{off}");
        }
        for path in [
            "/m/off",
            "off.onnx",
            "/data/model/birdnet-v24-range-v2.onnx",
            "",
        ] {
            assert!(!occurrence_filter_turned_off(Path::new(path)), "{path}");
        }
    }
}
