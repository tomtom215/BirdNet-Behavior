//! The models a station listens with, as a browser sees them: in use, missing,
//! damaged, being installed, or installed and waiting for a restart.
//!
//! # Why this exists
//!
//! The detection engine needs a species classifier (a neural network), a
//! labels file and, to be useful, a species-occurrence ("range") filter; a
//! station may run a second classifier beside the first. A station installed
//! with `install.sh` or the Docker image has them before it first runs. A
//! station that was just *downloaded* has none, a download can fail, and a file
//! can be damaged on the card. This module is the state the browser needs to
//! say which, and the one button that fixes it: what the configuration asks
//! for, what of it is installed and verifies, what the running station is
//! doing with it, and whether an install is in flight. It does not decide what
//! the daemon runs (that is [`birdnet_integrations::model_catalog`] and the
//! binary's start-up) and it does not restart anything.
//!
//! # What the station is set up for
//!
//! [`ClassifierSetup::snapshot`] reads it from the configuration the binary
//! started with ([`Layout`]) and what the start-up published
//! ([`ModelsState`]), and offers the catalogue's files for exactly that
//! (MDL-15):
//!
//! * the primary classifier the configuration names, when it is a catalogue
//!   model at the name an install gives it — the V3.0 model on a V3.0 station,
//!   never V2.4 — and the default V2.4 classifier on a station that names
//!   none;
//! * each further classifier (`MODEL_2_PATH`, …) that is a catalogue model;
//! * the occurrence filter that belongs to that primary
//!   ([`occurrence_filter_for`]): the configured one when it is the
//!   catalogue's, or, when none is configured, the one the next start adopts
//!   once it is installed beside the classifier.
//!
//! A file the operator named that is not the catalogue's is theirs and is not
//! offered: an install would put a file beside it that nothing reads.
//!
//! # What "in use" means
//!
//! Only the running station using a file is "in use", and that comes from
//! [`ModelsState`], not from the configuration (MDL-04, MDL-05): a filter
//! configured and off for want of coordinates says so, with the reason, and a
//! file installed since the start says "restart to use it" until a start has
//! loaded it (MDL-14). A file whose verification record says it is not the
//! pinned file is "damaged", and is offered for repair.
//!
//! # The install job
//!
//! A download is minutes on a good line and an hour on a field one, so it runs
//! on its own thread and the page polls for progress. One at a time: a second
//! request while one is running is refused, not queued, because two downloads of
//! the same file race on its `.part` name. The thread is a plain
//! `std::thread`, not `spawn_blocking`: a tokio runtime waits for blocking
//! tasks when it shuts down, and a restart or a Ctrl-C should not wait out a
//! download.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use birdnet_core::inference::model_state::{
    ClassifierFault, ClassifierState, ClassifierStatus, DetectorState, FilterOff, FilterState,
    ModelsState,
};
use birdnet_integrations::model_catalog::{
    CATALOG, CatalogEntry, DEFAULT_CLASSIFIER_ID, FileState, Progress, Rehash, Use, find,
    installed_paths, installed_state, occurrence_filter_for,
};

/// What the binary tells the web layer about this start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    /// Where an install lands, and where the default classifier is looked for.
    pub model_dir: PathBuf,
    /// The model file the detection engine was asked to run, if any.
    pub model: Option<PathBuf>,
    /// The species-occurrence filter it was asked to use, if any.
    pub range_filter: Option<PathBuf>,
    /// Whether the detection engine was started at all on this start.
    pub started: bool,
    /// Whether this process was started without a detection engine
    /// (`--web-only`), which no install can change.
    pub web_only: bool,
}

/// How an install run reports one finished or failing step. See
/// [`ClassifierSetup::begin_install`].
pub type InstallFn = fn(&str, &Path, &mut dyn FnMut(&Progress)) -> Result<(), String>;

/// The real installer: the catalogue's, which checks every file's SHA-256 and
/// size and renames it into place only when it matches.
///
/// # Errors
///
/// The installer's own message, which names the file and what was wrong.
pub fn install_from_catalogue(
    id: &str,
    dest: &Path,
    progress: &mut dyn FnMut(&Progress),
) -> Result<(), String> {
    birdnet_integrations::model_catalog::install_with_progress(id, dest, progress)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Job {
    Running {
        step: usize,
        steps: usize,
        file: String,
        /// Bytes finished in the steps before this one.
        before: u64,
        /// Bytes of this step received.
        done: u64,
        /// Bytes of every step.
        grand_total: u64,
    },
    Failed(String),
}

#[derive(Debug, Default)]
struct Inner {
    layout: Option<Layout>,
    job: Option<Job>,
    /// The catalogue files an install from this page put in place since this
    /// process started, by `(id, directory)`. The running station loaded what
    /// was there before them, so they are "installed, restart to use them".
    installed: Vec<(&'static str, PathBuf)>,
}

/// The shared setup state: written once at start-up, read by every request.
#[derive(Debug, Default)]
pub struct ClassifierSetup {
    /// An `Arc` so the install thread, which must be `'static`, can write its
    /// progress where the requests read it.
    inner: Arc<Mutex<Inner>>,
}

/// What the panel says about the station as a whole: its headline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No station layout: a process that does not run a detection engine (a
    /// test, a tool).
    Unavailable,
    /// The classifier the station needs is not installed, or is damaged. An
    /// install can fix this.
    Missing,
    /// An install is running.
    Installing,
    /// The last install failed. Another try can fix this.
    Failed,
    /// The files are in place, but this start did not use them.
    NeedsRestart,
    /// The detection engine is running with a classifier.
    Ready,
    /// The detection engine was started and has since stopped.
    Stopped,
    /// The classifier's files are in place and the detection engine did not
    /// start; [`Snapshot::failure`] says why.
    DidNotStart,
    /// This process runs the web pages only.
    WebOnly,
    /// A classifier is named in the configuration that is not one the
    /// catalogue installs, and the engine did not start with it.
    Configured,
}

impl Phase {
    /// The stable word the page's script and the JSON carry.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Missing => "missing",
            Self::Installing => "installing",
            Self::Failed => "failed",
            Self::NeedsRestart => "needs-restart",
            Self::Ready => "ready",
            Self::Stopped => "stopped",
            Self::DidNotStart => "did-not-start",
            Self::WebOnly => "web-only",
            Self::Configured => "configured",
        }
    }
}

/// What a model file is for on this station.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// The primary classifier (`MODEL_PATH`).
    Primary,
    /// A further classifier, by the operator's id for it (`MODEL_2_ID`, …).
    Second(String),
    /// The species occurrence filter (`METADATA_MODEL_PATH`).
    Filter,
}

impl Role {
    /// The key of the condition [`crate::model_conditions::conditions`] raises
    /// for the part this file is: an install of this file is the fix for it.
    #[must_use]
    pub fn condition_key(&self) -> String {
        match self {
            Self::Primary => crate::model_conditions::DETECTOR_KEY.to_owned(),
            Self::Second(id) => crate::model_conditions::classifier_key(id),
            Self::Filter => crate::model_conditions::FILTER_KEY.to_owned(),
        }
    }
}

/// Where one model file the station is set up for stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartState {
    /// The running station is using it.
    InUse,
    /// It is in place and verifies as far as a poll can tell, and this start
    /// did not load it: installed since, or adopted at the next start.
    Installed,
    /// It is not there.
    Missing,
    /// It is there and is not the pinned file, or the station could not load
    /// it and nobody has hashed it since: an install replaces it.
    Damaged,
    /// It is there and verifies, and the station leaves it out for a reason
    /// no install changes; [`ModelFile::note`] says which and what to do.
    NotUsed,
}

impl PartState {
    /// Whether an install from this page would change it.
    #[must_use]
    pub const fn needs_install(self) -> bool {
        matches!(self, Self::Missing | Self::Damaged)
    }
}

/// One model file the station is set up for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFile {
    /// Its catalogue id, which is also its installed file name.
    pub id: &'static str,
    /// What to call it, as the catalogue gives it.
    pub name: &'static str,
    /// What it is for here.
    pub role: Role,
    /// Where it stands.
    pub state: PartState,
    /// What the station says about it, when that is more than [`Self::state`]:
    /// the reason it is not used, in words that say what to do.
    pub note: Option<String>,
    /// What installing it downloads, in bytes.
    pub bytes: u64,
    /// The directory it is installed in.
    dir: PathBuf,
}

/// How far an install is, for the progress bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallProgress {
    /// Which of the files this is, from 1.
    pub step: usize,
    /// How many files the install fetches.
    pub steps: usize,
    /// The file being fetched.
    pub file: String,
    /// Bytes received across every file so far.
    pub done: u64,
    /// Bytes expected across every file.
    pub total: u64,
}

impl InstallProgress {
    /// Whole percent, never above 99 until the job says it is finished.
    #[must_use]
    pub fn percent(&self) -> u64 {
        if self.total == 0 {
            return 0;
        }
        (self.done.saturating_mul(100) / self.total).min(99)
    }
}

/// Everything the page needs to draw the classifier panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Which of the situations above this is.
    pub phase: Phase,
    /// The primary classifier's name, as the catalogue gives it.
    pub name: String,
    /// What the install button would download, in bytes.
    pub download_bytes: u64,
    /// The file the engine is running, by name, when it is.
    pub in_use: Option<String>,
    /// Every catalogue file the station is set up for, the primary first.
    pub files: Vec<ModelFile>,
    /// What the running station published about its occurrence filter, for
    /// a filter that is not the catalogue's file (and so has no row).
    pub published_filter: FilterState,
    /// Progress, while installing.
    pub progress: Option<InstallProgress>,
    /// Why the last install failed, when it did; why the detection engine did
    /// not start, in [`Phase::DidNotStart`].
    pub failure: Option<String>,
    /// The configured model file that is not there, in the `Configured` phase.
    pub missing_path: Option<String>,
    /// Whether the reader may open the Diagnostics page. `false` for a viewer, who
    /// is refused it, so the panel does not offer a link that answers `403`.
    pub diagnostics_link: bool,
}

impl Snapshot {
    /// The files an install from this page would fetch.
    pub fn to_install(&self) -> impl Iterator<Item = &ModelFile> {
        self.files.iter().filter(|f| f.state.needs_install())
    }

    /// Whether one of them is there and damaged, so the button says "Repair".
    #[must_use]
    pub fn repairs(&self) -> bool {
        self.files.iter().any(|f| f.state == PartState::Damaged)
    }

    /// Whether a file is in place that only a restart puts to use.
    #[must_use]
    pub fn awaits_restart(&self) -> bool {
        self.files.iter().any(|f| f.state == PartState::Installed)
    }

    /// The file an install from this page would fetch to fix the condition
    /// `key` ([`crate::model_conditions::ModelCondition::key`]), when one would.
    #[must_use]
    pub fn fix_for(&self, key: &str) -> Option<&ModelFile> {
        if self.phase == Phase::Installing {
            return None;
        }
        self.to_install().find(|f| f.role.condition_key() == key)
    }

    /// Whether an install from this page fixes the condition `key`.
    #[must_use]
    pub fn installs_fix(&self, key: &str) -> bool {
        self.fix_for(key).is_some()
    }

    /// The occurrence filter's file, when the station is set up for one the
    /// catalogue carries.
    #[must_use]
    pub fn filter(&self) -> Option<&ModelFile> {
        self.files.iter().find(|f| f.role == Role::Filter)
    }
}

impl ClassifierSetup {
    /// Record what this start is doing. Called once, by the binary, after the
    /// detection engine has been started or not.
    pub fn configure(&self, layout: Layout) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.layout = Some(layout);
        }
    }

    /// Where an install would land, if the station has said.
    #[must_use]
    pub fn model_dir(&self) -> Option<PathBuf> {
        self.inner
            .lock()
            .ok()
            .and_then(|i| i.layout.as_ref().map(|l| l.model_dir.clone()))
    }

    /// What the panel should show, given whether the detection engine is
    /// running right now and what the start-up published about its models.
    ///
    /// Reads only the files' verification records ([`Rehash::Never`]): a
    /// poll does not hash 541 MB.
    #[must_use]
    pub fn snapshot(&self, engine_running: bool, models: &ModelsState) -> Snapshot {
        let mut snap = Self::empty(Phase::Unavailable);
        let Ok(inner) = self.inner.lock() else {
            return snap;
        };
        let Some(layout) = inner.layout.as_ref() else {
            return snap;
        };
        snap.files = files(layout, models, &inner.installed);
        snap.published_filter = models.occurrence_filter.clone();
        snap.files
            .iter()
            .find(|f| f.role == Role::Primary)
            .map(|f| f.name)
            .or_else(|| find(DEFAULT_CLASSIFIER_ID).map(|e| e.name))
            .unwrap_or_default()
            .clone_into(&mut snap.name);
        snap.download_bytes = snap.to_install().map(|f| f.bytes).sum();

        if let Some(Job::Running {
            step,
            steps,
            file,
            before,
            done,
            grand_total,
        }) = &inner.job
        {
            snap.phase = Phase::Installing;
            snap.progress = Some(InstallProgress {
                step: *step,
                steps: *steps,
                file: file.clone(),
                done: before + done,
                total: *grand_total,
            });
            return snap;
        }
        if layout.web_only {
            snap.phase = Phase::WebOnly;
            return snap;
        }
        // Before the installed-file checks: a job whose classifier step
        // finished and whose filter step failed is a failed install, and its
        // reason is the one thing the reader needs (MDL-15).
        if let Some(Job::Failed(why)) = &inner.job {
            snap.phase = Phase::Failed;
            snap.failure = Some(why.clone());
            return snap;
        }
        if layout.started {
            snap.in_use = layout.model.as_deref().map(file_name);
            snap.phase = if engine_running {
                Phase::Ready
            } else {
                Phase::Stopped
            };
            return snap;
        }

        // Not started on this start.
        let primary = snap
            .files
            .iter()
            .find(|f| f.role == Role::Primary)
            .map(|f| f.state);
        snap.phase = match (primary, &models.detector) {
            (None, _) => {
                // The operator's own model: the catalogue's would not be what
                // runs, and an install button here would put a file beside one
                // nothing reads.
                if let Some(configured) = layout.model.as_deref()
                    && !configured.is_file()
                {
                    snap.missing_path = Some(configured.display().to_string());
                }
                Phase::Configured
            }
            (Some(s), _) if s.needs_install() => Phase::Missing,
            // Installed from this page since the start: the failure the start
            // met is the one the install was for.
            (Some(_), _) if !inner.installed.is_empty() => Phase::NeedsRestart,
            (Some(_), DetectorState::Failed { why }) => {
                snap.failure = Some(why.clone());
                Phase::DidNotStart
            }
            (Some(_), _) => Phase::NeedsRestart,
        };
        snap
    }

    fn empty(phase: Phase) -> Snapshot {
        Snapshot {
            phase,
            name: String::new(),
            download_bytes: 0,
            in_use: None,
            files: Vec::new(),
            published_filter: FilterState::off(FilterOff::NotConfigured, ""),
            progress: None,
            failure: None,
            missing_path: None,
            diagnostics_link: true,
        }
    }

    /// Start installing every file the station is set up for that is missing
    /// or damaged ([`Snapshot::to_install`]), each into its own directory.
    /// Returns the ids it will fetch, in order.
    ///
    /// `run` fetches one catalogue entry by id; production passes
    /// [`install_from_catalogue`]. Returns at once.
    ///
    /// # Errors
    ///
    /// [`StartError`] when the station has not said where to install, an install
    /// is already running, or nothing it is set up for needs installing.
    pub fn begin_install(
        &self,
        engine_running: bool,
        models: &ModelsState,
        run: InstallFn,
    ) -> Result<Vec<&'static str>, StartError> {
        let snap = self.snapshot(engine_running, models);
        match snap.phase {
            Phase::Unavailable => return Err(StartError::NoLayout),
            Phase::WebOnly => return Err(StartError::WebOnly),
            Phase::Installing => return Err(StartError::AlreadyRunning),
            _ => {}
        }
        let todo: Vec<(&'static str, PathBuf, u64)> = snap
            .to_install()
            .map(|f| (f.id, f.dir.clone(), f.bytes))
            .collect();
        if todo.is_empty() {
            return Err(StartError::NotNeeded);
        }
        let grand_total = todo.iter().map(|(_, _, b)| b).sum::<u64>();
        {
            let mut inner = self.inner.lock().map_err(|_| StartError::NoLayout)?;
            // Checked again under the lock that publishes it: two clicks in the
            // same instant must not both start.
            if matches!(inner.job, Some(Job::Running { .. })) {
                return Err(StartError::AlreadyRunning);
            }
            inner.job = Some(Job::Running {
                step: 1,
                steps: todo.len(),
                file: String::new(),
                before: 0,
                done: 0,
                grand_total,
            });
        }

        let ids: Vec<&'static str> = todo.iter().map(|(id, _, _)| *id).collect();
        let shared = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("classifier-install".to_owned())
            .spawn(move || run_job(&shared, &todo, grand_total, run));
        if let Err(e) = spawned {
            self.inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .job = Some(Job::Failed(format!("could not start the download: {e}")));
            return Err(StartError::Thread);
        }
        Ok(ids)
    }
}

/// The install thread's work: each file in turn, publishing progress, then
/// the result.
fn run_job(
    shared: &Mutex<Inner>,
    todo: &[(&'static str, PathBuf, u64)],
    grand_total: u64,
    run: InstallFn,
) {
    let lock = || {
        shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
    // A panic in the installer (or in a progress callback) must end the job
    // as Failed. Left alone, the unwound thread leaves the job Running for
    // good: the page shows a bar that never moves and every later attempt is
    // refused as "already running" until the station restarts.
    let work = std::panic::AssertUnwindSafe(|| {
        let mut before = 0_u64;
        for (i, (id, dir, bytes)) in todo.iter().enumerate() {
            let step = i + 1;
            let result = run(id, dir, &mut |p: &Progress| {
                lock().job = Some(Job::Running {
                    step,
                    steps: todo.len(),
                    file: p.file.clone(),
                    before,
                    done: p.done,
                    grand_total,
                });
            });
            if let Err(why) = result {
                lock().job = Some(Job::Failed(why));
                return;
            }
            let mut inner = lock();
            inner
                .installed
                .retain(|(done, d)| !(done == id && d == dir));
            inner.installed.push((id, dir.clone()));
            drop(inner);
            before += bytes;
        }
        lock().job = None;
    });
    if std::panic::catch_unwind(work).is_err() {
        tracing::error!("the classifier install thread panicked");
        lock().job = Some(Job::Failed(
            "the download stopped unexpectedly; the station's log has the details. \
             You can try again."
                .to_owned(),
        ));
    }
}

/// A path's file name, for the reader.
fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The catalogue entry whose installed model file is `path`, with the
/// directory it is installed in. An install of that entry into that directory
/// puts the very file the configuration names in place; any other path is the
/// operator's own.
fn catalogue_file_at(path: &Path) -> Option<(&'static CatalogEntry, PathBuf)> {
    let dir = path.parent()?;
    CATALOG
        .iter()
        .find(|e| installed_paths(e, dir).0 == path)
        .map(|e| (e, dir.to_path_buf()))
}

/// Every catalogue file the station is set up for, the primary first, then
/// the further classifiers in the configuration's order, then the filter.
fn files(
    layout: &Layout,
    models: &ModelsState,
    installed: &[(&'static str, PathBuf)],
) -> Vec<ModelFile> {
    let since_start = |id: &str, dir: &Path| installed.iter().any(|(i, d)| *i == id && d == dir);
    // The installers always write `MODEL_PATH` (naming `<model dir>/<id>.onnx`),
    // and a station that names none adopts the default classifier from the
    // model directory at its next start.
    let primary = layout.model.as_deref().map_or_else(
        || find(DEFAULT_CLASSIFIER_ID).map(|e| (e, layout.model_dir.clone())),
        |path| catalogue_file_at(path).filter(|(e, _)| e.usage != Use::RangeFilter),
    );
    let Some((primary, primary_dir)) = primary else {
        return Vec::new();
    };
    let mut out = vec![classifier_file(
        primary,
        &primary_dir,
        Role::Primary,
        models.classifiers.iter().find(|c| c.primary),
        since_start(primary.id, &primary_dir),
    )];
    for c in models.classifiers.iter().filter(|c| !c.primary) {
        if let Some((entry, dir)) =
            catalogue_file_at(&c.model_path).filter(|(e, _)| e.usage != Use::RangeFilter)
        {
            let fresh = since_start(entry.id, &dir);
            out.push(classifier_file(
                entry,
                &dir,
                Role::Second(c.id.clone()),
                Some(c),
                fresh,
            ));
        }
    }
    let filter = occurrence_filter_for(primary.id).and_then(|f| {
        layout.range_filter.as_deref().map_or_else(
            || Some((f, primary_dir.clone())),
            |path| catalogue_file_at(path).filter(|(e, _)| e.id == f.id),
        )
    });
    if let Some((entry, dir)) = filter {
        let fresh = since_start(entry.id, &dir);
        out.push(filter_file(entry, &dir, &models.occurrence_filter, fresh));
    }
    out
}

/// What the verification records say about an entry in `dir`, as far as a
/// poll can tell: `None` when it is in place and nothing says it is damaged,
/// otherwise the state an install changes; and whether the model file is
/// known to verify (rather than not hashed since it changed).
fn on_disk(entry: &CatalogEntry, dir: &Path) -> (Option<PartState>, bool) {
    let state = installed_state(entry, dir, Rehash::Never);
    let found = if state.is_damaged() {
        Some(PartState::Damaged)
    } else if state.is_present() {
        None
    } else {
        Some(PartState::Missing)
    };
    (found, state.model == FileState::Verified)
}

/// A classifier's file and where it stands, from its files and what the
/// station did with it.
fn classifier_file(
    entry: &'static CatalogEntry,
    dir: &Path,
    role: Role,
    published: Option<&ClassifierState>,
    installed_since_start: bool,
) -> ModelFile {
    let (disk, verified) = on_disk(entry, dir);
    let (state, note) = match (disk, published) {
        (Some(state), Some(c)) if matches!(c.status, ClassifierStatus::LeftOut { .. }) => {
            (state, Some(c.message()))
        }
        (Some(state), _) => (state, None),
        (None, _) if installed_since_start => (PartState::Installed, None),
        (None, None) => (PartState::Installed, None),
        (None, Some(c)) => match &c.status {
            ClassifierStatus::Judging { .. } => (PartState::InUse, None),
            ClassifierStatus::LeftOut {
                fault: ClassifierFault::DetectorNotRunning,
                ..
            } => (PartState::Installed, None),
            // The model file failed to load and nobody has hashed it since it
            // changed: an install re-hashes it and replaces it if it is not
            // the pinned file. One known to be the pinned file would come back
            // the same, and the reason is all there is to say. A labels fault
            // with the catalogue's labels in place (the poll found them) is a
            // labels file the configuration names elsewhere, which no install
            // touches.
            ClassifierStatus::LeftOut {
                fault: ClassifierFault::FileMissing | ClassifierFault::WillNotLoad,
                ..
            } if !verified => (PartState::Damaged, Some(c.message())),
            ClassifierStatus::Unrouted { .. } | ClassifierStatus::LeftOut { .. } => {
                (PartState::NotUsed, Some(c.message()))
            }
        },
    };
    ModelFile {
        id: entry.id,
        name: entry.name,
        role,
        state,
        note,
        bytes: entry.model_bytes,
        dir: dir.to_path_buf(),
    }
}

/// The occurrence filter's file and where it stands.
fn filter_file(
    entry: &'static CatalogEntry,
    dir: &Path,
    published: &FilterState,
    installed_since_start: bool,
) -> ModelFile {
    let (disk, verified) = on_disk(entry, dir);
    let (state, note) = match (disk, published) {
        (
            Some(state),
            FilterState::Off {
                reason: FilterOff::NotConfigured | FilterOff::DetectorNotRunning,
                ..
            }
            | FilterState::Active { .. },
        ) => (state, None),
        (Some(state), FilterState::Off { .. }) => (state, Some(published.message())),
        (None, _) if installed_since_start => (PartState::Installed, None),
        (None, FilterState::Active { .. }) => (PartState::InUse, None),
        (
            None,
            FilterState::Off {
                reason: FilterOff::NotConfigured | FilterOff::DetectorNotRunning,
                ..
            },
        ) => (PartState::Installed, None),
        // As for a classifier: only the model file, and only while unhashed.
        (
            None,
            FilterState::Off {
                reason: FilterOff::ModelMissing | FilterOff::ModelUnusable,
                ..
            },
        ) if !verified => (PartState::Damaged, Some(published.message())),
        (None, FilterState::Off { .. }) => (PartState::NotUsed, Some(published.message())),
    };
    ModelFile {
        id: entry.id,
        name: entry.name,
        role: Role::Filter,
        state,
        note,
        bytes: entry.model_bytes,
        dir: dir.to_path_buf(),
    }
}

/// Why an install did not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    /// The station has not said where classifiers go.
    NoLayout,
    /// This process runs web pages only.
    WebOnly,
    /// Everything the station is set up for is installed, or the
    /// configuration names models the catalogue does not install.
    NotNeeded,
    /// An install is already in flight.
    AlreadyRunning,
    /// The worker thread could not be started.
    Thread,
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_integrations::model_catalog::{
        DEFAULT_RANGE_FILTER_ID, V3_CLASSIFIER_ID, V3_GEOMODEL_ID,
    };
    use std::time::{Duration, Instant};

    /// Put an entry's files in `dir`, the right size, with no verification
    /// record: what a poll reads as in place and not hashed since.
    fn put(dir: &Path, id: &str) {
        let e = find(id).expect("entry");
        let (model, labels) = installed_paths(e, dir);
        std::fs::File::create(&model)
            .and_then(|f| f.set_len(e.model_bytes))
            .expect("model");
        if let Some(l) = labels {
            match e.labels_embedded {
                Some(text) => std::fs::write(l, text).expect("labels"),
                None => std::fs::File::create(&l)
                    .and_then(|f| f.set_len(e.labels_bytes))
                    .expect("labels"),
            }
        }
    }

    fn put_default(dir: &Path) {
        put(dir, DEFAULT_CLASSIFIER_ID);
        put(dir, DEFAULT_RANGE_FILTER_ID);
    }

    fn setup(layout: Layout) -> ClassifierSetup {
        let s = ClassifierSetup::default();
        s.configure(layout);
        s
    }

    fn layout(dir: &Path) -> Layout {
        Layout {
            model_dir: dir.to_path_buf(),
            ..Layout::default()
        }
    }

    fn none() -> ModelsState {
        ModelsState::default()
    }

    fn classifier(
        id: &str,
        primary: bool,
        path: PathBuf,
        status: ClassifierStatus,
    ) -> ClassifierState {
        ClassifierState {
            id: id.to_owned(),
            primary,
            model_path: path,
            labels_path: None,
            status,
        }
    }

    fn judging() -> ClassifierStatus {
        ClassifierStatus::Judging {
            sources: Vec::new(),
            unrouted_sources: true,
        }
    }

    /// A station running `primary` with the given filter state.
    fn running(primary: PathBuf, filter: FilterState) -> ModelsState {
        ModelsState {
            detector: DetectorState::Running,
            classifiers: vec![classifier("birdnet", true, primary, judging())],
            occurrence_filter: filter,
        }
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn ids(snap: &Snapshot) -> Vec<(&'static str, PartState)> {
        snap.files.iter().map(|f| (f.id, f.state)).collect()
    }

    /// A process that never said where classifiers go has nothing to report.
    #[test]
    fn an_unconfigured_setup_is_unavailable() {
        let s = ClassifierSetup::default();
        assert_eq!(s.snapshot(false, &none()).phase, Phase::Unavailable);
        assert_eq!(
            s.begin_install(false, &none(), |_, _, _| Ok(())),
            Err(StartError::NoLayout)
        );
    }

    /// The phases, one per situation a person can be in. The two that look
    /// alike — files installed but not loaded, and loaded and running — are
    /// told apart by the engine, not by the files.
    #[test]
    fn each_situation_has_its_own_phase() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Nothing installed, nothing configured, engine not started.
        let s = setup(layout(dir.path()));
        let snap = s.snapshot(false, &none());
        assert_eq!(snap.phase, Phase::Missing);
        assert_eq!(
            ids(&snap),
            [
                (DEFAULT_CLASSIFIER_ID, PartState::Missing),
                (DEFAULT_RANGE_FILTER_ID, PartState::Missing)
            ]
        );

        // Files in place but this start did not load them.
        put_default(dir.path());
        let snap = setup(layout(dir.path())).snapshot(false, &none());
        assert_eq!(snap.phase, Phase::NeedsRestart);
        assert!(snap.in_use.is_none(), "nothing is in use yet");
        assert!(snap.to_install().next().is_none());

        // The engine started with them, the filter on.
        let model = dir.path().join("birdnet-v24.onnx");
        let started = Layout {
            model: Some(model.clone()),
            range_filter: Some(dir.path().join("birdnet-v24-range-v2.onnx")),
            started: true,
            ..layout(dir.path())
        };
        let on = running(
            model,
            FilterState::Active {
                candidates: Some(137),
            },
        );
        let snap = setup(started.clone()).snapshot(true, &on);
        assert_eq!(snap.phase, Phase::Ready);
        assert_eq!(snap.in_use.as_deref(), Some("birdnet-v24.onnx"));
        assert_eq!(
            ids(&snap),
            [
                (DEFAULT_CLASSIFIER_ID, PartState::InUse),
                (DEFAULT_RANGE_FILTER_ID, PartState::InUse)
            ]
        );

        // The same start, with the engine since stopped: not "ready".
        assert_eq!(setup(started).snapshot(false, &on).phase, Phase::Stopped);

        // A model the operator named that is not there.
        let own = Layout {
            model: Some(dir.path().join("own.onnx")),
            ..layout(dir.path())
        };
        let snap = setup(own).snapshot(false, &none());
        assert_eq!(snap.phase, Phase::Configured);
        assert!(snap.missing_path.is_some_and(|p| p.ends_with("own.onnx")));
        assert!(
            snap.files.is_empty(),
            "nothing of the catalogue's is offered"
        );

        // Pages only.
        let web_only = Layout {
            web_only: true,
            ..layout(dir.path())
        };
        assert_eq!(
            setup(web_only).snapshot(false, &none()).phase,
            Phase::WebOnly
        );
    }

    /// MDL-04/05: the panel's filter reads what the station did with it, not
    /// whether the configuration names it. A configured filter that is off
    /// for want of coordinates is not "in use", and says why.
    #[test]
    fn the_filter_is_in_use_only_when_the_station_says_it_is_filtering() {
        let dir = tempfile::tempdir().expect("tempdir");
        put_default(dir.path());
        let model = dir.path().join("birdnet-v24.onnx");
        let started = Layout {
            model: Some(model.clone()),
            range_filter: Some(dir.path().join("birdnet-v24-range-v2.onnx")),
            started: true,
            ..layout(dir.path())
        };
        let off = running(
            model,
            FilterState::off(
                FilterOff::NoCoordinates,
                "the station has no position; set LATITUDE and LONGITUDE",
            ),
        );
        let snap = setup(started).snapshot(true, &off);
        let filter = snap.filter().expect("the filter's row");
        assert_eq!(filter.state, PartState::NotUsed);
        assert!(
            filter
                .note
                .as_deref()
                .is_some_and(|n| n.contains("set LATITUDE and LONGITUDE")),
            "{filter:?}"
        );
        assert!(snap.to_install().next().is_none(), "no install fixes that");
    }

    /// MDL-15: a V3.0 station is offered V3.0 and its geomodel — never V2.4
    /// — and a damaged file is offered for repair.
    #[test]
    fn a_station_is_offered_the_models_it_is_set_up_for() {
        let dir = tempfile::tempdir().expect("tempdir");
        let v3 = dir.path().join(format!("{V3_CLASSIFIER_ID}.onnx"));
        let v3_station = || Layout {
            model: Some(v3.clone()),
            ..layout(dir.path())
        };

        // Nothing there: V3.0 and the geomodel, and nothing of V2.4's.
        let snap = setup(v3_station()).snapshot(false, &none());
        assert_eq!(snap.phase, Phase::Missing);
        assert_eq!(
            ids(&snap),
            [
                (V3_CLASSIFIER_ID, PartState::Missing),
                (V3_GEOMODEL_ID, PartState::Missing)
            ]
        );
        let v3_bytes = find(V3_CLASSIFIER_ID).expect("v3").model_bytes
            + find(V3_GEOMODEL_ID).expect("geo").model_bytes;
        assert_eq!(snap.download_bytes, v3_bytes);

        // The classifier there and the wrong size: damaged, repaired.
        put(dir.path(), V3_CLASSIFIER_ID);
        std::fs::File::options()
            .write(true)
            .open(&v3)
            .and_then(|f| f.set_len(1000))
            .expect("truncate");
        let snap = setup(v3_station()).snapshot(false, &none());
        assert_eq!(ids(&snap)[0], (V3_CLASSIFIER_ID, PartState::Damaged));
        assert!(snap.repairs());

        // The geomodel there and its labels missing: the filter is offered.
        put(dir.path(), V3_CLASSIFIER_ID);
        put(dir.path(), V3_GEOMODEL_ID);
        let geo_labels = installed_paths(find(V3_GEOMODEL_ID).expect("geo"), dir.path())
            .1
            .expect("the geomodel ships labels");
        std::fs::remove_file(&geo_labels).expect("remove labels");
        let configured = Layout {
            model: Some(v3.clone()),
            range_filter: Some(dir.path().join(format!("{V3_GEOMODEL_ID}.onnx"))),
            started: true,
            ..layout(dir.path())
        };
        let published = ModelsState {
            detector: DetectorState::Running,
            classifiers: vec![classifier("birdnet", true, v3, judging())],
            occurrence_filter: FilterState::off(
                FilterOff::LabelsMissing,
                "its labels file is not there",
            ),
        };
        let snap = setup(configured).snapshot(true, &published);
        assert_eq!(snap.phase, Phase::Ready);
        assert_eq!(
            ids(&snap),
            [
                (V3_CLASSIFIER_ID, PartState::InUse),
                (V3_GEOMODEL_ID, PartState::Missing)
            ]
        );
        assert!(snap.installs_fix(crate::model_conditions::FILTER_KEY));
    }

    /// A second classifier the catalogue carries is offered when it is left
    /// out for its file, and its own reasons are said when no install helps.
    #[test]
    fn a_second_classifier_is_offered_for_repair_when_its_file_is_at_fault() {
        let dir = tempfile::tempdir().expect("tempdir");
        put_default(dir.path());
        let v24 = dir.path().join("birdnet-v24.onnx");
        let perch = dir.path().join("perch-v2.onnx");
        let started = Layout {
            model: Some(v24.clone()),
            range_filter: Some(dir.path().join("birdnet-v24-range-v2.onnx")),
            started: true,
            ..layout(dir.path())
        };
        let with_perch = |status| ModelsState {
            detector: DetectorState::Running,
            classifiers: vec![
                classifier("birdnet", true, v24.clone(), judging()),
                classifier("perch", false, perch.clone(), status),
            ],
            occurrence_filter: FilterState::Active { candidates: None },
        };

        // Missing.
        let snap = setup(started.clone()).snapshot(
            true,
            &with_perch(ClassifierStatus::left_out(
                ClassifierFault::FileMissing,
                "not there",
            )),
        );
        assert_eq!(snap.files[1].id, "perch-v2");
        assert_eq!(snap.files[1].role, Role::Second("perch".to_owned()));
        assert_eq!(snap.files[1].state, PartState::Missing);
        assert!(snap.installs_fix("models:classifier:perch"));

        // There, unhashed, and it would not load: repair.
        put(dir.path(), "perch-v2");
        let snap = setup(started.clone()).snapshot(
            true,
            &with_perch(ClassifierStatus::left_out(
                ClassifierFault::WillNotLoad,
                "will not load",
            )),
        );
        assert_eq!(snap.files[1].state, PartState::Damaged);

        // Too big for the memory allowed: no install changes that.
        let snap = setup(started.clone()).snapshot(
            true,
            &with_perch(ClassifierStatus::left_out(
                ClassifierFault::DoesNotFit,
                "it needs 646 MiB",
            )),
        );
        assert_eq!(snap.files[1].state, PartState::NotUsed);
        assert!(
            snap.files[1]
                .note
                .as_deref()
                .is_some_and(|n| n.contains("646 MiB"))
        );
        assert!(!snap.installs_fix("models:classifier:perch"));

        // Loaded and unrouted: not used, with the route that reaches it.
        let snap = setup(started).snapshot(
            true,
            &with_perch(ClassifierStatus::Unrouted {
                primary: "birdnet".to_owned(),
            }),
        );
        assert_eq!(snap.files[1].state, PartState::NotUsed);
        assert!(
            snap.files[1]
                .note
                .as_deref()
                .is_some_and(|n| n.contains("MODEL_ROUTES"))
        );
    }

    /// A labels fault with the catalogue's labels in place is a labels file
    /// the configuration names elsewhere: no install fixes it, so none is
    /// offered and the reason is said. Seen on a real station whose
    /// `LABELS_PATH` named V3.0's labels beside V2.4, which the panel first
    /// offered to "repair" with a download that would have changed nothing.
    #[test]
    fn a_labels_file_named_elsewhere_is_not_offered_for_repair() {
        let dir = tempfile::tempdir().expect("tempdir");
        put_default(dir.path());
        let model = dir.path().join("birdnet-v24.onnx");
        let mut failed = ModelsState {
            classifiers: vec![classifier(
                "birdnet",
                true,
                model.clone(),
                ClassifierStatus::left_out(
                    ClassifierFault::LabelsWrongLength,
                    "its model scores 6522 classes and its labels file names 11560",
                ),
            )],
            ..none()
        };
        failed.set_detector(DetectorState::Failed {
            why: "its labels file names 11560".to_owned(),
        });
        let named = Layout {
            model: Some(model),
            ..layout(dir.path())
        };
        let snap = setup(named).snapshot(false, &failed);
        assert_eq!(snap.files[0].state, PartState::NotUsed, "{:?}", snap.files);
        assert!(snap.to_install().next().is_none());
        assert_eq!(snap.phase, Phase::DidNotStart);
        assert!(!snap.installs_fix(crate::model_conditions::DETECTOR_KEY));
    }

    /// An install refused where it would change nothing, with the reason.
    #[test]
    fn an_install_is_refused_where_it_would_change_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ok = |_: &str, _: &Path, _: &mut dyn FnMut(&Progress)| Ok(());
        put_default(dir.path());
        let model = dir.path().join("birdnet-v24.onnx");
        let started = Layout {
            model: Some(model.clone()),
            range_filter: Some(dir.path().join("birdnet-v24-range-v2.onnx")),
            started: true,
            ..layout(dir.path())
        };
        let on = running(model, FilterState::Active { candidates: None });
        assert_eq!(
            setup(started).begin_install(true, &on, ok),
            Err(StartError::NotNeeded)
        );
        let named = Layout {
            model: Some(dir.path().join("own.onnx")),
            ..layout(dir.path())
        };
        assert_eq!(
            setup(named).begin_install(false, &none(), ok),
            Err(StartError::NotNeeded)
        );
        let web = Layout {
            web_only: true,
            ..layout(dir.path())
        };
        assert_eq!(
            setup(web).begin_install(false, &none(), ok),
            Err(StartError::WebOnly)
        );
    }

    /// The installers always write `MODEL_PATH`, naming the place the default
    /// classifier installs to. When its download failed, that is a station with
    /// a configured model and no file, and it read as the operator's own model:
    /// no install button, and advice to put a file there by hand (OOB-20).
    #[test]
    fn a_configured_path_that_is_the_default_install_is_the_default_classifier() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ok = |_: &str, _: &Path, _: &mut dyn FnMut(&Progress)| Ok(());
        let default_path = dir.path().join(format!("{DEFAULT_CLASSIFIER_ID}.onnx"));
        let named = || Layout {
            model: Some(default_path.clone()),
            ..layout(dir.path())
        };

        // Not there yet: offered, and installable.
        let snap = setup(named()).snapshot(false, &none());
        assert_eq!(snap.phase, Phase::Missing);
        assert!(snap.missing_path.is_none());
        assert_eq!(
            setup(named()).begin_install(false, &none(), ok),
            Ok(vec![DEFAULT_CLASSIFIER_ID, DEFAULT_RANGE_FILTER_ID])
        );

        // There, and this start did not load it.
        put_default(dir.path());
        assert_eq!(
            setup(named()).snapshot(false, &none()).phase,
            Phase::NeedsRestart
        );

        // The same file name in another directory is the catalogue's model
        // there: an install into that directory puts the configured file in
        // place, so that is where it goes.
        let other = dir.path().join("other");
        std::fs::create_dir_all(&other).expect("other");
        let elsewhere = Layout {
            model: Some(other.join(format!("{DEFAULT_CLASSIFIER_ID}.onnx"))),
            ..layout(dir.path())
        };
        let snap = setup(elsewhere).snapshot(false, &none());
        assert_eq!(snap.phase, Phase::Missing);
        assert!(
            snap.files.iter().all(|f| f.dir == other),
            "{:?}",
            snap.files
        );
    }

    /// A station whose files are in place and whose detector did not start
    /// says why, instead of "restart" for ever.
    #[test]
    fn files_in_place_and_a_detector_that_failed_say_why() {
        let dir = tempfile::tempdir().expect("tempdir");
        put_default(dir.path());
        let mut failed = none();
        failed.set_detector(DetectorState::Failed {
            why: "a route names a classifier that does not exist".to_owned(),
        });
        let snap = setup(layout(dir.path())).snapshot(false, &failed);
        assert_eq!(snap.phase, Phase::DidNotStart);
        assert!(
            snap.failure
                .as_deref()
                .is_some_and(|f| f.contains("does not exist"))
        );
    }

    /// The whole job, against a stand-in installer: every file is fetched in
    /// order, progress is visible while the first is in flight, and the result
    /// is "installed, restart" once the files exist.
    #[test]
    fn an_install_runs_every_file_and_reports_while_it_does() {
        use std::sync::mpsc;
        static HOLD: Mutex<Option<mpsc::Receiver<()>>> = Mutex::new(None);
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

        fn stand_in(
            id: &str,
            dest: &Path,
            progress: &mut dyn FnMut(&Progress),
        ) -> Result<(), String> {
            SEEN.lock().expect("seen").push(id.to_owned());
            let e = find(id).expect("entry");
            progress(&Progress {
                file: format!("{id}.onnx"),
                done: e.model_bytes / 2,
                total: e.model_bytes,
            });
            if id == DEFAULT_CLASSIFIER_ID
                && let Some(gate) = HOLD.lock().expect("hold").take()
            {
                // Wait here, with the first file half fetched, until released.
                let _ = gate.recv_timeout(Duration::from_secs(10));
            }
            let (model, labels) = installed_paths(e, dest);
            std::fs::File::create(&model)
                .and_then(|f| f.set_len(e.model_bytes))
                .map_err(|e| e.to_string())?;
            if let Some(l) = labels {
                std::fs::write(l, e.labels_embedded.unwrap_or_default())
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let s = setup(layout(dir.path()));
        let (release, gate) = mpsc::channel::<()>();
        *HOLD.lock().expect("hold") = Some(gate);
        SEEN.lock().expect("seen").clear();

        s.begin_install(false, &none(), stand_in).expect("starts");
        wait_until("the first file to be half fetched", || {
            s.snapshot(false, &none())
                .progress
                .is_some_and(|p| p.done > 0)
        });
        let mid = s.snapshot(false, &none());
        assert_eq!(mid.phase, Phase::Installing);
        let p = mid.progress.expect("progress");
        assert_eq!((p.step, p.steps), (1, 2));
        assert!(p.percent() > 0 && p.percent() < 99, "{p:?}");
        assert_eq!(
            s.begin_install(false, &none(), stand_in),
            Err(StartError::AlreadyRunning),
            "a second download beside the first would race on the .part file"
        );

        release.send(()).expect("release");
        wait_until("the install to finish", || {
            s.snapshot(false, &none()).phase == Phase::NeedsRestart
        });
        assert_eq!(
            *SEEN.lock().expect("seen"),
            [DEFAULT_CLASSIFIER_ID, DEFAULT_RANGE_FILTER_ID],
            "the classifier first, then its range filter"
        );
    }

    /// MDL-14, the dashboard half: a filter installed beside a running
    /// station is "installed, restart", not "on", until a start has loaded
    /// it — the running station decided its filter when it started.
    #[test]
    fn a_filter_installed_beside_a_running_station_waits_for_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        put(dir.path(), DEFAULT_CLASSIFIER_ID);
        let model = dir.path().join("birdnet-v24.onnx");
        let started = Layout {
            model: Some(model.clone()),
            started: true,
            ..layout(dir.path())
        };
        let unfiltered = running(
            model,
            FilterState::off(FilterOff::NotConfigured, "none is configured"),
        );
        let s = setup(started);
        let snap = s.snapshot(true, &unfiltered);
        assert_eq!(
            ids(&snap),
            [
                (DEFAULT_CLASSIFIER_ID, PartState::InUse),
                (DEFAULT_RANGE_FILTER_ID, PartState::Missing)
            ]
        );
        assert_eq!(
            s.begin_install(true, &unfiltered, |id, dest, _| {
                put(dest, id);
                Ok(())
            }),
            Ok(vec![DEFAULT_RANGE_FILTER_ID])
        );
        // The file is in place before the job that put it there has ended, so
        // the wait is for both: on a slow machine the first alone was true
        // while the phase still read `Installing`.
        wait_until("the install", || {
            let snap = s.snapshot(true, &unfiltered);
            snap.awaits_restart() && snap.phase == Phase::Ready
        });
        let snap = s.snapshot(true, &unfiltered);
        assert_eq!(snap.phase, Phase::Ready);
        assert_eq!(snap.filter().map(|f| f.state), Some(PartState::Installed));
    }

    /// A failed step stops the job, is reported with its reason — also when
    /// the classifier before it was installed — and leaves the way open to
    /// try again.
    #[test]
    fn a_failed_install_says_why_and_can_be_retried() {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = setup(layout(dir.path()));
        s.begin_install(false, &none(), |id, dest, _| {
            if id == DEFAULT_CLASSIFIER_ID {
                put(dest, id);
                Ok(())
            } else {
                Err("download failed: no route to host".to_owned())
            }
        })
        .expect("starts");
        wait_until("the failure", || {
            s.snapshot(false, &none()).phase == Phase::Failed
        });
        let snap = s.snapshot(false, &none());
        assert_eq!(
            snap.failure.as_deref(),
            Some("download failed: no route to host"),
            "the classifier step succeeded; the filter's failure is still said"
        );
        assert_eq!(
            ids(&snap)[1],
            (DEFAULT_RANGE_FILTER_ID, PartState::Missing),
            "and the retry fetches only what is missing"
        );
        // Another try is allowed, and clears the failure while it runs.
        s.begin_install(false, &none(), |id, dest, _| {
            put(dest, id);
            Ok(())
        })
        .expect("a retry starts");
        wait_until("the retry", || {
            s.snapshot(false, &none()).phase == Phase::NeedsRestart
        });
        assert!(s.snapshot(false, &none()).failure.is_none());
    }

    /// An installer that panics ends the job as Failed, and a retry is allowed.
    #[test]
    fn a_panicking_install_is_a_failed_install_not_a_running_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = setup(layout(dir.path()));
        s.begin_install(false, &none(), |_, _, _| panic!("boom"))
            .expect("starts");
        wait_until("the failure", || {
            s.snapshot(false, &none()).phase == Phase::Failed
        });
        assert!(
            s.snapshot(false, &none())
                .failure
                .as_deref()
                .is_some_and(|f| f.contains("stopped unexpectedly"))
        );
        s.begin_install(false, &none(), |id, dest, _| {
            put(dest, id);
            Ok(())
        })
        .expect("not refused as already running");
        wait_until("the retry", || {
            s.snapshot(false, &none()).phase == Phase::NeedsRestart
        });
    }
}
