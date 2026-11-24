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

pub use process::{process_and_infer_filtered, process_file_pipeline_only};
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
    /// Each classifier's health after every file the pipeline ran inference
    /// on, primary first (`e853776`).
    model_health: Option<ModelHealthCallback>,
    /// What each classifier is, once, after they are loaded.
    model_info: Option<ModelInfoCallback>,
    /// What the filters discarded from each analysed file (ACC-02).
    filtered: Option<FilteredCallback>,
}

/// A shared callback taking a file path and what the filters discarded from it.
type FilteredCallback =
    std::sync::Arc<dyn Fn(&std::path::Path, &crate::detection::FilterTally) + Send + Sync>;

/// A shared callback taking what each loaded classifier is.
type ModelInfoCallback =
    std::sync::Arc<dyn Fn(&[crate::inference::registry::ModelInfo]) + Send + Sync>;

/// A shared callback taking each classifier's id and health.
type ModelHealthCallback =
    std::sync::Arc<dyn Fn(&[(String, crate::inference::health::ModelHealth)]) + Send + Sync>;

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
            model_health: None,
            model_info: None,
            filtered: None,
        }
    }

    /// Also report what the filters discarded from each analysed file
    /// (ACC-02), after [`Self::analysed`] for it.
    #[must_use]
    pub fn with_filtered<F>(mut self, f: F) -> Self
    where
        F: Fn(&std::path::Path, &crate::detection::FilterTally) + Send + Sync + 'static,
    {
        self.filtered = Some(std::sync::Arc::new(f));
        self
    }

    /// Report what the filters discarded from one analysed file.
    pub fn filtered(&self, path: &std::path::Path, tally: &crate::detection::FilterTally) {
        if let Some(f) = &self.filtered {
            f(path, tally);
        }
    }

    /// Also report each classifier's health after every file it ran on.
    #[must_use]
    pub fn with_model_health<F>(mut self, f: F) -> Self
    where
        F: Fn(&[(String, crate::inference::health::ModelHealth)]) + Send + Sync + 'static,
    {
        self.model_health = Some(std::sync::Arc::new(f));
        self
    }

    /// Also report what each classifier is, once, when they have loaded.
    #[must_use]
    pub fn with_model_info<F>(mut self, f: F) -> Self
    where
        F: Fn(&[crate::inference::registry::ModelInfo]) + Send + Sync + 'static,
    {
        self.model_info = Some(std::sync::Arc::new(f));
        self
    }

    /// Report what each classifier is.
    pub fn model_info(&self, info: &[crate::inference::registry::ModelInfo]) {
        if let Some(f) = &self.model_info {
            f(info);
        }
    }

    /// Report each classifier's health.
    pub fn model_health(&self, health: &[(String, crate::inference::health::ModelHealth)]) {
        if let Some(f) = &self.model_health {
            f(health);
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
            .map_or_default(|n| n.to_string_lossy().into_owned());
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
            .map_or_default(|set| set.iter().cloned().collect())
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
    /// Extra classifiers beyond the primary, in declaration order (`G-10`
    /// Stage 2).
    ///
    /// Empty on every station that has not asked for a second opinion, which
    /// is the default and loads exactly one classifier — the behaviour this
    /// project has always had. The application layer decides what goes here;
    /// it refuses a classifier it cannot show will fit in memory, because an
    /// unattended station being OOM-killed at three in the morning is a worse
    /// outcome than running with one model and saying so.
    pub extra_models: Vec<crate::inference::registry::ModelSpec>,
    /// The primary classifier's id: recorded on its detections and named in
    /// routes (`MODEL_ID`; `birdnet` when unset).
    pub primary_id: String,
    /// The primary's own confidence threshold (`MODEL_THRESHOLD`), or `None`
    /// to use the station's.
    pub primary_threshold: Option<f32>,
    /// The primary's declared sample rate (`MODEL_SAMPLE_RATE`), or `None` to
    /// derive it from the model's input shape.
    pub primary_sample_rate: Option<u32>,
    /// The primary's declared output scale (`MODEL_OUTPUT_SCALE`), or `None`
    /// to derive it from the model's input shape, which refuses a shape it
    /// does not recognise (MMP-11).
    pub primary_output_scale: Option<crate::inference::model::OutputScale>,
    /// Which classifiers judge which audio source, by source id.
    ///
    /// A source absent from this map is judged by the primary classifier —
    /// never by none. A route naming a classifier that is not configured stops
    /// the daemon at startup rather than leaving that microphone silently
    /// unjudged.
    pub model_routes: std::collections::HashMap<String, Vec<String>>,
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
    /// Privacy rank rule: suppress a window when a human class is among the
    /// model's this-many highest-scoring classes (0 = off). See
    /// [`crate::detection::privacy::PrivacyFilter::with_rank_cutoff`].
    pub privacy_rank: usize,
    /// How far a saved clip reaches beyond its detection, so the privacy
    /// filter can check the whole of it for speech (`PIPE7`). The binary takes
    /// it from `ExtractionConfig::clip_reach`.
    pub privacy_clip_reach: crate::detection::privacy::ClipReach,
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
    /// The lowest confidence any species may be recorded at, kept current by
    /// the detection processor. See [`ThresholdFloor`]. `None` runs every
    /// classifier at its own threshold for the life of the daemon.
    pub threshold_floor: Option<Arc<ThresholdFloor>>,
}

/// The lowest confidence any species may currently be recorded at, and the
/// bar each species is actually stored at.
///
/// # Why the inference loop needs to know
///
/// A classifier discards every score below its own threshold inside
/// `predict_chunk`, before the processor that applies per-species thresholds
/// ever sees it. The model ran at the global confidence, so an operator who
/// followed the tuning guide and lowered one rare owl to 0.5 under a global
/// 0.75 changed nothing: every 0.6 owl was dropped inside the model. The
/// feature was on, configured, and inert.
///
/// The processor re-reads the per-species thresholds every 30 s and publishes
/// the lowest one in force here; the inference loop runs each classifier at the
/// lower of its own threshold and this floor before every file
/// ([`apply_threshold_floor`]). Everything above the floor is still decided by
/// the processor, per species, exactly as before — the floor only stops the
/// model from deciding first.
///
/// # Why the floor alone is not enough (ACC-04)
///
/// The floor is one number for every class. Lowering one owl to 0.3 lowered
/// the cut for all of them, so classes the station would drop took the model's
/// top-N slots ahead of a species it would store, and counted as corroborating
/// neighbours for every species; with dynamic thresholds on, the whole station
/// ran at their floor. The processor therefore also publishes
/// [`StorableBars`], and each classifier holds every species to its own bar
/// before ranking ([`crate::inference::model::SpeciesBars`]). Until the
/// processor first publishes them, the model cuts at the floor alone.
#[derive(Debug)]
pub struct ThresholdFloor {
    floor: std::sync::atomic::AtomicU32,
    bars: std::sync::RwLock<Option<Arc<StorableBars>>>,
}

impl ThresholdFloor {
    /// A floor starting at `value`, with no storable bars published.
    #[must_use]
    pub const fn new(value: f32) -> Self {
        Self {
            floor: std::sync::atomic::AtomicU32::new(value.to_bits()),
            bars: std::sync::RwLock::new(None),
        }
    }

    /// The floor now.
    #[must_use]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.floor.load(Ordering::Relaxed))
    }

    /// Move the floor.
    pub fn set(&self, value: f32) {
        self.floor.store(value.to_bits(), Ordering::Relaxed);
    }

    /// The storable bars last published, if any.
    #[must_use]
    pub fn bars(&self) -> Option<Arc<StorableBars>> {
        self.bars
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Publish the bars the next file is cut at.
    pub fn publish_bars(&self, bars: StorableBars) {
        *self
            .bars
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(bars));
    }
}

/// What the processor would store a candidate at, species by species: the
/// inputs of its `decide_disposition`, for the model to cut at (ACC-04).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StorableBars {
    /// The station's confidence threshold: the bar for a species with neither
    /// an override nor a learned adjustment, on a classifier with no
    /// threshold of its own.
    pub station: f32,
    /// Every species with an operator override or a live dynamic adjustment.
    pub species: std::collections::HashMap<String, SpeciesBar>,
}

/// One species' entry in [`StorableBars`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpeciesBar {
    /// The operator's per-species threshold, if one is set.
    pub threshold: Option<f64>,
    /// The dynamic-threshold adjustment in force for it, if any.
    pub adjustment: Option<crate::detection::dynamic_threshold::Adjustment>,
}

impl StorableBars {
    /// The bars one classifier's candidates are held to. `own` is that
    /// classifier's own threshold, which replaces the station's for it, as
    /// the processor's floor does for the detections it names.
    ///
    /// A species with an override is stored at any confidence that reaches the
    /// processor (accepted at or above the override, quarantined below it), so
    /// its bar is the lower of the override and the classifier's bar: the
    /// quarantine keeps what the station would have seen with no other species
    /// lowered, and no more. Every other species is held to the classifier's
    /// bar. Each is adjusted as the processor adjusts it.
    #[must_use]
    pub fn for_classifier(&self, own: Option<f32>) -> crate::inference::model::SpeciesBars {
        let base = f64::from(own.unwrap_or(self.station));
        let by_species = self
            .species
            .iter()
            .map(|(name, entry)| {
                let adjust = |t: f64| entry.adjustment.map_or(t, |a| a.apply(t));
                let held = entry
                    .threshold
                    .map_or_else(|| adjust(base), |t| adjust(t).min(adjust(base)));
                (name.clone(), held)
            })
            .collect();
        crate::inference::model::SpeciesBars::new(base, by_species)
    }
}

/// Run each classifier at the lower of its own threshold (`base[i]`, what it
/// was loaded with) and the shared floor. A no-op without a floor.
///
/// Each species is also held to the bar the station would store it at, once
/// the processor has published one ([`StorableBars`]).
pub fn apply_threshold_floor(
    registry: &mut crate::inference::registry::ClassifierRegistry,
    base: &[f32],
    floor: Option<&ThresholdFloor>,
) {
    let Some(floor) = floor else {
        return;
    };
    let bars = floor.bars();
    let floor = floor.get();
    for (idx, &own) in base.iter().enumerate() {
        if let Some(registered) = registry.model_mut(idx) {
            registered.model.set_confidence_threshold(own.min(floor));
            let held = bars
                .as_ref()
                .map(|b| b.for_classifier(registered.threshold));
            registered.model.set_storable_bars(held);
        }
    }
}

/// Each classifier's own threshold, as loaded — the `base` for
/// [`apply_threshold_floor`].
#[must_use]
pub fn loaded_thresholds(registry: &crate::inference::registry::ClassifierRegistry) -> Vec<f32> {
    (0..registry.len())
        .filter_map(|idx| registry.model(idx))
        .map(|registered| registered.model.config().confidence_threshold)
        .collect()
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
    /// The source segment's in-flight claim, shared by every event from that
    /// segment (`PIPE8a`). The processor reads the segment again to cut each
    /// detection's clip, so the claim — which the stream directory's purge
    /// honours — must outlive the analysis loop's hold on it and last until
    /// the last of these events is dropped. `None` where nothing claims the
    /// file (no lease table, or an event not made from a segment).
    pub lease: Option<Arc<InFlightGuard>>,
    /// `Some(p)` when the occurrence filter held this species back, `p`
    /// being the occurrence model's probability for it here this week
    /// (ACC-01): not a detection, but a record for the "Unlikely here"
    /// quarantine. `None` for every detection the filters admitted.
    pub out_of_range: Option<f32>,
}

/// What the daemon made of one file: the events to process, and what its
/// filters discarded (ACC-02).
#[derive(Debug)]
pub struct FilteredFile {
    /// Detections, and out-of-range records for the quarantine.
    pub events: Vec<DetectionEvent>,
    /// The discards, by reason.
    pub tally: crate::detection::FilterTally,
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
            extra_models: Vec::new(),
            primary_id: "birdnet".to_owned(),
            primary_threshold: None,
            primary_sample_rate: None,
            primary_output_scale: None,
            model_routes: std::collections::HashMap::new(),
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
            privacy_rank: 0,
            privacy_clip_reach: crate::detection::privacy::ClipReach::default(),
            noise_threshold: 0.0,
            noise_classes: Vec::new(),
            noise_remember_secs: 0.0,
            confirmation: ConfirmationLevel::Off,
            latitude: None,
            longitude: None,
            species_thresholds: std::collections::HashMap::new(),
            threshold_floor: None,
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
                model_id: None,
                agreeing_models: None,
                model_scores: Vec::new(),
            },
            source_file: PathBuf::from("/tmp/x.wav"),
            latency_ms: 42,
            correlation_id: "e-12345-0001".to_owned(),
            lease: None,
            out_of_range: None,
        };
        // Use the clone path even though we drop the original immediately —
        // the clippy::redundant_clone flag will fire, but pinning Clone is
        // exactly the point of this test. Bind both so the second move
        // through Clone is observable.
        let cloned = event.clone();
        drop(event);
        assert_eq!(cloned.correlation_id, "e-12345-0001");
    }

    // ─── Storable bars (ACC-04) ───────────────────────────────────────────

    /// Each species' bar, as the processor would judge it: the station's for
    /// a species with nothing set, the lower of its override and the station's
    /// for one with an override, each adjusted where a level is learned; a
    /// classifier's own threshold replaces the station's for that classifier.
    #[test]
    fn a_species_is_held_to_the_bar_the_processor_would_store_it_at() {
        use crate::detection::dynamic_threshold::Adjustment;
        let halved = Adjustment {
            multiplier: 0.5,
            floor: 0.25,
        };
        let entry = |threshold, adjustment| SpeciesBar {
            threshold,
            adjustment,
        };
        let bars = StorableBars {
            station: 0.7,
            species: [
                ("lowered", entry(Some(0.4), None)),
                ("raised", entry(Some(0.9), None)),
                ("learned", entry(None, Some(halved))),
                ("both", entry(Some(0.6), Some(halved))),
            ]
            .into_iter()
            .map(|(n, b)| (n.to_owned(), b))
            .collect(),
        };
        let station = f64::from(0.7_f32);
        let at = bars.for_classifier(None);
        assert!((at.bar_for("lowered") - 0.4).abs() < 1e-12);
        assert!((at.bar_for("raised") - station).abs() < 1e-12);
        assert!((at.bar_for("learned") - station * 0.5).abs() < 1e-12);
        assert!((at.bar_for("both") - 0.3).abs() < 1e-12);
        assert!((at.bar_for("anything else") - station).abs() < 1e-12);

        let own = f64::from(0.5_f32);
        let at = bars.for_classifier(Some(0.5));
        assert!((at.bar_for("anything else") - own).abs() < 1e-12);
        assert!((at.bar_for("raised") - own).abs() < 1e-12);
        assert!((at.bar_for("lowered") - 0.4).abs() < 1e-12);
        assert!((at.bar_for("learned") - 0.25).abs() < 1e-12, "floored");
    }
}
