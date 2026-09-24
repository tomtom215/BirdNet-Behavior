//! The daemon run loop: watch the directory, debounce writes, and drive each
//! settled clip through the processing pipeline.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crate::audio::capture::is_audio_file;
use crate::detection::pipeline::{self, PipelineConfig};
use crate::detection::privacy::PrivacyFilter;
use crate::detection::{ChunkFilters, noise::NoiseFilter};
use crate::file_settle::{FILE_SETTLE, PendingFiles};
use crate::inference::labels::LabelSet;
use crate::inference::registry::ClassifierRegistry;
use crate::inference::species_filter::SpeciesFilter;
use crate::inference::vocabulary::parse_alias_file;

use super::process::process_and_infer_filtered;
use super::{
    DaemonConfig, DaemonError, DaemonHandle, DetectionEvent, ShedReason, new_event_correlation_id,
    shed_split,
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
/// The primary's id, threshold and rate are the operator's (`MODEL_ID`,
/// `MODEL_THRESHOLD`, `MODEL_SAMPLE_RATE`). They were resolved by the plan and
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
/// watch loop on the current thread (blocking).
///
/// # Errors
///
/// Returns `DaemonError` if no classifier is configured, if one cannot be
/// loaded, if a route names a classifier that does not exist, or if the
/// watcher fails. Every one of those is refused here at startup rather than
/// at the first audio file, because an unattended station must fail where
/// somebody can see it.
///
/// # Panics
///
/// Does not panic in practice: the `expect` on the primary classifier is
/// guaranteed by [`crate::inference::registry::ClassifierRegistry::load`],
/// which refuses to build a registry with no classifiers in it.
#[allow(clippy::too_many_lines)]
pub fn run_daemon(
    config: &DaemonConfig,
    event_tx: mpsc::SyncSender<DetectionEvent>,
) -> Result<DaemonHandle, DaemonError> {
    // Every classifier this station runs, primary first (`G-10` Stage 2).
    //
    // One entry on a station that has not asked for a second opinion, which is
    // every station today: the registry is then exactly the single model this
    // loaded before, reached through `primary()`.
    //
    // Built here, before anything else, because every way it can fail —
    // nothing configured, a route naming a classifier that does not exist, a
    // model that will not load — must stop the daemon at startup where the
    // journal and `--doctor` will show it. An unattended station that starts
    // with a microphone routed to nothing looks identical to one having a
    // quiet month.
    let specs = classifier_specs(config);

    let mut registry = crate::inference::registry::ClassifierRegistry::load(
        &specs,
        &config.model_routes,
        &config.model,
    )
    .map_err(|e| DaemonError::Model(e.to_string()))?;

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

    // The primary is what the pipeline below runs on, so this is byte-for-byte
    // the model that was loaded here before Stage 2. Borrowed immutably for
    // the setup that follows; the registry itself moves into the loop thread,
    // where the mutable borrow is taken.
    let model = &registry
        .model(0)
        .expect("a registry always has a primary")
        .model;

    tracing::info!(
        species_count = model.labels().len(),
        labels_path = %config.labels_path.display(),
        "labels loaded"
    );

    // Auto-detect the sample rate the model expects from its input shape.
    // V2.4 → [1, 144_000] = 48 kHz × 3 s; V3.0 → [1, 96_000] = 32 kHz × 3 s.
    let model_sample_rate = model.infer_sample_rate();

    tracing::info!(
        model_path = %config.model_path.display(),
        input_shape = ?model.input_shape(),
        sample_rate = model_sample_rate,
        "model loaded, starting daemon"
    );

    // Build pipeline config, overriding sample rate and input mode to match the model.
    let mut pipeline_config = config.pipeline.clone();
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
    let spec = model.input_spec();
    let raw_mode = spec.is_waveform();
    if raw_mode != pipeline_config.raw_audio_input {
        tracing::info!(
            raw_audio_input = raw_mode,
            "adjusting pipeline input mode to match model"
        );
        pipeline_config.raw_audio_input = raw_mode;
    }

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

    // Load the species occurrence filter (the metadata / "geo" model).
    //
    // Every exit from this block that is not a loaded model leaves the station
    // reporting every species the classifier knows, anywhere on Earth, in any
    // week. That used to be reached silently — no metadata model configured
    // logged nothing at all — and the symptom (implausible birds) reads as a
    // bad classifier rather than as a missing file. Each branch now says so at
    // a level an operator will actually see, and `--doctor` reports the same
    // state before the service starts.
    let mut species_filter = match config.metadata_model_path.as_ref() {
        None => {
            tracing::warn!(
                species = model.labels().len(),
                "no metadata model configured (METADATA_MODEL_PATH / BIRDNET_METADATA_MODEL): species occurrence filtering is OFF and every species in the model stays a candidate regardless of the station's location. Run `birdnet-behavior --doctor` for how to enable it"
            );
            SpeciesFilter::new_passthrough(config.species_filter.clone())
        }
        Some(mdata_path) => {
            let meta_labels = match config.metadata_labels_path.as_ref() {
                None => None,
                Some(p) => match LabelSet::load(p) {
                    Ok(ls) => Some(ls),
                    Err(e) => {
                        tracing::error!(
                            path = %p.display(),
                            error = %e,
                            "metadata label file could not be read; species occurrence filtering is OFF"
                        );
                        return Err(DaemonError::Model(format!("metadata labels: {e}")));
                    }
                },
            };
            // An unreadable alias file is a warning, not a failure: the
            // alignment works without it, and taking the occurrence filter off
            // a station over a typo'd path would admit every species the
            // classifier knows, wherever it is.
            let aliases = config.species_aliases_path.as_ref().map_or_else(
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
            );
            match SpeciesFilter::load_with_vocabulary(
                mdata_path,
                meta_labels,
                model.labels(),
                &aliases,
                config.species_filter.clone(),
            ) {
                Ok(sf) => sf,
                Err(e) => {
                    tracing::error!(
                        path = %mdata_path.display(),
                        error = %e,
                        "metadata model could not be used; species occurrence filtering is OFF and every species in the model stays a candidate"
                    );
                    SpeciesFilter::new_passthrough(config.species_filter.clone())
                }
            }
        }
    };

    let filter_observer = config.on_species_filter_state.clone();
    let throughput = config.on_file_analysed.clone();
    let in_flight = config.in_flight.clone();
    let shed_policy = config.shed.clone();
    // The shed decision is logged on change, not on every 500 ms sweep.
    let mut shedding: Option<ShedReason> = None;
    if let Some(observer) = filter_observer.as_ref() {
        observer.report(species_filter.has_model(), None);
    }

    // Create the whole-chunk filters.
    let chunk_filters = ChunkFilters {
        privacy: PrivacyFilter::new(config.privacy_threshold)
            .with_clip_reach(config.privacy_clip_reach)
            .with_chunk_secs(config.pipeline.chunk_duration_secs),
        noise: NoiseFilter::new(config.noise_threshold, config.noise_classes.clone())
            .remembering(config.noise_remember_secs),
        confirmation: config.confirmation,
    };

    if chunk_filters.privacy.is_enabled() {
        tracing::info!(
            threshold = config.privacy_threshold,
            "privacy filter enabled"
        );
        if !model.has_human_labels() {
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
        let overlap = config.pipeline.chunk_overlap_secs;
        let chunk_secs = config.pipeline.chunk_duration_secs;
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
            // reject anything at this overlap, so a station that set it and
            // saw no change would otherwise have nothing to read.
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

    let lat = config.latitude;
    let lon = config.longitude;
    // Cloned out of the borrowed `config` so the 'static loop thread below can
    // own it (an `Arc` clone, not a deep copy).
    let species_lists_provider = config.species_lists_provider.clone();

    // Create stop channel
    let (stop_tx, stop_rx) = mpsc::channel();

    // Liveness heartbeat: the loop bumps this once per iteration so an external
    // watchdog can distinguish a hung pipeline (no progress) from an idle one.
    let heartbeat = Arc::new(AtomicU64::new(0));
    let heartbeat_loop = Arc::clone(&heartbeat);

    // Liveness of the loop thread itself: `true` until the thread returns.
    // The guard travels into the thread so every exit path clears it.
    let running = Arc::new(AtomicBool::new(true));
    let running_guard = super::RunningGuard(Arc::clone(&running));

    // Start file watcher. The `RecommendedWatcher` MUST live for the
    // lifetime of the spawned thread — dropping it stops delivery and
    // closes the channel. Bound to a name that the closure captures so
    // `move ||` takes ownership and keeps it alive.
    let (file_watcher, file_rx) =
        pipeline::watch_directory(&config.watch_dir).map_err(DaemonError::Pipeline)?;

    // Snapshot the backlog settings to move into the loop thread; `config` is
    // a borrow and cannot outlive this call on the spawned 'static thread.
    let process_existing = config.process_existing;
    // Each classifier's own threshold as loaded, and the floor the processor
    // publishes: before every file each runs at the lower of the two, so a
    // per-species threshold below the global one reaches the model at all.
    let threshold_floor = config.threshold_floor.clone();
    let loaded_thresholds = super::loaded_thresholds(&registry);
    let watch_dir = config.watch_dir.clone();

    // Main daemon loop -- runs on its own thread
    std::thread::spawn(move || {
        // Keep the watcher alive for the lifetime of this thread.
        // Without this the `RecommendedWatcher` gets dropped when
        // `start_detection_daemon` returns, the underlying `notify`
        // backend stops, and `file_rx` immediately reports
        // `Disconnected` — silently breaking the watch path.
        let _watcher = file_watcher;
        // Dropped when this closure returns, however it returns, clearing
        // `DaemonHandle::running_flag`.
        let _alive = running_guard;
        tracing::info!("detection daemon started");

        // Process any pre-existing backlog here, on the loop thread, rather
        // than before signalling readiness. The event consumer is already
        // draining by now, so a large backlog cannot block startup past the
        // systemd TimeoutStartSec, and with a bounded event channel it applies
        // backpressure instead of dead-locking an undrained queue.
        // Debounce watcher events: a clip is decoded only once its size has
        // been stable for FILE_SETTLE (see PendingFiles), so an in-progress
        // ffmpeg/RTSP segment isn't decoded mid-write (which fails with
        // "unexpected end of file" and reprocesses the same growing file).
        let mut pending = PendingFiles::new();

        if process_existing {
            super::apply_threshold_floor(
                &mut registry,
                &loaded_thresholds,
                threshold_floor.as_deref(),
            );
            process_existing_files(
                &watch_dir,
                &pipeline_config,
                &mut registry,
                &chunk_filters,
                &mut species_filter,
                filter_observer.as_ref(),
                throughput.as_ref(),
                in_flight.as_ref(),
                &mut pending,
                lat,
                lon,
                &event_tx,
            );
        }

        // Track when the operator's species lists were last re-read, so a
        // change on /admin/species applies to the next file rather than the
        // next restart. Checked per file rather than per loop tick: the tick is
        // a 500 ms poll and the lists live in a database.
        let mut lists_refreshed = Instant::now();

        // Segments a stop cut off mid-sweep (PIPE8b), reported on the way out
        // with whatever is still settling.
        let mut left: Vec<PathBuf> = Vec::new();
        let mut stop_requested = false;

        loop {
            // Heartbeat: record that the loop is still cycling so a watchdog
            // can tell a hung pipeline from an idle one.
            heartbeat_loop.fetch_add(1, Ordering::Relaxed);

            // Check for stop signal (non-blocking)
            if stop_requested || stop_rx.try_recv().is_ok() {
                tracing::info!("detection daemon stopping");
                report_unanalysed(
                    left.into_iter().chain(pending.into_paths()),
                    process_existing,
                    throughput.as_ref(),
                );
                break;
            }

            // Collect every watcher event currently available (blocking briefly
            // for the first) into the pending set. A burst of modify events for
            // a file still being written collapses into a single entry.
            match file_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(path) => {
                    pending.note(path, Instant::now());
                    while let Ok(path) = file_rx.try_recv() {
                        pending.note(path, Instant::now());
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("file watcher disconnected, stopping daemon");
                    break;
                }
            }

            // Process whichever files have finished being written. Polling the
            // size each sweep means a clip settles even after its last watcher
            // event, so the final segment is never stranded.
            let settled = pending.drain_settled_reporting(Instant::now(), FILE_SETTLE, |p| {
                std::fs::metadata(p).map(|m| m.len()).ok()
            });
            // A segment the watcher announced that is gone before the pipeline
            // could open it is audio that was recorded and never analysed
            // (PR-1 / S-3): a purge running ahead of a backlogged pipeline.
            // It used to fall out of the pending set without a word.
            for path in &settled.vanished {
                tracing::warn!(
                    file = %path.display(),
                    "segment vanished before analysis — recorded audio the pipeline never \
                     read is gone; if this repeats, the stream directory is being drained \
                     faster than inference keeps up"
                );
                if let Some(observer) = throughput.as_ref() {
                    observer.dropped(path);
                }
            }
            // The queue is what is still settling plus what this sweep will
            // analyse (PR-2). Published once per sweep; the shed policy
            // decides on it.
            let queue_depth = pending.len() + settled.ready.len();
            if let Some(observer) = throughput.as_ref() {
                observer.queue_depth(queue_depth);
            }
            let decision = shed_policy.as_ref().and_then(|p| p.decide(queue_depth));
            if decision != shedding {
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
                shedding = decision;
            }
            let ready = match decision {
                Some(reason) => {
                    let (analyse, shed) = shed_split(settled.ready);
                    for path in &shed {
                        tracing::debug!(file = %path.display(), reason = reason.as_str(), "segment shed");
                        if let Some(observer) = throughput.as_ref() {
                            observer.shed(path, reason);
                        }
                    }
                    analyse
                }
                None => settled.ready,
            };
            let mut ready = ready.into_iter();
            while let Some(path) = ready.next() {
                // A stop between files, not only between sweeps: one sweep can
                // hold a long backlog, and shutdown waits on this loop.
                if stop_rx.try_recv().is_ok() {
                    stop_requested = true;
                    left.push(path);
                    left.extend(ready);
                    break;
                }
                // Keep the watchdog fed if a single sweep processes several files.
                heartbeat_loop.fetch_add(1, Ordering::Relaxed);

                // Refresh the operator's species lists if they have gone stale.
                // A failed or absent provider leaves the previous lists in
                // place rather than clearing them: dropping an exclude list on
                // a transient database error would start recording exactly the
                // species the operator asked to suppress.
                if let Some(ref provider) = species_lists_provider
                    && lists_refreshed.elapsed() >= SPECIES_LISTS_TTL
                {
                    let lists = provider.get();
                    species_filter.set_lists(lists.include, lists.exclude);
                    lists_refreshed = Instant::now();
                }

                // Stamp a correlation ID on every event we publish for this
                // file so the operator can trace one file through the entire
                // pipeline by grepping a single string.
                let correlation_id = new_event_correlation_id();
                // DEBUG, not INFO (OB-15): with "file processing complete"
                // this was two lines per 15 s segment, 92 % of the journal's
                // volume and 1.6-2.8 GB a year, carrying nothing the
                // `birdnet_files_analysed_total` counter does not. The
                // per-detection line keeps the correlation id at INFO.
                tracing::debug!(
                    correlation_id = %correlation_id,
                    file = %path.display(),
                    "begin processing file"
                );

                super::apply_threshold_floor(
                    &mut registry,
                    &loaded_thresholds,
                    threshold_floor.as_deref(),
                );

                // Claimed for as long as the pipeline reads it (PR-1 / S-3):
                // the stream directory's purge skips a claimed name. Each
                // event carries a share of the claim, so it lasts until the
                // processor has cut the clips too (PIPE8a).
                let lease = in_flight
                    .as_ref()
                    .map(|table| std::sync::Arc::new(table.claim(&path)));
                let route = route_for_segment(&registry, &path);
                match process_and_infer_filtered(
                    &path,
                    &pipeline_config,
                    &mut registry,
                    &route,
                    &chunk_filters,
                    &mut species_filter,
                    filter_observer.as_ref(),
                    lat,
                    lon,
                    &correlation_id,
                ) {
                    Ok(events) => {
                        // Counted here and not before the call: this says
                        // "analysed", and a file the pipeline failed on was
                        // not. The counter is the only thing that separates a
                        // model answering nothing from a pipeline that is not
                        // running, so it must mean exactly one thing.
                        if let Some(observer) = throughput.as_ref() {
                            observer.analysed(&path);
                        }
                        for mut event in events {
                            event.lease.clone_from(&lease);
                            if event_tx.send(event).is_err() {
                                tracing::warn!(
                                    correlation_id = %correlation_id,
                                    "event receiver dropped, stopping daemon"
                                );
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        // Gone between settling and opening: the same loss as
                        // a vanished pending file, counted the same way.
                        if path.exists() {
                            tracing::warn!(
                                correlation_id = %correlation_id,
                                file = %path.display(),
                                error = %e,
                                "failed to process file"
                            );
                        } else {
                            tracing::warn!(
                                correlation_id = %correlation_id,
                                file = %path.display(),
                                "segment vanished while being analysed — recorded audio the \
                                 pipeline never finished reading is gone"
                            );
                            if let Some(observer) = throughput.as_ref() {
                                observer.dropped(&path);
                            }
                        }
                    }
                }
            }
        }

        tracing::info!("detection daemon stopped");
    });

    Ok(DaemonHandle {
        stop_tx,
        heartbeat,
        running,
    })
}

/// The classifiers that judge a segment: its source's `MODEL_ROUTES` entry.
///
/// A capture source names its segments with its id — the `audio_sources` row
/// id, the key routes are written against — so the id is read back from the
/// file name. A name without one (a lone microphone's BirdNET-Pi-style name, or
/// a file dropped in by hand) gets the default route, the primary, as does a
/// source with no route. Before this every segment got the default, whatever
/// its source, and no route ever applied.
fn route_for_segment(registry: &ClassifierRegistry, path: &Path) -> Vec<usize> {
    let source = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::detection::types::RecordingFile::parse)
        .and_then(|f| f.rtsp_id);
    registry.route_for(source.as_deref())
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

/// Process any audio files already present in the watch directory.
///
/// A file modified within [`FILE_SETTLE`] may still be being written, so it is
/// handed to `pending` — the watcher's queue, which analyses it once it has
/// settled — instead of being read now (`PIPE8c`). Read here, it was decoded
/// part-written and then again by the watcher when the recorder finished it.
/// Each file read here is claimed in `in_flight` like the loop's, and its
/// events carry the claim.
#[allow(clippy::too_many_arguments)]
fn process_existing_files(
    dir: &Path,
    pipeline_config: &PipelineConfig,
    registry: &mut ClassifierRegistry,
    chunk_filters: &ChunkFilters,
    species_filter: &mut SpeciesFilter,
    filter_observer: Option<&super::SpeciesFilterObserver>,
    throughput: Option<&super::ThroughputObserver>,
    in_flight: Option<&super::InFlight>,
    pending: &mut PendingFiles,
    lat: Option<f64>,
    lon: Option<f64>,
    event_tx: &mpsc::SyncSender<DetectionEvent>,
) {
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
        if !path.is_file() {
            continue;
        }

        if !is_audio_file(&path) {
            continue;
        }

        let recently_written = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_none_or(|age| age < FILE_SETTLE);
        if recently_written {
            pending.note(path, Instant::now());
            continue;
        }

        let lease = in_flight.map(|table| std::sync::Arc::new(table.claim(&path)));
        let correlation_id = new_event_correlation_id();
        let route = route_for_segment(registry, &path);
        match process_and_infer_filtered(
            &path,
            pipeline_config,
            registry,
            &route,
            chunk_filters,
            species_filter,
            filter_observer,
            lat,
            lon,
            &correlation_id,
        ) {
            Ok(events) => {
                if let Some(observer) = throughput {
                    observer.analysed(&path);
                }
                for mut event in events {
                    event.lease.clone_from(&lease);
                    // Surface a closed receiver instead of swallowing it: with
                    // the prior `let _ =` a consumer that dropped mid-backlog
                    // left this loop spinning through the rest of the watch
                    // directory pointlessly (each `send` errored, was ignored,
                    // and we processed the next file anyway). The main runtime
                    // loop treats a closed receiver as fatal — match that here.
                    if event_tx.send(event).is_err() {
                        tracing::debug!("existing-file backlog stopping: event receiver closed");
                        return;
                    }
                }
                count += 1;
            }
            Err(e) => {
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
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
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
