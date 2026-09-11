//! Detection daemon: orchestrates the file-watch → process → infer → report loop.
//!
//! This module provides the core detection loop that:
//! 1. Watches a directory for new audio files (via `notify`)
//! 2. Decodes, resamples, and generates mel spectrograms
//! 3. Runs inference to classify bird species
//! 4. Reports detections via a callback (database insert, WebSocket broadcast, etc.)
//!
//! The daemon is synchronous internally (all audio processing and inference is CPU-bound)
//! and designed to be spawned on a blocking thread from the async runtime.
//!
//! This module owns the shared types and the public surface; the work is split
//! into two submodules:
//!
//! - `process` — per-file decode → pipeline → inference → filtering.
//! - `run` — the directory-watch loop that drives settled clips through it.

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use crate::detection::corroboration::ConfirmationLevel;
use crate::detection::pipeline::{self, PipelineConfig};
use crate::detection::types::Detection;
use crate::inference::model::{InferenceError, ModelConfig};

mod process;
mod run;

pub use process::{process_and_infer, process_and_infer_filtered, process_file_pipeline_only};
pub use run::run_daemon;

/// Errors from the detection daemon.
#[derive(Debug)]
pub enum DaemonError {
    /// Pipeline error (decode, resample, spectrogram).
    Pipeline(pipeline::PipelineError),
    /// Inference error.
    Inference(InferenceError),
    /// Model loading error.
    Model(String),
    /// Configuration error.
    Config(String),
    /// The daemon was stopped.
    Stopped,
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pipeline(e) => write!(f, "pipeline: {e}"),
            Self::Inference(e) => write!(f, "inference: {e}"),
            Self::Model(msg) => write!(f, "model: {msg}"),
            Self::Config(msg) => write!(f, "config: {msg}"),
            Self::Stopped => write!(f, "daemon stopped"),
        }
    }
}

impl std::error::Error for DaemonError {}

impl From<pipeline::PipelineError> for DaemonError {
    fn from(e: pipeline::PipelineError) -> Self {
        Self::Pipeline(e)
    }
}

impl From<InferenceError> for DaemonError {
    fn from(e: InferenceError) -> Self {
        Self::Inference(e)
    }
}

/// Observer for the species occurrence filter's state.
///
/// `(active, candidates)`: whether the filter is running at all, and how many
/// species it admits once it has run (`None` before its first evaluation).
#[derive(Clone)]
pub struct SpeciesFilterObserver(std::sync::Arc<dyn Fn(bool, Option<u64>) + Send + Sync>);

impl std::fmt::Debug for SpeciesFilterObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpeciesFilterObserver(..)")
    }
}

impl SpeciesFilterObserver {
    /// Wrap a closure.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(bool, Option<u64>) + Send + Sync + 'static,
    {
        Self(std::sync::Arc::new(f))
    }

    /// Report the current state.
    pub fn report(&self, active: bool, candidates: Option<u64>) {
        (self.0)(active, candidates);
    }
}

/// Reports each audio file the daemon finished analysing.
///
/// # Why this exists
///
/// Nothing counted the pipeline's *throughput*. The only latency histogram is
/// observed once per **stored detection** — its own `# HELP` says so — so on a
/// station where inference runs perfectly and returns nothing (wrong labels,
/// wrong sample rate, a model swapped by a bad update) every series was flat
/// and empty, **identical** to a station where inference was not running at
/// all. The four drop-reason labels do not separate them either: all of them
/// live downstream of a prediction the model actually made.
///
/// One counter answers it. A station analysing 15-second segments produces
/// about 5 760 files a day per source, so:
///
/// * counter flat while `birdnet_audio_source_up == 1` → capture is writing
///   files and nothing is analysing them;
/// * counter rising with no detections → the model is answering nothing.
///
/// The callback takes the file's path rather than a label because the
/// source-label convention (`local`, or the RTSP id out of the filename) lives
/// in the binary, next to every other use of it, and there should not be a
/// second copy here.
///
/// # What is and is not gated
///
/// The metric and this type's contract are covered by CI-runnable tests. The
/// two **call sites** in `run.rs` are not, because reaching them needs a loaded
/// `BirdNetModel` and therefore the 541 MB model file — the same limit
/// `tests/species_filter_e2e.rs` documents for its own second layer. What
/// keeps them honest instead is that both sit in the `Ok` arm of
/// `process_and_infer_filtered`, so "analysed" cannot drift to mean "attempted".
#[derive(Clone)]
pub struct ThroughputObserver {
    analysed: PathCallback,
    /// A segment the watcher announced that was gone before the pipeline
    /// opened it (PR-1 / S-3): audio recorded and never analysed.
    dropped: Option<PathCallback>,
    /// The analysis queue's depth, once per sweep (PR-2).
    queue_depth: Option<std::sync::Arc<dyn Fn(usize) + Send + Sync>>,
    /// A segment the shed policy chose not to analyse, and why (PR-2).
    shed: Option<ShedCallback>,
}

/// A shared callback taking a file path.
type PathCallback = std::sync::Arc<dyn Fn(&std::path::Path) + Send + Sync>;
/// A shared callback taking a shed segment's path and the reason.
type ShedCallback = std::sync::Arc<dyn Fn(&std::path::Path, ShedReason) + Send + Sync>;

/// Why a sweep analysed one segment in two (PR-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShedReason {
    /// More segments were waiting than the policy's backlog threshold:
    /// inference is slower than real time here.
    Backlog,
    /// The board is at its thermal limit or throttling; analysing less is
    /// what lets it cool.
    Thermal,
}

impl ShedReason {
    /// The metric label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Thermal => "thermal",
        }
    }
}

/// Segments waiting above which the daemon sheds (PR-2).
///
/// Forty is ten minutes of 15 s segments from one source: well past any
/// burst a settle period explains, and well before a 600 s stream drain
/// starts taking unanalysed audio.
pub const DEFAULT_SHED_BACKLOG_ABOVE: usize = 40;

/// When to analyse one segment in two rather than fall further behind (PR-2).
///
/// "Inference slower than real time for an hour" used to resolve to "delete
/// the oldest audio and say nothing": the stream drain took unanalysed
/// segments by age while the pipeline worked through a queue nothing
/// measured. The policy is deliberately simple and stated here so an operator
/// can predict it: while the queue is deeper than `backlog_above`, or while
/// the thermal signal says the board is at its limit, every second ready
/// segment in capture order is skipped, reported through
/// [`ThroughputObserver::shed`] with its reason, and left for the stream
/// drain (and the raw-audio keep, R-4) like any other analysed segment. The
/// queue then drains at twice the rate it would otherwise, the skipped audio
/// is counted rather than lost quietly, and nothing else changes.
#[derive(Clone)]
pub struct ShedPolicy {
    /// Shed while more than this many segments are waiting.
    pub backlog_above: usize,
    thermal: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl std::fmt::Debug for ShedPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShedPolicy")
            .field("backlog_above", &self.backlog_above)
            .field("thermal", &self.thermal.is_some())
            .finish()
    }
}

impl ShedPolicy {
    /// A policy that sheds on backlog only.
    #[must_use]
    pub const fn new(backlog_above: usize) -> Self {
        Self {
            backlog_above,
            thermal: None,
        }
    }

    /// Also shed while `signal` says the board is at its thermal limit.
    #[must_use]
    pub fn with_thermal<F>(mut self, signal: F) -> Self
    where
        F: Fn() -> bool + Send + Sync + 'static,
    {
        self.thermal = Some(std::sync::Arc::new(signal));
        self
    }

    /// Whether this sweep sheds, and why. Backlog is named first: it is the
    /// measurable one, and the thermal signal is only consulted when the
    /// queue alone does not decide it.
    #[must_use]
    pub fn decide(&self, queue_depth: usize) -> Option<ShedReason> {
        if queue_depth > self.backlog_above {
            return Some(ShedReason::Backlog);
        }
        if self.thermal.as_ref().is_some_and(|f| f()) {
            return Some(ShedReason::Thermal);
        }
        None
    }
}

/// Split a sweep's ready segments under a shed: in capture order (the file
/// name carries it), the first, third, fifth … are analysed and the rest shed.
#[must_use]
pub fn shed_split(mut ready: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    ready.sort();
    let mut analyse = Vec::new();
    let mut shed = Vec::new();
    for (i, path) in ready.into_iter().enumerate() {
        if i % 2 == 0 {
            analyse.push(path);
        } else {
            shed.push(path);
        }
    }
    (analyse, shed)
}

#[cfg(test)]
mod shed_tests {
    use super::{DEFAULT_SHED_BACKLOG_ABOVE, ShedPolicy, ShedReason, shed_split};
    use std::path::PathBuf;

    /// PR-2. The decision is backlog first, thermal second, nothing when the
    /// queue is short and the board cool.
    #[test]
    fn shed_is_backlog_first_then_thermal_and_nothing_when_cool() {
        let cool = ShedPolicy::new(DEFAULT_SHED_BACKLOG_ABOVE);
        assert_eq!(cool.decide(0), None);
        assert_eq!(
            cool.decide(DEFAULT_SHED_BACKLOG_ABOVE),
            None,
            "at, not above"
        );
        assert_eq!(
            cool.decide(DEFAULT_SHED_BACKLOG_ABOVE + 1),
            Some(ShedReason::Backlog)
        );
        let hot = ShedPolicy::new(DEFAULT_SHED_BACKLOG_ABOVE).with_thermal(|| true);
        assert_eq!(hot.decide(0), Some(ShedReason::Thermal));
        assert_eq!(
            hot.decide(DEFAULT_SHED_BACKLOG_ABOVE + 1),
            Some(ShedReason::Backlog),
            "the measurable reason is named when both hold"
        );
        let cooled = ShedPolicy::new(DEFAULT_SHED_BACKLOG_ABOVE).with_thermal(|| false);
        assert_eq!(cooled.decide(3), None);
    }

    /// Every second segment in capture order, whatever order the sweep
    /// yielded them in.
    #[test]
    fn shed_split_analyses_every_second_segment_in_capture_order() {
        let names = ["06:00:45", "06:00:00", "06:00:30", "06:00:15", "06:01:00"];
        let ready: Vec<PathBuf> = names
            .iter()
            .map(|t| PathBuf::from(format!("/s/2026-03-14-birdnet-{t}.wav")))
            .collect();
        let (analyse, shed) = shed_split(ready);
        let stem = |p: &PathBuf| p.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            analyse.iter().map(stem).collect::<Vec<_>>(),
            vec![
                "2026-03-14-birdnet-06:00:00.wav",
                "2026-03-14-birdnet-06:00:30.wav",
                "2026-03-14-birdnet-06:01:00.wav"
            ]
        );
        assert_eq!(
            shed.iter().map(stem).collect::<Vec<_>>(),
            vec![
                "2026-03-14-birdnet-06:00:15.wav",
                "2026-03-14-birdnet-06:00:45.wav"
            ]
        );
        assert_eq!(shed_split(Vec::new()), (Vec::new(), Vec::new()));
    }
}

impl std::fmt::Debug for ThroughputObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThroughputObserver(..)")
    }
}

impl ThroughputObserver {
    /// Wrap a closure.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&std::path::Path) + Send + Sync + 'static,
    {
        Self {
            analysed: std::sync::Arc::new(f),
            dropped: None,
            queue_depth: None,
            shed: None,
        }
    }

    /// Also report the analysis queue's depth once per sweep (PR-2).
    #[must_use]
    pub fn with_queue_depth<F>(mut self, f: F) -> Self
    where
        F: Fn(usize) + Send + Sync + 'static,
    {
        self.queue_depth = Some(std::sync::Arc::new(f));
        self
    }

    /// Also report segments the shed policy skipped (PR-2).
    #[must_use]
    pub fn with_shed<F>(mut self, f: F) -> Self
    where
        F: Fn(&std::path::Path, ShedReason) + Send + Sync + 'static,
    {
        self.shed = Some(std::sync::Arc::new(f));
        self
    }

    /// Report how many segments are waiting after this sweep.
    pub fn queue_depth(&self, depth: usize) {
        if let Some(f) = &self.queue_depth {
            f(depth);
        }
    }

    /// Report a segment the shed policy skipped.
    pub fn shed(&self, path: &std::path::Path, reason: ShedReason) {
        if let Some(f) = &self.shed {
            f(path, reason);
        }
    }

    /// Also report segments that vanished before analysis.
    #[must_use]
    pub fn with_dropped<F>(mut self, f: F) -> Self
    where
        F: Fn(&std::path::Path) + Send + Sync + 'static,
    {
        self.dropped = Some(std::sync::Arc::new(f));
        self
    }

    /// Report that one file finished analysis.
    pub fn analysed(&self, path: &std::path::Path) {
        (self.analysed)(path);
    }

    /// Report a segment that was gone before the pipeline could open it.
    pub fn dropped(&self, path: &std::path::Path) {
        if let Some(f) = &self.dropped {
            f(path);
        }
    }
}

/// The segments the pipeline is reading right now (PR-1 / S-3): a lease the
/// stream directory's purge honours, so a segment held open by a live reader
/// is never the one it deletes.
///
/// The daemon claims a segment before opening it and the claim is released
/// when the guard drops; the disk manager's locked-file provider for the
/// stream directory lists the claimed names. A probe against the shipped
/// `DiskManagerConfig` deleted a segment under a live reader before this
/// existed; the age floor protected nothing once the purge was the
/// disk-full one, which takes the oldest first whatever its age.
#[derive(Clone, Default)]
pub struct InFlight(std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>);

impl std::fmt::Debug for InFlight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("InFlight").field(&self.names()).finish()
    }
}

impl InFlight {
    /// An empty lease table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim `path` (by its file name) until the returned guard drops.
    pub fn claim(&self, path: &std::path::Path) -> InFlightGuard {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Ok(mut set) = self.0.lock() {
            set.insert(name.clone());
        }
        InFlightGuard {
            table: self.clone(),
            name,
        }
    }

    /// The file names currently claimed, sorted.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.0
            .lock()
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// Releases one [`InFlight`] claim when dropped.
#[derive(Debug)]
pub struct InFlightGuard {
    table: InFlight,
    name: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.table.0.lock() {
            set.remove(&self.name);
        }
    }
}

#[cfg(test)]
mod in_flight_tests {
    use super::InFlight;

    /// The lease is the whole contract: the name is listed exactly while a
    /// guard for it is alive.
    #[test]
    fn a_claim_lists_the_name_until_the_guard_drops() {
        let table = InFlight::new();
        assert!(table.names().is_empty());
        let a = table.claim(std::path::Path::new(
            "/tmp/s/2026-05-19-birdnet-06:30:00.wav",
        ));
        let b = table.claim(std::path::Path::new(
            "/tmp/s/2026-05-19-birdnet-06:30:15.wav",
        ));
        assert_eq!(
            table.names(),
            vec![
                "2026-05-19-birdnet-06:30:00.wav".to_owned(),
                "2026-05-19-birdnet-06:30:15.wav".to_owned()
            ]
        );
        drop(a);
        assert_eq!(
            table.names(),
            vec!["2026-05-19-birdnet-06:30:15.wav".to_owned()]
        );
        drop(b);
        assert!(table.names().is_empty());
    }
}

#[cfg(test)]
mod throughput_observer_tests {
    use std::sync::{Arc, Mutex};

    use super::ThroughputObserver;

    /// The observer must hand the caller the path it was given, unchanged.
    ///
    /// That is the whole contract: the source label is derived from the
    /// filename on the binary side, where `derive_source_label` lives, so an
    /// observer that dropped or rewrote the path would silently collapse every
    /// source onto one series.
    #[test]
    fn the_path_reaches_the_callback_unchanged() {
        let seen: Arc<Mutex<Vec<std::path::PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let observer = ThroughputObserver::new(move |p| sink.lock().unwrap().push(p.to_path_buf()));

        observer.analysed(std::path::Path::new(
            "/x/2026-05-19-birdnet-cam1-06:30:00.wav",
        ));
        observer.analysed(std::path::Path::new("/x/2026-05-19-birdnet-06:30:00.wav"));

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "every reported file must reach the callback");
        assert_eq!(
            seen[0],
            std::path::Path::new("/x/2026-05-19-birdnet-cam1-06:30:00.wav")
        );
        assert_ne!(
            seen[0], seen[1],
            "two files from different sources must not arrive identical, or the \
             per-source label the binary derives from them is meaningless"
        );
    }

    /// PR-1 / S-3: a dropped segment reaches its own callback, and an
    /// observer without one stays silent rather than counting it as analysed.
    #[test]
    fn a_dropped_segment_reaches_the_dropped_callback_only() {
        let analysed: Arc<Mutex<Vec<std::path::PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let dropped: Arc<Mutex<Vec<std::path::PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let (a, d) = (Arc::clone(&analysed), Arc::clone(&dropped));
        let observer = ThroughputObserver::new(move |p| a.lock().unwrap().push(p.to_path_buf()))
            .with_dropped(move |p| d.lock().unwrap().push(p.to_path_buf()));
        observer.dropped(std::path::Path::new("/x/2026-05-19-birdnet-06:30:00.wav"));
        assert!(analysed.lock().unwrap().is_empty());
        assert_eq!(dropped.lock().unwrap().len(), 1);

        let plain = ThroughputObserver::new(|_| {});
        plain.dropped(std::path::Path::new("/x/y.wav"));
    }
}

/// Configuration for the detection daemon.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Directory to watch for new audio files.
    pub watch_dir: PathBuf,
    /// Path to the ONNX model file.
    pub model_path: PathBuf,
    /// Path to the labels file.
    pub labels_path: PathBuf,
    /// Pipeline configuration (sample rate, chunk size, etc.).
    pub pipeline: PipelineConfig,
    /// Model configuration (sensitivity, threshold, etc.).
    pub model: ModelConfig,
    /// Whether to process files already present in the watch directory on startup.
    pub process_existing: bool,
    /// Optional path to the metadata ONNX model for species filtering.
    pub metadata_model_path: Option<PathBuf>,
    /// Optional path to the metadata model's own label file.
    ///
    /// The BirdNET geomodel scores a different species list from the
    /// classifier (12 012 against the V3.0 classifier's 11 560), so its
    /// outputs can only be read through its own labels. Leave `None` only for
    /// a metadata model indexed identically to the classifier — a matched
    /// BirdNET pair — which the loader verifies rather than assumes.
    pub metadata_labels_path: Option<PathBuf>,
    /// Optional path to an operator's scientific-name alias file.
    ///
    /// Tab-separated `legacy<TAB>canonical` lines, `#` comments skipped. It
    /// exists because the automatic alignment between the two label files
    /// cannot place every reclassified species by itself, and a station that
    /// hits one has no other way to say "these two names are the same bird".
    /// No table ships: see
    /// [`crate::inference::vocabulary`] for why the obvious one is neither
    /// usable here nor useful on the pinned model pair.
    pub species_aliases_path: Option<PathBuf>,
    /// Species filter configuration (threshold, whitelist, include/exclude).
    pub species_filter: crate::inference::species_filter::SpeciesFilterConfig,
    /// Optional callback re-read on a short TTL to refresh the operator's
    /// include/exclude species lists without a restart.
    ///
    /// See [`crate::inference::species_filter::SpeciesListsProvider`]. When
    /// `None`, the lists in [`Self::species_filter`] are used as given and never
    /// change for the life of the daemon.
    pub species_lists_provider: Option<crate::inference::species_filter::SpeciesListsProvider>,
    /// Called when the species occurrence filter's state changes: whether it
    /// is actually running, and how many species it currently admits.
    ///
    /// A callback rather than a metrics handle because `birdnet-core` does not
    /// depend on the web crate — the same reason `species_lists_provider` is
    /// shaped this way. The binary wires it to the Prometheus registry, where
    /// `birdnet_occurrence_filter_active` is the single number that would have
    /// made an inert filter visible from a dashboard instead of from reading
    /// the code.
    pub on_species_filter_state: Option<SpeciesFilterObserver>,
    /// Called once for every audio file the daemon finishes analysing.
    ///
    /// See [`ThroughputObserver`] for what this answers that nothing else did:
    /// "the model returns nothing" and "the pipeline is not running" produce
    /// identical, empty series without it. A callback for the same reason as
    /// the field above — `birdnet-core` does not depend on the web crate.
    pub on_file_analysed: Option<ThroughputObserver>,
    /// The lease table the stream directory's purge honours (PR-1 / S-3):
    /// a segment is claimed here while the pipeline reads it. `None` claims
    /// nothing.
    pub in_flight: Option<InFlight>,
    /// When to analyse one segment in two rather than fall further behind
    /// (PR-2); `None` never sheds.
    pub shed: Option<ShedPolicy>,
    /// Privacy filter threshold (0.0 = disabled).
    pub privacy_threshold: f32,
    /// Confidence at or above which a watched non-bird noise class suppresses
    /// its chunk (0.0 = disabled).
    pub noise_threshold: f32,
    /// Non-bird label names the noise filter watches.
    pub noise_classes: Vec<String>,
    /// How long a noise-suppressed chunk's species stay suppressed, in seconds
    /// (`G-17`). `0.0` — the default — is no window at all.
    pub noise_remember_secs: f32,
    /// How much corroboration from neighbouring windows a species needs before
    /// it is recorded ([`ConfirmationLevel::Off`] = disabled).
    ///
    /// Only does anything when the analysis windows overlap: with no overlap a
    /// six-second neighbourhood holds two windows, and the two gentler levels
    /// round down to "itself". [`ConfirmationLevel::is_effective_at`] says so
    /// for a given overlap, and the daemon warns at startup when it is not.
    pub confirmation: ConfirmationLevel,
    /// Station latitude (for species occurrence filtering).
    pub latitude: Option<f64>,
    /// Station longitude (for species occurrence filtering).
    pub longitude: Option<f64>,
    /// Per-species confidence threshold overrides (`sci_name` → threshold).
    ///
    /// Species in this map use the specified threshold instead of the global one.
    pub species_thresholds: std::collections::HashMap<String, f64>,
}

/// A detection event produced by the daemon.
#[derive(Debug, Clone)]
pub struct DetectionEvent {
    /// The detection result.
    pub detection: Detection,
    /// Source audio file path.
    pub source_file: PathBuf,
    /// Processing latency in milliseconds.
    pub latency_ms: u64,
    /// Correlation ID stamped at file-arrival time and propagated through
    /// every event the daemon emits for that file. Every log line, DB
    /// write, and notification dispatched downstream tags this ID so the
    /// operator can trace one audio file end-to-end with a single grep.
    /// Empty when the upstream call site did not set one (older API).
    pub correlation_id: String,
}

/// Handle for controlling a running daemon.
pub struct DaemonHandle {
    stop_tx: mpsc::Sender<()>,
    heartbeat: Arc<AtomicU64>,
    /// `true` while the loop thread is alive; cleared by the thread itself as
    /// it returns, whichever way it returns. See [`RunningGuard`].
    running: Arc<AtomicBool>,
}

/// Clears a [`DaemonHandle`]'s `running` flag when dropped.
///
/// The loop thread holds one for its whole life, so a `break` on the stop
/// signal, a watcher disconnect, and any early return all record the exit
/// without each having to remember to. Before this the flag the health
/// endpoint reads was stored once at start-up by the binary and cleared by
/// nothing, so `?strict=1` reported a daemon that had died as running
/// (`PR-5`, `OP-2`). With `panic = "abort"` there is no unwinding path to
/// miss.
struct RunningGuard(Arc<AtomicBool>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl fmt::Debug for DaemonHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DaemonHandle").finish()
    }
}

impl DaemonHandle {
    /// Signal the daemon to stop.
    pub fn stop(&self) {
        let _ = self.stop_tx.send(());
    }

    /// A shared flag that is `true` while the detection loop's thread is alive.
    ///
    /// The thread clears it as it returns, so a reader that sees `false` is
    /// looking at a daemon that has exited — not one that is idle, which the
    /// heartbeat distinguishes. The binary mirrors it into the health
    /// endpoint's daemon flag.
    #[must_use]
    pub fn running_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.running)
    }

    /// Whether the detection loop's thread is still alive.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// A shared counter the detection loop increments on every iteration.
    ///
    /// Sample it periodically (e.g. from the systemd watchdog pinger): if it
    /// stops advancing, the detection pipeline has hung and the process should
    /// be restarted rather than left silently frozen.
    #[must_use]
    pub fn heartbeat(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.heartbeat)
    }
}

/// Generate a short correlation ID for a single audio file.
///
/// Format: `e-{ms-since-epoch}-{counter:04x}`. Sortable by arrival time,
/// monotonically increasing per-process, short enough for one log line.
/// Used only as a debug aid for tracing one file through the pipeline —
/// uniqueness across processes is not required (every process scrapes
/// its own log).
#[must_use]
pub fn new_event_correlation_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("e-{ms}-{n:04x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn daemon_config_defaults() {
        let config = DaemonConfig {
            watch_dir: PathBuf::from("/tmp/StreamData"),
            model_path: PathBuf::from("/opt/birdnet/model.onnx"),
            labels_path: PathBuf::from("/opt/birdnet/labels.txt"),
            pipeline: PipelineConfig::default(),
            model: ModelConfig::default(),
            process_existing: false,
            metadata_model_path: None,
            metadata_labels_path: None,
            species_aliases_path: None,
            on_species_filter_state: None,
            on_file_analysed: None,
            in_flight: None,
            shed: None,
            species_filter: crate::inference::species_filter::SpeciesFilterConfig::default(),
            species_lists_provider: None,
            privacy_threshold: 0.0,
            noise_threshold: 0.0,
            noise_classes: Vec::new(),
            noise_remember_secs: 0.0,
            confirmation: ConfirmationLevel::Off,
            latitude: None,
            longitude: None,
            species_thresholds: std::collections::HashMap::new(),
        };
        assert_eq!(config.watch_dir, PathBuf::from("/tmp/StreamData"));
        assert!(!config.process_existing);
        assert!(config.metadata_model_path.is_none());
        assert!((config.privacy_threshold).abs() < f32::EPSILON);
        assert!(config.species_thresholds.is_empty());
    }

    #[test]
    fn daemon_handle_stop_does_not_panic() {
        let (stop_tx, _stop_rx) = mpsc::channel();
        let handle = DaemonHandle {
            stop_tx,
            heartbeat: Arc::new(AtomicU64::new(0)),
            running: Arc::new(AtomicBool::new(true)),
        };
        handle.stop(); // Should not panic even if receiver is alive
    }

    #[test]
    fn daemon_handle_exposes_shared_heartbeat() {
        let (stop_tx, _stop_rx) = mpsc::channel();
        let heartbeat = Arc::new(AtomicU64::new(7));
        let handle = DaemonHandle {
            stop_tx,
            heartbeat: Arc::clone(&heartbeat),
            running: Arc::new(AtomicBool::new(true)),
        };
        // The accessor returns a handle onto the *same* counter, so a watchdog
        // observes the loop's increments.
        assert_eq!(handle.heartbeat().load(Ordering::Relaxed), 7);
        heartbeat.fetch_add(1, Ordering::Relaxed);
        assert_eq!(handle.heartbeat().load(Ordering::Relaxed), 8);
    }

    /// The flag is cleared when the thread holding the guard returns — and
    /// not before, which is the counterpart: a live loop must read as live.
    ///
    /// Observed failing with `RunningGuard`'s `Drop` body emptied: the final
    /// assertion reports `the thread returned but the flag still says running`.
    #[test]
    fn the_running_flag_is_cleared_when_the_loop_thread_returns_and_not_before() {
        let running = Arc::new(AtomicBool::new(true));
        let guard = RunningGuard(Arc::clone(&running));
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            let _alive = guard;
            let _ = go_rx.recv();
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            running.load(Ordering::Acquire),
            "a loop that is still running must read as running"
        );
        go_tx.send(()).expect("thread alive");
        thread.join().expect("join");
        assert!(
            !running.load(Ordering::Acquire),
            "the thread returned but the flag still says running"
        );

        let (stop_tx, _stop_rx) = mpsc::channel();
        let handle = DaemonHandle {
            stop_tx,
            heartbeat: Arc::new(AtomicU64::new(0)),
            running: Arc::clone(&running),
        };
        assert!(!handle.is_running());
        assert!(!handle.running_flag().load(Ordering::Acquire));
    }

    // ─── Correlation-ID generator ─────────────────────────────────────────
    //
    // Operators rely on the correlation_id to trace one audio file through
    // decode → infer → notify → DB write with a single `grep` over the log.
    // The contract is "unique enough that two files arriving in the same
    // millisecond still get distinct IDs."

    #[test]
    fn correlation_id_has_recognisable_shape() {
        let id = new_event_correlation_id();
        // `e-{ms}-{counter:04x}`
        assert!(id.starts_with("e-"), "expected prefix 'e-', got: {id}");
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 3, "expected 3 hyphen-segments in: {id}");
        assert!(parts[1].chars().all(|c| c.is_ascii_digit()));
        assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn correlation_id_is_unique_across_rapid_calls() {
        // Generate a bunch quickly — the per-process counter must keep
        // them distinct even when SystemTime returns the same millisecond.
        let mut ids = std::collections::HashSet::new();
        for _ in 0..1000 {
            ids.insert(new_event_correlation_id());
        }
        assert_eq!(ids.len(), 1000, "correlation IDs are not unique");
    }

    #[test]
    fn detection_event_carries_correlation_id() {
        // Pin the contract: DetectionEvent has the field and round-trips
        // through clone without losing it.
        use crate::detection::types::Detection;
        let event = DetectionEvent {
            detection: Detection {
                date: "2026-05-19".to_owned(),
                time: "09:00:00".to_owned(),
                scientific_name: "Pica pica".to_owned(),
                common_name: "Eurasian Magpie".to_owned(),
                confidence: 0.93,
                start: 0.0,
                stop: 4.5,
                week: 20,
                file_name_extr: None,
            },
            source_file: PathBuf::from("/tmp/x.wav"),
            latency_ms: 42,
            correlation_id: "e-12345-0001".to_owned(),
        };
        // Use the clone path even though we drop the original immediately —
        // the clippy::redundant_clone flag will fire, but pinning Clone is
        // exactly the point of this test. Bind both so the second move
        // through Clone is observable.
        let cloned = event.clone();
        drop(event);
        assert_eq!(cloned.correlation_id, "e-12345-0001");
    }
}
