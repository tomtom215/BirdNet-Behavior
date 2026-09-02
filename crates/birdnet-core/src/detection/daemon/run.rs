//! The daemon run loop: watch the directory, debounce writes, and drive each
//! settled clip through the processing pipeline.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crate::audio::capture::is_audio_file;
use crate::detection::pipeline::{self, PipelineConfig};
use crate::detection::privacy::PrivacyFilter;
use crate::detection::{ChunkFilters, noise::NoiseFilter};
use crate::file_settle::{FILE_SETTLE, PendingFiles};
use crate::inference::labels::LabelSet;
use crate::inference::model::BirdNetModel;
use crate::inference::species_filter::SpeciesFilter;

use super::process::process_and_infer_filtered;
use super::{DaemonConfig, DaemonError, DaemonHandle, DetectionEvent, new_event_correlation_id};

/// How stale the operator's species include/exclude lists may get before the
/// next file re-reads them.
///
/// Matches the per-species threshold cache in the application's event
/// processor: both are small, indexed reads standing between an operator
/// changing something on `/admin/species` and seeing it take effect, and
/// neither is worth a query per detection during a dawn-chorus burst.
const SPECIES_LISTS_TTL: Duration = Duration::from_secs(30);

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
/// Returns `DaemonError` if the model cannot be loaded or the watcher fails.
#[allow(clippy::too_many_lines)]
pub fn run_daemon(
    config: &DaemonConfig,
    event_tx: mpsc::SyncSender<DetectionEvent>,
) -> Result<DaemonHandle, DaemonError> {
    // Load labels
    let labels = LabelSet::load(&config.labels_path)
        .map_err(|e| DaemonError::Model(format!("labels: {e}")))?;

    tracing::info!(
        species_count = labels.len(),
        labels_path = %config.labels_path.display(),
        "labels loaded"
    );

    // Load model
    let mut model = BirdNetModel::load(&config.model_path, labels, config.model.clone())?;

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
    // V3.0 models expect raw audio; V2.4 models expect a mel spectrogram.
    let raw_mode = model.infer_sample_rate() == 32_000;
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
    let model_chunk_secs = model.recommended_chunk_secs();
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
            match SpeciesFilter::load_with_vocabulary(
                mdata_path,
                meta_labels,
                model.labels().len(),
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
    if let Some(observer) = filter_observer.as_ref() {
        observer.report(species_filter.has_model(), None);
    }

    // Create the whole-chunk filters.
    let chunk_filters = ChunkFilters {
        privacy: PrivacyFilter::new(config.privacy_threshold),
        noise: NoiseFilter::new(config.noise_threshold, config.noise_classes.clone()),
    };

    if chunk_filters.privacy.is_enabled() {
        tracing::info!(
            threshold = config.privacy_threshold,
            "privacy filter enabled"
        );
    }
    if chunk_filters.noise.is_enabled() {
        tracing::info!(
            threshold = config.noise_threshold,
            classes = ?chunk_filters.noise.classes(),
            "noise filter enabled"
        );
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

    // Start file watcher. The `RecommendedWatcher` MUST live for the
    // lifetime of the spawned thread — dropping it stops delivery and
    // closes the channel. Bound to a name that the closure captures so
    // `move ||` takes ownership and keeps it alive.
    let (file_watcher, file_rx) =
        pipeline::watch_directory(&config.watch_dir).map_err(DaemonError::Pipeline)?;

    // Snapshot the backlog settings to move into the loop thread; `config` is
    // a borrow and cannot outlive this call on the spawned 'static thread.
    let process_existing = config.process_existing;
    let watch_dir = config.watch_dir.clone();

    // Main daemon loop -- runs on its own thread
    std::thread::spawn(move || {
        // Keep the watcher alive for the lifetime of this thread.
        // Without this the `RecommendedWatcher` gets dropped when
        // `start_detection_daemon` returns, the underlying `notify`
        // backend stops, and `file_rx` immediately reports
        // `Disconnected` — silently breaking the watch path.
        let _watcher = file_watcher;
        tracing::info!("detection daemon started");

        // Process any pre-existing backlog here, on the loop thread, rather
        // than before signalling readiness. The event consumer is already
        // draining by now, so a large backlog cannot block startup past the
        // systemd TimeoutStartSec, and with a bounded event channel it applies
        // backpressure instead of dead-locking an undrained queue.
        if process_existing {
            process_existing_files(
                &watch_dir,
                &pipeline_config,
                &mut model,
                &chunk_filters,
                &mut species_filter,
                filter_observer.as_ref(),
                lat,
                lon,
                &event_tx,
            );
        }

        // Debounce watcher events: a clip is decoded only once its size has
        // been stable for FILE_SETTLE (see PendingFiles), so an in-progress
        // ffmpeg/RTSP segment isn't decoded mid-write (which fails with
        // "unexpected end of file" and reprocesses the same growing file).
        let mut pending = PendingFiles::new();

        // Track when the operator's species lists were last re-read, so a
        // change on /admin/species applies to the next file rather than the
        // next restart. Checked per file rather than per loop tick: the tick is
        // a 500 ms poll and the lists live in a database.
        let mut lists_refreshed = Instant::now();

        loop {
            // Heartbeat: record that the loop is still cycling so a watchdog
            // can tell a hung pipeline from an idle one.
            heartbeat_loop.fetch_add(1, Ordering::Relaxed);

            // Check for stop signal (non-blocking)
            if stop_rx.try_recv().is_ok() {
                tracing::info!("detection daemon stopping");
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
            for path in pending.drain_settled(Instant::now(), FILE_SETTLE, |p| {
                std::fs::metadata(p).map(|m| m.len()).ok()
            }) {
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
                tracing::info!(
                    correlation_id = %correlation_id,
                    file = %path.display(),
                    "begin processing file"
                );

                match process_and_infer_filtered(
                    &path,
                    &pipeline_config,
                    &mut model,
                    &chunk_filters,
                    &mut species_filter,
                    filter_observer.as_ref(),
                    lat,
                    lon,
                    0, // week will be computed by caller
                    &correlation_id,
                ) {
                    Ok(events) => {
                        for event in events {
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
                        tracing::warn!(
                            correlation_id = %correlation_id,
                            file = %path.display(),
                            error = %e,
                            "failed to process file"
                        );
                    }
                }
            }
        }

        tracing::info!("detection daemon stopped");
    });

    Ok(DaemonHandle { stop_tx, heartbeat })
}

/// Process any audio files already present in the watch directory.
#[allow(clippy::too_many_arguments)]
fn process_existing_files(
    dir: &Path,
    pipeline_config: &PipelineConfig,
    model: &mut BirdNetModel,
    chunk_filters: &ChunkFilters,
    species_filter: &mut SpeciesFilter,
    filter_observer: Option<&super::SpeciesFilterObserver>,
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

        let correlation_id = new_event_correlation_id();
        match process_and_infer_filtered(
            &path,
            pipeline_config,
            model,
            chunk_filters,
            species_filter,
            filter_observer,
            lat,
            lon,
            0,
            &correlation_id,
        ) {
            Ok(events) => {
                for event in events {
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

    #[test]
    fn run_daemon_loop_advances_heartbeat() {
        // A healthy detection loop must keep advancing its heartbeat, so the
        // watchdog never mistakes a *running* daemon for a hung one and
        // needlessly restarts a healthy field station. Stand the real loop up
        // against the tiny bundled model and assert the counter climbs.
        const TINY_V24: &[u8] = include_bytes!("../../testdata/tiny_v24_test.onnx");

        let tmp = tempfile::tempdir().unwrap();
        let watch_dir = tmp.path().join("recs");
        std::fs::create_dir_all(&watch_dir).unwrap();
        let model_path = tmp.path().join("model.onnx");
        std::fs::write(&model_path, TINY_V24).unwrap();
        let labels_path = tmp.path().join("labels.txt");
        let labels = (0..11)
            .map(|i| format!("Species{i}_Bird {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&labels_path, labels).unwrap();

        let config = DaemonConfig {
            watch_dir,
            model_path,
            labels_path,
            pipeline: PipelineConfig::default(),
            model: ModelConfig::default(),
            process_existing: false,
            metadata_model_path: None,
            metadata_labels_path: None,
            on_species_filter_state: None,
            species_filter: crate::inference::species_filter::SpeciesFilterConfig::default(),
            species_lists_provider: None,
            privacy_threshold: 0.0,
            noise_threshold: 0.0,
            noise_classes: Vec::new(),
            latitude: None,
            longitude: None,
            species_thresholds: std::collections::HashMap::new(),
        };

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
