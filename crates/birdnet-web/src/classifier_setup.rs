//! The classifier a station listens with, as a browser sees it: ready, missing,
//! being installed, or installed and waiting for a restart.
//!
//! # Why this exists
//!
//! The detection engine needs a species classifier (a 54 MB neural network), a
//! labels file and, to be useful, a species-occurrence ("range") filter. A
//! station installed with `install.sh` or the Docker image has them before it
//! first runs. A station that was just *downloaded* has none, and the only way
//! to get them was `birdnet-behavior --install-model birdnet-v24` in a terminal,
//! after which the log said "configure --model, --labels, --watch-dir … and
//! restart". The setup wizard reported nothing about it at all.
//!
//! This module is the state the browser needs: what is installed, what the
//! running station is using, and whether an install is in flight. It does not
//! decide what the daemon runs (that is [`birdnet_integrations::model_catalog`]
//! and the binary's start-up) and it does not restart anything.
//!
//! # What "ready" means
//!
//! Only the detection engine being up *with a classifier* is "ready". Files on
//! disk are "installed, restart to use them": a station that has not loaded a
//! model detects nothing, and a page that says "installed" in green would be
//! the wizard's old mistake again — a confident yes about something that is not
//! happening.
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

use birdnet_integrations::model_catalog::{
    CatalogEntry, DEFAULT_CLASSIFIER_ID, DEFAULT_RANGE_FILTER_ID, Progress, find, looks_installed,
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
    Idle,
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
}

/// The shared setup state: written once at start-up, read by every request.
#[derive(Debug, Default)]
pub struct ClassifierSetup {
    /// An `Arc` so the install thread, which must be `'static`, can write its
    /// progress where the requests read it.
    inner: Arc<Mutex<Inner>>,
}

/// One of the five things the page can say about the classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No station layout: a process that does not run a detection engine (a
    /// test, a tool).
    Unavailable,
    /// Nothing usable is installed. An install can fix this.
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
    /// This process runs the web pages only.
    WebOnly,
    /// A classifier is named in the configuration and the engine did not start
    /// with it, so an install of the default would not be what runs.
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
            Self::WebOnly => "web-only",
            Self::Configured => "configured",
        }
    }
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
    /// The classifier's name, as the catalogue gives it.
    pub name: String,
    /// What the install would download, in bytes, for the button.
    pub download_bytes: u64,
    /// The file the engine is running, by name, when it is.
    pub in_use: Option<String>,
    /// Whether the occurrence filter is in use (when ready) or installed.
    pub range_filter: bool,
    /// Progress, while installing.
    pub progress: Option<InstallProgress>,
    /// Why the last install failed, when it did.
    pub failure: Option<String>,
    /// The configured model file that is not there, in the `Configured` phase.
    pub missing_path: Option<String>,
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

    /// The files an install of the default classifier fetches, in order.
    fn default_entries() -> Option<(&'static CatalogEntry, &'static CatalogEntry)> {
        Some((find(DEFAULT_CLASSIFIER_ID)?, find(DEFAULT_RANGE_FILTER_ID)?))
    }

    /// What the panel should show, given whether the detection engine is
    /// running right now.
    #[must_use]
    pub fn snapshot(&self, engine_running: bool) -> Snapshot {
        let Some((model, range)) = Self::default_entries() else {
            return Self::empty(Phase::Unavailable);
        };
        let mut snap = Snapshot {
            phase: Phase::Unavailable,
            name: model.name.to_owned(),
            download_bytes: model.model_bytes + range.model_bytes,
            in_use: None,
            range_filter: false,
            progress: None,
            failure: None,
            missing_path: None,
        };
        let Ok(inner) = self.inner.lock() else {
            return snap;
        };
        let Some(layout) = inner.layout.as_ref() else {
            return snap;
        };

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

        let file_name = |p: &Path| {
            p.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        };
        let installed = looks_installed(model, &layout.model_dir);
        let range_installed = looks_installed(range, &layout.model_dir);

        if layout.web_only {
            snap.phase = Phase::WebOnly;
            return snap;
        }
        if layout.started {
            snap.in_use = layout.model.as_deref().map(file_name);
            snap.range_filter = layout.range_filter.is_some();
            snap.phase = if engine_running {
                Phase::Ready
            } else {
                Phase::Stopped
            };
            return snap;
        }

        // Not started on this start.
        snap.range_filter = range_installed;
        if let Some(configured) = layout.model.as_deref() {
            // The operator's own model: the default would not be what runs, and
            // an install button here would put 84 MB beside a file nothing reads.
            if !configured.is_file() {
                snap.missing_path = Some(configured.display().to_string());
            }
            snap.phase = Phase::Configured;
            return snap;
        }
        if installed {
            snap.phase = Phase::NeedsRestart;
        } else if let Some(Job::Failed(why)) = &inner.job {
            snap.phase = Phase::Failed;
            snap.failure = Some(why.clone());
        } else {
            snap.phase = Phase::Missing;
        }
        snap
    }

    const fn empty(phase: Phase) -> Snapshot {
        Snapshot {
            phase,
            name: String::new(),
            download_bytes: 0,
            in_use: None,
            range_filter: false,
            progress: None,
            failure: None,
            missing_path: None,
        }
    }

    /// Start installing the default classifier and its range filter.
    ///
    /// `run` fetches one catalogue entry by id; production passes
    /// [`install_from_catalogue`]. Returns at once.
    ///
    /// # Errors
    ///
    /// [`StartError`] when the station has not said where to install, an install
    /// is already running, or the classifier is already in use or named by the
    /// configuration (installing then would not change what runs).
    pub fn begin_install(&self, run: InstallFn) -> Result<(), StartError> {
        let (model, range) = Self::default_entries().ok_or(StartError::NoCatalogue)?;
        let dest = {
            let mut inner = self.inner.lock().map_err(|_| StartError::NoLayout)?;
            let layout = inner.layout.as_ref().ok_or(StartError::NoLayout)?;
            if layout.web_only {
                return Err(StartError::WebOnly);
            }
            if layout.started || layout.model.is_some() {
                return Err(StartError::NotNeeded);
            }
            if matches!(inner.job, Some(Job::Running { .. })) {
                return Err(StartError::AlreadyRunning);
            }
            let dest = layout.model_dir.clone();
            let grand_total = model.model_bytes + range.model_bytes;
            inner.job = Some(Job::Running {
                step: 1,
                steps: 2,
                file: String::new(),
                before: 0,
                done: 0,
                grand_total,
            });
            dest
        };

        let ids = [model, range];
        let shared = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("classifier-install".to_owned())
            .spawn(move || {
                let publish = |job: Job| {
                    if let Ok(mut inner) = shared.lock() {
                        inner.job = Some(job);
                    }
                };
                let grand_total = ids.iter().map(|e| e.model_bytes).sum::<u64>();
                let mut before = 0_u64;
                for (i, entry) in ids.iter().enumerate() {
                    let step = i + 1;
                    let result = run(entry.id, &dest, &mut |p: &Progress| {
                        publish(Job::Running {
                            step,
                            steps: ids.len(),
                            file: p.file.clone(),
                            before,
                            done: p.done,
                            grand_total,
                        });
                    });
                    if let Err(why) = result {
                        publish(Job::Failed(why));
                        return;
                    }
                    before += entry.model_bytes;
                }
                publish(Job::Idle);
            });
        if let Err(e) = spawned {
            self.set_job(Job::Failed(format!("could not start the download: {e}")));
            return Err(StartError::Thread);
        }
        Ok(())
    }

    fn set_job(&self, job: Job) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.job = Some(job);
        }
    }
}

/// Why an install did not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    /// The catalogue does not carry the default classifier.
    NoCatalogue,
    /// The station has not said where classifiers go.
    NoLayout,
    /// This process runs web pages only.
    WebOnly,
    /// The classifier is already running, or the configuration names another.
    NotNeeded,
    /// An install is already in flight.
    AlreadyRunning,
    /// The worker thread could not be started.
    Thread,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn put_default(dir: &Path) {
        for id in [DEFAULT_CLASSIFIER_ID, DEFAULT_RANGE_FILTER_ID] {
            let e = find(id).expect("entry");
            let (model, labels) = birdnet_integrations::model_catalog::installed_paths(e, dir);
            std::fs::File::create(&model)
                .and_then(|f| f.set_len(e.model_bytes))
                .expect("model");
            if let Some(l) = labels {
                std::fs::write(l, "x\n").expect("labels");
            }
        }
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

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A process that never said where classifiers go has nothing to report.
    #[test]
    fn an_unconfigured_setup_is_unavailable() {
        let s = ClassifierSetup::default();
        assert_eq!(s.snapshot(false).phase, Phase::Unavailable);
        assert_eq!(s.begin_install(|_, _, _| Ok(())), Err(StartError::NoLayout));
    }

    /// The phases, one per situation a person can be in. The two that look
    /// alike — files installed but not loaded, and loaded and running — are
    /// told apart by the engine, not by the files.
    #[test]
    fn each_situation_has_its_own_phase() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Nothing installed, nothing configured, engine not started.
        let s = setup(layout(dir.path()));
        assert_eq!(s.snapshot(false).phase, Phase::Missing);

        // Files in place but this start did not load them.
        put_default(dir.path());
        let snap = setup(layout(dir.path())).snapshot(false);
        assert_eq!(snap.phase, Phase::NeedsRestart);
        assert!(snap.range_filter, "the filter file is installed");
        assert!(snap.in_use.is_none(), "nothing is in use yet");

        // The engine started with them.
        let model = dir.path().join("birdnet-v24.onnx");
        let started = Layout {
            model: Some(model),
            range_filter: Some(dir.path().join("birdnet-v24-range-v2.onnx")),
            started: true,
            ..layout(dir.path())
        };
        let snap = setup(started.clone()).snapshot(true);
        assert_eq!(snap.phase, Phase::Ready);
        assert_eq!(snap.in_use.as_deref(), Some("birdnet-v24.onnx"));
        assert!(snap.range_filter);

        // The same start, with the engine since stopped: not "ready".
        assert_eq!(setup(started).snapshot(false).phase, Phase::Stopped);

        // A model the operator named that is not there.
        let own = Layout {
            model: Some(dir.path().join("own.onnx")),
            ..layout(dir.path())
        };
        let snap = setup(own).snapshot(false);
        assert_eq!(snap.phase, Phase::Configured);
        assert!(snap.missing_path.is_some_and(|p| p.ends_with("own.onnx")));

        // Pages only.
        let web_only = Layout {
            web_only: true,
            ..layout(dir.path())
        };
        assert_eq!(setup(web_only).snapshot(false).phase, Phase::WebOnly);
    }

    /// An install refused where it would change nothing, with the reason.
    #[test]
    fn an_install_is_refused_where_it_would_change_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ok = |_: &str, _: &Path, _: &mut dyn FnMut(&Progress)| Ok(());
        let started = Layout {
            started: true,
            ..layout(dir.path())
        };
        assert_eq!(setup(started).begin_install(ok), Err(StartError::NotNeeded));
        let named = Layout {
            model: Some(dir.path().join("own.onnx")),
            ..layout(dir.path())
        };
        assert_eq!(setup(named).begin_install(ok), Err(StartError::NotNeeded));
        let web = Layout {
            web_only: true,
            ..layout(dir.path())
        };
        assert_eq!(setup(web).begin_install(ok), Err(StartError::WebOnly));
    }

    /// The whole job, against a stand-in installer: both files are fetched in
    /// order, progress is visible while the first is in flight, and the result
    /// is "installed, restart" once the files exist.
    #[test]
    fn an_install_runs_both_files_and_reports_while_it_does() {
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
            let (model, labels) = birdnet_integrations::model_catalog::installed_paths(e, dest);
            std::fs::File::create(&model)
                .and_then(|f| f.set_len(e.model_bytes))
                .map_err(|e| e.to_string())?;
            if let Some(l) = labels {
                std::fs::write(l, "x\n").map_err(|e| e.to_string())?;
            }
            Ok(())
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let s = setup(layout(dir.path()));
        let (release, gate) = mpsc::channel::<()>();
        *HOLD.lock().expect("hold") = Some(gate);
        SEEN.lock().expect("seen").clear();

        s.begin_install(stand_in).expect("starts");
        wait_until("the first file to be half fetched", || {
            s.snapshot(false).progress.is_some_and(|p| p.done > 0)
        });
        let mid = s.snapshot(false);
        assert_eq!(mid.phase, Phase::Installing);
        let p = mid.progress.expect("progress");
        assert_eq!((p.step, p.steps), (1, 2));
        assert!(p.percent() > 0 && p.percent() < 99, "{p:?}");
        assert_eq!(
            s.begin_install(stand_in),
            Err(StartError::AlreadyRunning),
            "a second download beside the first would race on the .part file"
        );

        release.send(()).expect("release");
        wait_until("the install to finish", || {
            s.snapshot(false).phase == Phase::NeedsRestart
        });
        assert_eq!(
            *SEEN.lock().expect("seen"),
            [DEFAULT_CLASSIFIER_ID, DEFAULT_RANGE_FILTER_ID],
            "the classifier first, then its range filter"
        );
    }

    /// A failed step stops the job, is reported with its reason, and leaves
    /// the way open to try again.
    #[test]
    fn a_failed_install_says_why_and_can_be_retried() {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = setup(layout(dir.path()));
        s.begin_install(|_, _, _| Err("download failed: no route to host".to_owned()))
            .expect("starts");
        wait_until("the failure", || s.snapshot(false).phase == Phase::Failed);
        let snap = s.snapshot(false);
        assert_eq!(
            snap.failure.as_deref(),
            Some("download failed: no route to host")
        );
        // Another try is allowed, and clears the failure while it runs.
        s.begin_install(|_, dest, _| {
            put_default(dest);
            Ok(())
        })
        .expect("a retry starts");
        wait_until("the retry", || {
            s.snapshot(false).phase == Phase::NeedsRestart
        });
        assert!(s.snapshot(false).failure.is_none());
    }
}
