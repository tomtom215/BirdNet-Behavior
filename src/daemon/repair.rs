//! The station looks after its own model files (MDL-10).
//!
//! A model file the configuration names, that the catalogue knows (it sits
//! under the name `--install-model` gives it), and that is missing or not the
//! file this build pins, is fetched again by the station itself, on a thread of
//! its own, without the owner doing anything: first with the catalogue's whole
//! retry policy, then once per [`FetchPolicy::repair_interval`] (an hour) while
//! it stays unavailable. A file that verifies is looked at again at the same
//! interval, from its verification record, so one deleted or rewritten while
//! the station runs is found too.
//!
//! # Using a file that verifies again: a restart, not a load in place
//!
//! (What it is the station is "without": a file missing or refused at the
//! start, or one that was damaged when the start loaded it. Eight bytes
//! changed in a model's weights still load, and run on wrong weights.)
//!
//! Starting the detector is the one path that loads classifiers, wires the
//! event processor, the run record, the watchdog's heartbeat, the liveness
//! mirror and the published model state together (`app::start_detector`).
//! Loading a repaired file into a running station would be a second way of
//! doing all of that, and the one nobody's start-up exercises. So when a file
//! that the running station is *without* — the detector failed, a classifier
//! was left out over its file, the occurrence filter is off over its file —
//! verifies again, the station restarts through the same control the browser's
//! restart button uses (systemd's `Restart=always`, or the binary running
//! itself again where nothing supervises it).
//!
//! # Why this cannot restart a station in a loop
//!
//! A restart is asked for only on a *transition*: a file this process saw
//! missing or damaged now verifies. After the restart the file verifies at the
//! first check, so there is no transition and no restart, whether or not the
//! file then loads: a file that verifies and will not load (the wrong model
//! for the slot it is configured in) leaves the station running on what it
//! has, never restarting. And a file the running station already has loaded
//! (damaged on the disk after it was read) is fetched again without a restart:
//! a healthy detector is never restarted by this.
//!
//! # What it does not do
//!
//! * A station started `--web-only` runs no detector, so nothing here runs.
//! * A station started `--offline` makes no unsolicited connection: it fetches
//!   from the operator's mirror (`BIRDNET_MODEL_MIRROR`) alone, and with none
//!   it says so once and looks again at the interval for a file put in place
//!   by hand.
//! * A file under a name of the operator's own choosing is not the
//!   catalogue's to replace, and is left alone.

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use birdnet_core::config::Config;
use birdnet_core::inference::model_state::{
    ClassifierFault, ClassifierStatus, FilterOff, FilterState, ModelFile, ModelsState,
    RepairRestart, RepairState,
};
use birdnet_integrations::model_catalog::{
    self as catalog, CatalogEntry, FetchPolicy, FileState, Part, Progress, Rehash,
};

use crate::cli::Cli;

/// What reads a configured model file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    /// The primary classifier (`MODEL_PATH`, `LABELS_PATH`).
    Primary,
    /// An extra classifier (`MODEL_n_PATH`, `MODEL_n_LABELS`).
    Extra,
    /// The occurrence filter (`METADATA_MODEL_PATH`, `METADATA_LABELS_PATH`).
    Filter,
}

/// One file the configuration names, and what reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configured {
    /// The file, as configured.
    pub path: PathBuf,
    /// What reads it.
    pub reader: Reader,
}

/// Every model file the configuration names, as the detector reads them: the
/// command line first, then the configuration file.
#[must_use]
pub fn configured_files(cli: &Cli, config: Option<&Config>) -> Vec<Configured> {
    let key = |k: &str| config.and_then(|c| c.get(k)).map(PathBuf::from);
    let mut files = Vec::new();
    let mut add = |path: Option<PathBuf>, reader| {
        if let Some(path) = path {
            files.push(Configured { path, reader });
        }
    };
    add(
        cli.model.clone().or_else(|| key("MODEL_PATH")),
        Reader::Primary,
    );
    add(
        cli.labels.clone().or_else(|| key("LABELS_PATH")),
        Reader::Primary,
    );
    for (model, labels) in crate::helpers::models::configured_extra_files(config) {
        add(Some(model), Reader::Extra);
        add(labels, Reader::Extra);
    }
    add(
        cli.metadata_model
            .clone()
            .or_else(|| key("METADATA_MODEL_PATH")),
        Reader::Filter,
    );
    add(
        cli.metadata_labels
            .clone()
            .or_else(|| key("METADATA_LABELS_PATH")),
        Reader::Filter,
    );
    files
}

/// One configured file the catalogue knows: the entry it belongs to, its
/// directory, and which of the entry's files it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    /// The entry.
    pub entry: &'static CatalogEntry,
    /// The directory it is in.
    pub dir: PathBuf,
    /// Which of the entry's files it is.
    pub part: Part,
    /// The file, and what reads it.
    pub file: Configured,
}

/// The catalogue's file at `path`, when it is one: its name is the one
/// `--install-model` gives an entry's model or labels.
#[must_use]
pub fn catalogue_file(file: &Configured) -> Option<Watched> {
    let dir = file.path.parent()?.to_path_buf();
    catalog::CATALOG.iter().find_map(|entry| {
        catalog::part_at(entry, &dir, &file.path).map(|part| Watched {
            entry,
            dir: dir.clone(),
            part,
            file: file.clone(),
        })
    })
}

/// The configured files the catalogue knows, each once, in the order the
/// configuration names them.
#[must_use]
pub fn watched(files: &[Configured]) -> Vec<Watched> {
    let mut out: Vec<Watched> = Vec::new();
    for w in files.iter().filter_map(catalogue_file) {
        if !out.iter().any(|o| o.file.path == w.file.path) {
            out.push(w);
        }
    }
    out
}

/// Whether the running station is without `path`: what reads it did not get
/// it, so a restart is what would load it. `false` for a file whose reader is
/// running with it, which is what keeps a healthy detector from being
/// restarted.
#[must_use]
pub fn left_without(reader: Reader, path: &Path, models: &ModelsState) -> bool {
    match reader {
        Reader::Primary => !models.detector.is_running(),
        Reader::Extra => models.classifiers.iter().any(|c| {
            (c.model_path == path || c.labels_path.as_deref() == Some(path))
                && matches!(
                    c.status,
                    ClassifierStatus::LeftOut {
                        fault: ClassifierFault::FileMissing
                            | ClassifierFault::WillNotLoad
                            | ClassifierFault::LabelsMissing
                            | ClassifierFault::LabelsUnreadable
                            | ClassifierFault::LabelsWrongLength,
                        ..
                    }
                )
        }),
        Reader::Filter => matches!(
            models.occurrence_filter,
            FilterState::Off {
                reason: FilterOff::ModelMissing
                    | FilterOff::ModelUnusable
                    | FilterOff::LabelsMissing
                    | FilterOff::LabelsUnreadable,
                ..
            }
        ),
    }
}

/// Where the repair publishes and what it asks of the station: the running
/// application, or a test's stand-in.
pub trait Station: Send + 'static {
    /// Publish what is being done about one file.
    fn publish(&self, file: ModelFile);
    /// What the detector made of the files at its start.
    fn models(&self) -> ModelsState;
    /// Restart the station; `true` when a restart is under way.
    fn restart(&self) -> bool;
}

impl Station for birdnet_web::state::AppState {
    fn publish(&self, file: ModelFile) {
        self.metrics().set_model_file(file);
    }

    fn models(&self) -> ModelsState {
        self.models_state()
    }

    fn restart(&self) -> bool {
        self.request_restart()
    }
}

/// Start looking after the configured model files on a thread of its own;
/// on a `--web-only` station, which runs no detector, only the re-hash below.
///
/// Also re-hashes, once, every catalogue model installed in the model
/// directory (MDL-11): a file damaged in place keeps its size and its
/// modification time, so only reading it finds the damage. A plain thread, not
/// the runtime's blocking pool, which the runtime waits for at shutdown.
pub fn spawn(cli: &Cli, config: Option<&Config>, station: impl Station) {
    let model_dir = crate::helpers::models::model_dir(config);
    let watched = if cli.web_only {
        Vec::new()
    } else {
        watched(&configured_files(cli, config))
    };
    let mut policy = FetchPolicy::from_env();
    policy.mirror_only = cli.offline;
    if !watched.is_empty() {
        tracing::info!(
            files = watched.len(),
            every_secs = policy.repair_interval.as_secs(),
            "looking after the model files the configuration names: a missing or damaged one \
             is fetched again"
        );
    }
    let spawned = std::thread::Builder::new()
        .name("model-repair".to_owned())
        .spawn(move || {
            let rehashed = reverify(&model_dir, &watched);
            Repair::new(watched, policy, station, &rehashed).run();
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start looking after the model files");
    }
}

/// Re-hash every catalogue model installed in `model_dir`, and warn about one
/// that is not the pinned file and that the configuration does not name (a
/// named one is repaired). Returns the files it hashed, which the first look
/// then need not hash again.
fn reverify(model_dir: &Path, watched: &[Watched]) -> Vec<PathBuf> {
    let mut hashed = Vec::new();
    for (entry, state) in catalog::reverify_installed(model_dir) {
        let (model, labels) = catalog::installed_paths(entry, model_dir);
        let named = watched
            .iter()
            .any(|w| w.file.path == model || Some(&w.file.path) == labels.as_ref());
        hashed.push(model);
        hashed.extend(labels);
        if state.is_verified() {
            tracing::info!(id = entry.id, "installed model re-verified");
        } else if !named {
            tracing::warn!(
                id = entry.id,
                dir = %model_dir.display(),
                ?state,
                "an installed model is not the file this build pins; reinstall it with \
                 `birdnet-behavior --install-model {}`",
                entry.id
            );
        }
    }
    hashed
}

/// Seconds since the Unix epoch, now.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One watched file and where its repair has got to.
struct Tracked {
    watched: Watched,
    /// When it is next looked at.
    next: Instant,
    /// How the first look hashes it, until it has been looked at: in full,
    /// unless the start-up's re-hash has just read it.
    first: Option<Rehash>,
    /// Whether this process has seen it missing or damaged since it last
    /// verified.
    broken: bool,
    /// Whether it was damaged at the first look, just after the detector's
    /// start: what reads it then loaded the damaged bytes, if it loaded at
    /// all, and a file damaged in its weights still loads.
    loaded_damaged: bool,
    /// Fetch attempts since it last verified.
    attempts: u32,
    /// Whether the station has said it may not fetch it.
    said_not_fetched: bool,
}

/// What one look at a file came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    /// Nothing changed that the station must act on.
    Nothing,
    /// It was missing or damaged and now verifies; `without` when the running
    /// station is without it.
    VerifiesAgain {
        /// Whether what reads it went without it.
        without: bool,
    },
}

/// The repair loop.
struct Repair<S: Station> {
    tracked: Vec<Tracked>,
    policy: FetchPolicy,
    station: S,
}

impl<S: Station> Repair<S> {
    fn new(watched: Vec<Watched>, policy: FetchPolicy, station: S, rehashed: &[PathBuf]) -> Self {
        let now = Instant::now();
        let tracked = watched
            .into_iter()
            .map(|w| {
                station.publish(model_file(&w, RepairState::Checking));
                // Hashed in full at the first look unless `reverify` has just
                // read it: the record cannot see damage in place.
                let first = if rehashed.contains(&w.file.path) {
                    Rehash::IfChanged
                } else {
                    Rehash::Always
                };
                Tracked {
                    watched: w,
                    next: now,
                    first: Some(first),
                    broken: false,
                    loaded_damaged: false,
                    attempts: 0,
                    said_not_fetched: false,
                }
            })
            .collect();
        Self {
            tracked,
            policy,
            station,
        }
    }

    /// Look after the files until a restart is under way; return at once
    /// when there is nothing to look after.
    fn run(mut self) {
        loop {
            if self.pass() {
                return;
            }
            let Some(next) = self.tracked.iter().map(|t| t.next).min() else {
                return;
            };
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
        }
    }

    /// Look at every file that is due, then act once on what verified again:
    /// one restart for every file the station was without, after all of them
    /// are fetched, rather than one restart per file. `true` when a restart is
    /// under way.
    fn pass(&mut self) -> bool {
        let now = Instant::now();
        let mut again = Vec::new();
        for i in 0..self.tracked.len() {
            if self.tracked[i].next > now {
                continue;
            }
            if let Look::VerifiesAgain { without } = self.look(i) {
                again.push((i, without));
            }
        }
        if again.is_empty() {
            return false;
        }
        let restart = if !again.iter().any(|(_, without)| *without) {
            RepairRestart::NotNeeded
        } else if self.station.restart() {
            RepairRestart::Restarting
        } else {
            RepairRestart::NeededByHand
        };
        let at_unix = unix_now();
        for (i, without) in again {
            let w = &self.tracked[i].watched;
            let restart = if without {
                restart
            } else {
                RepairRestart::NotNeeded
            };
            match restart {
                RepairRestart::NotNeeded => tracing::info!(
                    file = %w.file.path.display(),
                    "the model file verifies again; the running station already had it, so it \
                     keeps running"
                ),
                RepairRestart::Restarting => tracing::warn!(
                    file = %w.file.path.display(),
                    "the model file verifies again; restarting the station to load it"
                ),
                RepairRestart::NeededByHand => tracing::warn!(
                    file = %w.file.path.display(),
                    "the model file verifies again; restart the station to load it (nothing \
                     here can restart this process)"
                ),
            }
            self.station
                .publish(model_file(w, RepairState::Repaired { at_unix, restart }));
        }
        restart == RepairRestart::Restarting
    }

    /// Look at one file, and fetch it again when it is missing or damaged.
    fn look(&mut self, i: usize) -> Look {
        let t = &mut self.tracked[i];
        let first = t.first.take();
        let rehash = first.unwrap_or(Rehash::IfChanged);
        t.next = Instant::now() + self.policy.repair_interval;
        let w = &t.watched;
        let state = catalog::part_state(w.entry, &w.dir, w.part, rehash);
        if first.is_some() {
            t.loaded_damaged = state == Some(FileState::Damaged);
        }
        if state == Some(FileState::Verified) {
            return self.verified(i);
        }
        t.broken = true;
        let embedded = w.part == Part::Labels && w.entry.labels_embedded.is_some();
        if !self.policy.can_fetch() && !embedded {
            self.not_fetched(i, state);
            return Look::Nothing;
        }
        self.fetch(i, state)
    }

    /// The file verifies. Said once when it was broken; otherwise published as
    /// it is.
    fn verified(&mut self, i: usize) -> Look {
        let t = &mut self.tracked[i];
        t.attempts = 0;
        t.said_not_fetched = false;
        if std::mem::replace(&mut t.broken, false) {
            let f = &t.watched.file;
            let loaded_damaged = std::mem::replace(&mut t.loaded_damaged, false);
            return Look::VerifiesAgain {
                without: loaded_damaged || left_without(f.reader, &f.path, &self.station.models()),
            };
        }
        self.station
            .publish(model_file(&t.watched, RepairState::Verified));
        Look::Nothing
    }

    /// The station may not fetch: say so once, publish it, and look again at
    /// the interval.
    fn not_fetched(&mut self, i: usize, state: Option<FileState>) {
        let t = &mut self.tracked[i];
        let why = format!(
            "the station was started with --offline and {} is not set, so it fetches nothing \
             on its own; set it to a mirror on this network, or install the file with \
             `birdnet-behavior --install-model {}`",
            catalog::MIRROR_ENV,
            t.watched.entry.id
        );
        if !std::mem::replace(&mut t.said_not_fetched, true) {
            tracing::warn!(
                file = %t.watched.file.path.display(),
                ?state,
                "a configured model file is missing or damaged, and is not being fetched: {why}"
            );
        }
        let looks_again_unix = unix_now() + self.policy.repair_interval.as_secs();
        self.station.publish(model_file(
            &t.watched,
            RepairState::NotFetched {
                why,
                looks_again_unix,
            },
        ));
    }

    /// Fetch the file again: the whole retry policy the first time, one round
    /// of its origins after that.
    fn fetch(&mut self, i: usize, state: Option<FileState>) -> Look {
        let t = &mut self.tracked[i];
        let policy = if t.attempts == 0 {
            self.policy.clone()
        } else {
            self.policy.one_round()
        };
        t.attempts += 1;
        let w = t.watched.clone();
        let attempts = t.attempts;
        let since_unix = unix_now();
        self.station.publish(model_file(
            &w,
            RepairState::Fetching {
                since_unix,
                done: 0,
                total: pinned_size(w.entry, w.part),
            },
        ));
        tracing::warn!(
            file = %w.file.path.display(),
            attempt = attempts,
            ?state,
            "a configured model file is missing or damaged; fetching it again"
        );
        let result = {
            let station = &self.station;
            let mut progress = |p: &Progress| {
                station.publish(model_file(
                    &w,
                    RepairState::Fetching {
                        since_unix,
                        done: p.done,
                        total: p.total,
                    },
                ));
            };
            catalog::install_part(w.entry.id, &w.dir, w.part, &policy, &mut progress)
        };
        match result {
            Ok(()) => self.verified(i),
            Err(e) => {
                let retry_in = self.policy.repair_interval.as_secs();
                tracing::warn!(
                    file = %w.file.path.display(),
                    attempt = attempts,
                    retry_in_secs = retry_in,
                    error = %e,
                    "could not fetch the model file again; the station keeps running on what it \
                     has and tries again later"
                );
                self.station.publish(model_file(
                    &w,
                    RepairState::WillRetry {
                        at_unix: unix_now() + retry_in,
                        attempts,
                        why: e.to_string(),
                    },
                ));
                Look::Nothing
            }
        }
    }
}

/// The size the catalogue pins for one of `entry`'s files.
fn pinned_size(entry: &CatalogEntry, part: Part) -> u64 {
    match part {
        Part::Model => entry.model_bytes,
        Part::Labels => entry
            .labels_embedded
            .map_or(entry.labels_bytes, |text| text.len() as u64),
    }
}

/// What is published about one file.
fn model_file(watched: &Watched, repair: RepairState) -> ModelFile {
    ModelFile {
        catalogue_id: watched.entry.id.to_owned(),
        path: watched.file.path.clone(),
        repair,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_core::inference::model_state::{ClassifierState, DetectorState};
    use clap::Parser as _;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn cli(args: &[&str]) -> Cli {
        let mut full = vec!["birdnet-behavior"];
        full.extend_from_slice(args);
        Cli::parse_from(full)
    }

    fn at(path: &str) -> Option<Watched> {
        catalogue_file(&Configured {
            path: PathBuf::from(path),
            reader: Reader::Primary,
        })
    }

    /// A file is the catalogue's by the name `--install-model` gives it, in
    /// whatever directory; any other name is the operator's own.
    #[test]
    fn a_file_is_the_catalogues_by_its_installed_name() {
        let w = at("/m/birdnet-v24.onnx").unwrap();
        assert_eq!(
            (w.entry.id, w.dir, w.part),
            ("birdnet-v24", PathBuf::from("/m"), Part::Model)
        );
        let w = at("/x/birdnet-v24_labels.txt").unwrap();
        assert_eq!((w.entry.id, w.part), ("birdnet-v24", Part::Labels));
        assert_eq!(
            at("/x/birdnet-v24-range-v2.onnx").unwrap().entry.id,
            "birdnet-v24-range-v2"
        );
        assert!(at("/m/my-own-model.onnx").is_none());
        assert!(at("/m/birdnet-v24.onnx.download").is_none());
    }

    /// Every slot the detector reads is looked after, the command line
    /// before the file, each file once.
    #[test]
    fn every_configured_file_the_catalogue_knows_is_watched_once() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("birdnet.conf");
        std::fs::write(
            &conf,
            "MODEL_PATH=/cfg/ignored.onnx\nLABELS_PATH=/m/birdnet-v24_labels.txt\n\
             MODEL_2_PATH=/m/perch-v2.onnx\nMODEL_2_LABELS=/m/perch-v2_labels.csv\n\
             MODEL_3_PATH=/m/own.onnx\nMODEL_3_LABELS=/m/birdnet-v24_labels.txt\n\
             METADATA_MODEL_PATH=/m/birdnet-v24-range-v2.onnx\n",
        )
        .unwrap();
        let config = Config::load_from(&conf).unwrap();
        let cli = cli(&["--model", "/m/birdnet-v24.onnx"]);
        let files = configured_files(&cli, Some(&config));
        assert_eq!(files[0].path, PathBuf::from("/m/birdnet-v24.onnx"));
        let w: Vec<_> = watched(&files)
            .into_iter()
            .map(|w| (w.file.path.display().to_string(), w.file.reader))
            .collect();
        assert_eq!(
            w,
            [
                ("/m/birdnet-v24.onnx".to_owned(), Reader::Primary),
                ("/m/birdnet-v24_labels.txt".to_owned(), Reader::Primary),
                ("/m/perch-v2.onnx".to_owned(), Reader::Extra),
                ("/m/perch-v2_labels.csv".to_owned(), Reader::Extra),
                ("/m/birdnet-v24-range-v2.onnx".to_owned(), Reader::Filter),
            ]
        );
    }

    fn models(
        detector: DetectorState,
        extra: ClassifierStatus,
        filter: FilterState,
    ) -> ModelsState {
        let mut m = ModelsState {
            classifiers: vec![ClassifierState {
                id: "second".to_owned(),
                primary: false,
                model_path: PathBuf::from("/m/perch-v2.onnx"),
                labels_path: Some(PathBuf::from("/m/perch-v2_labels.csv")),
                status: extra,
            }],
            occurrence_filter: filter,
            ..ModelsState::default()
        };
        m.detector = detector;
        m
    }

    /// A restart is wanted only for a reader that went without its file: the
    /// running detector, the judging classifier and the active filter are
    /// not restarted, and a fault that is not the file's (no coordinates, no
    /// memory) is not one a restart mends.
    #[test]
    fn only_a_reader_left_without_its_file_wants_a_restart() {
        let judging = ClassifierStatus::Judging {
            sources: Vec::new(),
            unrouted_sources: false,
        };
        let active = FilterState::Active { candidates: None };
        let healthy = models(DetectorState::Running, judging.clone(), active.clone());
        let p = Path::new("/m/perch-v2.onnx");
        let l = Path::new("/m/perch-v2_labels.csv");
        assert!(!left_without(Reader::Primary, p, &healthy));
        assert!(!left_without(Reader::Extra, p, &healthy));
        assert!(!left_without(Reader::Filter, p, &healthy));

        let failed = models(
            DetectorState::Failed { why: "x".into() },
            judging.clone(),
            active.clone(),
        );
        assert!(left_without(Reader::Primary, p, &failed));

        let left = |fault| {
            models(
                DetectorState::Running,
                ClassifierStatus::left_out(fault, "x"),
                active.clone(),
            )
        };
        assert!(left_without(
            Reader::Extra,
            p,
            &left(ClassifierFault::WillNotLoad)
        ));
        assert!(left_without(
            Reader::Extra,
            l,
            &left(ClassifierFault::LabelsMissing)
        ));
        assert!(!left_without(
            Reader::Extra,
            p,
            &left(ClassifierFault::DoesNotFit)
        ));
        assert!(!left_without(
            Reader::Extra,
            Path::new("/m/other.onnx"),
            &left(ClassifierFault::FileMissing)
        ));

        let off = |reason| {
            models(
                DetectorState::Running,
                judging.clone(),
                FilterState::off(reason, "x"),
            )
        };
        assert!(left_without(
            Reader::Filter,
            p,
            &off(FilterOff::ModelUnusable)
        ));
        assert!(left_without(
            Reader::Filter,
            p,
            &off(FilterOff::LabelsMissing)
        ));
        assert!(!left_without(
            Reader::Filter,
            p,
            &off(FilterOff::NoCoordinates)
        ));
    }

    /// A stand-in station that records what was published and restarts.
    #[derive(Clone, Default)]
    struct Recorder {
        published: Arc<Mutex<Vec<ModelFile>>>,
        restarts: Arc<Mutex<u32>>,
        models: Arc<Mutex<ModelsState>>,
    }

    impl Station for Recorder {
        fn publish(&self, file: ModelFile) {
            self.published.lock().unwrap().push(file);
        }
        fn models(&self) -> ModelsState {
            self.models.lock().unwrap().clone()
        }
        fn restart(&self) -> bool {
            *self.restarts.lock().unwrap() += 1;
            true
        }
    }

    fn repair_of(path: &Path, reader: Reader, station: &Recorder) -> Repair<Recorder> {
        let policy = FetchPolicy {
            mirror_only: true,
            repair_interval: Duration::ZERO,
            ..FetchPolicy::default()
        };
        let w = watched(&[Configured {
            path: path.to_path_buf(),
            reader,
        }]);
        Repair::new(w, policy, station.clone(), &[])
    }

    fn last(station: &Recorder) -> RepairState {
        station
            .published
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap()
            .repair
    }

    /// The V2.4 labels are in the binary, so a damaged copy is written again
    /// with no network at all, even by an `--offline` station with no mirror;
    /// the detector that failed over it is restarted once, and the next look,
    /// finding the file verified, restarts nothing.
    #[test]
    fn a_damaged_embedded_labels_file_is_written_again_and_restarts_a_failed_detector_once() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("birdnet-v24_labels.txt");
        std::fs::write(&labels, "not the labels").unwrap();
        let station = Recorder::default();
        *station.models.lock().unwrap() = models(
            DetectorState::Failed {
                why: "labels".into(),
            },
            ClassifierStatus::Unrouted {
                primary: "birdnet".to_owned(),
            },
            FilterState::Active { candidates: None },
        );
        let mut repair = repair_of(&labels, Reader::Primary, &station);
        assert!(repair.pass(), "the failed detector is restarted");
        assert_eq!(*station.restarts.lock().unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(&labels).unwrap(),
            birdnet_core::inference::labels::BIRDNET_V24_ENGLISH_LABELS
        );
        assert!(matches!(
            last(&station),
            RepairState::Repaired {
                restart: RepairRestart::Restarting,
                ..
            }
        ));
        assert!(!repair.pass(), "a file that verifies asks for nothing");
        assert_eq!(*station.restarts.lock().unwrap(), 1);
        assert_eq!(last(&station), RepairState::Verified);
    }

    /// The counterpart: the same repair under a running detector restarts
    /// nothing, and is published as needing nothing.
    #[test]
    fn a_repair_under_a_running_detector_restarts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("birdnet-v24_labels.txt");
        let station = Recorder::default();
        *station.models.lock().unwrap() = models(
            DetectorState::Running,
            ClassifierStatus::Unrouted {
                primary: "birdnet".to_owned(),
            },
            FilterState::Active { candidates: None },
        );
        let mut repair = repair_of(&labels, Reader::Primary, &station);
        assert!(!repair.pass());
        assert_eq!(*station.restarts.lock().unwrap(), 0);
        assert!(labels.exists(), "the missing file was written");
        assert!(matches!(
            last(&station),
            RepairState::Repaired {
                restart: RepairRestart::NotNeeded,
                ..
            }
        ));
    }

    /// A file damaged when the detector loaded it is one the station runs
    /// wrongly, even with the detector running (eight bytes of weights still
    /// load): it restarts once it verifies. The same file damaged only after
    /// the first look, which found it whole, restarts nothing.
    #[test]
    fn a_file_damaged_when_it_was_loaded_restarts_a_running_detector() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("birdnet-v24_labels.txt");
        std::fs::write(&labels, "damaged before the start").unwrap();
        let station = Recorder::default();
        *station.models.lock().unwrap() = models(
            DetectorState::Running,
            ClassifierStatus::Unrouted {
                primary: "birdnet".to_owned(),
            },
            FilterState::Active { candidates: None },
        );
        let mut repair = repair_of(&labels, Reader::Primary, &station);
        assert!(repair.pass(), "loaded damaged: restarted");
        assert_eq!(*station.restarts.lock().unwrap(), 1);

        let station = Recorder::default();
        *station.models.lock().unwrap() = models(
            DetectorState::Running,
            ClassifierStatus::Unrouted {
                primary: "birdnet".to_owned(),
            },
            FilterState::Active { candidates: None },
        );
        let mut repair = repair_of(&labels, Reader::Primary, &station);
        assert!(!repair.pass(), "whole at the first look");
        std::fs::write(&labels, "damaged while running").unwrap();
        assert!(!repair.pass(), "damaged after it was loaded: no restart");
        assert_eq!(*station.restarts.lock().unwrap(), 0);
        assert!(matches!(
            last(&station),
            RepairState::Repaired {
                restart: RepairRestart::NotNeeded,
                ..
            }
        ));
    }

    /// A model an `--offline` station with no mirror cannot fetch is said
    /// once however often it is looked at, and nothing is fetched.
    #[test]
    fn an_offline_station_without_a_mirror_says_so_and_fetches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("birdnet-v24-range-v2.onnx");
        std::fs::write(&model, "damaged").unwrap();
        let station = Recorder::default();
        let mut repair = repair_of(&model, Reader::Filter, &station);
        for _ in 0..3 {
            assert!(!repair.pass());
        }
        assert!(matches!(last(&station), RepairState::NotFetched { .. }));
        assert!(repair.tracked[0].said_not_fetched);
        assert_eq!(repair.tracked[0].attempts, 0, "nothing was fetched");
        assert_eq!(std::fs::read(&model).unwrap(), b"damaged");
    }
}
