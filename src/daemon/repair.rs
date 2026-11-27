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
    /// Whether the configuration names it by the name an earlier installer
    /// gave it rather than the one `--install-model` gives it (MDL-25).
    pub legacy_name: bool,
}

/// The catalogue's file at `path`, when it is one: its name is the one
/// `--install-model` gives an entry's model or labels, or the one an earlier
/// installer gave it (the V3.0 files' long upstream names), which a station
/// the installer has not run on since still has in its configuration.
#[must_use]
pub fn catalogue_file(file: &Configured) -> Option<Watched> {
    let dir = file.path.parent()?.to_path_buf();
    let watched = |entry, part, legacy_name| Watched {
        entry,
        dir: dir.clone(),
        part,
        file: file.clone(),
        legacy_name,
    };
    catalog::CATALOG
        .iter()
        .find_map(|entry| {
            catalog::part_at(entry, &dir, &file.path).map(|part| watched(entry, part, false))
        })
        .or_else(|| {
            catalog::CATALOG.iter().find_map(|entry| {
                catalog::legacy_part_at(entry, &dir, &file.path)
                    .map(|part| watched(entry, part, true))
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
    /// Restart the station: [`RepairRestart::Restarting`] when a restart is
    /// under way, [`RepairRestart::NeededByHand`] when nothing here can
    /// restart this process.
    fn restart(&self) -> RepairRestart;
}

impl Station for birdnet_web::state::AppState {
    fn publish(&self, file: ModelFile) {
        self.metrics().set_model_file(file);
    }

    fn models(&self) -> ModelsState {
        self.models_state()
    }

    fn restart(&self) -> RepairRestart {
        if self.request_restart() {
            RepairRestart::Restarting
        } else {
            RepairRestart::NeededByHand
        }
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
        self.pass_at(Instant::now())
    }

    /// [`Self::pass`], with `now` the instant a file is due by.
    fn pass_at(&mut self, now: Instant) -> bool {
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
        let restart = if again.iter().any(|(_, without)| *without) {
            self.station.restart()
        } else {
            RepairRestart::NotNeeded
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
        // The file the configuration names, which for an earlier installer's
        // name is not the one `part_state` reads.
        let state = if w.legacy_name {
            catalog::state_at(w.entry, w.part, &w.file.path, rehash)
        } else {
            catalog::part_state(w.entry, &w.dir, w.part, rehash)
        };
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
        let policy = policy_for(&self.policy, t.attempts);
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
            if w.legacy_name {
                catalog::install_part_under_legacy_name(
                    w.entry.id,
                    &w.dir,
                    w.part,
                    &w.file.path,
                    &policy,
                    &mut progress,
                )
            } else {
                catalog::install_part(w.entry.id, &w.dir, w.part, &policy, &mut progress)
            }
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

/// The retry policy for a file's next fetch, `attempts` having been made
/// since it last verified: the whole policy for the first, one round of the
/// origins after that (each later one is already an interval apart).
fn policy_for(policy: &FetchPolicy, attempts: u32) -> FetchPolicy {
    if attempts == 0 {
        policy.clone()
    } else {
        policy.one_round()
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
    use crate::daemon::test_support::logged;
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

    /// MDL-25: a station whose configuration still names the V3.0 files by the
    /// names an earlier installer gave them is looked after too, under those
    /// names.
    #[test]
    fn a_file_under_an_earlier_installers_name_is_the_catalogues_too() {
        let w = at("/m/BirdNET+_V3.0-preview3_Global_11K_FP32.onnx").expect("watched");
        assert_eq!(
            (w.entry.id, w.part, w.legacy_name),
            ("birdnet-v3", Part::Model, true)
        );
        let w = at("/m/BirdNET+_V3.0-preview3_Global_11K_Labels.csv").expect("watched");
        assert_eq!(
            (w.entry.id, w.part, w.legacy_name),
            ("birdnet-v3", Part::Labels, true)
        );
        let w = at("/m/BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx").expect("watched");
        assert_eq!(w.entry.id, "birdnet-v3-geomodel");
        // Counterparts: the installed name is not a legacy one, and a name that
        // is neither is still the operator's own.
        assert!(!at("/m/birdnet-v3.onnx").unwrap().legacy_name);
        assert!(at("/m/BirdNET+_V3.0-preview3_Global_11K_FP16.onnx").is_none());
    }

    /// And the repair acts on it: missing under its configured name, it is
    /// published under that name and, with no way to fetch, said so, where the
    /// station used to say nothing at all about it.
    #[test]
    fn a_missing_file_under_an_earlier_installers_name_is_reported_under_that_name() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir
            .path()
            .join("BirdNET+_V3.0-preview3_Global_11K_FP32.onnx");
        let station = Recorder::default();
        let mut repair = repair_of(&legacy, Reader::Primary, &station);
        assert_eq!(repair.tracked.len(), 1, "the file is watched");
        assert!(!repair.pass());
        let published = station.published.lock().unwrap().last().cloned().unwrap();
        assert_eq!(published.path, legacy);
        assert!(
            matches!(published.repair, RepairState::NotFetched { .. }),
            "{:?}",
            published.repair
        );
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
        fn restart(&self) -> RepairRestart {
            *self.restarts.lock().unwrap() += 1;
            RepairRestart::Restarting
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

    const HOUR: Duration = Duration::from_secs(3_600);

    fn repair_with(
        path: &Path,
        reader: Reader,
        station: &Recorder,
        policy: FetchPolicy,
    ) -> Repair<Recorder> {
        let w = watched(&[Configured {
            path: path.to_path_buf(),
            reader,
        }]);
        Repair::new(w, policy, station.clone(), &[])
    }

    fn running() -> ModelsState {
        models(
            DetectorState::Running,
            ClassifierStatus::Unrouted {
                primary: "birdnet".to_owned(),
            },
            FilterState::Active { candidates: None },
        )
    }

    /// Seconds since the epoch by the system clock, read here rather than
    /// through the code under test.
    fn epoch_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    /// An instant well past any file's next look.
    fn hours_later() -> Instant {
        Instant::now() + 2 * HOUR
    }

    /// A file due at exactly this instant is looked at; one due a nanosecond
    /// later is not, yet.
    #[test]
    fn a_file_is_looked_at_once_it_is_due_and_not_before() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("birdnet-v24_labels.txt");
        let station = Recorder::default();
        *station.models.lock().unwrap() = running();
        let mut repair = repair_of(&labels, Reader::Primary, &station);
        let due = repair.tracked[0].next;
        assert!(!repair.pass_at(due.checked_sub(Duration::from_nanos(1)).unwrap()));
        assert_eq!(last(&station), RepairState::Checking, "not due yet");
        assert!(!labels.exists());
        assert!(!repair.pass_at(due));
        assert!(
            labels.exists(),
            "due at this instant: looked at, and written"
        );
        assert!(matches!(last(&station), RepairState::Repaired { .. }));
    }

    /// An `--offline` station with no mirror says, once, that it is not
    /// fetching the file, tells the page when it looks again (an interval
    /// from now), and does not look again before then.
    #[test]
    fn a_file_not_fetched_is_said_once_and_looked_at_again_an_interval_later() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("birdnet-v24-range-v2.onnx");
        std::fs::write(&model, "damaged").unwrap();
        let station = Recorder::default();
        let policy = FetchPolicy {
            mirror_only: true,
            repair_interval: HOUR,
            ..FetchPolicy::default()
        };
        let mut repair = repair_with(&model, Reader::Filter, &station, policy);
        let ((), said) = logged(tracing::Level::WARN, || {
            let before = epoch_secs();
            assert!(!repair.pass());
            let after = epoch_secs();
            let RepairState::NotFetched {
                looks_again_unix, ..
            } = last(&station)
            else {
                panic!("{:?}", last(&station));
            };
            assert!(
                (before + HOUR.as_secs()..=after + HOUR.as_secs()).contains(&looks_again_unix),
                "{looks_again_unix} is not an hour after {before}"
            );
            let published = station.published.lock().unwrap().len();
            assert!(!repair.pass());
            assert_eq!(
                station.published.lock().unwrap().len(),
                published,
                "looked at again before the interval had passed"
            );
            assert!(!repair.pass_at(hours_later()));
            assert!(!repair.pass_at(hours_later()));
            assert!(matches!(last(&station), RepairState::NotFetched { .. }));
        });
        assert_eq!(
            said.matches("is not being fetched").count(),
            1,
            "said once over three looks: {said}"
        );
    }

    /// The V2.4 model's labels are in the binary and its weights are not: an
    /// `--offline` station with no mirror writes the labels again but does not
    /// try to fetch the model.
    #[test]
    fn an_offline_station_does_not_fetch_a_model_whose_labels_are_embedded() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("birdnet-v24.onnx");
        std::fs::write(&model, "damaged").unwrap();
        let station = Recorder::default();
        *station.models.lock().unwrap() = running();
        let mut repair = repair_of(&model, Reader::Primary, &station);
        assert!(!repair.pass());
        assert!(
            matches!(last(&station), RepairState::NotFetched { .. }),
            "{:?}",
            last(&station)
        );
        assert_eq!(repair.tracked[0].attempts, 0, "nothing was fetched");
    }

    /// A fetch that fails is published with how many attempts there have been
    /// and when the next is (an interval from now), and is not tried again
    /// before then. The mirror here is a port on this machine nothing listens
    /// on, so the attempt fails at once.
    #[test]
    fn a_failed_fetch_counts_its_attempts_and_waits_an_interval() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("birdnet-v24-range-v2.onnx");
        std::fs::write(&model, "damaged").unwrap();
        let station = Recorder::default();
        let policy = FetchPolicy {
            mirror: Some(format!("http://127.0.0.1:{port}")),
            mirror_only: true,
            retry_waits: Vec::new(),
            connect_timeout: Duration::from_secs(5),
            stall_timeout: Duration::from_secs(5),
            repair_interval: HOUR,
        };
        let mut repair = repair_with(&model, Reader::Filter, &station, policy);
        let before = epoch_secs();
        assert!(!repair.pass());
        let after = epoch_secs();
        let RepairState::WillRetry {
            at_unix, attempts, ..
        } = last(&station)
        else {
            panic!("{:?}", last(&station));
        };
        assert_eq!(attempts, 1);
        assert!(
            (before + HOUR.as_secs()..=after + HOUR.as_secs()).contains(&at_unix),
            "{at_unix} is not an hour after {before}"
        );
        let published = station.published.lock().unwrap().len();
        assert!(!repair.pass());
        assert_eq!(
            station.published.lock().unwrap().len(),
            published,
            "tried again before the interval had passed"
        );
        assert!(!repair.pass_at(hours_later()));
        assert!(
            matches!(last(&station), RepairState::WillRetry { attempts: 2, .. }),
            "{:?}",
            last(&station)
        );
    }

    /// The first fetch of a file gets the catalogue's whole retry policy;
    /// every later one, already an interval after the last, one round.
    #[test]
    fn the_first_fetch_retries_and_later_ones_try_once() {
        let policy = FetchPolicy {
            retry_waits: vec![Duration::from_secs(30), Duration::from_secs(90)],
            ..FetchPolicy::default()
        };
        assert_eq!(policy_for(&policy, 0).retry_waits, policy.retry_waits);
        assert!(policy_for(&policy, 1).retry_waits.is_empty());
        assert!(policy_for(&policy, 2).retry_waits.is_empty());
    }

    /// While a file is fetched the page is told how big it is: the size the
    /// catalogue pins, which for embedded labels is the text the binary holds.
    #[test]
    fn a_fetch_is_published_with_the_files_pinned_size() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("birdnet-v24_labels.txt");
        let station = Recorder::default();
        *station.models.lock().unwrap() = running();
        let mut repair = repair_of(&labels, Reader::Primary, &station);
        assert!(!repair.pass());
        let totals: Vec<u64> = station
            .published
            .lock()
            .unwrap()
            .iter()
            .filter_map(|f| match f.repair {
                RepairState::Fetching { total, .. } => Some(total),
                _ => None,
            })
            .collect();
        let embedded = birdnet_core::inference::labels::BIRDNET_V24_ENGLISH_LABELS.len() as u64;
        assert!(!totals.is_empty(), "the fetch was published");
        assert!(totals.iter().all(|t| *t == embedded), "{totals:?}");

        let model = at("/m/birdnet-v24.onnx").unwrap();
        assert_eq!(
            pinned_size(model.entry, Part::Model),
            model.entry.model_bytes
        );
    }

    /// The running application is the station the repair reports to: what it
    /// publishes is what the Station Health page reads, the models it reads
    /// are the ones the detector published, and a station nothing supervises
    /// (as a test's is) says a restart must be made by hand rather than
    /// signalling itself.
    #[test]
    fn the_application_is_the_station_the_repair_reports_to() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let file = ModelFile {
            catalogue_id: "birdnet-v24".to_owned(),
            path: PathBuf::from("/m/birdnet-v24.onnx"),
            repair: RepairState::Checking,
        };
        Station::publish(&state, file.clone());
        assert_eq!(state.metrics().model_files(), [file]);

        let failed = DetectorState::Failed {
            why: "labels".into(),
        };
        state.metrics().set_detector_state(failed.clone());
        assert_eq!(Station::models(&state).detector, failed);

        assert_eq!(Station::restart(&state), RepairRestart::NeededByHand);
    }

    /// A station that only forwards what is published, to a test on another
    /// thread.
    struct Forwarding(std::sync::mpsc::Sender<ModelFile>);

    impl Station for Forwarding {
        fn publish(&self, file: ModelFile) {
            let _ = self.0.send(file);
        }
        fn models(&self) -> ModelsState {
            running()
        }
        fn restart(&self) -> RepairRestart {
            RepairRestart::NeededByHand
        }
    }

    /// Starting the repair starts it: a missing labels file the catalogue
    /// embeds is published as checked and then written, by the repair's own
    /// thread, with no network (the station is `--offline`).
    #[test]
    fn spawning_the_repair_looks_after_the_configured_files() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("birdnet-v24_labels.txt");
        let cli = cli(&["--offline", "--labels", labels.to_str().unwrap()]);
        let (tx, rx) = std::sync::mpsc::channel();
        spawn(&cli, None, Forwarding(tx));
        let mut seen = Vec::new();
        while let Ok(file) = rx.recv_timeout(Duration::from_secs(30)) {
            assert_eq!(file.path, labels);
            let repaired = matches!(file.repair, RepairState::Repaired { .. });
            seen.push(file.repair);
            if repaired {
                break;
            }
        }
        assert_eq!(seen.first(), Some(&RepairState::Checking), "{seen:?}");
        assert!(
            matches!(seen.last(), Some(RepairState::Repaired { .. })),
            "{seen:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&labels).unwrap(),
            birdnet_core::inference::labels::BIRDNET_V24_ENGLISH_LABELS
        );
    }

    /// Starting the repair says so in the journal when there are files to look
    /// after, and a `--web-only` station, which looks after none, does not.
    #[test]
    fn starting_the_repair_says_so_only_when_there_is_something_to_look_after() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("birdnet-v24_labels.txt");
        let path = labels.to_str().unwrap();
        let start = |args: &[&str]| {
            let (tx, _rx) = std::sync::mpsc::channel();
            logged(tracing::Level::INFO, || {
                spawn(&cli(args), None, Forwarding(tx));
            })
            .1
        };
        let said = start(&["--offline", "--web-only", "--labels", path]);
        assert!(!said.contains("looking after the model files"), "{said}");
        let said = start(&["--offline", "--labels", path]);
        assert!(said.contains("looking after the model files"), "{said}");
    }

    /// At the start every catalogue model installed in the model directory is
    /// hashed again, and its files are handed to the first look so it need not
    /// hash them twice. One that is not the pinned file is warned about when
    /// the configuration does not name it (by its model or its labels), and
    /// left to the repair, without a warning, when it does.
    #[test]
    fn an_installed_model_is_rehashed_and_warned_about_only_when_nothing_repairs_it() {
        const WARNING: &str = "is not the file this build pins";
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("birdnet-v24.onnx");
        let labels = dir.path().join("birdnet-v24_labels.txt");
        std::fs::write(&model, "not the pinned model").unwrap();
        let named_by = |path: &Path| {
            watched(&[Configured {
                path: path.to_path_buf(),
                reader: Reader::Primary,
            }])
        };

        let (hashed, said) = logged(tracing::Level::WARN, || reverify(dir.path(), &[]));
        assert_eq!(hashed, [model.clone(), labels.clone()]);
        assert!(said.contains(WARNING), "not named: {said}");

        let (hashed, said) = logged(tracing::Level::WARN, || {
            reverify(dir.path(), &named_by(&model))
        });
        assert_eq!(hashed, [model, labels.clone()]);
        assert!(!said.contains(WARNING), "named by its model: {said}");

        let (_, said) = logged(tracing::Level::WARN, || {
            reverify(dir.path(), &named_by(&labels))
        });
        assert!(!said.contains(WARNING), "named by its labels: {said}");
    }
}
