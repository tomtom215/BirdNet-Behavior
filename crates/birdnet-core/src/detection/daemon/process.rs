//! Per-file processing: decode → pipeline → inference → (optional) filtering.

use std::path::Path;
use std::time::Instant;

use crate::detection::ChunkFilters;
use crate::detection::pipeline::{self, PipelineConfig, PreparedChunk};
use crate::detection::types::ChunkPrediction;
use crate::inference::health::ErrorClass;
use crate::inference::registry::ClassifierRegistry;
use crate::inference::species_filter::SpeciesFilter;

use super::{DaemonError, DetectionEvent};

/// Process a single audio file through the full pipeline (no model -- pipeline-only mode).
///
/// This is useful for testing the audio pipeline without a model,
/// or when running in "prepare only" mode.
///
/// # Errors
///
/// Returns `DaemonError` if any pipeline stage fails.
pub fn process_file_pipeline_only(
    path: &Path,
    config: &PipelineConfig,
) -> Result<Vec<PreparedChunk>, DaemonError> {
    let chunks = pipeline::process_file(path, config)?;
    Ok(chunks)
}

/// The BirdNET geomodel week for the recording this file came from.
///
/// The week is a property of *when the audio was recorded*, which is what the
/// filename says, and never of when it was analysed: a backlog drained three
/// days after a power cut must be scored against the season it was recorded
/// in. [`crate::civil::birdnet_week`] documents the 48-week year the model was
/// trained on and why week 0 is not a point in it.
///
/// `pipeline::process_file` already refuses a file whose name does not parse
/// as `YYYY-MM-DD-birdnet-…`, so the only way to reach the fallback is a name
/// whose date is digit-shaped but names no day (`2026-99-99`). That warns and
/// continues rather than discarding real audio, and week 1 is named here so
/// the substitution is auditable rather than invented somewhere downstream.
fn geomodel_week(date: &str, path: &Path) -> u32 {
    crate::civil::birdnet_week_from_date(date).unwrap_or_else(|| {
        tracing::warn!(
            file = %path.display(),
            date,
            "recording date does not name a day; scoring the species-occurrence \
             filter against week 1"
        );
        1
    })
}

/// Minimum seconds between two logged runtime failures of the metadata model.
///
/// The failure repeats on every file (a segment every few seconds) for as long
/// as it lasts; one line per file would bury everything else in the journal,
/// and none would hide it.
const SPECIES_FILTER_FAILURE_LOG_INTERVAL_SECS: u64 = 600;

/// When the last runtime failure was logged, in seconds since the Unix epoch
/// (`0`: never).
static SPECIES_FILTER_FAILURE_LOGGED_AT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Log a runtime failure of the metadata model, at most once per
/// [`SPECIES_FILTER_FAILURE_LOG_INTERVAL_SECS`].
fn report_species_filter_failure(error: &crate::inference::model::InferenceError) {
    use std::sync::atomic::Ordering;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let last = SPECIES_FILTER_FAILURE_LOGGED_AT.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < SPECIES_FILTER_FAILURE_LOG_INTERVAL_SECS {
        return;
    }
    SPECIES_FILTER_FAILURE_LOGGED_AT.store(now.max(1), Ordering::Relaxed);
    tracing::error!(
        error = %error,
        repeat_suppressed_secs = SPECIES_FILTER_FAILURE_LOG_INTERVAL_SECS,
        "metadata model failed at run time; species occurrence filtering is OFF for this \
         file and every species in the model stays a candidate (the include/exclude lists \
         still apply). Recording continues"
    );
}

/// How often a source that delivers digital silence is named in the log, in
/// seconds. A muted source fills every recording, so a line per recording
/// would be a line every few seconds.
const SILENT_AUDIO_LOG_INTERVAL_SECS: u64 = 600;

/// When digital silence was last logged, in seconds since the Unix epoch
/// (`0`: never).
static SILENT_AUDIO_LOGGED_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Say, at most once per [`SILENT_AUDIO_LOG_INTERVAL_SECS`], that a recording
/// held a stretch of exactly constant audio.
fn report_silent_audio(recording: &crate::detection::types::RecordingFile) {
    use std::sync::atomic::Ordering;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let last = SILENT_AUDIO_LOGGED_AT.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < SILENT_AUDIO_LOG_INTERVAL_SECS {
        return;
    }
    SILENT_AUDIO_LOGGED_AT.store(now.max(1), Ordering::Relaxed);
    tracing::warn!(
        date = %recording.date,
        time = %recording.time,
        repeat_suppressed_secs = SILENT_AUDIO_LOG_INTERVAL_SECS,
        "part of this recording is digital silence (every sample identical): the microphone \
         is muted or unplugged, or the source delivers zeros. Those windows are not \
         classified and are not counted against any classifier"
    );
}

/// Record one window's outcome on a classifier's health, and log the two
/// moments worth a line: the first failure of a run (with its error, which
/// is otherwise not logged when another classifier answered), and the window
/// that makes the classifier failing. Recovery from failing is logged too.
/// Nothing in between — a model failing every window would otherwise write a
/// line every three seconds.
fn record_window(
    registered: &mut crate::inference::registry::RegisteredModel,
    outcome: Result<(), (ErrorClass, Option<&crate::inference::model::InferenceError>)>,
    now: u64,
) {
    let health = &mut registered.health;
    match outcome {
        Ok(()) => {
            let was_failing = health.is_failing();
            health.succeeded(now);
            if was_failing {
                tracing::info!(model = %registered.id, "classifier is answering again");
            }
        }
        Err((class, error)) => {
            health.failed(class, now);
            if health.consecutive_failures == 1 {
                tracing::warn!(
                    model = %registered.id,
                    error_class = class.as_str(),
                    error = error.map(ToString::to_string),
                    "classifier failed a window"
                );
            }
            if health.consecutive_failures == crate::inference::health::FAILING_AFTER {
                tracing::error!(
                    model = %registered.id,
                    error_class = class.as_str(),
                    windows = health.consecutive_failures,
                    "classifier has failed every window for {} in a row; it is reported as \
                     failing until it answers one",
                    health.consecutive_failures
                );
            }
        }
    }
}

/// Run every classifier routed to this chunk and combine what they said.
///
/// One classifier is the overwhelmingly common case and costs one pass, as it
/// always did. With more than one the detections are merged by
/// [`crate::detection::merge::merge_verdicts`] — union, each model judged
/// against its own threshold, agreement counted rather than folded into the
/// confidence.
///
/// The human score is the **highest** any classifier reported, not the
/// primary's. It drives the privacy gate, and if any model heard speech the
/// safe reading is that there was speech: erring towards suppressing a bird is
/// recoverable, erring towards publishing a conversation is not.
fn infer_chunk(
    registry: &mut ClassifierRegistry,
    route: &[usize],
    chunk: &crate::detection::pipeline::PreparedChunk,
    week: u32,
) -> Result<ChunkPrediction, DaemonError> {
    // A chunk with no variation at all has nothing to classify, and the real
    // V3.0 model answers it with NaN. Running it would count that as a failed
    // window, flip the classifier to "failing" after ten of them — a muted
    // microphone does that in about 45 seconds (ten windows at a 4.5 s step) — and read the NaN as speech, so the
    // privacy filter would also suppress the chunks beside it. No window is
    // recorded either way: the classifier neither answered nor failed.
    if chunk.silent {
        report_silent_audio(&chunk.recording);
        return Ok(ChunkPrediction::default());
    }
    let mut verdicts = Vec::with_capacity(route.len());
    let mut human_score = 0.0_f32;
    let mut human_rank: Option<usize> = None;
    let mut loudest_noise = None;
    // The first classifier error, returned only if no routed classifier
    // answered: one model failing must not silence the others (`e853776`).
    let mut first_error = None;
    let now = crate::inference::health::unix_now();
    // Which routed classifiers answered this chunk and which failed it, in route
    // order. Their kept scores are those of the *last* window they ran, so a
    // classifier that failed this one must never be asked what it scored.
    let mut windows: Vec<(usize, bool)> = Vec::with_capacity(route.len());

    for &idx in route {
        let Some(registered) = registry.model_mut(idx) else {
            // Unreachable: routes are resolved against the loaded models at
            // startup and refused if they name one that does not exist.
            continue;
        };
        let model_id = registered.id.clone();
        let own_threshold = registered.own_threshold;
        let inference_started = Instant::now();
        let prediction = match registered.model.predict_chunk(
            &chunk.spectrogram.data,
            &chunk.recording.date,
            &chunk.recording.time,
            chunk.start_secs,
            chunk.end_secs,
            week,
        ) {
            Ok(prediction) => {
                // Only an answered window is timed; see `record_inference_time`.
                registered
                    .health
                    .record_inference_time(inference_started.elapsed());
                if registered.model.last_non_finite_scores() > 0 {
                    record_window(registered, Err((ErrorClass::NonFiniteOutput, None)), now);
                } else {
                    record_window(registered, Ok(()), now);
                }
                prediction
            }
            Err(e) => {
                record_window(registered, Err((ErrorClass::Inference, Some(&e))), now);
                first_error.get_or_insert(e);
                windows.push((idx, false));
                continue;
            }
        };
        windows.push((idx, true));
        human_score = human_score.max(prediction.human_score);
        human_rank = match (human_rank, prediction.human_rank) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // Likewise the loudest watched noise class across classifiers.
        if let Some(noise) = prediction.loudest_noise
            && loudest_noise
                .as_ref()
                .is_none_or(|l: &crate::detection::types::Detection| {
                    noise.confidence > l.confidence
                })
        {
            loudest_noise = Some(noise);
        }
        verdicts.push(crate::detection::merge::ModelVerdict {
            model_id,
            detections: prediction.detections,
            threshold: own_threshold,
        });
    }

    if verdicts.is_empty()
        && let Some(e) = first_error
    {
        return Err(e.into());
    }

    let mut detections = crate::detection::merge::merge_verdicts(&verdicts);
    attach_answers(registry, &windows, chunk.start_secs, &mut detections);
    Ok(ChunkPrediction {
        detections,
        human_score,
        human_rank,
        loudest_noise,
    })
}

/// A classifier's most recent window, as something a species can be looked up in.
type Lookup<'a> = Box<dyn Fn(&str) -> crate::detection::answers::ClassScore + 'a>;

/// Give each merged detection what every classifier that listened to the chunk
/// said about its species (migrations 57 and 58).
///
/// The merge keeps one detection per species and the winner's confidence; this
/// is the rest. `windows` is `(route index, answered)` for each classifier that
/// ran: a classifier that failed the chunk is recorded as failed and its kept
/// scores, which are the previous window's, are never read.
///
/// **Only when two or more classifiers listened.** The answers exist so that
/// classifiers can be compared. With one, the detection row is already the whole
/// record (its `Confidence` is that classifier's score, its `Cutoff` the bar),
/// and a row per detection would be a write per detection on every station, for
/// a card that has nothing to compare. A source routed to the primary alone on a
/// station that runs several classifiers is in the same position.
fn attach_answers(
    registry: &ClassifierRegistry,
    windows: &[(usize, bool)],
    chunk_start_secs: f32,
    detections: &mut [crate::detection::types::Detection],
) {
    use crate::detection::answers::{WindowOutcome, answers_for};
    if detections.is_empty() {
        return;
    }
    let listening: Vec<(&crate::inference::registry::RegisteredModel, bool)> = windows
        .iter()
        .filter_map(|&(idx, answered)| Some((registry.model(idx)?, answered)))
        .collect();
    if listening.len() < 2 {
        return;
    }
    let lookups: Vec<Option<Lookup<'_>>> = listening
        .iter()
        .map(|&(registered, answered)| {
            answered.then(|| {
                Box::new(move |name: &str| registered.model.score_of_last_window(name))
                    as Lookup<'_>
            })
        })
        .collect();
    let outcomes: Vec<WindowOutcome<'_>> = listening
        .iter()
        .zip(&lookups)
        .map(|(&(registered, _), lookup)| WindowOutcome {
            model_id: &registered.id,
            bar: registered.own_threshold,
            window_start_secs: chunk_start_secs,
            window_end_secs: chunk_start_secs + registered.spec.window_secs(),
            scores: lookup.as_deref(),
        })
        .collect();
    for detection in detections {
        detection.model_scores = answers_for(&outcomes, &detection.scientific_name);
    }
}

/// `correlation_id`, if non-empty, is stamped on every event emitted for
/// this file and surfaced in every log line — see [`DetectionEvent::correlation_id`].
///
/// # Errors
///
/// Returns `DaemonError` if any stage fails.
#[allow(clippy::cast_possible_truncation)]
pub fn process_and_infer(
    path: &Path,
    pipeline_config: &PipelineConfig,
    registry: &mut ClassifierRegistry,
    route: &[usize],
    correlation_id: &str,
) -> Result<Vec<DetectionEvent>, DaemonError> {
    let start = Instant::now();

    let chunks = pipeline::process_file(path, pipeline_config)?;
    let pipeline_elapsed = start.elapsed();

    tracing::debug!(
        correlation_id,
        file = %path.display(),
        chunks = chunks.len(),
        pipeline_ms = pipeline_elapsed.as_millis(),
        "audio pipeline complete"
    );

    let mut events = Vec::new();

    for chunk in &chunks {
        let infer_start = Instant::now();

        let detections = infer_chunk(
            registry,
            route,
            chunk,
            geomodel_week(&chunk.recording.date, path),
        )?
        .detections;

        let infer_elapsed = infer_start.elapsed();
        let total_ms = start.elapsed().as_millis() as u64;

        for detection in detections {
            tracing::info!(
                correlation_id,
                species = %detection.common_name,
                confidence = format!("{:.1}%", detection.confidence * 100.0),
                chunk = format!("{:.1}s-{:.1}s", chunk.start_secs, chunk.end_secs),
                infer_ms = infer_elapsed.as_millis(),
                "detection"
            );

            events.push(DetectionEvent {
                detection,
                source_file: path.to_path_buf(),
                latency_ms: total_ms,
                correlation_id: correlation_id.to_owned(),
                lease: None,
            });
        }
    }

    let total = start.elapsed();
    // DEBUG, not INFO: one of the two per-file lines OB-15 measured at 92 % of
    // the journal's volume; the counter carries the count.
    tracing::debug!(
        correlation_id,
        file = %path.display(),
        detections = events.len(),
        total_ms = total.as_millis(),
        "file processing complete"
    );

    Ok(events)
}

/// Process a single audio file with privacy and species occurrence filters.
///
/// After running inference, applies the whole-chunk filters (suppressing
/// chunks with human voice, and chunks a dog barked in) and the species
/// occurrence filter (only keeping species that are likely present at the
/// given location and time of year).
///
/// # Errors
///
/// Returns `DaemonError` if any stage fails.
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
pub fn process_and_infer_filtered(
    path: &Path,
    pipeline_config: &PipelineConfig,
    registry: &mut ClassifierRegistry,
    route: &[usize],
    chunk_filters: &ChunkFilters,
    species_filter: &mut SpeciesFilter,
    filter_observer: Option<&crate::detection::daemon::SpeciesFilterObserver>,
    lat: Option<f64>,
    lon: Option<f64>,
    correlation_id: &str,
) -> Result<Vec<DetectionEvent>, DaemonError> {
    let start = Instant::now();

    let chunks = pipeline::process_file(path, pipeline_config)?;
    let pipeline_elapsed = start.elapsed();

    tracing::debug!(
        correlation_id,
        file = %path.display(),
        chunks = chunks.len(),
        pipeline_ms = pipeline_elapsed.as_millis(),
        "audio pipeline complete"
    );

    // One week per file: every chunk carries the same `recording`, because the
    // date comes from the filename rather than from the chunk's offset within
    // it. Deriving it once says so, and leaves one place for it to be wrong.
    let week = chunks
        .first()
        .map_or(1, |c| geomodel_week(&c.recording.date, path));

    // Run inference on all chunks first to collect raw predictions
    let mut all_predictions: Vec<ChunkPrediction> = Vec::with_capacity(chunks.len());

    for chunk in &chunks {
        all_predictions.push(infer_chunk(registry, route, chunk, week)?);
    }

    // Apply the whole-chunk filters: human speech, then non-bird noise, then
    // corroboration — which needs to know where each chunk sits in the
    // recording to tell which chunks are neighbours.
    let starts: Vec<f32> = chunks.iter().map(|c| c.start_secs).collect();
    let filtered_predictions = chunk_filters.apply(&starts, &all_predictions);

    // Build the allowed species set from the species filter.
    //
    // Always consulted, even with no coordinates: only the metadata model needs
    // to know where the station is, and `filter_species` skips just that stage
    // when `location` is `None`. The operator's include/exclude lists are an
    // explicit instruction and still apply — gating the whole filter on
    // coordinates, as this used to, meant a station that never set a latitude
    // kept recording every species its operator had asked to suppress.
    // The primary classifier's labels. The occurrence filter is a BirdNET
    // geomodel scored against a BirdNET label set; a second classifier with a
    // different vocabulary is filtered by its own thresholds and lists, not by
    // a geomodel that has never heard of its labels.
    let primary_labels = registry.primary().model.labels();
    //
    // A metadata model that fails *here*, at run time, falls back to the
    // operator's lists — the same thing a model that fails to load does. This
    // was a `?`, which failed every file for as long as the fault lasted: a
    // station that recorded nothing, rather than one that recorded without an
    // occurrence filter.
    let (allowed_species, geo_ok) =
        match species_filter.filter_species(lat.zip(lon), week, primary_labels) {
            Ok(set) => (set, true),
            Err(e) => {
                report_species_filter_failure(&e);
                (species_filter.allowed_by_lists(primary_labels), false)
            }
        };
    // Species only another routed classifier knows: the operator's lists
    // alone. Every detection used to be checked against the primary's set,
    // so a species outside BirdNET's vocabulary was dropped whichever model
    // heard it.
    let primary_names: std::collections::HashSet<&str> = primary_labels
        .iter()
        .map(|l| l.scientific_name.as_str())
        .collect();
    let allowed_elsewhere: std::collections::HashSet<String> = route
        .iter()
        .filter(|&&idx| idx != 0)
        .filter_map(|&idx| registry.model(idx))
        .flat_map(|m| species_filter.allowed_by_lists(m.model.labels()))
        .filter(|name| !primary_names.contains(name.as_str()))
        .collect();
    let allowed = |sci: &str| {
        if primary_names.contains(sci) {
            allowed_species.contains(sci)
        } else {
            allowed_elsewhere.contains(sci)
        }
    };
    if let Some(observer) = filter_observer {
        observer.report(
            species_filter.has_model() && geo_ok,
            Some(allowed_species.len() as u64),
        );
    }

    // Collect events, applying species filter
    let mut events = Vec::new();
    let total_ms = start.elapsed().as_millis() as u64;

    for (chunk, detections) in chunks.iter().zip(filtered_predictions.iter()) {
        for detection in detections {
            if !allowed(&detection.scientific_name) {
                continue;
            }

            // Apply per-species confidence threshold (checked in event_processor instead)
            // The daemon produces raw events; threshold filtering is done downstream.

            tracing::info!(
                correlation_id,
                species = %detection.common_name,
                confidence = format!("{:.1}%", detection.confidence * 100.0),
                chunk = format!("{:.1}s-{:.1}s", chunk.start_secs, chunk.end_secs),
                "detection (filtered)"
            );

            events.push(DetectionEvent {
                detection: detection.clone(),
                source_file: path.to_path_buf(),
                latency_ms: total_ms,
                correlation_id: correlation_id.to_owned(),
                lease: None,
            });
        }
    }

    let total = start.elapsed();
    // DEBUG, not INFO: one of the two per-file lines OB-15 measured at 92 % of
    // the journal's volume; the counter carries the count.
    tracing::debug!(
        correlation_id,
        file = %path.display(),
        detections = events.len(),
        total_ms = total.as_millis(),
        privacy = chunk_filters.privacy.is_enabled(),
        noise = chunk_filters.noise.is_enabled(),
        confirmation = chunk_filters.confirmation.as_str(),
        species_filter = species_filter.has_model(),
        "filtered file processing complete"
    );

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_nonexistent_file_returns_error() {
        let config = PipelineConfig::default();
        let result = process_file_pipeline_only(
            Path::new("/nonexistent/2026-03-11-birdnet-08:30:00.wav"),
            &config,
        );
        assert!(result.is_err());
    }

    /// A metadata model that loads but fails every time it is *run* must not
    /// silence the station.
    ///
    /// At load time a metadata model that cannot be used already falls back
    /// to passthrough (the operator's lists still apply). At run time the
    /// same failure was a `?`, which failed every file: nothing recorded, one
    /// error per segment, for as long as the station ran.
    ///
    /// The fixture is the tiny V2.4 classifier loaded as a metadata model: its
    /// declared output width (11) matches the 11-label vocabulary, so it
    /// loads, and its `[1, 144 000]` input then refuses the `[1, 3]`
    /// `(lat, lon, week)` tensor on every run.
    #[test]
    fn a_metadata_model_that_fails_at_run_time_falls_back_to_the_lists() {
        use crate::detection::corroboration::ConfirmationLevel;
        use crate::detection::noise::NoiseFilter;
        use crate::detection::privacy::PrivacyFilter;
        use crate::inference::labels::LabelSet;
        use crate::inference::model::{BirdNetModel, ModelConfig};
        use crate::inference::species_filter::SpeciesFilterConfig;

        const TINY_V24: &[u8] = include_bytes!("../../testdata/tiny_v24_test.onnx");
        let tmp = tempfile::tempdir().unwrap();
        let model_path = tmp.path().join("m.onnx");
        std::fs::write(&model_path, TINY_V24).unwrap();
        let labels = LabelSet::from_entries(
            (0..11)
                .map(|i| (format!("Species{i} bird"), format!("Bird {i}")))
                .collect(),
        );

        let config = ModelConfig {
            confidence_threshold: 0.0,
            ..ModelConfig::default()
        };
        let model = BirdNetModel::load(&model_path, labels.clone(), config).unwrap();
        let mut registry = ClassifierRegistry::single("birdnet", model);

        let mut species_filter = SpeciesFilter::load_with_vocabulary(
            &model_path,
            None,
            &labels,
            &std::collections::HashMap::new(),
            SpeciesFilterConfig {
                exclude_list: vec!["Species0 bird".to_owned()],
                ..SpeciesFilterConfig::default()
            },
        )
        .expect("precondition: the fixture loads as a metadata model");
        assert!(
            species_filter
                .filter_species(Some((52.0, 13.0)), 10, &labels)
                .is_err(),
            "precondition: the fixture fails when run"
        );

        // Three seconds of low noise at 48 kHz.
        let wav = tmp.path().join("2026-05-19-birdnet-09:00:00.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        let mut x: u32 = 1;
        for _ in 0..48_000 * 3 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            w.write_sample(((x >> 16) as i16) / 8).unwrap();
        }
        w.finalize().unwrap();

        let filters = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise: NoiseFilter::new(0.0, Vec::new()),
            confirmation: ConfirmationLevel::Off,
        };
        let pipeline_config = PipelineConfig {
            raw_audio_input: true,
            target_sample_rate: 48_000,
            chunk_duration_secs: 3.0,
            ..PipelineConfig::default()
        };
        let events = process_and_infer_filtered(
            &wav,
            &pipeline_config,
            &mut registry,
            &[0],
            &filters,
            &mut species_filter,
            None,
            Some(52.0),
            Some(13.0),
            "",
        )
        .expect("a failing metadata model must not fail the file");
        assert!(!events.is_empty(), "threshold 0 reports every species");
        assert!(
            events
                .iter()
                .all(|e| e.detection.scientific_name != "Species0 bird"),
            "the operator's exclude list still applies"
        );
    }
    /// Two classifiers whose inputs differ, so one chunk can fail the first
    /// and not the second: the tiny V2.4 model refuses fewer than half its
    /// 144 000 values, and the tiny V3.0 model, declared at 48 kHz so the two
    /// may share a station, takes 96 000.
    fn two_classifiers(dir: &Path) -> ClassifierRegistry {
        two_classifiers_with(dir, (None, None), 11)
    }

    /// As [`two_classifiers`], with each classifier's own bar and the number of
    /// classes the second one has names for (its outputs past that are unnamed).
    fn two_classifiers_with(
        dir: &Path,
        bars: (Option<f32>, Option<f32>),
        second_labels: usize,
    ) -> ClassifierRegistry {
        use crate::inference::model::ModelConfig;
        use crate::inference::registry::ModelSpec;
        const TINY_V24: &[u8] = include_bytes!("../../testdata/tiny_v24_test.onnx");
        const TINY_V30: &[u8] = include_bytes!("../../testdata/tiny_v30_test.onnx");
        let names = |n: usize| {
            (0..n)
                .map(|i| format!("Species{i} bird_Bird {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let labels = dir.join("labels.txt");
        std::fs::write(&labels, names(11)).unwrap();
        let labels_second = dir.join("labels_second.txt");
        std::fs::write(&labels_second, names(second_labels)).unwrap();
        let v24 = dir.join("v24.onnx");
        let v30 = dir.join("v30.onnx");
        std::fs::write(&v24, TINY_V24).unwrap();
        std::fs::write(&v30, TINY_V30).unwrap();
        let spec = |id: &str, path: &Path, labels_path: &Path, rate, threshold| ModelSpec {
            id: id.to_owned(),
            model_path: path.to_path_buf(),
            labels_path: labels_path.to_path_buf(),
            threshold,
            sample_rate: rate,
        };
        ClassifierRegistry::load(
            &[
                spec("birdnet", &v24, &labels, None, bars.0),
                spec("v3", &v30, &labels_second, Some(48_000), bars.1),
            ],
            &std::collections::HashMap::new(),
            &ModelConfig {
                confidence_threshold: 0.0,
                ..ModelConfig::default()
            },
        )
        .expect("both tiny classifiers load")
    }

    fn chunk_of(values: usize) -> PreparedChunk {
        PreparedChunk {
            spectrogram: crate::audio::spectrogram::MelSpectrogram {
                data: vec![0.01; values],
                n_mels: 1,
                n_frames: values,
            },
            start_secs: 0.0,
            end_secs: 3.0,
            recording: crate::detection::types::RecordingFile::parse(
                "/x/2026-05-19-birdnet-09:00:00.wav",
            )
            .unwrap(),
            silent: false,
        }
    }

    /// A window with no variation at all is not given to any classifier, and
    /// neither answers nor fails it. The real V3.0 model returns NaN for such
    /// a window, which used to count as a failed window per classifier, turn
    /// the classifier "failing" after ten, and read the NaN as speech.
    #[test]
    fn a_silent_chunk_is_not_a_failed_window() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers(tmp.path());
        let mut silent = chunk_of(144_000);
        silent.silent = true;
        for _ in 0..crate::inference::health::FAILING_AFTER + 2 {
            let out =
                infer_chunk(&mut registry, &[0, 1], &silent, 20).expect("silence is not an error");
            assert!(out.detections.is_empty());
            assert!(out.human_score.abs() < f32::EPSILON);
            assert_eq!(out.human_rank, None, "silence is not speech");
        }
        for (id, health) in registry.health() {
            assert_eq!(health.windows, 0, "{id}: no window was run or recorded");
            assert!(!health.is_failing(), "{id}: {health:?}");
            assert_eq!(health.last_success_at, None, "{id}: it answered nothing");
        }
        // The counterpart: the same data not marked silent is classified, so the
        // skip is the flag's doing and not a property of the stub models.
        let heard = infer_chunk(&mut registry, &[0, 1], &chunk_of(144_000), 20).unwrap();
        assert!(!heard.detections.is_empty());
        assert!(registry.health().iter().all(|(_, h)| h.windows == 1));
    }

    /// `e853776`: a classifier that fails a window is recorded as failing it,
    /// and does not silence the classifiers that answered. Before, its error
    /// was a `?` that failed the whole file — every other model's detections
    /// with it — and was recorded nowhere but the journal.
    #[test]
    fn a_classifier_failing_a_window_does_not_silence_the_others() {
        use crate::inference::health::{ErrorClass, FAILING_AFTER};
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers(tmp.path());

        // Precondition: this chunk is one the V2.4 model refuses and the V3.0
        // model takes, or the test below proves nothing.
        let half = chunk_of(60_000);
        for _ in 0..FAILING_AFTER {
            let prediction = infer_chunk(&mut registry, &[0, 1], &half, 20)
                .expect("the answering classifier carries the window");
            assert!(
                !prediction.detections.is_empty(),
                "threshold 0: the V3.0 model's species are reported"
            );
            assert!(
                prediction
                    .detections
                    .iter()
                    .all(|d| d.model_id.as_deref() == Some("v3")),
                "only the classifier that answered: {:?}",
                prediction.detections
            );
        }
        let health = registry.health();
        assert_eq!(health[0].0, "birdnet");
        assert!(health[0].1.is_failing(), "{:?}", health[0].1);
        assert_eq!(health[0].1.error_class, Some(ErrorClass::Inference));
        assert_eq!(health[0].1.last_success_at, None);
        assert!(!health[1].1.is_failing(), "{:?}", health[1].1);
        assert_eq!(health[1].1.windows, u64::from(FAILING_AFTER));
        assert!(health[1].1.last_success_at.is_some());

        // A full window: both answer, and the failing one is well again.
        infer_chunk(&mut registry, &[0, 1], &chunk_of(144_000), 20).unwrap();
        let health = registry.health();
        assert!(!health[0].1.is_failing());
        assert_eq!(health[0].1.consecutive_failures, 0);
        assert!(health[0].1.last_success_at.is_some());
    }

    /// A full 144 000-value window whose first values are `head`: both tiny
    /// models read them as the scores of their first classes (V3.0 directly,
    /// V2.4 as logits through the sigmoid).
    fn chunk_scoring(head: &[f32]) -> PreparedChunk {
        let mut chunk = chunk_of(144_000);
        chunk.spectrogram.data.fill(0.0);
        chunk.spectrogram.data[..head.len()].copy_from_slice(head);
        chunk
    }

    fn answer_of<'a>(
        d: &'a crate::detection::types::Detection,
        model: &str,
    ) -> &'a crate::detection::types::ModelAnswer {
        d.model_scores
            .iter()
            .find(|a| a.model_id == model)
            .unwrap_or_else(|| panic!("no answer from {model}: {:?}", d.model_scores))
    }

    /// The point of migration 57, end to end through two real ONNX sessions: a
    /// species one classifier reported and the other scored below its bar keeps
    /// *both* scores, each against its own bar. Before, the second classifier's
    /// number was gone by the time the row was written.
    #[test]
    fn every_merged_species_carries_each_classifier_s_own_score_against_its_own_bar() {
        use crate::detection::types::AnswerState::{BelowBar, Heard};
        let tmp = tempfile::tempdir().unwrap();
        // birdnet (V2.4, logits through a sigmoid) bar 0.60; v3 (probabilities) bar 0.70.
        let mut registry = two_classifiers_with(tmp.path(), (Some(0.60), Some(0.70)), 11);
        // Species0: raw 0.9. Species1: raw 0.2. Species2: raw 0.5.
        let chunk = chunk_scoring(&[0.9, 0.2, 0.5]);
        let out = infer_chunk(&mut registry, &[0, 1], &chunk, 20).expect("both answer");

        let by_name = |n: &str| {
            out.detections
                .iter()
                .find(|d| d.scientific_name == n)
                .unwrap_or_else(|| panic!("{n} not reported: {:?}", out.detections))
        };
        // Species0: both clear their own bar.
        let s0 = by_name("Species0 bird");
        assert_eq!(answer_of(s0, "v3").state, Heard);
        assert_eq!(answer_of(s0, "birdnet").state, Heard);
        assert_eq!(s0.agreeing_models, Some(2));
        // Species2: only birdnet reported it (sigmoid(0.5) = 0.622 clears 0.60);
        // v3 did not report it, and its own score, 0.5, is kept and is under 0.70.
        let s2 = by_name("Species2 bird");
        assert_eq!(s2.agreeing_models, Some(1));
        let birdnet = answer_of(s2, "birdnet");
        assert_eq!(birdnet.state, Heard);
        assert!(
            (birdnet.score.unwrap() - 0.622_459).abs() < 1e-4,
            "{birdnet:?}"
        );
        let v3 = answer_of(s2, "v3");
        assert_eq!(
            v3.state, BelowBar,
            "a score the classifier did not report is still kept"
        );
        assert!((v3.score.unwrap() - 0.5).abs() < 1e-6, "{v3:?}");
        assert_eq!(v3.bar, Some(0.70));
        // Species1 scored under both bars, so nobody reported it: no detection.
        assert!(
            out.detections
                .iter()
                .all(|d| d.scientific_name != "Species1 bird")
        );
        // The classifier whose answer the row carries scored exactly the row's number.
        for d in &out.detections {
            let lead = answer_of(d, d.model_id.as_deref().unwrap());
            assert_eq!(lead.score, Some(d.confidence), "{}", d.scientific_name);
        }
    }

    /// A classifier that has no class for the species is `NoLabel`, with no
    /// score and no bar, and not a score of zero.
    #[test]
    fn a_classifier_without_the_species_says_so_and_scores_nothing() {
        use crate::detection::types::AnswerState::{Heard, NoLabel};
        let tmp = tempfile::tempdir().unwrap();
        // v3 is named for only its first 8 classes; outputs 8..10 have no label.
        let mut registry = two_classifiers_with(tmp.path(), (None, None), 8);
        let out = infer_chunk(&mut registry, &[0, 1], &chunk_scoring(&[0.9]), 20).unwrap();
        let s9 = out
            .detections
            .iter()
            .find(|d| d.scientific_name == "Species9 bird")
            .expect("birdnet reports it");
        let v3 = answer_of(s9, "v3");
        assert_eq!(v3.state, NoLabel);
        assert_eq!((v3.score, v3.bar), (None, None));
        assert_eq!(answer_of(s9, "birdnet").state, Heard);
    }

    /// The scores a classifier keeps are those of its *last* window. If a failed
    /// window were answered from them, a classifier that failed this chunk would
    /// be reported as having heard what it heard in the previous one.
    #[test]
    fn a_classifier_that_failed_this_chunk_is_failed_and_not_answered_from_the_last_one() {
        use crate::detection::types::AnswerState::{Failed, Heard};
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers_with(tmp.path(), (Some(0.60), Some(0.70)), 11);
        // A full window first: birdnet answers and keeps 0.71 for Species0.
        let full = infer_chunk(&mut registry, &[0, 1], &chunk_scoring(&[0.9]), 20).unwrap();
        let s0 = full
            .detections
            .iter()
            .find(|d| d.scientific_name == "Species0 bird")
            .unwrap();
        assert_eq!(answer_of(s0, "birdnet").state, Heard, "precondition");
        // Then a half window, which the V2.4 model refuses and V3.0 takes.
        let mut half = chunk_of(60_000);
        half.spectrogram.data[..1].copy_from_slice(&[0.9]);
        let out = infer_chunk(&mut registry, &[0, 1], &half, 20).unwrap();
        let s0 = out
            .detections
            .iter()
            .find(|d| d.scientific_name == "Species0 bird")
            .expect("v3 reports it");
        let birdnet = answer_of(s0, "birdnet");
        assert_eq!(birdnet.state, Failed, "{birdnet:?}");
        assert_eq!((birdnet.score, birdnet.bar), (None, None));
        assert_eq!(answer_of(s0, "v3").state, Heard);
    }

    /// Classifiers' windows differ in length, so each answer says which instants
    /// of the recording that classifier listened to.
    #[test]
    fn each_answer_names_the_window_that_classifier_listened_to() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers_with(tmp.path(), (Some(0.60), Some(0.70)), 11);
        let mut chunk = chunk_scoring(&[0.9]);
        chunk.start_secs = 6.0;
        let out = infer_chunk(&mut registry, &[0, 1], &chunk, 20).unwrap();
        let d = &out.detections[0];
        // birdnet: 144 000 samples at 48 kHz = 3 s. v3: 96 000 declared at 48 kHz = 2 s.
        let b = answer_of(d, "birdnet");
        assert!(
            (b.window_start_secs - 6.0).abs() < 1e-6 && (b.window_end_secs - 9.0).abs() < 1e-6,
            "{b:?}"
        );
        let v = answer_of(d, "v3");
        assert!(
            (v.window_start_secs - 6.0).abs() < 1e-6 && (v.window_end_secs - 8.0).abs() < 1e-6,
            "{v:?}"
        );
    }

    /// A classifier the source is not routed to was not asked, so it is absent
    /// from the answers. With only one left listening there is nothing to
    /// compare, and the detection row alone is the record: no answers at all.
    #[test]
    fn a_chunk_only_one_classifier_listened_to_leaves_no_answers() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers_with(tmp.path(), (None, None), 11);
        let out = infer_chunk(&mut registry, &[1], &chunk_scoring(&[0.9]), 20).unwrap();
        assert!(
            !out.detections.is_empty(),
            "the classifier still detects; only the comparison is skipped"
        );
        for d in &out.detections {
            assert!(d.model_scores.is_empty(), "{:?}", d.model_scores);
        }
    }

    /// The counterpart: routed to both, the same chunk carries an answer from
    /// each. Without it the test above would pass for a writer that never
    /// wrote anything.
    #[test]
    fn a_chunk_two_classifiers_listened_to_carries_an_answer_from_each() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers_with(tmp.path(), (None, None), 11);
        let out = infer_chunk(&mut registry, &[0, 1], &chunk_scoring(&[0.9]), 20).unwrap();
        assert!(!out.detections.is_empty());
        for d in &out.detections {
            let mut ids: Vec<_> = d.model_scores.iter().map(|a| a.model_id.as_str()).collect();
            ids.sort_unstable();
            assert_eq!(ids, ["birdnet", "v3"], "{:?}", d.model_scores);
        }
    }

    /// What each classifier is, read from the loaded models: the facts an
    /// operator checks against what they think they configured.
    #[test]
    fn the_registry_states_what_each_loaded_classifier_is() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = two_classifiers_with(tmp.path(), (Some(0.60), Some(0.70)), 8);
        let info = registry.info();
        assert_eq!(info.len(), 2);
        assert_eq!(info[0].id, "birdnet");
        assert!((info[0].window_secs - 3.0).abs() < 1e-6, "{:?}", info[0]);
        assert_eq!((info[0].sample_rate, info[0].labels), (48_000, 11));
        assert!((info[0].bar - 0.60).abs() < 1e-6);
        // v3 is declared 48 kHz over 96 000 samples: a 2 s window; named for 8 classes.
        assert_eq!(info[1].id, "v3");
        assert!((info[1].window_secs - 2.0).abs() < 1e-6, "{:?}", info[1]);
        assert_eq!((info[1].sample_rate, info[1].labels), (48_000, 8));
        assert!((info[1].bar - 0.70).abs() < 1e-6);
    }

    /// A classifier that answers with scores that are not numbers fails the
    /// window as `non_finite_output`, though it did not error: those scores are
    /// never recorded, so a model doing this every window is silent, not
    /// broken-looking. NaN audio is how the tiny model is made to do it.
    #[test]
    fn a_classifier_answering_nan_fails_the_window() {
        use crate::inference::health::ErrorClass;
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers(tmp.path());
        let mut nan = chunk_of(144_000);
        nan.spectrogram.data.fill(f32::NAN);
        infer_chunk(&mut registry, &[0], &nan, 20).expect("it answers");
        assert!(
            registry.primary().model.last_non_finite_scores() > 0,
            "precondition: NaN in, NaN out"
        );
        let health = registry.health();
        assert_eq!(health[0].1.consecutive_failures, 1);
        assert_eq!(health[0].1.error_class, Some(ErrorClass::NonFiniteOutput));
        assert_eq!(health[0].1.last_success_at, None);

        // Counterpart: finite audio is a clean window.
        infer_chunk(&mut registry, &[0], &chunk_of(144_000), 20).unwrap();
        assert_eq!(registry.primary().model.last_non_finite_scores(), 0);
        assert_eq!(registry.health()[0].1.consecutive_failures, 0);
    }

    /// The counterpart: when no routed classifier answers, the window is still
    /// an error, so the file is not reported as analysed.
    #[test]
    fn a_window_no_classifier_answers_is_still_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers(tmp.path());
        assert!(infer_chunk(&mut registry, &[0, 1], &chunk_of(40_000), 20).is_err());
        assert!(infer_chunk(&mut registry, &[0], &chunk_of(60_000), 20).is_err());
        let health = registry.health();
        assert_eq!(health[0].1.consecutive_failures, 2);
        assert_eq!(health[1].1.consecutive_failures, 1, "not run on the second");
    }
}
