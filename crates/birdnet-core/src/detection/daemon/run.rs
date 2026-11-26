//! The daemon run loop: watch the directory, debounce writes, and drive each
//! settled clip through the processing pipeline.
//!
//! [`run_daemon`] is the start-up, in named steps, each with a typed result:
//!
//! 1. [`load_classifiers`] — the registry, and what became of every declared
//!    classifier, reported to [`super::ModelStateObserver`]. Only a primary
//!    that will not load (or a set that cannot be a station) stops here.
//! 2. [`fit_pipeline_to_classifiers`] — the chunk arithmetic the loaded
//!    classifiers need.
//! 3. [`load_occurrence_filter`] — the filter, or the reason it is off. Never
//!    fatal (MDL-02, MDL-05).
//! 4. [`chunk_filters`] and [`announce_chunk_filters`] — the whole-chunk
//!    filters, and the start-up lines that say what each will do.
//! 5. [`DaemonLoop::spawn`] — the watcher and the loop thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crate::audio::band::{BandWarning, BandWatch};
use crate::audio::capture::is_audio_file;
use crate::detection::pipeline::{self, PipelineConfig};
use crate::detection::privacy::PrivacyFilter;
use crate::detection::{ChunkFilters, noise::NoiseFilter};
use crate::file_settle::{FILE_SETTLE, PendingFiles};
use crate::inference::labels::LabelSet;
use crate::inference::model_state::{FilterOff, FilterState};
use crate::inference::registry::{ClassifierRegistry, LOCAL_SOURCE};
use crate::inference::species_filter::{SpeciesFilter, SpeciesListsProvider};
use crate::inference::vocabulary::parse_alias_file;

use super::process::process_and_infer_filtered;
use super::{
    DaemonConfig, DaemonError, DaemonHandle, DetectionEvent, InFlight, ModelStateObserver,
    ShedPolicy, ShedReason, Shedder, ThresholdFloor, ThroughputObserver, new_event_correlation_id,
};

/// How stale the operator's species include/exclude lists may get before the
/// next file re-reads them.
///
/// Matches the per-species threshold cache in the application's event
/// processor: both are small, indexed reads standing between an operator
/// changing something on `/admin/species` and seeing it take effect, and
/// neither is worth a query per detection during a dawn-chorus burst.
const SPECIES_LISTS_TTL: Duration = Duration::from_secs(30);

/// Every classifier the daemon loads, primary first.
///
/// The primary's id, threshold, rate and output scale are the operator's
/// (`MODEL_ID`, `MODEL_THRESHOLD`, `MODEL_SAMPLE_RATE`, `MODEL_OUTPUT_SCALE`). They were resolved by the plan and
/// then replaced here with `birdnet` and two `None`s, so a route naming the
/// primary by its configured id stopped the daemon, and the other two did
/// nothing at all.
fn classifier_specs(config: &DaemonConfig) -> Vec<crate::inference::registry::ModelSpec> {
    let mut specs = Vec::with_capacity(1 + config.extra_models.len());
    specs.push(crate::inference::registry::ModelSpec {
        id: config.primary_id.clone(),
        model_path: config.model_path.clone(),
        labels_path: config.labels_path.clone(),
        threshold: config.primary_threshold,
        // `None` derives the rate from the model's shape, which is right for
        // the BirdNET shapes that derivation was built from.
        sample_rate: config.primary_sample_rate,
        // `None` derives the scale from the shape, and refuses a shape it
        // does not recognise (MMP-11).
        output_scale: config.primary_output_scale,
    });
    specs.extend(config.extra_models.iter().cloned());
    specs
}

/// Run the detection daemon loop.
///
/// Watches `watch_dir` for new audio files and processes them through
/// the full pipeline. Detections are sent to `event_tx`.
///
/// Returns a `DaemonHandle` for stopping the daemon, and spawns the
/// watch loop on its own thread.
///
/// What became of each classifier and of the occurrence filter is reported to
/// [`DaemonConfig::on_model_state`] as each is decided, including when this
/// returns an error.
///
/// # Errors
///
/// Returns `DaemonError` if no classifier is configured, if the primary
/// cannot be loaded, if a route names a classifier that was never declared,
/// or if the watcher fails. Every one of those is refused here at startup
/// rather than at the first audio file, because an unattended station must
/// fail where somebody can see it. An extra classifier or an occurrence
/// filter that cannot be loaded is left out with its reason, and the daemon
/// starts without it (MDL-01, MDL-02).
pub fn run_daemon(
    config: &DaemonConfig,
    event_tx: mpsc::SyncSender<DetectionEvent>,
) -> Result<DaemonHandle, DaemonError> {
    let mut registry = load_classifiers(config)?;

    // The pipeline is fitted to the classifiers as the *registry* sees them,
    // i.e. after any declared `MODEL_SAMPLE_RATE` has replaced the rate the
    // model's shape suggests. See `fit_pipeline_to_classifiers`.
    let mut pipeline_config = config.pipeline.clone();
    fit_pipeline_to_classifiers(&mut pipeline_config, &registry);

    let (species_filter, filter_state) =
        load_occurrence_filter(config, registry.primary().model.labels());
    if let Some(observer) = config.on_model_state.as_ref() {
        observer.filter(&filter_state);
    }
    // What each classifier is, stated once now that they are loaded. The window
    // record (`model_health`) says how they are doing; this says what they are.
    if let Some(observer) = config.on_file_analysed.as_ref() {
        observer.model_info(&registry.info());
    }

    let chunk_filters = chunk_filters(config, &pipeline_config);
    announce_chunk_filters(config, &pipeline_config, &chunk_filters, &registry);
    // Every classifier scores the watched noise classes before its detection
    // threshold, so the noise filter's own threshold binds even below it.
    if chunk_filters.noise.is_enabled() {
        for idx in 0..registry.len() {
            if let Some(registered) = registry.model_mut(idx) {
                registered
                    .model
                    .watch_noise_classes(chunk_filters.noise.classes());
            }
        }
    }

    DaemonLoop::new(
        config,
        registry,
        pipeline_config,
        chunk_filters,
        species_filter,
        event_tx,
    )
    .spawn(&config.watch_dir, config.process_existing)
}

/// Step 1: every declared classifier, loaded or left out with why.
///
/// One entry on a station that has not asked for a second opinion, which is
/// the registry is then exactly the single model this loaded before, reached
/// through `primary()`.
///
/// Built before anything else, because every way the *set* can fail —
/// nothing configured, the primary will not load, a route naming a classifier
/// that was never declared — must stop the daemon at startup where the
/// journal and `--doctor` will show it. An extra classifier that will not
/// load does not (MDL-01): it is reported, by name and reason, and the rest
/// of the station runs.
///
/// # Errors
///
/// [`DaemonError::Model`] with the registry's refusal.
fn load_classifiers(config: &DaemonConfig) -> Result<ClassifierRegistry, DaemonError> {
    let specs = classifier_specs(config);
    let registry = match ClassifierRegistry::load(&specs, &config.model_routes, &config.model) {
        Ok(registry) => registry,
        Err(e) => {
            if let Some(observer) = config.on_model_state.as_ref() {
                observer.classifiers(&crate::inference::registry::refused_states(&specs, &e));
            }
            return Err(DaemonError::Model(e.to_string()));
        }
    };
    if let Some(observer) = config.on_model_state.as_ref() {
        observer.classifiers(&registry.states());
    }

    for (id, spec) in registry.specs() {
        tracing::info!(
            classifier = id,
            sample_rate = spec.sample_rate,
            window_secs = spec.window_secs(),
            waveform = spec.is_waveform(),
            "classifier loaded"
        );
    }
    if registry.len() > 1 {
        tracing::info!(
            classifiers = ?registry.ids(),
            routes = ?config.model_routes,
            "running more than one classifier"
        );
    }
    let unrouted = registry.unrouted();
    if !unrouted.is_empty() {
        tracing::warn!(
            classifiers = ?unrouted,
            "loaded but named by no MODEL_ROUTES entry, so they judge nothing: a source \
             without a route gets the primary alone. Name a source for each, as \
             MODEL_ROUTES=<source id>:<primary>+<classifier> (`{LOCAL_SOURCE}` for a \
             microphone set in the configuration file), or remove it to free its memory"
        );
    }

    let primary = registry.primary();
    tracing::info!(
        species_count = primary.model.labels().len(),
        labels_path = %config.labels_path.display(),
        "labels loaded"
    );
    tracing::info!(
        model_path = %config.model_path.display(),
        input_shape = ?primary.model.input_shape(),
        sample_rate = primary.spec.sample_rate,
        "model loaded, starting daemon"
    );
    Ok(registry)
}

/// Step 3: the species occurrence filter (the metadata / "geo" model), or the
/// reason it is off.
///
/// Every outcome that is not a loaded model with a position leaves the station
/// reporting every species the classifier knows, anywhere on Earth, in any
/// week, so each one is said at a level an operator will see and returned as
/// a [`FilterState`] the station publishes. None of them stops the detector:
/// a missing labels file used to (MDL-02), and a real station recorded
/// nothing for it.
///
/// The model is still loaded without coordinates — the operator's lists ride
/// on it, and a damaged file is worth finding before the position is set —
/// but the filter is not called on, because it cannot be asked what occurs
/// nowhere (MDL-04).
fn load_occurrence_filter(
    config: &DaemonConfig,
    classifier_labels: &LabelSet,
) -> (SpeciesFilter, FilterState) {
    let passthrough = || SpeciesFilter::new_passthrough(config.species_filter.clone());
    let Some(mdata_path) = config.metadata_model_path.as_ref() else {
        tracing::warn!(
            species = classifier_labels.len(),
            "no metadata model configured (METADATA_MODEL_PATH / BIRDNET_METADATA_MODEL): species occurrence filtering is OFF and every species in the model stays a candidate regardless of the station's location. Run `birdnet-behavior --doctor` for how to enable it"
        );
        return (
            passthrough(),
            FilterState::off(
                FilterOff::NotConfigured,
                "no occurrence model is configured (METADATA_MODEL_PATH); run \
                 `birdnet-behavior --doctor` for how to enable one",
            ),
        );
    };
    if let Err((reason, why)) =
        occurrence_files_present(mdata_path, config.metadata_labels_path.as_deref())
    {
        tracing::error!(
            path = %mdata_path.display(),
            "species occurrence filtering is OFF and every species in the model stays a \
             candidate: {why}"
        );
        return (passthrough(), FilterState::off(reason, why));
    }
    let meta_labels = match config
        .metadata_labels_path
        .as_ref()
        .map(|p| (p, LabelSet::load(p)))
    {
        None => None,
        Some((_, Ok(labels))) => Some(labels),
        Some((path, Err(e))) => {
            tracing::error!(
                path = %path.display(),
                error = %e,
                "metadata label file could not be read; species occurrence filtering is OFF"
            );
            return (
                passthrough(),
                FilterState::off(
                    FilterOff::LabelsUnreadable,
                    format!(
                        "its labels file {} cannot be read as labels ({e}); install the labels \
                         file that ships with the occurrence model",
                        path.display()
                    ),
                ),
            );
        }
    };
    match SpeciesFilter::load_with_vocabulary(
        mdata_path,
        meta_labels,
        classifier_labels,
        &species_aliases(config),
        config.species_filter.clone(),
    ) {
        Ok(filter) if config.latitude.zip(config.longitude).is_some() => {
            (filter, FilterState::Active { candidates: None })
        }
        Ok(filter) => {
            tracing::warn!(
                "the occurrence model loaded and the station has no coordinates \
                 (LATITUDE / LONGITUDE), so species occurrence filtering is OFF: it cannot be \
                 asked what occurs here. Set the station's position to switch it on"
            );
            (
                filter,
                FilterState::off(
                    FilterOff::NoCoordinates,
                    "the station has no LATITUDE and LONGITUDE, so the filter cannot be asked \
                     what occurs here; set the station's position",
                ),
            )
        }
        Err(e) => {
            tracing::error!(
                path = %mdata_path.display(),
                error = %e,
                "metadata model could not be used; species occurrence filtering is OFF and every species in the model stays a candidate"
            );
            (
                passthrough(),
                FilterState::off(
                    FilterOff::ModelUnusable,
                    format!(
                        "its model file {} could not be used ({e}); a truncated or damaged \
                         download does this, as does a model that is not an occurrence model: \
                         install it again",
                        mdata_path.display()
                    ),
                ),
            )
        }
    }
}

/// Whether the occurrence model's files are where the configuration says;
/// when one is not, why the filter is off, as a kind and in words.
fn occurrence_files_present(
    model: &Path,
    labels: Option<&Path>,
) -> Result<(), (FilterOff, String)> {
    if !model.exists() {
        return Err((
            FilterOff::ModelMissing,
            format!(
                "its model file {} is not there; install it again or correct \
                 METADATA_MODEL_PATH",
                model.display()
            ),
        ));
    }
    match labels {
        Some(path) if !path.exists() => Err((
            FilterOff::LabelsMissing,
            format!(
                "its labels file {} is not there; install the labels file that ships with \
                 the occurrence model, or correct METADATA_LABELS_PATH",
                path.display()
            ),
        )),
        _ => Ok(()),
    }
}

/// The operator's scientific-name alias file, or none.
///
/// An unreadable alias file is a warning, not a failure: the alignment works
/// without it, and taking the occurrence filter off a station over a typo'd
/// path would admit every species the classifier knows, wherever it is.
fn species_aliases(config: &DaemonConfig) -> std::collections::HashMap<String, String> {
    config
        .species_aliases_path
        .as_ref()
        .map_or_else(
            std::collections::HashMap::new,
            |p| match std::fs::read_to_string(p) {
                Ok(text) => {
                    let (map, skipped) = parse_alias_file(&text);
                    tracing::info!(
                        path = %p.display(),
                        aliases = map.len(),
                        skipped_lines = skipped,
                        "species alias file loaded"
                    );
                    map
                }
                Err(e) => {
                    tracing::warn!(
                        path = %p.display(),
                        error = %e,
                        "species alias file could not be read; continuing without it"
                    );
                    std::collections::HashMap::new()
                }
            },
        )
}

/// Step 4: say what each whole-chunk filter will do, and warn where one is
/// on and cannot act.
fn announce_chunk_filters(
    config: &DaemonConfig,
    pipeline_config: &PipelineConfig,
    chunk_filters: &ChunkFilters,
    registry: &ClassifierRegistry,
) {
    if chunk_filters.privacy.is_enabled() {
        tracing::info!(
            threshold = config.privacy_threshold,
            rank = config.privacy_rank,
            "privacy filter enabled"
        );
        if !privacy_can_fire(registry) {
            tracing::warn!(
                "privacy filter enabled but no label in the loaded model names a human class; \
                 the filter can never fire with this model"
            );
        }
    }
    if chunk_filters.noise.is_enabled() {
        tracing::info!(
            threshold = config.noise_threshold,
            classes = ?chunk_filters.noise.classes(),
            remember_secs = chunk_filters.noise.remember_secs(),
            "noise filter enabled"
        );
    } else if config.noise_remember_secs > 0.0 {
        // The window is a rider on the chunk filter and does nothing without
        // it. Said out loud, because an operator who set only the window has
        // configured a protection that cannot fire, and silence here would
        // look exactly like one that is working.
        tracing::warn!(
            remember_secs = config.noise_remember_secs,
            "NOISE_REMEMBER_SECS is set but the noise filter is off; set \
             NOISE_THRESHOLD above 0 and name at least one class for it to do anything"
        );
    }
    if chunk_filters.confirmation.enabled() {
        announce_confirmation(pipeline_config, chunk_filters);
    }
}

/// The repeat-confirmation filter's start-up line: what it requires at the
/// overlap the pipeline cuts, or that it cannot reject anything there.
fn announce_confirmation(pipeline_config: &PipelineConfig, chunk_filters: &ChunkFilters) {
    let overlap = effective_overlap_secs(pipeline_config);
    let chunk_secs = pipeline_config.chunk_duration_secs;
    if chunk_filters
        .confirmation
        .is_effective_at(overlap, chunk_secs)
    {
        tracing::info!(
            level = chunk_filters.confirmation.as_str(),
            overlap,
            required = chunk_filters
                .confirmation
                .required_confirmations_at(overlap, chunk_secs),
            "repeat-confirmation filter enabled"
        );
    } else {
        // Not a hard error: the level is still honoured, it just cannot
        // reject anything at this overlap, so a station that set it and saw
        // no change would otherwise have nothing to read.
        tracing::warn!(
            level = chunk_filters.confirmation.as_str(),
            overlap,
            chunk_secs,
            minimum_overlap = ?chunk_filters.confirmation.minimum_overlap(chunk_secs),
            "repeat-confirmation filter will not reject anything at this overlap: \
             a single window is already the whole neighbourhood it is asked to \
             agree with. Raise the analysis overlap or the confirmation level."
        );
    }
}

/// What one file's analysis came to, for the two callers that read it
/// differently (the live loop and the start-up backlog).
enum Analysed {
    /// Analysed; its events were handed on.
    Done,
    /// The pipeline failed on it.
    Failed(DaemonError),
    /// The event receiver is gone, so nothing more can be recorded.
    ReceiverGone,
}

/// Why a sweep ended.
enum SweepEnd {
    /// Every ready file was analysed.
    Swept,
    /// A stop arrived between files; these were not reached.
    Stop(Vec<PathBuf>),
    /// The event receiver is gone.
    ReceiverGone,
}

/// Step 5: everything the loop thread owns.
///
/// One struct rather than a closure capturing twenty locals, so each part of
/// the loop — a sweep, one file, the backlog — is a method that can be read,
/// and changed, on its own.
struct DaemonLoop {
    registry: ClassifierRegistry,
    pipeline_config: PipelineConfig,
    chunk_filters: ChunkFilters,
    species_filter: SpeciesFilter,
    model_state: Option<ModelStateObserver>,
    throughput: Option<ThroughputObserver>,
    in_flight: Option<InFlight>,
    shed_policy: Option<ShedPolicy>,
    /// The shed decision in force, logged on change rather than on every
    /// 500 ms sweep.
    shedding: Option<ShedReason>,
    shedder: Shedder,
    lat: Option<f64>,
    lon: Option<f64>,
    species_lists_provider: Option<SpeciesListsProvider>,
    /// When the operator's species lists were last re-read, so a change on
    /// /admin/species applies to the next file rather than the next restart.
    /// Checked per file rather than per loop tick: the tick is a 500 ms poll
    /// and the lists live in a database.
    lists_refreshed: Instant,
    /// The floor the processor publishes: before every file each classifier
    /// runs at the lower of it and its own threshold as loaded, so a
    /// per-species threshold below the global one reaches the model at all.
    threshold_floor: Option<Arc<ThresholdFloor>>,
    loaded_thresholds: Vec<f32>,
    /// Debounce watcher events: a clip is decoded only once its size has
    /// been stable for `FILE_SETTLE` (see `PendingFiles`), so an in-progress
    /// ffmpeg/RTSP segment isn't decoded mid-write (which fails with
    /// "unexpected end of file" and reprocesses the same growing file).
    pending: PendingFiles,
    /// One band verdict per source (ACC-14): see `band_check`.
    band_watch: BandWatch,
    event_tx: mpsc::SyncSender<DetectionEvent>,
    /// Bumped once per iteration and per file, so a watchdog can tell a hung
    /// pipeline (no progress) from an idle one.
    heartbeat: Arc<AtomicU64>,
}

impl DaemonLoop {
    fn new(
        config: &DaemonConfig,
        registry: ClassifierRegistry,
        pipeline_config: PipelineConfig,
        chunk_filters: ChunkFilters,
        species_filter: SpeciesFilter,
        event_tx: mpsc::SyncSender<DetectionEvent>,
    ) -> Self {
        let loaded_thresholds = super::loaded_thresholds(&registry);
        Self {
            registry,
            pipeline_config,
            chunk_filters,
            species_filter,
            model_state: config.on_model_state.clone(),
            throughput: config.on_file_analysed.clone(),
            in_flight: config.in_flight.clone(),
            shed_policy: config.shed.clone(),
            shedding: None,
            shedder: Shedder::default(),
            lat: config.latitude,
            lon: config.longitude,
            // An `Arc` clone, so the 'static loop thread can own it.
            species_lists_provider: config.species_lists_provider.clone(),
            lists_refreshed: Instant::now(),
            threshold_floor: config.threshold_floor.clone(),
            loaded_thresholds,
            pending: PendingFiles::new(),
            band_watch: BandWatch::default(),
            event_tx,
            heartbeat: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Attach the watcher and start the loop thread.
    ///
    /// # Errors
    ///
    /// [`DaemonError::Pipeline`] when the directory cannot be watched.
    fn spawn(self, watch_dir: &Path, process_existing: bool) -> Result<DaemonHandle, DaemonError> {
        let (stop_tx, stop_rx) = mpsc::channel();
        // Liveness of the loop thread itself: `true` until the thread returns.
        // The guard travels into the thread so every exit path clears it.
        let running = Arc::new(AtomicBool::new(true));
        let running_guard = super::RunningGuard(Arc::clone(&running));
        let heartbeat = Arc::clone(&self.heartbeat);
        // The `RecommendedWatcher` MUST live for the lifetime of the spawned
        // thread: dropping it stops delivery and closes the channel, and
        // `file_rx` then reports `Disconnected` — silently breaking the watch
        // path. It moves into the thread below for that reason.
        let (file_watcher, file_rx) =
            pipeline::watch_directory(watch_dir).map_err(DaemonError::Pipeline)?;
        let watch_dir = watch_dir.to_path_buf();
        std::thread::spawn(move || {
            let _watcher = file_watcher;
            // Dropped when this closure returns, however it returns, clearing
            // `DaemonHandle::running_flag`.
            let _alive = running_guard;
            self.run(&watch_dir, process_existing, &file_rx, &stop_rx);
        });
        Ok(DaemonHandle {
            stop_tx,
            heartbeat,
            running,
        })
    }

    /// The loop thread's body: the backlog, then sweep after sweep until a
    /// stop, a closed watcher or a closed receiver.
    fn run(
        mut self,
        watch_dir: &Path,
        process_existing: bool,
        file_rx: &mpsc::Receiver<PathBuf>,
        stop_rx: &mpsc::Receiver<()>,
    ) {
        tracing::info!("detection daemon started");
        // The pre-existing backlog is read here, on the loop thread, rather
        // than before signalling readiness. The event consumer is already
        // draining by now, so a large backlog cannot block startup past the
        // systemd TimeoutStartSec, and with a bounded event channel it applies
        // backpressure instead of dead-locking an undrained queue.
        if process_existing {
            self.apply_threshold_floor();
            self.process_existing_files(watch_dir);
        }
        loop {
            self.heartbeat.fetch_add(1, Ordering::Relaxed);
            if stop_rx.try_recv().is_ok() {
                self.stop(Vec::new(), process_existing);
                break;
            }
            // Collect every watcher event currently available (blocking
            // briefly for the first) into the pending set. A burst of modify
            // events for a file still being written collapses into one entry.
            match file_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(path) => {
                    self.pending.note(path, Instant::now());
                    while let Ok(path) = file_rx.try_recv() {
                        self.pending.note(path, Instant::now());
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("file watcher disconnected, stopping daemon");
                    break;
                }
            }
            let ready = self.settled();
            match self.sweep(ready, stop_rx) {
                SweepEnd::Swept => {}
                SweepEnd::Stop(left) => {
                    self.stop(left, process_existing);
                    break;
                }
                SweepEnd::ReceiverGone => return,
            }
        }
        tracing::info!("detection daemon stopped");
    }

    /// Say what a stop leaves unanalysed: segments a stop cut off mid-sweep
    /// (`PIPE8b`), and whatever is still settling.
    fn stop(&mut self, left: Vec<PathBuf>, process_existing: bool) {
        tracing::info!("detection daemon stopping");
        let pending = std::mem::take(&mut self.pending);
        report_unanalysed(
            left.into_iter().chain(pending.into_paths()),
            process_existing,
            self.throughput.as_ref(),
        );
    }

    /// The files that have finished being written, less any the shed policy
    /// skips. Polling the size each sweep means a clip settles even after its
    /// last watcher event, so the final segment is never stranded.
    fn settled(&mut self) -> Vec<PathBuf> {
        let settled = self
            .pending
            .drain_settled_reporting(Instant::now(), FILE_SETTLE, |p| {
                std::fs::metadata(p).map(|m| m.len()).ok()
            });
        // A segment the watcher announced that is gone before the pipeline
        // could open it is audio that was recorded and never analysed (PR-1 /
        // S-3): a purge running ahead of a backlogged pipeline. It used to
        // fall out of the pending set without a word.
        for path in &settled.vanished {
            tracing::warn!(
                file = %path.display(),
                "segment vanished before analysis — recorded audio the pipeline never \
                 read is gone; if this repeats, the stream directory is being drained \
                 faster than inference keeps up"
            );
            if let Some(observer) = self.throughput.as_ref() {
                observer.dropped(path);
            }
        }
        // The queue is what is still settling plus what this sweep will
        // analyse (PR-2). Published once per sweep; the shed policy decides
        // on it.
        let queue_depth = self.pending.len() + settled.ready.len();
        if let Some(observer) = self.throughput.as_ref() {
            observer.queue_depth(queue_depth);
        }
        let decision = self
            .shed_policy
            .as_ref()
            .and_then(|p| p.decide(queue_depth));
        self.note_shedding(decision, queue_depth);
        let Some(reason) = decision else {
            return settled.ready;
        };
        let (analyse, shed) = self.shedder.split(settled.ready);
        for path in &shed {
            tracing::debug!(file = %path.display(), reason = reason.as_str(), "segment shed");
            if let Some(observer) = self.throughput.as_ref() {
                observer.shed(path, reason);
            }
        }
        analyse
    }

    /// Log a change in the shed decision, once.
    fn note_shedding(&mut self, decision: Option<ShedReason>, queue_depth: usize) {
        if decision == self.shedding {
            return;
        }
        if let Some(reason) = decision {
            tracing::warn!(
                queue_depth,
                reason = reason.as_str(),
                "analysing one segment in two until this clears — inference is \
                 behind real time here; the skipped segments are counted in \
                 birdnet_segments_shed_total, not lost quietly"
            );
        } else {
            tracing::info!(
                queue_depth,
                "shedding stopped; every segment is analysed again"
            );
        }
        self.shedding = decision;
    }

    /// Analyse every ready file, checking for a stop between files: one sweep
    /// can hold a long backlog, and shutdown waits on this loop.
    fn sweep(&mut self, ready: Vec<PathBuf>, stop_rx: &mpsc::Receiver<()>) -> SweepEnd {
        let mut ready = ready.into_iter();
        while let Some(path) = ready.next() {
            if stop_rx.try_recv().is_ok() {
                let mut left = vec![path];
                left.extend(ready);
                return SweepEnd::Stop(left);
            }
            // Keep the watchdog fed if a single sweep processes several files.
            self.heartbeat.fetch_add(1, Ordering::Relaxed);
            self.refresh_species_lists();
            // Stamp a correlation ID on every event we publish for this file
            // so the operator can trace one file through the entire pipeline
            // by grepping a single string.
            let correlation_id = new_event_correlation_id();
            // DEBUG, not INFO (OB-15): with "file processing complete" this
            // was two lines per 15 s segment, 92 % of the journal's volume and
            // 1.6-2.8 GB a year, carrying nothing the
            // `birdnet_files_analysed_total` counter does not. The
            // per-detection line keeps the correlation id at INFO.
            tracing::debug!(
                correlation_id = %correlation_id,
                file = %path.display(),
                "begin processing file"
            );
            self.apply_threshold_floor();
            match self.analyse(&path, &correlation_id) {
                Analysed::Done => {}
                Analysed::ReceiverGone => {
                    tracing::warn!(
                        correlation_id = %correlation_id,
                        "event receiver dropped, stopping daemon"
                    );
                    return SweepEnd::ReceiverGone;
                }
                Analysed::Failed(e) => self.report_failure(&path, &correlation_id, &e),
            }
        }
        SweepEnd::Swept
    }

    /// Re-read the operator's species lists if they have gone stale.
    ///
    /// A failed or absent provider leaves the previous lists in place rather
    /// than clearing them: dropping an exclude list on a transient database
    /// error would start recording exactly the species the operator asked to
    /// suppress.
    fn refresh_species_lists(&mut self) {
        if let Some(provider) = self.species_lists_provider.as_ref()
            && self.lists_refreshed.elapsed() >= SPECIES_LISTS_TTL
        {
            self.species_filter.set_lists(provider.get());
            self.lists_refreshed = Instant::now();
        }
    }

    fn apply_threshold_floor(&mut self) {
        super::apply_threshold_floor(
            &mut self.registry,
            &self.loaded_thresholds,
            self.threshold_floor.as_deref(),
        );
    }

    /// One file through the pipeline, its events handed on.
    ///
    /// The file is claimed for as long as the pipeline reads it (PR-1 / S-3):
    /// the stream directory's purge skips a claimed name. Each event carries a
    /// share of the claim, so it lasts until the processor has cut the clips
    /// too (`PIPE8a`).
    fn analyse(&mut self, path: &Path, correlation_id: &str) -> Analysed {
        let lease = self
            .in_flight
            .as_ref()
            .map(|table| Arc::new(table.claim(path)));
        let route = route_for_segment(&self.registry, path);
        let outcome = process_and_infer_filtered(
            path,
            &self.pipeline_config,
            &mut self.registry,
            &route,
            &self.chunk_filters,
            &mut self.species_filter,
            self.model_state.as_ref(),
            self.lat,
            self.lon,
            correlation_id,
        );
        // After every file, failed or not: a classifier that fails every
        // window fails every file, and that is the case this report exists
        // for.
        if let Some(observer) = self.throughput.as_ref() {
            observer.model_health(&self.registry.health());
        }
        let file = match outcome {
            Ok(file) => file,
            Err(e) => return Analysed::Failed(e),
        };
        warn_if_narrow(&mut self.band_watch, &self.registry, &route, path, &file);
        // Counted here and not before the call: this says "analysed", and a
        // file the pipeline failed on was not. The counter is the only thing
        // that separates a model answering nothing from a pipeline that is not
        // running, so it must mean exactly one thing.
        if let Some(observer) = self.throughput.as_ref() {
            observer.analysed(path);
            observer.filtered(path, &file.tally);
        }
        for mut event in file.events {
            event.lease.clone_from(&lease);
            if self.event_tx.send(event).is_err() {
                return Analysed::ReceiverGone;
            }
        }
        Analysed::Done
    }

    /// A live file the pipeline failed on: a warning, and a dropped segment
    /// when it is gone — the same loss as a vanished pending file, counted
    /// the same way.
    fn report_failure(&self, path: &Path, correlation_id: &str, error: &DaemonError) {
        if path.exists() {
            tracing::warn!(
                correlation_id = %correlation_id,
                file = %path.display(),
                error = %error,
                "failed to process file"
            );
        } else {
            tracing::warn!(
                correlation_id = %correlation_id,
                file = %path.display(),
                "segment vanished while being analysed — recorded audio the \
                 pipeline never finished reading is gone"
            );
            if let Some(observer) = self.throughput.as_ref() {
                observer.dropped(path);
            }
        }
    }

    /// Process any audio files already present in the watch directory.
    ///
    /// A file modified within [`FILE_SETTLE`] may still be being written, so
    /// it is handed to `pending` — the watcher's queue, which analyses it once
    /// it has settled — instead of being read now (`PIPE8c`). Read here, it was
    /// decoded part-written and then again by the watcher when the recorder
    /// finished it. Each file read here is claimed in `in_flight` like the
    /// loop's, and its events carry the claim.
    fn process_existing_files(&mut self, dir: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    dir = %dir.display(),
                    error = %e,
                    "cannot read watch directory for existing files"
                );
                return;
            }
        };

        let mut count = 0_u32;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !is_audio_file(&path) {
                continue;
            }
            let recently_written = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_none_or(|age| age < FILE_SETTLE);
            if recently_written {
                self.pending.note(path, Instant::now());
                continue;
            }
            let correlation_id = new_event_correlation_id();
            // As in the live loop: a backlog the station restarts into is read
            // by the same classifiers, and one failing them is no less broken.
            match self.analyse(&path, &correlation_id) {
                Analysed::Done => count += 1,
                // Surfaced rather than swallowed: a consumer that dropped
                // mid-backlog would otherwise leave this spinning through the
                // rest of the watch directory pointlessly. The main loop treats
                // a closed receiver as fatal; so does this.
                Analysed::ReceiverGone => {
                    tracing::debug!("existing-file backlog stopping: event receiver closed");
                    return;
                }
                Analysed::Failed(e) => {
                    tracing::debug!(
                        correlation_id = %correlation_id,
                        file = %path.display(),
                        error = %e,
                        "skipping existing file"
                    );
                }
            }
        }
        if count > 0 {
            tracing::info!(count, "processed existing audio files");
        }
    }
}

/// The whole-chunk filters, sized to the chunks the pipeline actually cuts.
///
/// `pipeline_config` is the one [`fit_pipeline_to_classifiers`] adjusted, not
/// `config.pipeline`: for BirdNET+ V3.0 the configured 3.0 s chunk is replaced
/// by 4.5 s, and a privacy filter told 3.0 s measured every flagged chunk's
/// speech span a second and a half short, so a clip reaching into the tail of
/// a chunk with a human in it was kept.
fn chunk_filters(config: &DaemonConfig, pipeline_config: &PipelineConfig) -> ChunkFilters {
    ChunkFilters {
        privacy: PrivacyFilter::new(config.privacy_threshold)
            .with_rank_cutoff(config.privacy_rank)
            .with_clip_reach(config.privacy_clip_reach)
            .with_chunk_secs(pipeline_config.chunk_duration_secs),
        noise: NoiseFilter::new(config.noise_threshold, config.noise_classes.clone())
            .remembering(config.noise_remember_secs),
        confirmation: config.confirmation,
    }
}

/// Whether any loaded classifier names a human class, so the privacy filter
/// can fire at all (MMP-16).
///
/// Any of them, not the primary's: the privacy gate reads the highest human
/// score any routed classifier reported (`infer_chunk`).
fn privacy_can_fire(registry: &ClassifierRegistry) -> bool {
    (0..registry.len())
        .filter_map(|idx| registry.model(idx))
        .any(|m| m.model.has_human_labels())
}

/// How far consecutive chunks overlap as the pipeline will cut them (MMP-16):
/// the chunk length less the step [`crate::detection::pipeline::chunk_step_samples`]
/// gives, which is the shortest classifier window when there are several,
/// not the configured overlap.
fn effective_overlap_secs(pipeline_config: &PipelineConfig) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let step_secs = crate::detection::pipeline::chunk_step_samples(pipeline_config) as f32
        / pipeline_config.target_sample_rate as f32;
    (pipeline_config.chunk_duration_secs - step_secs).max(0.0)
}

/// Fit the pipeline's resample rate, input mode and chunk arithmetic to the
/// loaded classifiers.
///
/// Read from the **registry's** effective specs, never from
/// [`crate::inference::model::BirdNetModel::input_spec`] /
/// `infer_sample_rate`: the model can only derive its rate from the input
/// shape, and a declared `MODEL_SAMPLE_RATE` is applied by the registry on top
/// of that derivation. Reading the model directly resampled every recording to
/// the shape-guessed rate and cut chunks from it, so a declared rate changed
/// the log line and nothing else — the classifier was fed audio at the wrong
/// rate, with a window of the wrong length in seconds.
pub fn fit_pipeline_to_classifiers(
    pipeline_config: &mut PipelineConfig,
    registry: &ClassifierRegistry,
) {
    let spec = registry.primary().spec;
    // V2.4 → [1, 144_000] = 48 kHz × 3 s; V3.0 → [1, 96_000] = 32 kHz × 3 s,
    // unless the operator declared otherwise.
    let model_sample_rate = spec.sample_rate;
    if pipeline_config.target_sample_rate != model_sample_rate {
        tracing::info!(
            configured = pipeline_config.target_sample_rate,
            model = model_sample_rate,
            "adjusting pipeline sample rate to match model"
        );
        pipeline_config.target_sample_rate = model_sample_rate;
    }
    // Asked of the model rather than guessed from its sample rate (`G-10`
    // Stage 1). The guess here was `infer_sample_rate() == 32_000`, which read
    // "32 kHz" as "waveform" — true of V3.0 by coincidence, and false of V2.4,
    // which is a 48 kHz waveform model that was therefore sent a mel
    // spectrogram zero-padded to three-quarters of its input tensor.
    let raw_mode = spec.is_waveform();
    if raw_mode != pipeline_config.raw_audio_input {
        tracing::info!(
            raw_audio_input = raw_mode,
            "adjusting pipeline input mode to match model"
        );
        pipeline_config.raw_audio_input = raw_mode;
    }

    // A classifier at another rate is fed its own resample of each recording
    // (MMP-01): one more resample per file and per rate, logged because on a
    // Pi that is a real cost.
    let extra_rates = registry.extra_sample_rates();
    if !extra_rates.is_empty() {
        tracing::info!(
            primary_rate = model_sample_rate,
            extra_rates = ?extra_rates,
            "classifiers want different sample rates: each recording is resampled once more \
             per extra rate, from the decoded audio, and each classifier reads its own"
        );
    }
    pipeline_config.extra_sample_rates = extra_rates;

    // Adopt the model's recommended chunk length when it differs from the
    // pipeline default. This matters most for V3.0 preview3 (dynamic input
    // shape): with 3.0 s × 32 kHz = 96 000 samples the Magpie reference
    // confidence on the bundled WAV is ~0.52, but at 4.5 s × 32 kHz =
    // 144 000 samples it rises to ~0.72. The model accepts variable length
    // so this is purely a per-chunk accuracy tuning. Fixed-shape V2.4 keeps
    // its trained 3.0 s window.
    //
    // With more than one classifier the chunk is cut to the longest window
    // and stepped by the shortest (`G-10` Stage 4). One classifier leaves both
    // equal, so this is the single-model arithmetic unchanged.
    let (longest, shortest) = registry.window_bounds();
    #[allow(clippy::cast_precision_loss)]
    let longest_secs = longest as f32 / spec.sample_rate as f32;
    #[allow(clippy::cast_precision_loss)]
    let shortest_secs = shortest as f32 / spec.sample_rate as f32;
    if longest != shortest {
        #[allow(clippy::cast_precision_loss)]
        let ratio = longest as f32 / shortest as f32;
        tracing::info!(
            longest_window_secs = longest_secs,
            shortest_window_secs = shortest_secs,
            extra_inference_ratio = ratio,
            "classifiers want different windows: chunking to the longest and stepping by the \
             shortest, so no classifier sees less than it would alone. Classifiers with the \
             longer window run proportionally more inferences"
        );
        pipeline_config.chunk_step_secs = Some(shortest_secs);
    }

    // The longest window across the loaded classifiers. For the one classifier
    // every station runs today this is exactly `model.recommended_chunk_secs()`
    // — the call this line made before Stage 4 — on any waveform shape,
    // because `input_spec`'s window and `recommended_chunk_samples` derive the
    // same number from the same shape. Gated by
    // `the_window_and_the_chunk_recommendation_agree_on_every_waveform_shape`.
    //
    // A mel shape is the one place the two part, and there the window is the
    // right of them: `recommended_chunk_samples` would read a count of
    // spectrogram columns as a sample count and ask for a six-millisecond
    // chunk. No model here takes mel input today.
    let model_chunk_secs = longest_secs;
    let configured_chunk_secs = pipeline_config.chunk_duration_secs;
    if (model_chunk_secs - configured_chunk_secs).abs() > 0.01 {
        tracing::info!(
            configured_chunk_secs = configured_chunk_secs,
            model_chunk_secs,
            "adjusting pipeline chunk duration to match model recommendation"
        );
        pipeline_config.chunk_duration_secs = model_chunk_secs;
    }
}

/// The classifiers that judge a segment: its source's `MODEL_ROUTES` entry.
///
/// A capture source names its segments with its id — the `audio_sources` row
/// id, the key routes are written against — so the id is read back from the
/// file name. A name without one (a lone microphone's BirdNET-Pi-style name, or
/// a file dropped in by hand) is the [`LOCAL_SOURCE`], whose route is written
/// as `local:…` (MDL-07); a source with no route gets the primary. Before this
/// every segment got the default, whatever its source, and no route ever
/// applied.
fn route_for_segment(registry: &ClassifierRegistry, path: &Path) -> Vec<usize> {
    registry.route_for(segment_source(path).as_deref())
}

/// The source id a segment's file name carries, if any.
fn segment_source(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::detection::types::RecordingFile::parse)
        .and_then(|f| f.rtsp_id)
}

/// Whether a segment's source cannot carry the band its classifiers listen to
/// (ACC-14), judged once per source and rate.
///
/// Here, at the first decoded recording of each source, and not when capture
/// starts: capture does not know the classifiers' rates (the registry is loaded
/// on this thread), and the decoded rate is what actually reached them,
/// including a microphone the capture layer fell back to another rate for. The
/// source is the segment's id as the file name carries it (`default` for a
/// name with none), and the classifier rate is the highest among those the
/// route sends it to: the widest band any of them listens to.
///
/// An RTSP stream cannot be judged here. ffmpeg resamples it to 48 kHz before
/// writing the segment, so its recordings decode at 48 kHz whatever the camera
/// sends; the capture layer reads the camera's own rate from ffmpeg instead.
fn band_check(
    watch: &mut BandWatch,
    registry: &ClassifierRegistry,
    route: &[usize],
    path: &Path,
    source_rate_hz: u32,
) -> Option<(String, BandWarning)> {
    let model_rate = route
        .iter()
        .filter_map(|&idx| registry.model(idx))
        .map(|m| m.spec.sample_rate)
        .max()?;
    let source = segment_source(path).unwrap_or_else(|| "default".to_owned());
    watch
        .first_look(&source, source_rate_hz, model_rate)
        .map(|w| (source, w))
}

/// [`band_check`], logged.
fn warn_if_narrow(
    watch: &mut BandWatch,
    registry: &ClassifierRegistry,
    route: &[usize],
    path: &Path,
    file: &super::FilteredFile,
) {
    if let Some((source, warning)) = band_check(watch, registry, route, path, file.source_rate_hz) {
        tracing::warn!(source = %source, "{warning}");
    }
}

/// Say what a stopping daemon leaves unanalysed (`PIPE8b`).
///
/// Nothing reads the watch directory's backlog at the next start unless
/// `process_existing` is set, so without it these segments are never analysed:
/// each is reported as dropped, and one warning says how many. With it they
/// are deferred, not lost, and are logged as such.
fn report_unanalysed(
    paths: impl Iterator<Item = PathBuf>,
    process_existing: bool,
    throughput: Option<&super::ThroughputObserver>,
) {
    let paths: Vec<PathBuf> = paths.filter(|p| is_audio_file(p)).collect();
    if paths.is_empty() {
        return;
    }
    if process_existing {
        tracing::info!(
            segments = paths.len(),
            "stopping before these segments were analysed; the next start's backlog pass \
             reads them if they are still there"
        );
        return;
    }
    tracing::warn!(
        segments = paths.len(),
        "stopping before these recorded segments were analysed, and nothing will analyse \
         them later: the next start does not read the backlog unless --process-existing is set"
    );
    if let Some(observer) = throughput {
        for path in &paths {
            observer.dropped(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::model::ModelConfig;

    /// A daemon over the tiny bundled model, watching an empty directory.
    fn tiny_config(tmp: &std::path::Path) -> DaemonConfig {
        const TINY_V24: &[u8] = include_bytes!("../../testdata/tiny_v24_test.onnx");

        let watch_dir = tmp.join("recs");
        std::fs::create_dir_all(&watch_dir).unwrap();
        let model_path = tmp.join("model.onnx");
        std::fs::write(&model_path, TINY_V24).unwrap();
        let labels_path = tmp.join("labels.txt");
        let labels = (0..11)
            .map(|i| format!("Species{i}_Bird {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&labels_path, labels).unwrap();

        DaemonConfig {
            watch_dir,
            model_path,
            labels_path,
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
            on_model_state: None,
            on_file_analysed: None,
            in_flight: None,
            shed: None,
            species_filter: crate::inference::species_filter::SpeciesFilterConfig::default(),
            species_lists_provider: None,
            privacy_threshold: 0.0,
            privacy_rank: 0,
            privacy_clip_reach: crate::detection::privacy::ClipReach::default(),
            noise_threshold: 0.0,
            noise_remember_secs: 0.0,
            noise_classes: Vec::new(),
            confirmation: crate::detection::corroboration::ConfirmationLevel::Off,
            latitude: None,
            longitude: None,
            species_thresholds: std::collections::HashMap::new(),
            threshold_floor: None,
        }
    }

    /// `MODEL_ID`, `MODEL_THRESHOLD` and `MODEL_SAMPLE_RATE` were resolved into
    /// the primary's spec by the plan and then thrown away: the daemon rebuilt
    /// the primary as `birdnet` with neither. A route naming the primary by
    /// its configured id then stopped the daemon at startup
    /// (`UnknownRouteTarget`), and without routes the other two were ignored.
    #[test]
    fn the_primary_is_loaded_as_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.primary_id = "mybird".to_owned();
        config.primary_threshold = Some(0.6);
        config.primary_sample_rate = Some(48_000);
        config
            .model_routes
            .insert("garden".to_owned(), vec!["mybird".to_owned()]);

        let specs = classifier_specs(&config);
        assert_eq!(specs[0].id, "mybird");
        assert_eq!(specs[0].threshold, Some(0.6));
        assert_eq!(specs[0].sample_rate, Some(48_000));

        let (event_tx, _event_rx) = mpsc::sync_channel(64);
        let handle = run_daemon(&config, event_tx).expect("a route to the configured primary");
        handle.stop();
    }

    /// ACC-14 at the call site: a source whose recordings decode at 16 kHz is
    /// reported once, by its id, against the 48 kHz the tiny V2.4-shaped
    /// classifier listens at; a 48 kHz source is not reported; and a source
    /// already judged is not judged again.
    #[test]
    fn a_narrow_source_is_reported_once_at_its_first_recording() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tiny_config(tmp.path());
        let registry = ClassifierRegistry::load(
            &classifier_specs(&config),
            &config.model_routes,
            &config.model,
        )
        .expect("loads");
        let cam = Path::new("/recs/2026-05-19-birdnet-cam1-09:00:00.wav");
        let mic = Path::new("/recs/2026-05-19-birdnet-09:00:00.wav");
        let route = route_for_segment(&registry, cam);
        let mut watch = BandWatch::default();

        let (source, warning) =
            band_check(&mut watch, &registry, &route, cam, 16_000).expect("a 16 kHz camera");
        assert_eq!(source, "cam1");
        assert_eq!(warning.model_rate_hz, 48_000);
        assert!(band_check(&mut watch, &registry, &route, cam, 16_000).is_none());
        assert!(band_check(&mut watch, &registry, &route, mic, 48_000).is_none());
        let (source, _) =
            band_check(&mut watch, &registry, &route, mic, 8_000).expect("an 8 kHz microphone");
        assert_eq!(source, "default");
    }

    /// A declared `MODEL_SAMPLE_RATE` has to reach the pipeline, not only the
    /// registry. The daemon fitted the pipeline from the model's own
    /// `infer_sample_rate()` / `input_spec()`, which derive the rate from the
    /// tensor shape, so the declared rate was logged and then ignored: the
    /// tiny V2.4-shaped model ([1, 144 000]) declared at 32 kHz was fed audio
    /// resampled to 48 kHz, in 3.0 s chunks instead of 4.5 s.
    #[test]
    fn a_declared_sample_rate_reaches_the_pipeline() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.primary_sample_rate = Some(32_000);
        let registry = ClassifierRegistry::load(
            &classifier_specs(&config),
            &config.model_routes,
            &config.model,
        )
        .expect("loads");

        let mut pipeline_config = config.pipeline.clone();
        fit_pipeline_to_classifiers(&mut pipeline_config, &registry);
        assert_eq!(pipeline_config.target_sample_rate, 32_000);
        assert!(
            (pipeline_config.chunk_duration_secs - 4.5).abs() < 1e-4,
            "144 000 samples at 32 kHz is 4.5 s, got {}",
            pipeline_config.chunk_duration_secs
        );
        assert!(pipeline_config.raw_audio_input);

        // Counterpart: without a declaration the shape-derived 48 kHz / 3.0 s
        // stands, so the assertion above is about the declaration.
        config.primary_sample_rate = None;
        let registry = ClassifierRegistry::load(
            &classifier_specs(&config),
            &config.model_routes,
            &config.model,
        )
        .expect("loads");
        let mut pipeline_config = config.pipeline;
        fit_pipeline_to_classifiers(&mut pipeline_config, &registry);
        assert_eq!(pipeline_config.target_sample_rate, 48_000);
        assert!((pipeline_config.chunk_duration_secs - 3.0).abs() < 1e-4);
    }

    /// MMP-16: the startup check of whether the confirmation filter can
    /// reject anything judged the *configured* overlap, while the pipeline
    /// steps by the shortest classifier window. V2.4 (3 s) beside a 4.5 s
    /// classifier cuts 4.5 s chunks every 3 s, a 1.5 s overlap with none
    /// configured, at which `balanced` does reject; the daemon logged that it
    /// "will not reject anything".
    #[test]
    fn the_confirmation_check_reads_the_overlap_the_pipeline_cuts() {
        use crate::detection::corroboration::ConfirmationLevel;
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config
            .extra_models
            .push(crate::inference::registry::ModelSpec {
                id: "longer".to_owned(),
                model_path: config.model_path.clone(),
                labels_path: config.labels_path.clone(),
                threshold: None,
                sample_rate: Some(32_000),
                output_scale: None,
            });
        let registry = ClassifierRegistry::load(
            &classifier_specs(&config),
            &config.model_routes,
            &config.model,
        )
        .expect("loads");
        let mut fitted = config.pipeline.clone();
        fit_pipeline_to_classifiers(&mut fitted, &registry);
        assert!(
            fitted.chunk_overlap_secs.abs() < 1e-6,
            "precondition: none configured"
        );
        let overlap = effective_overlap_secs(&fitted);
        assert!(
            (overlap - 1.5).abs() < 1e-3,
            "4.5 s chunks every 3 s: {overlap}"
        );
        assert!(
            ConfirmationLevel::Balanced.is_effective_at(overlap, fitted.chunk_duration_secs),
            "balanced rejects at the overlap the pipeline cuts"
        );

        // Counterpart: with one classifier the overlap is the configured one,
        // none or some, so the number above is the step's doing.
        let single = ClassifierRegistry::load(
            &classifier_specs(&tiny_config(tmp.path())),
            &config.model_routes,
            &config.model,
        )
        .expect("loads");
        for configured in [0.0_f32, 1.0] {
            let mut alone = config.pipeline.clone();
            alone.chunk_overlap_secs = configured;
            fit_pipeline_to_classifiers(&mut alone, &single);
            let got = effective_overlap_secs(&alone);
            assert!(
                (got - configured).abs() < 1e-3,
                "{configured} configured, {got}"
            );
        }
    }

    /// MMP-16: "no label in the loaded model names a human class" was judged
    /// on the primary alone, so a station whose second classifier names one
    /// was told its privacy filter could never fire.
    #[test]
    fn the_privacy_check_reads_every_classifier() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        let human = tmp.path().join("human.txt");
        let mut labels: Vec<String> = (0..10).map(|i| format!("Other{i}_Other {i}")).collect();
        labels.push("Human vocal_Human vocal".to_owned());
        std::fs::write(&human, labels.join("\n")).unwrap();
        let alone = ClassifierRegistry::load(
            &classifier_specs(&config),
            &config.model_routes,
            &config.model,
        )
        .expect("loads");
        assert!(
            !privacy_can_fire(&alone),
            "precondition: the primary names no human"
        );

        config
            .extra_models
            .push(crate::inference::registry::ModelSpec {
                id: "second".to_owned(),
                model_path: config.model_path.clone(),
                labels_path: human,
                threshold: None,
                sample_rate: None,
                output_scale: None,
            });
        let both = ClassifierRegistry::load(
            &classifier_specs(&config),
            &config.model_routes,
            &config.model,
        )
        .expect("loads");
        assert!(
            privacy_can_fire(&both),
            "the second classifier names a human"
        );
    }

    /// The privacy filter measures a flagged chunk's span with the chunk
    /// length the pipeline actually cuts. It was given the configured 3.0 s
    /// after the pipeline had been moved to the model's 4.5 s, so the last
    /// second and a half of every chunk with a human in it was outside the
    /// span a saved clip was checked against.
    #[test]
    fn the_privacy_filter_is_sized_to_the_fitted_chunk() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.privacy_threshold = 0.5;
        assert!((config.pipeline.chunk_duration_secs - 3.0).abs() < 1e-6);
        let mut fitted = config.pipeline.clone();
        fitted.chunk_duration_secs = 4.5;

        let filters = chunk_filters(&config, &fitted);
        assert!((filters.privacy.chunk_secs() - 4.5).abs() < 1e-6);
    }

    /// Write `secs` seconds of low noise at 48 kHz into `dir/name`.
    fn write_noise(dir: &std::path::Path, name: &str, secs: u32) -> std::path::PathBuf {
        let path = dir.join(name);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        let mut x: u32 = 1;
        for _ in 0..48_000 * secs {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            w.write_sample(((x >> 16) as i16) / 8).unwrap();
        }
        w.finalize().unwrap();
        path
    }

    /// `PIPE8a`: a segment stays claimed until its detections have been handled.
    ///
    /// The claim was a local in the loop, dropped as soon as the file's events
    /// were queued. The processor reads the segment again afterwards to cut
    /// each detection's clip, and a disk-full purge — oldest first, which is
    /// exactly a segment whose events are still queued behind a busy
    /// processor — was free to delete it in between.
    #[test]
    fn a_segment_stays_claimed_while_its_events_are_queued() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.model.confidence_threshold = 0.0;
        let table = super::super::InFlight::new();
        config.in_flight = Some(table.clone());
        let (event_tx, event_rx) = mpsc::sync_channel(4096);
        let handle = run_daemon(&config, event_tx).expect("daemon starts");
        write_noise(&config.watch_dir, "2026-05-19-birdnet-09:00:00.wav", 3);

        let first = event_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("precondition: the tiny model emits events at threshold 0");
        // Let the loop finish the file and drop its own hold.
        std::thread::sleep(Duration::from_millis(1_500));
        assert_eq!(
            table.names(),
            ["2026-05-19-birdnet-09:00:00.wav"],
            "the segment was released while its events were still queued"
        );

        // Counterpart: once every event is handled, the claim is gone.
        drop(first);
        while event_rx.try_recv().is_ok() {}
        assert!(table.names().is_empty(), "{:?}", table.names());
        handle.stop();
    }

    /// Start a daemon, give it one segment, and stop it before the segment
    /// settles. Returns the paths reported as dropped.
    fn stop_with_a_segment_settling(process_existing: bool) -> Vec<std::path::PathBuf> {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.process_existing = process_existing;
        let dropped = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let d = std::sync::Arc::clone(&dropped);
        config.on_file_analysed = Some(
            super::super::ThroughputObserver::new(|_| {})
                .with_dropped(move |p| d.lock().unwrap().push(p.to_path_buf())),
        );
        let (event_tx, _event_rx) = mpsc::sync_channel(64);
        let handle = run_daemon(&config, event_tx).expect("daemon starts");
        std::thread::sleep(Duration::from_millis(300));
        write_noise(&config.watch_dir, "2026-05-19-birdnet-09:00:00.wav", 1);
        // Long enough for the watcher event, well short of FILE_SETTLE.
        std::thread::sleep(Duration::from_millis(800));
        handle.stop();
        let deadline = Instant::now() + Duration::from_secs(10);
        while handle.is_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!handle.is_running(), "the loop exits on stop");
        dropped.lock().unwrap().clone()
    }

    /// `PIPE8b`: a segment the daemon stops before analysing is reported.
    ///
    /// On stop the loop broke out and discarded whatever was still settling,
    /// without a word. Nothing reads the stream directory's backlog at the
    /// next start unless `--process-existing` is set, so that audio was never
    /// analysed and nothing said so.
    #[test]
    fn a_segment_left_at_stop_is_reported() {
        let dropped = stop_with_a_segment_settling(false);
        assert_eq!(dropped.len(), 1, "{dropped:?}");
        // Counterpart: when the next start's backlog pass will read it, it is
        // not lost, and not reported as lost.
        assert!(stop_with_a_segment_settling(true).is_empty());
    }

    /// `PIPE8c`: the startup backlog pass leaves a segment still being written
    /// to the watcher, so it is analysed once, whole.
    ///
    /// The pass analysed every file in the directory at once, with no settle
    /// check and no claim. A segment the recorder was still writing was
    /// decoded part-written, and then again by the watcher when it finished.
    #[test]
    fn a_segment_being_written_at_start_is_analysed_once() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.process_existing = true;
        let analysed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let a = std::sync::Arc::clone(&analysed);
        config.on_file_analysed = Some(super::super::ThroughputObserver::new(move |p| {
            a.lock()
                .unwrap()
                .push(p.file_name().unwrap().to_string_lossy().into_owned());
        }));
        // An old, finished segment, and one the recorder has just started.
        let old = write_noise(&config.watch_dir, "2026-05-19-birdnet-08:00:00.wav", 3);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        write_noise(&config.watch_dir, "2026-05-19-birdnet-09:00:00.wav", 1);

        let (event_tx, _event_rx) = mpsc::sync_channel(4096);
        let handle = run_daemon(&config, event_tx).expect("daemon starts");
        std::thread::sleep(Duration::from_millis(300));
        // The recorder finishes the segment.
        write_noise(&config.watch_dir, "2026-05-19-birdnet-09:00:00.wav", 3);
        std::thread::sleep(Duration::from_millis(4_000));
        handle.stop();

        let seen = analysed.lock().unwrap().clone();
        let count = |n: &str| seen.iter().filter(|s| s.as_str() == n).count();
        assert_eq!(count("2026-05-19-birdnet-09:00:00.wav"), 1, "{seen:?}");
        // Counterpart: the finished backlog is still read, once.
        assert_eq!(count("2026-05-19-birdnet-08:00:00.wav"), 1, "{seen:?}");
    }

    /// `e853776`: after a file, the loop reports each classifier's health —
    /// the report the station-health alert, `/api/v2/health` and the
    /// `birdnet_model_*` series are built from.
    #[test]
    fn each_file_reports_the_classifiers_health() {
        use crate::inference::health::ModelHealth;
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.process_existing = true;
        let reported = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let r = std::sync::Arc::clone(&reported);
        let stated = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stated_in = std::sync::Arc::clone(&stated);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_in = std::sync::Arc::clone(&calls);
        config.on_file_analysed = Some(
            super::super::ThroughputObserver::new(|_| {})
                .with_model_health(move |h| r.lock().unwrap().clone_from(&h.to_vec()))
                .with_model_info(move |info| {
                    calls_in.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    stated_in.lock().unwrap().clone_from(&info.to_vec());
                }),
        );
        let old = write_noise(&config.watch_dir, "2026-05-19-birdnet-08:00:00.wav", 3);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
            .unwrap();

        let (event_tx, _event_rx) = mpsc::sync_channel(4096);
        let handle = run_daemon(&config, event_tx).expect("daemon starts");
        let windows = |r: &std::sync::Mutex<Vec<(String, ModelHealth)>>| {
            r.lock().unwrap().first().map_or(0, |(_, h)| h.windows)
        };
        let wait_for = |n: u64| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while windows(&reported) < n && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        // The startup backlog pass, then a segment the watcher sees: the
        // two places the loop runs inference.
        wait_for(1);
        let after_backlog = windows(&reported);
        assert!(after_backlog >= 1, "the backlog pass reports");
        write_noise(&config.watch_dir, "2026-05-19-birdnet-09:00:00.wav", 3);
        wait_for(after_backlog + 1);
        handle.stop();

        let health = reported.lock().unwrap().clone();
        assert_eq!(health.len(), 1, "{health:?}");
        assert_eq!(health[0].0, "birdnet");
        assert!(
            health[0].1.windows > after_backlog,
            "the live loop reports too: {health:?}"
        );
        assert!(health[0].1.last_success_at.is_some(), "{health:?}");
        assert!(!health[0].1.is_failing(), "{health:?}");
        // The time each answered window took is measured, so a mean exists.
        assert!(health[0].1.timed_windows >= 1, "{health:?}");
        assert!(health[0].1.mean_inference_micros().is_some(), "{health:?}");

        // What the classifier is was stated exactly once, at startup, from the
        // loaded model: not once per file, and not from the config's say-so.
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let info = stated.lock().unwrap().clone();
        assert_eq!(info.len(), 1, "{info:?}");
        assert_eq!(info[0].id, "birdnet");
        assert!(info[0].window_secs > 0.0 && info[0].labels > 0, "{info:?}");
        assert!(
            (info[0].bar - config.model.confidence_threshold).abs() < f32::EPSILON,
            "the bar is the one detections are judged against: {info:?}"
        );
    }

    /// A watched segment is judged by its source's route (`MODEL_ROUTES`).
    ///
    /// The loop gave every file the default route — the primary alone —
    /// because "files arriving through the watch directory carry no
    /// audio-source id". They do: a capture source's segments are named with
    /// its `audio_sources` row id, the key `MODEL_ROUTES` uses. So no route
    /// ever applied, and a second classifier never ran on anything.
    #[test]
    fn a_segment_is_judged_by_its_sources_route() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.model.confidence_threshold = 0.0;
        config
            .extra_models
            .push(crate::inference::registry::ModelSpec {
                id: "second".to_owned(),
                model_path: config.model_path.clone(),
                labels_path: config.labels_path.clone(),
                threshold: None,
                sample_rate: None,
                output_scale: None,
            });
        config.model_routes.insert(
            "pond".to_owned(),
            vec!["birdnet".to_owned(), "second".to_owned()],
        );
        let (event_tx, event_rx) = mpsc::sync_channel(4096);
        let handle = run_daemon(&config, event_tx).expect("daemon starts");
        write_noise(&config.watch_dir, "2026-05-19-birdnet-pond-09:00:00.wav", 3);
        write_noise(
            &config.watch_dir,
            "2026-05-19-birdnet-garden-09:00:00.wav",
            3,
        );

        let mut agreement: std::collections::HashMap<String, Option<u8>> =
            std::collections::HashMap::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        while agreement.len() < 2 && Instant::now() < deadline {
            if let Ok(ev) = event_rx.recv_timeout(Duration::from_millis(200)) {
                let name = ev
                    .source_file
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                agreement
                    .entry(name)
                    .or_insert(ev.detection.agreeing_models);
            }
        }
        handle.stop();
        assert_eq!(
            agreement.get("2026-05-19-birdnet-pond-09:00:00.wav"),
            Some(&Some(2)),
            "the routed source was not judged by both classifiers: {agreement:?}"
        );
        // Counterpart: an unrouted source gets the primary alone.
        assert_eq!(
            agreement.get("2026-05-19-birdnet-garden-09:00:00.wav"),
            Some(&Some(1)),
            "{agreement:?}"
        );
    }

    /// A species only the second classifier knows is recorded, and the
    /// operator's lists still apply to it.
    ///
    /// Every detection was checked against an allow-list built from the
    /// primary's labels alone, so a species outside BirdNET's vocabulary — the
    /// reason to run Perch in the tropics — was dropped whichever model heard
    /// it, contradicting both the docs and the comment above that line.
    #[test]
    fn a_species_only_the_second_classifier_knows_is_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.model.confidence_threshold = 0.0;
        let other_labels = tmp.path().join("other.txt");
        std::fs::write(
            &other_labels,
            (0..11)
                .map(|i| format!("Other{i}_Other bird {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        config
            .extra_models
            .push(crate::inference::registry::ModelSpec {
                id: "second".to_owned(),
                model_path: config.model_path.clone(),
                labels_path: other_labels,
                threshold: None,
                sample_rate: None,
                output_scale: None,
            });
        config.model_routes.insert(
            "pond".to_owned(),
            vec!["birdnet".to_owned(), "second".to_owned()],
        );
        config.species_filter.exclude_list = vec!["Other1".to_owned()];
        let (event_tx, event_rx) = mpsc::sync_channel(4096);
        let handle = run_daemon(&config, event_tx).expect("daemon starts");
        write_noise(&config.watch_dir, "2026-05-19-birdnet-pond-09:00:00.wav", 3);

        let mut species = std::collections::BTreeSet::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            match event_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(ev) => {
                    species.insert(ev.detection.scientific_name);
                }
                Err(_) if !species.is_empty() => break,
                Err(_) => {}
            }
        }
        handle.stop();
        assert!(
            species.iter().any(|s| s.starts_with("Species")),
            "precondition: the primary's species are recorded: {species:?}"
        );
        assert!(
            species.contains("Other0"),
            "the second classifier's own species were dropped: {species:?}"
        );
        // Counterpart: the operator's exclude list still binds on them.
        assert!(!species.contains("Other1"), "{species:?}");
    }

    /// MMP-01: the recipe the manual and `--install-model perch-v2` give — a
    /// 32 kHz second classifier on a station whose primary is the default
    /// 48 kHz V2.4 — starts the daemon, and both classifiers judge the routed
    /// source. `run_daemon` returned `Err(DaemonError::Model)` for it, and the
    /// station recorded nothing while its web page stayed up.
    #[test]
    fn a_second_classifier_at_another_rate_does_not_stop_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = tiny_config(tmp.path());
        config.model.confidence_threshold = 0.0;
        config
            .extra_models
            .push(crate::inference::registry::ModelSpec {
                id: "perch".to_owned(),
                model_path: config.model_path.clone(),
                labels_path: config.labels_path.clone(),
                threshold: None,
                sample_rate: Some(32_000),
                output_scale: None,
            });
        config.model_routes.insert(
            "pond".to_owned(),
            vec!["birdnet".to_owned(), "perch".to_owned()],
        );
        let reported = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let r = std::sync::Arc::clone(&reported);
        config.on_file_analysed = Some(
            super::super::ThroughputObserver::new(|_| {})
                .with_model_health(move |h| r.lock().unwrap().clone_from(&h.to_vec())),
        );
        let (event_tx, event_rx) = mpsc::sync_channel(4096);
        let handle =
            run_daemon(&config, event_tx).expect("a 32 kHz extra beside a 48 kHz primary starts");
        write_noise(&config.watch_dir, "2026-05-19-birdnet-pond-09:00:00.wav", 6);

        let mut agreement = None;
        let deadline = Instant::now() + Duration::from_secs(30);
        while agreement.is_none() && Instant::now() < deadline {
            if let Ok(ev) = event_rx.recv_timeout(Duration::from_millis(200)) {
                agreement = Some(ev.detection.agreeing_models);
            }
        }
        handle.stop();
        assert_eq!(
            agreement,
            Some(Some(2)),
            "both classifiers judged the routed source"
        );
        let health = reported.lock().unwrap().clone();
        assert_eq!(health.len(), 2, "{health:?}");
        assert!(
            health
                .iter()
                .all(|(_, h)| h.windows >= 1 && !h.is_failing()),
            "each answered its windows: {health:?}"
        );
    }

    /// Counterpart to the test above: a primary that will not load still
    /// stops the daemon at startup, where the journal and `--doctor` show it.
    #[test]
    fn a_primary_that_will_not_load_still_stops_the_daemon() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tiny_config(tmp.path());
        std::fs::write(&config.model_path, b"not an onnx file").unwrap();
        let (event_tx, _event_rx) = mpsc::sync_channel(64);
        let Err(err) = run_daemon(&config, event_tx) else {
            panic!("a corrupt primary must be refused");
        };
        assert!(matches!(err, DaemonError::Model(_)), "{err:?}");
    }

    #[test]
    fn run_daemon_loop_advances_heartbeat() {
        // A healthy detection loop must keep advancing its heartbeat, so the
        // watchdog never mistakes a *running* daemon for a hung one and
        // needlessly restarts a healthy field station. Stand the real loop up
        // against the tiny bundled model and assert the counter climbs.
        let tmp = tempfile::tempdir().unwrap();
        let config = tiny_config(tmp.path());

        let (event_tx, _event_rx) = mpsc::sync_channel(64);
        let handle = run_daemon(&config, event_tx).expect("daemon starts with the tiny model");

        let hb = handle.heartbeat();
        let start = hb.load(Ordering::Relaxed);
        // The loop polls on a 500 ms timeout, so ~1.2 s is at least two cycles.
        std::thread::sleep(Duration::from_millis(1_200));
        let after = hb.load(Ordering::Relaxed);
        handle.stop();

        assert!(
            after > start,
            "detection loop heartbeat must advance while running (start={start}, after={after})"
        );
    }
}
