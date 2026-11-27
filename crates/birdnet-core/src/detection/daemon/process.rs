//! Per-file processing: decode → pipeline → inference → (optional) filtering.

use std::path::Path;
use std::time::Instant;

use crate::detection::ChunkFilters;
use crate::detection::pipeline::{
    self, ChannelAnalysis, OccurrenceWeek, PipelineConfig, PreparedChunk,
};
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

/// The occurrence filter's state after it ran on one file: active with the
/// species it admitted, or off with why when it failed (MDL-05). The next file
/// it runs on sets it again, so a passing fault is not published for longer
/// than it lasts.
fn filter_state_after_file(
    failure: Option<&crate::inference::model::InferenceError>,
    admitted: u64,
) -> crate::inference::model_state::FilterState {
    use crate::inference::model_state::{FilterOff, FilterState};
    failure.map_or(
        FilterState::Active {
            candidates: Some(admitted),
        },
        |e| {
            FilterState::off(
                FilterOff::ModelUnusable,
                format!(
                    "the occurrence model failed on the last recording ({e}); recording goes \
                     on with the operator's species lists alone. If this persists, install the \
                     occurrence model again"
                ),
            )
        },
    )
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
/// Each answering classifier's own human score, human rank and loudest noise
/// are carried in [`ChunkPrediction::by_classifier`], and the privacy and
/// noise filters judge each against its own rule, flagging the chunk when
/// **any** classifier's rule fires (review F09): if any model heard speech the
/// safe reading is that there was speech, and erring towards suppressing a
/// bird is recoverable where erring towards publishing a conversation is not.
/// Scores are never compared across classifiers for that verdict; the
/// highest score, best rank and loudest noise kept beside them are summaries
/// for a reader of one number. With no per-classifier rule the verdict is the
/// one the summaries gave against the station's rule.
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
    let mut by_classifier = Vec::with_capacity(route.len());
    // Whether a classifier able to hear people failed this window (F08).
    let mut privacy_unassessed = false;
    // The first classifier error, returned only if no routed classifier
    // answered: one model failing must not silence the others (`e853776`).
    let mut first_error = None;
    let now = crate::inference::health::unix_now();
    // Which routed classifiers answered this chunk and which failed it, in route
    // order. Their kept scores are those of the *last* window they ran, so a
    // classifier that failed this one must never be asked what it scored.
    let mut windows: Vec<(usize, bool)> = Vec::with_capacity(route.len());
    let primary_rate = registry.primary().spec.sample_rate;

    for &idx in route {
        let Some(registered) = registry.model_mut(idx) else {
            // Unreachable: routes are resolved against the loaded models at
            // startup and refused if they name one that does not exist.
            continue;
        };
        let model_id = registered.id.clone();
        let own_threshold = registered.own_threshold;
        // The primary's copy, or the copy the pipeline prepared at this
        // classifier's own rate (MMP-01). The pipeline prepares one per rate
        // the registry reports, so a missing copy means a pipeline built
        // without them: that is this classifier failing the window, said out
        // loud, never fed audio at another rate.
        let rate = registered.spec.sample_rate;
        let input = if rate == primary_rate {
            Some(chunk.spectrogram.data.as_slice())
        } else {
            chunk.samples_at(rate)
        };
        let Some(input) = input else {
            let e = crate::inference::model::InferenceError::Shape(format!(
                "no audio was prepared at {rate} Hz for classifier `{model_id}`"
            ));
            record_window(registered, Err((ErrorClass::Inference, Some(&e))), now);
            privacy_unassessed |= registered.model.has_human_labels();
            first_error.get_or_insert(e);
            windows.push((idx, false));
            continue;
        };
        let inference_started = Instant::now();
        let prediction = match registered.model.predict_chunk(
            input,
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
                privacy_unassessed |= registered.model.has_human_labels();
                first_error.get_or_insert(e);
                windows.push((idx, false));
                continue;
            }
        };
        windows.push((idx, true));
        // Each classifier's own evidence, on its own scale, for filters that
        // judge each by its own rule (review F09). The summaries below are
        // kept for readers of one number; no filter decides on them while
        // this is filled.
        by_classifier.push(crate::detection::types::ClassifierEvidence {
            model_id: model_id.clone(),
            human_score: prediction.human_score,
            human_rank: prediction.human_rank,
            loudest_noise: prediction.loudest_noise.clone().map(|mut noise| {
                noise.model_id = Some(model_id.clone());
                noise
            }),
        });
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
    // One name per species on the station, the primary's (MMP-10). Only a
    // station that ran more than one classifier on this chunk can differ.
    if verdicts.len() > 1 {
        let primary = registry.primary().model.labels();
        crate::detection::merge::name_as_primary(&mut detections, |sci| {
            primary
                .iter()
                .find(|l| l.scientific_name == sci)
                .map(|l| l.common_name.as_str())
        });
    }
    attach_answers(
        registry,
        &windows,
        (chunk.start_secs, chunk.end_secs),
        &mut detections,
    );
    Ok(ChunkPrediction {
        detections,
        human_score,
        human_rank,
        privacy_unassessed,
        loudest_noise,
        by_classifier,
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
    (chunk_start_secs, chunk_end_secs): (f32, f32),
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
        .map(|(&(registered, _), lookup)| {
            let (window_end_secs, padded_secs) = crate::detection::answers::window_support(
                chunk_start_secs,
                registered.spec.window_secs(),
                chunk_end_secs,
            );
            WindowOutcome {
                model_id: &registered.id,
                bar: registered.own_threshold,
                window_start_secs: chunk_start_secs,
                window_end_secs,
                padded_secs,
                scores: lookup.as_deref(),
            }
        })
        .collect();
    for detection in detections {
        detection.model_scores = answers_for(&outcomes, &detection.scientific_name);
    }
}

/// Every window's prediction, by `infer`, and how many failed (review F18).
///
/// A window no routed classifier answered is kept as an empty prediction, so
/// the windows around it keep theirs, and counted; when the station can hear
/// people (`can_hear_people`) it is marked unassessed for speech, so a privacy
/// filter masks it. Only when every window failed is the file an error, as a
/// file the pipeline could not analyse at all.
fn infer_windows<F>(
    chunks: &[crate::detection::pipeline::PreparedChunk],
    can_hear_people: bool,
    mut infer: F,
) -> Result<(Vec<ChunkPrediction>, u64), DaemonError>
where
    F: FnMut(&crate::detection::pipeline::PreparedChunk) -> Result<ChunkPrediction, DaemonError>,
{
    let mut out = Vec::with_capacity(chunks.len());
    let mut failed = 0_u64;
    let mut first_error = None;
    for chunk in chunks {
        match infer(chunk) {
            Ok(prediction) => out.push(prediction),
            Err(e) => {
                failed += 1;
                first_error.get_or_insert(e);
                out.push(ChunkPrediction {
                    privacy_unassessed: can_hear_people,
                    ..ChunkPrediction::default()
                });
            }
        }
    }
    match first_error {
        Some(e) if failed == chunks.len() as u64 => Err(e),
        _ => Ok((out, failed)),
    }
}

/// What the occurrence filter and the operator's lists made of one detection.
#[derive(Debug, Clone, Copy)]
enum Verdict {
    /// A sound or person class, not a species: not recorded (MMP-04).
    Sound,
    /// Held back by the operator's lists: counted only.
    Lists,
    /// Held back because the occurrence model does not score the species.
    Unscored,
    /// Kept: a detection (`None`), or an out-of-range record for the
    /// quarantine with the occurrence model's score (ACC-01).
    Keep(Option<f32>),
}

/// The species the filter admits for one week: the primary's, and those only
/// another routed classifier knows.
struct WeekSets {
    /// The primary's admitted species.
    primary: std::collections::HashSet<String>,
    /// Admitted species no primary label names.
    elsewhere: std::collections::HashSet<String>,
    /// Why the occurrence model failed for the primary, if it did.
    geo_failure: Option<crate::inference::model::InferenceError>,
}

impl WeekSets {
    /// Whether a detection of `sci` is admitted this week.
    fn allows(&self, primary_names: &std::collections::HashSet<&str>, sci: &str) -> bool {
        if primary_names.contains(sci) {
            self.primary.contains(sci)
        } else {
            self.elsewhere.contains(sci)
        }
    }
}

/// Build [`WeekSets`] for `week`.
///
/// A metadata model that fails *here*, at run time, falls back to the
/// operator's lists — the same thing a model that fails to load does. This was
/// a `?`, which failed every file for as long as the fault lasted: a station
/// that recorded nothing, rather than one that recorded without an occurrence
/// filter.
///
/// Species only another routed classifier knows are judged in that
/// classifier's own vocabulary: the occurrence model by name where it scores
/// the species, nothing where it does not (unless whitelisted), and the
/// operator's lists (MMP-03). They used to get the lists alone, so a station
/// with its range filter on recorded extralimital birds, frogs and insects
/// from Perch unfiltered. A model failing here falls back to the lists, as it
/// does for the primary.
fn week_sets(
    species_filter: &mut SpeciesFilter,
    registry: &ClassifierRegistry,
    route: &[usize],
    location: Option<(f64, f64)>,
    week: u32,
    primary_labels: &crate::inference::labels::LabelSet,
    primary_names: &std::collections::HashSet<&str>,
) -> WeekSets {
    let (primary, geo_failure) = match species_filter.filter_species(location, week, primary_labels)
    {
        Ok(set) => (set, None),
        Err(e) => {
            report_species_filter_failure(&e);
            (species_filter.allowed_by_lists(primary_labels), Some(e))
        }
    };
    let mut elsewhere = std::collections::HashSet::new();
    for m in route
        .iter()
        .filter(|&&idx| idx != 0)
        .filter_map(|&idx| registry.model(idx))
    {
        let set = species_filter
            .filter_species_of(&m.id, location, week)
            .unwrap_or_else(|e| {
                report_species_filter_failure(&e);
                species_filter.allowed_by_lists(m.model.labels())
            });
        elsewhere.extend(
            set.into_iter()
                .filter(|name| !primary_names.contains(name.as_str())),
        );
    }
    WeekSets {
        primary,
        elsewhere,
        geo_failure,
    }
}

/// Each distinct value of `weeks`, in the order it first appears.
fn distinct_in_order(weeks: &[u32]) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for &w in weeks {
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// The occurrence week one window is scored against (review F13).
///
/// [`OccurrenceWeek::PerFile`], the default, is BirdNET-Pi's rule: the week of
/// the date in the recording's filename, `file_week`, for every window. With
/// [`OccurrenceWeek::PerWindow`] it is the week of the window's own capture
/// date, the recording's start shifted by the window's start
/// ([`crate::civil::birdnet_week_at`]), so a window past midnight on the last
/// day of a week bin is scored in the next bin. A recording date or time that
/// does not parse keeps `file_week`.
fn window_week(chunk: &PreparedChunk, mode: OccurrenceWeek, file_week: u32) -> u32 {
    match mode {
        OccurrenceWeek::PerFile => file_week,
        OccurrenceWeek::PerWindow => crate::civil::birdnet_week_at(
            &chunk.recording.date,
            &chunk.recording.time,
            f64::from(chunk.start_secs),
        )
        .unwrap_or(file_week),
    }
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
///
/// [`ChannelAnalysis::Each`] gives one result per channel and is refused here
/// with [`DaemonError::Config`]; [`process_and_infer_channels`] gives it.
#[allow(clippy::too_many_arguments)]
pub fn process_and_infer_filtered(
    path: &Path,
    pipeline_config: &PipelineConfig,
    registry: &mut ClassifierRegistry,
    route: &[usize],
    chunk_filters: &ChunkFilters,
    species_filter: &mut SpeciesFilter,
    model_state: Option<&crate::detection::daemon::ModelStateObserver>,
    lat: Option<f64>,
    lon: Option<f64>,
    correlation_id: &str,
) -> Result<super::FilteredFile, DaemonError> {
    if pipeline_config.channels == ChannelAnalysis::Each {
        return Err(DaemonError::Config(
            "each channel analysed on its own gives one result per channel; call \
             process_and_infer_channels"
                .to_owned(),
        ));
    }
    let mut files = process_and_infer_channels(
        path,
        pipeline_config,
        registry,
        route,
        chunk_filters,
        species_filter,
        model_state,
        lat,
        lon,
        correlation_id,
    )?;
    files
        .pop()
        .ok_or_else(|| DaemonError::Config("the pipeline returned no channel".to_owned()))
}

/// [`process_and_infer_filtered`] under every [`ChannelAnalysis`] (review F12).
///
/// One [`super::FilteredFile`] for the mean or the chosen channel, and
/// one per channel, in channel order, for [`ChannelAnalysis::Each`], each
/// tagged with its channel ([`super::FilteredFile::channel`]).
///
/// Each channel is analysed as its own recording, through its own filters:
/// the privacy filter's neighbours, the noise filter's window and the
/// corroboration count never reach across channels. The channels are still
/// **one station**: one place, one clock, one recorder, and a bird heard on
/// both is one bird, not two. A caller storing these must map each channel
/// to the station's identity (and keep the channel as provenance), never to a
/// station of its own, and must not count a minute of listening once per
/// channel.
///
/// # Errors
///
/// As [`process_and_infer_filtered`]; a channel whose every window failed
/// fails the call.
#[allow(clippy::too_many_arguments)]
pub fn process_and_infer_channels(
    path: &Path,
    pipeline_config: &PipelineConfig,
    registry: &mut ClassifierRegistry,
    route: &[usize],
    chunk_filters: &ChunkFilters,
    species_filter: &mut SpeciesFilter,
    model_state: Option<&crate::detection::daemon::ModelStateObserver>,
    lat: Option<f64>,
    lon: Option<f64>,
    correlation_id: &str,
) -> Result<Vec<super::FilteredFile>, DaemonError> {
    let start = Instant::now();
    let (groups, source_rate_hz, provenance) =
        pipeline::process_file_channels(path, pipeline_config)?;
    groups
        .into_iter()
        .map(|group| {
            let mut file = analyse_chunks(
                path,
                group.chunks,
                source_rate_hz,
                start,
                pipeline_config,
                registry,
                route,
                chunk_filters,
                species_filter,
                model_state,
                lat,
                lon,
                correlation_id,
            )?;
            file.channel = group.channel;
            file.channels = Some(provenance.clone());
            Ok(file)
        })
        .collect()
}

/// One stream's chunks through inference and every filter.
#[allow(
    clippy::cast_possible_truncation,
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "the chunks are this stream's, handed over whole as the old body owned them"
)]
fn analyse_chunks(
    path: &Path,
    chunks: Vec<PreparedChunk>,
    source_rate_hz: u32,
    start: Instant,
    pipeline_config: &PipelineConfig,
    registry: &mut ClassifierRegistry,
    route: &[usize],
    chunk_filters: &ChunkFilters,
    species_filter: &mut SpeciesFilter,
    model_state: Option<&crate::detection::daemon::ModelStateObserver>,
    lat: Option<f64>,
    lon: Option<f64>,
    correlation_id: &str,
) -> Result<super::FilteredFile, DaemonError> {
    let pipeline_elapsed = start.elapsed();

    tracing::debug!(
        correlation_id,
        file = %path.display(),
        chunks = chunks.len(),
        pipeline_ms = pipeline_elapsed.as_millis(),
        "audio pipeline complete"
    );

    // One week per file by default, as BirdNET-Pi scores one: every chunk
    // carries the same `recording`, and the date comes from the filename. With
    // `OccurrenceWeek::PerWindow` each window's own capture date decides its
    // week instead (review F13); see `window_week`.
    let file_week = chunks
        .first()
        .map_or(1, |c| geomodel_week(&c.recording.date, path));
    let mode = pipeline_config.occurrence_week;
    let weeks: Vec<u32> = chunks
        .iter()
        .map(|c| window_week(c, mode, file_week))
        .collect();

    // Run inference on all chunks first to collect raw predictions
    let mut all_predictions: Vec<ChunkPrediction> = Vec::with_capacity(chunks.len());

    let can_hear_people = route.iter().any(|&idx| {
        registry
            .model(idx)
            .is_some_and(|m| m.model.has_human_labels())
    });
    let (predictions, failed_windows) = infer_windows(&chunks, can_hear_people, |chunk| {
        infer_chunk(registry, route, chunk, window_week(chunk, mode, file_week))
    })?;
    all_predictions.extend(predictions);

    // Apply the whole-chunk filters: human speech, then non-bird noise, then
    // corroboration — which needs to know where each chunk sits in the
    // recording to tell which chunks are neighbours.
    let starts: Vec<f32> = chunks.iter().map(|c| c.start_secs).collect();
    let (filtered_predictions, mut tally) = chunk_filters.apply_tallied(&starts, &all_predictions);
    tally.date = chunks.first().map_or_default(|c| c.recording.date.clone());
    tally.failed_windows = failed_windows;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a window's end is a non-negative number of seconds far below u64's range"
    )]
    {
        tally.decoded_ms = chunks.last().map_or(0, |c| {
            (f64::from(c.end_secs.max(0.0)) * 1000.0).round() as u64
        });
    }
    if failed_windows > 0 {
        tracing::warn!(
            correlation_id,
            file = %path.display(),
            failed_windows,
            windows = chunks.len(),
            "no classifier answered some of this file's windows; the rest are kept, and the \
             failed windows are not counted as listening"
        );
    }

    // Build the allowed species set from the species filter.
    //
    // Always consulted, even with no coordinates: only the metadata model needs
    // to know where the station is, and `filter_species` skips just that stage
    // when `location` is `None`. The operator's include/exclude lists are an
    // explicit instruction and still apply — gating the whole filter on
    // coordinates, as this used to, meant a station that never set a latitude
    // kept recording every species its operator had asked to suppress.
    // Every classifier's vocabulary, so an include list is read against the
    // whole station and a second classifier's species can be scored (MMP-03).
    // A no-op after the first file.
    for idx in 0..registry.len() {
        if let Some(m) = registry.model(idx) {
            species_filter.know_vocabulary(&m.id, m.model.labels());
        }
    }
    let primary_labels = registry.primary().model.labels();
    let primary_names: std::collections::HashSet<&str> = primary_labels
        .iter()
        .map(|l| l.scientific_name.as_str())
        .collect();

    // What the filter decided for each detection, by chunk, filled one week at
    // a time: `rejection` reads the scores of the week the filter last ran
    // for, so each week's detections are judged right after its sets are
    // built. One file has one week unless `OccurrenceWeek::PerWindow` is on,
    // and then at most a handful (a midnight inside it); each distinct week is
    // asked once per file, and the filter's own cache keeps the last.
    let mut verdicts: Vec<Vec<Verdict>> = filtered_predictions
        .iter()
        .map(|d| vec![Verdict::Keep(None); d.len()])
        .collect();
    for week in distinct_in_order(&weeks) {
        let sets = week_sets(
            species_filter,
            registry,
            route,
            lat.zip(lon),
            week,
            primary_labels,
            &primary_names,
        );
        // Only a filter that runs here reports, so a filter that start-up
        // found off (no model, or no position: MDL-04) keeps the reason it was
        // given. Reported for the file's own week, which is every week unless
        // the per-window mode is on.
        if week == weeks.first().copied().unwrap_or(file_week)
            && let Some(observer) = model_state
            && species_filter.has_model()
            && lat.zip(lon).is_some()
        {
            observer.filter(&filter_state_after_file(
                sets.geo_failure.as_ref(),
                sets.primary.len() as u64,
            ));
        }
        for (i, detections) in filtered_predictions.iter().enumerate() {
            if weeks[i] != week {
                continue;
            }
            for (j, detection) in detections.iter().enumerate() {
                // A sound or a person, not a species (MMP-04): it has done its
                // work in the chunk filters above and is not recorded.
                if crate::detection::sound_class::is_sound_class(
                    &detection.scientific_name,
                    &detection.common_name,
                ) {
                    verdicts[i][j] = Verdict::Sound;
                    continue;
                }
                // Held back here is counted, and an out-of-range bird the
                // model scored goes to the "Unlikely here" quarantine with that
                // score (ACC-01): a vagrant is the record most worth having,
                // and it vanished without a trace at any confidence. The
                // operator's lists are an instruction and are only counted; a
                // species the model does not score is held back as the
                // maintainer decided (MMP-03).
                if !sets.allows(&primary_names, &detection.scientific_name) {
                    verdicts[i][j] = match species_filter.rejection(&detection.scientific_name) {
                        crate::inference::species_filter::Rejection::Lists => Verdict::Lists,
                        crate::inference::species_filter::Rejection::Unscored => Verdict::Unscored,
                        crate::inference::species_filter::Rejection::Score(p) => {
                            Verdict::Keep(Some(p))
                        }
                    };
                }
            }
        }
    }

    // Collect events, in chunk order.
    let mut events = Vec::new();
    let total_ms = start.elapsed().as_millis() as u64;

    for ((chunk, detections), verdicts) in chunks
        .iter()
        .zip(filtered_predictions.iter())
        .zip(&verdicts)
    {
        for (detection, verdict) in detections.iter().zip(verdicts) {
            let out_of_range = match *verdict {
                Verdict::Sound => continue,
                Verdict::Lists => {
                    tally.species_list_detections += 1;
                    continue;
                }
                Verdict::Unscored => {
                    tally.occurrence_detections += 1;
                    continue;
                }
                Verdict::Keep(Some(p)) => {
                    tally.occurrence_detections += 1;
                    Some(p)
                }
                Verdict::Keep(None) => None,
            };

            // Apply per-species confidence threshold (checked in event_processor instead)
            // The daemon produces raw events; threshold filtering is done downstream.

            if out_of_range.is_none() {
                tracing::info!(
                    correlation_id,
                    species = %detection.common_name,
                    confidence = format!("{:.1}%", detection.confidence * 100.0),
                    chunk = format!("{:.1}s-{:.1}s", chunk.start_secs, chunk.end_secs),
                    "detection (filtered)"
                );
            }

            events.push(DetectionEvent {
                detection: detection.clone(),
                source_file: path.to_path_buf(),
                latency_ms: total_ms,
                correlation_id: correlation_id.to_owned(),
                lease: None,
                out_of_range,
            });
        }
    }

    let total = start.elapsed();
    // DEBUG, not INFO: one of the two per-file lines OB-15 measured at 92 % of
    // the journal's volume; the counter carries the count.
    tracing::debug!(
        correlation_id,
        file = %path.display(),
        detections = events.iter().filter(|e| e.out_of_range.is_none()).count(),
        total_ms = total.as_millis(),
        privacy = chunk_filters.privacy.is_enabled(),
        noise = chunk_filters.noise.is_enabled(),
        confirmation = chunk_filters.confirmation.as_str(),
        species_filter = species_filter.has_model(),
        "filtered file processing complete"
    );

    Ok(super::FilteredFile {
        events,
        tally,
        source_rate_hz,
        channel: None,
        channels: None,
    })
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
        .expect("a failing metadata model must not fail the file")
        .events;
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
        shared: usize,
    ) -> ClassifierRegistry {
        two_classifiers_naming(dir, bars, shared, None)
    }

    /// As [`two_classifiers_with`], the primary's first class named
    /// `primary_first` when given (a human class, for the privacy tests).
    fn two_classifiers_naming(
        dir: &Path,
        bars: (Option<f32>, Option<f32>),
        shared: usize,
        primary_first: Option<&str>,
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
        let primary_names = primary_first.map_or_else(
            || names(11),
            |first| {
                std::iter::once(first.to_owned())
                    .chain((1..11).map(|i| format!("Species{i} bird_Bird {i}")))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        );
        std::fs::write(&labels, primary_names).unwrap();
        // The second classifier shares the primary's first `shared` species;
        // its other classes are species the primary does not have. Always one
        // name per class it scores: a labels file of another length is
        // refused at load (MDL-16).
        let labels_second = dir.join("labels_second.txt");
        let second_names = (0..11)
            .map(|i| {
                if i < shared {
                    format!("Species{i} bird_Bird {i}")
                } else {
                    format!("Unshared{i} bird_Other {i}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&labels_second, second_names).unwrap();
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
            output_scale: None,
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
            at_rates: Vec::new(),
            channel: None,
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

    /// MMP-01: a classifier at another rate than the primary's is handed the
    /// chunk's copy at its own rate, never the primary's samples.
    ///
    /// The tiny V3.0 model (32 kHz, from its shape) reads its input's first
    /// values as its scores, so what it reports says which buffer it was
    /// given. Observed failing with `infer_chunk` handing every classifier
    /// `chunk.spectrogram.data`: the 32 kHz classifier scored the primary's
    /// head, -5, and reported nothing.
    #[test]
    fn a_classifier_at_another_rate_reads_its_own_copy() {
        use crate::inference::model::ModelConfig;
        use crate::inference::registry::ModelSpec;
        const TINY_V24: &[u8] = include_bytes!("../../testdata/tiny_v24_test.onnx");
        const TINY_V30: &[u8] = include_bytes!("../../testdata/tiny_v30_test.onnx");
        let tmp = tempfile::tempdir().unwrap();
        let labels = tmp.path().join("labels.txt");
        std::fs::write(
            &labels,
            (0..11)
                .map(|i| format!("Species{i} bird_Bird {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let spec = |id: &str, bytes: &[u8]| {
            let path = tmp.path().join(format!("{id}.onnx"));
            std::fs::write(&path, bytes).unwrap();
            ModelSpec {
                id: id.to_owned(),
                model_path: path,
                labels_path: labels.clone(),
                threshold: Some(0.5),
                sample_rate: None,
                output_scale: None,
            }
        };
        let mut registry = ClassifierRegistry::load(
            &[spec("birdnet", TINY_V24), spec("v3", TINY_V30)],
            &std::collections::HashMap::new(),
            &ModelConfig {
                confidence_threshold: 0.0,
                ..ModelConfig::default()
            },
        )
        .expect("a 48 kHz primary and a 32 kHz extra load together");
        assert_eq!(registry.extra_sample_rates(), vec![32_000], "precondition");

        // The primary's copy scores everything far below either bar; the
        // 32 kHz copy scores Species0 at 0.9.
        let mut chunk = chunk_of(144_000);
        chunk.spectrogram.data.fill(-5.0);
        let mut at_32k = vec![0.0_f32; 96_000];
        at_32k[0] = 0.9;
        chunk.at_rates = vec![(32_000, at_32k)];

        let out = infer_chunk(&mut registry, &[0, 1], &chunk, 20).expect("both answer");
        let s0 = out
            .detections
            .iter()
            .find(|d| d.scientific_name == "Species0 bird")
            .unwrap_or_else(|| panic!("the 32 kHz copy was not read: {:?}", out.detections));
        assert_eq!(s0.model_id.as_deref(), Some("v3"));
        assert!((s0.confidence - 0.9).abs() < 1e-6, "{s0:?}");
        assert_eq!(out.detections.len(), 1, "{:?}", out.detections);

        // Counterpart: a chunk with no copy at that rate is that classifier's
        // failed window, said in its health; the primary still answers.
        chunk.at_rates.clear();
        let out = infer_chunk(&mut registry, &[0, 1], &chunk, 20).expect("the primary answers");
        assert!(out.detections.is_empty(), "{:?}", out.detections);
        let health = registry.health();
        assert_eq!(health[1].1.consecutive_failures, 1, "{:?}", health[1]);
        assert_eq!(health[0].1.consecutive_failures, 0, "{:?}", health[0]);
    }

    /// MMP-10 through two real ONNX sessions: the extra classifier wins the
    /// chunk, and the row is named as the primary's label file names the
    /// species, not as the winner's.
    #[test]
    fn a_detection_the_extra_classifier_wins_is_named_by_the_primary() {
        use crate::inference::model::ModelConfig;
        use crate::inference::registry::ModelSpec;
        const TINY_V24: &[u8] = include_bytes!("../../testdata/tiny_v24_test.onnx");
        const TINY_V30: &[u8] = include_bytes!("../../testdata/tiny_v30_test.onnx");
        let tmp = tempfile::tempdir().unwrap();
        let spec = |id: &str, bytes: &[u8], name: &str| {
            let path = tmp.path().join(format!("{id}.onnx"));
            std::fs::write(&path, bytes).unwrap();
            let labels = tmp.path().join(format!("{id}.txt"));
            std::fs::write(
                &labels,
                (0..11)
                    .map(|i| format!("Species{i} bird_{name} {i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .unwrap();
            ModelSpec {
                id: id.to_owned(),
                model_path: path,
                labels_path: labels,
                threshold: Some(0.5),
                sample_rate: None,
                output_scale: None,
            }
        };
        let mut registry = ClassifierRegistry::load(
            &[
                spec("birdnet", TINY_V24, "Primary name"),
                spec("v3", TINY_V30, "Other name"),
            ],
            &std::collections::HashMap::new(),
            &ModelConfig {
                confidence_threshold: 0.0,
                ..ModelConfig::default()
            },
        )
        .expect("both load");
        let mut chunk = chunk_of(144_000);
        chunk.spectrogram.data.fill(-5.0);
        let mut at_32k = vec![0.0_f32; 96_000];
        at_32k[0] = 0.9;
        chunk.at_rates = vec![(32_000, at_32k)];

        let out = infer_chunk(&mut registry, &[0, 1], &chunk, 20).expect("both answer");
        let s0 = out
            .detections
            .iter()
            .find(|d| d.scientific_name == "Species0 bird")
            .unwrap_or_else(|| panic!("{:?}", out.detections));
        assert_eq!(s0.model_id.as_deref(), Some("v3"), "precondition: v3 won");
        assert_eq!(s0.common_name, "Primary name 0");
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
        // v3 shares only the primary's first 8 species; Species9 is not one
        // of its classes.
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
        // A whole chunk, as the pipeline cuts one: it ends a chunk later.
        chunk.end_secs = 9.0;
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

    /// Review F09: on the zero-padded last chunk of a recording a classifier's
    /// window runs past the audio. Its answer records the audio that exists
    /// (clamped to the recording's end, as the detection row is) and how much
    /// of its window was padding. A 7 s recording's chunk at 6 s holds one
    /// second of audio: birdnet's 3 s window is 2 s padding, v3's 2 s window 1 s.
    ///
    /// Counterpart: a whole chunk (ending at 9 s) is clamped nowhere and padded
    /// nowhere, so the clamp is the recording's end and not a property of the
    /// windows.
    #[test]
    fn a_padded_tail_window_records_the_audio_that_exists_and_its_padding() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers_with(tmp.path(), (Some(0.60), Some(0.70)), 11);
        let mut tail = chunk_scoring(&[0.9]);
        tail.start_secs = 6.0;
        tail.end_secs = 7.0;
        let out = infer_chunk(&mut registry, &[0, 1], &tail, 20).unwrap();
        let d = &out.detections[0];
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        let b = answer_of(d, "birdnet");
        assert!(
            close(b.window_end_secs, 7.0) && close(b.padded_secs, 2.0),
            "birdnet's answer claims audio past the recording's end: {b:?}"
        );
        let v = answer_of(d, "v3");
        assert!(
            close(v.window_end_secs, 7.0) && close(v.padded_secs, 1.0),
            "v3's answer claims audio past the recording's end: {v:?}"
        );

        let mut whole = chunk_scoring(&[0.9]);
        whole.start_secs = 6.0;
        whole.end_secs = 9.0;
        let out = infer_chunk(&mut registry, &[0, 1], &whole, 20).unwrap();
        let d = &out.detections[0];
        let b = answer_of(d, "birdnet");
        assert!(
            close(b.window_end_secs, 9.0) && close(b.padded_secs, 0.0),
            "{b:?}"
        );
        let v = answer_of(d, "v3");
        assert!(
            close(v.window_end_secs, 8.0) && close(v.padded_secs, 0.0),
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
        // v3 is declared 48 kHz over 96 000 samples: a 2 s window; 11 classes,
        // each named (a labels file of another length is refused at load).
        assert_eq!(info[1].id, "v3");
        assert!((info[1].window_secs - 2.0).abs() < 1e-6, "{:?}", info[1]);
        assert_eq!((info[1].sample_rate, info[1].labels), (48_000, 11));
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

    /// Review F08: a window that a classifier able to hear people failed has
    /// not been judged for speech, whatever the others said, and a privacy
    /// filter masks it rather than keep audio nobody assessed. The primary
    /// here names a human class and fails the window (V2.4 refuses a 60 000
    /// value chunk); the second, which names none, answers it.
    ///
    /// Counterparts: with the privacy filter off nothing is masked, and the
    /// same failure of a primary that cannot hear people masks nothing.
    #[test]
    fn a_window_a_classifier_that_hears_people_failed_is_masked() {
        use crate::detection::privacy::PrivacyFilter;
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers_naming(
            tmp.path(),
            (None, None),
            11,
            Some("Human vocal_Human vocal"),
        );
        assert!(
            registry.primary().model.has_human_labels(),
            "precondition: the primary can hear people"
        );
        let out = infer_chunk(&mut registry, &[0, 1], &chunk_of(60_000), 20)
            .expect("the second classifier answers");
        assert_eq!(
            registry.health()[0].1.consecutive_failures,
            1,
            "precondition: the primary failed the window"
        );
        assert_eq!(
            PrivacyFilter::new(0.03).masked_windows(std::slice::from_ref(&out)),
            [true],
            "a window no classifier able to hear people judged was kept"
        );
        assert_eq!(PrivacyFilter::new(0.0).masked_windows(&[out]), [false]);

        let other = tempfile::tempdir().unwrap();
        let mut deaf = two_classifiers(other.path());
        assert!(!deaf.primary().model.has_human_labels(), "precondition");
        let out = infer_chunk(&mut deaf, &[0, 1], &chunk_of(60_000), 20).unwrap();
        assert_eq!(deaf.health()[0].1.consecutive_failures, 1, "precondition");
        assert_eq!(
            PrivacyFilter::new(0.03).masked_windows(&[out]),
            [false],
            "a failure that cost no privacy coverage masked the window"
        );
    }

    /// Review F09, end to end through two real ONNX sessions: each answering
    /// classifier's own human evidence is carried, so a privacy rule named for
    /// a classifier is read against that classifier's score. The primary names
    /// a human class and scores it sigmoid(0.9) = 0.71; v3 has none. A
    /// station threshold of 0.80 masks nothing; a 0.50 rule for birdnet masks
    /// the chunk; the same rule for v3, which heard nobody, does not.
    #[test]
    fn each_classifier_s_own_human_evidence_reaches_the_privacy_rule() {
        use crate::detection::privacy::{ClassifierPrivacyRule, PrivacyFilter};
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = two_classifiers_naming(
            tmp.path(),
            (None, None),
            11,
            Some("Human vocal_Human vocal"),
        );
        let out = infer_chunk(&mut registry, &[0, 1], &chunk_scoring(&[0.9]), 20).unwrap();
        let ids: Vec<&str> = out
            .by_classifier
            .iter()
            .map(|e| e.model_id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["birdnet", "v3"],
            "each answering classifier, in route order"
        );
        assert!(
            (out.by_classifier[0].human_score - 0.710_949_5).abs() < 1e-4,
            "{:?}",
            out.by_classifier
        );
        assert_eq!(
            out.by_classifier[1].human_rank, None,
            "v3 has no human class"
        );
        let rule = ClassifierPrivacyRule {
            threshold: Some(0.50),
            rank_cutoff: None,
        };
        let masks = |f: PrivacyFilter| f.masked_windows(std::slice::from_ref(&out))[0];
        assert!(!masks(PrivacyFilter::new(0.80)), "precondition");
        assert!(masks(
            PrivacyFilter::new(0.80).with_classifier_rule("birdnet", rule)
        ));
        assert!(!masks(
            PrivacyFilter::new(0.80).with_classifier_rule("v3", rule)
        ));
    }

    /// Review F12, through the daemon's entry point: antiphase stereo
    /// analysed as its mean is silence, so nothing is classified; analysed
    /// channel by channel each channel is classified and its result says which
    /// channel it was. The one-result entry point refuses per-channel analysis
    /// rather than merge two channels' events into one file's.
    #[test]
    fn each_channel_s_result_is_tagged_with_its_channel() {
        use crate::detection::corroboration::ConfirmationLevel;
        use crate::detection::noise::NoiseFilter;
        use crate::detection::pipeline::ChannelAnalysis;
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
        let model = BirdNetModel::load(
            &model_path,
            labels,
            ModelConfig {
                confidence_threshold: 0.0,
                ..ModelConfig::default()
            },
        )
        .unwrap();
        let mut registry = ClassifierRegistry::single("birdnet", model);
        let mut species_filter = SpeciesFilter::new_passthrough(SpeciesFilterConfig::default());
        let wav = crate::detection::pipeline::channel_tests::write_channels(tmp.path(), &[1, -1]);
        let filters = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise: NoiseFilter::new(0.0, Vec::new()),
            confirmation: ConfirmationLevel::Off,
        };
        let mut run = |channels| {
            let config = PipelineConfig {
                raw_audio_input: true,
                target_sample_rate: 48_000,
                chunk_duration_secs: 3.0,
                channels,
                ..PipelineConfig::default()
            };
            process_and_infer_channels(
                &wav,
                &config,
                &mut registry,
                &[0],
                &filters,
                &mut species_filter,
                None,
                None,
                None,
                "",
            )
            .unwrap()
        };
        let mean = run(ChannelAnalysis::Mean);
        assert_eq!(mean.len(), 1);
        assert_eq!(mean[0].channel, None);
        assert!(
            mean[0].events.is_empty(),
            "the mean of antiphase stereo is silence"
        );
        assert!(
            mean[0]
                .channels
                .as_ref()
                .is_some_and(crate::audio::decode::ChannelProvenance::cancellation_suspected)
        );

        let each = run(ChannelAnalysis::Each);
        let tags: Vec<Option<u16>> = each.iter().map(|f| f.channel).collect();
        assert_eq!(tags, [Some(0), Some(1)]);
        for f in &each {
            assert!(
                !f.events.is_empty(),
                "channel {:?} was classified",
                f.channel
            );
            assert_eq!(f.tally.windows, 2, "each channel is its own six seconds");
        }

        let config = PipelineConfig {
            raw_audio_input: true,
            target_sample_rate: 48_000,
            chunk_duration_secs: 3.0,
            channels: ChannelAnalysis::Each,
            ..PipelineConfig::default()
        };
        assert!(matches!(
            process_and_infer_filtered(
                &wav,
                &config,
                &mut registry,
                &[0],
                &filters,
                &mut species_filter,
                None,
                None,
                None,
                "",
            ),
            Err(DaemonError::Config(_))
        ));
    }

    /// Review F18: a window no classifier answered costs that window, not the
    /// file. The two before it and the one after are kept, the failed one is
    /// counted and, on a station that can hear people, unassessed for speech;
    /// a file whose every window failed is still an error.
    #[test]
    fn windows_answered_around_a_failed_one_are_kept() {
        use crate::detection::types::Detection;
        let chunks: Vec<PreparedChunk> = (0..4).map(|_| chunk_of(10)).collect();
        let answered = |i: usize| ChunkPrediction {
            detections: vec![Detection {
                date: "2026-05-19".into(),
                time: "09:00:00".into(),
                scientific_name: format!("Species{i}"),
                common_name: format!("Bird {i}"),
                confidence: 0.9,
                start: 0.0,
                stop: 3.0,
                week: 20,
                file_name_extr: None,
                model_id: None,
                agreeing_models: None,
                model_scores: Vec::new(),
            }],
            ..ChunkPrediction::default()
        };
        let mut n = 0;
        let (out, failed) = infer_windows(&chunks, true, |_| {
            n += 1;
            if n == 3 {
                Err(DaemonError::Inference(
                    crate::inference::model::InferenceError::Shape("refused".into()),
                ))
            } else {
                Ok(answered(n))
            }
        })
        .expect("three windows answered");
        assert_eq!(failed, 1);
        assert_eq!(out.len(), 4);
        let names: Vec<&str> = out
            .iter()
            .flat_map(|p| p.detections.iter().map(|d| d.scientific_name.as_str()))
            .collect();
        assert_eq!(names, ["Species1", "Species2", "Species4"]);
        assert!(out[2].privacy_unassessed && !out[0].privacy_unassessed);
        let every_window_failed = infer_windows(&chunks[..2], false, |_| {
            Err(DaemonError::Inference(
                crate::inference::model::InferenceError::Shape("refused".into()),
            ))
        });
        assert!(
            every_window_failed.is_err(),
            "every window failed: the file is an error"
        );
        let mut m = 0;
        let (out, failed) = infer_windows(&chunks[..2], false, |_| {
            m += 1;
            if m == 1 {
                Ok(answered(1))
            } else {
                Err(DaemonError::Inference(
                    crate::inference::model::InferenceError::Shape("refused".into()),
                ))
            }
        })
        .unwrap();
        assert_eq!(failed, 1);
        assert!(
            !out[1].privacy_unassessed,
            "nothing could hear people to miss"
        );
    }

    /// Review F10: the tally says how much audio decoded, which the effort
    /// ledger credits instead of the header's claim; here a whole 3 s file,
    /// every window answered.
    #[test]
    fn a_files_tally_says_how_much_audio_decoded() {
        let file = analysed_with(
            &[],
            &SECOND_ONLY,
            None,
            crate::inference::species_filter::SpeciesFilterConfig::default(),
            crate::detection::noise::NoiseFilter::new(0.0, Vec::new()),
        );
        assert_eq!(file.tally.decoded_ms, 3000, "{:?}", file.tally);
        assert_eq!(file.tally.failed_windows, 0);
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

    // MMP-03: the occurrence filter and the operator's lists over every routed
    // classifier's species, not only the primary's.
    //
    // The station: the tiny V2.4 model as primary, naming eleven species no
    // geomodel knows ("Species0 bird" …), and the tiny V3.0 model as a second
    // classifier naming four real species and seven placeholders. The
    // geomodel is `tiny_meta_test.onnx`, whose scores are fixed whatever the
    // place: Pica pica 0.90, Parus major 0.10, Corvus corax 0.80, Turdus
    // merula 0.047, Erithacus rubecula 0.70, against a threshold of 0.5. Both
    // classifiers report every class (threshold 0, top 100), so what is
    // emitted is exactly what the filter admits.
    const SECOND_ONLY: [&str; 4] = [
        "Turdus merula",   // scored 0.047: the geomodel rejects it here
        "Corvus corax",    // scored 0.80: the geomodel admits it here
        "Rana temporaria", // not in the geomodel's vocabulary at all
        "Bufo bufo",       // not in the geomodel's vocabulary at all
    ];

    /// The species emitted for one three-second file, by scientific name.
    fn emitted(
        location: Option<(f64, f64)>,
        lists: crate::inference::species_filter::SpeciesFilterConfig,
    ) -> std::collections::HashSet<String> {
        emitted_with(
            &[],
            &SECOND_ONLY,
            location,
            lists,
            crate::detection::noise::NoiseFilter::new(0.0, Vec::new()),
        )
    }

    /// [`emitted`], with the first of the primary's names replaced by
    /// `primary`, the second classifier's first names by `second`, and a noise
    /// filter.
    fn emitted_with(
        primary: &[&str],
        second: &[&str],
        location: Option<(f64, f64)>,
        lists: crate::inference::species_filter::SpeciesFilterConfig,
        noise: crate::detection::noise::NoiseFilter,
    ) -> std::collections::HashSet<String> {
        analysed_with(primary, second, location, lists, noise)
            .events
            .into_iter()
            .filter(|e| e.out_of_range.is_none())
            .map(|e| e.detection.scientific_name)
            .collect()
    }

    /// The whole of what the daemon makes of the one file [`emitted_with`]
    /// stages: detections, out-of-range records and the discard tally.
    fn analysed_with(
        primary: &[&str],
        second: &[&str],
        location: Option<(f64, f64)>,
        lists: crate::inference::species_filter::SpeciesFilterConfig,
        noise: crate::detection::noise::NoiseFilter,
    ) -> super::super::FilteredFile {
        use crate::detection::corroboration::ConfirmationLevel;
        use crate::detection::privacy::PrivacyFilter;
        use crate::inference::labels::LabelSet;
        use crate::inference::model::ModelConfig;
        use crate::inference::registry::ModelSpec;
        const TINY_V24: &[u8] = include_bytes!("../../testdata/tiny_v24_test.onnx");
        const TINY_V30: &[u8] = include_bytes!("../../testdata/tiny_v30_test.onnx");
        const TINY_META: &[u8] = include_bytes!("../../testdata/tiny_meta_test.onnx");

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let primary_names: Vec<String> = primary
            .iter()
            .map(|n| format!("{n}_{n}"))
            .chain((primary.len()..11).map(|i| format!("Species{i} bird_Bird {i}")))
            .collect();
        let second_names: Vec<String> = second
            .iter()
            .map(|n| format!("{n}_{n}"))
            .chain((second.len()..11).map(|i| format!("Placeholder{i} sp_Placeholder {i}")))
            .collect();
        std::fs::write(dir.join("labels.txt"), primary_names.join("\n")).unwrap();
        std::fs::write(dir.join("labels_second.txt"), second_names.join("\n")).unwrap();
        std::fs::write(dir.join("v24.onnx"), TINY_V24).unwrap();
        std::fs::write(dir.join("v30.onnx"), TINY_V30).unwrap();
        std::fs::write(dir.join("meta.onnx"), TINY_META).unwrap();
        let spec = |id: &str, model: &str, labels: &str, rate| ModelSpec {
            id: id.to_owned(),
            model_path: dir.join(model),
            labels_path: dir.join(labels),
            threshold: None,
            sample_rate: rate,
            output_scale: None,
        };
        let mut registry = ClassifierRegistry::load(
            &[
                spec("birdnet", "v24.onnx", "labels.txt", None),
                spec("second", "v30.onnx", "labels_second.txt", Some(48_000)),
            ],
            &std::collections::HashMap::new(),
            &ModelConfig {
                confidence_threshold: 0.0,
                top_n: 100,
                ..ModelConfig::default()
            },
        )
        .expect("both tiny classifiers load");

        let meta = LabelSet::from_entries(
            [
                ("Pica pica", "Eurasian Magpie"),
                ("Parus major", "Great Tit"),
                ("Corvus corax", "Common Raven"),
                ("Turdus merula", "Eurasian Blackbird"),
                ("Erithacus rubecula", "European Robin"),
            ]
            .iter()
            .map(|(s, c)| ((*s).to_owned(), (*c).to_owned()))
            .collect(),
        );
        let mut species_filter = SpeciesFilter::load_with_vocabulary(
            &dir.join("meta.onnx"),
            Some(meta),
            registry.primary().model.labels(),
            &std::collections::HashMap::new(),
            crate::inference::species_filter::SpeciesFilterConfig {
                sf_thresh: 0.5,
                ..lists
            },
        )
        .expect("the tiny geomodel loads");

        let wav = dir.join("2026-05-19-birdnet-09:00:00.wav");
        let wav_spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, wav_spec).unwrap();
        let mut x: u32 = 1;
        for _ in 0..48_000 * 3 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            w.write_sample(((x >> 16) as i16) / 8).unwrap();
        }
        w.finalize().unwrap();

        let filters = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise,
            confirmation: ConfirmationLevel::Off,
        };
        let pipeline_config = PipelineConfig {
            raw_audio_input: true,
            target_sample_rate: 48_000,
            chunk_duration_secs: 3.0,
            ..PipelineConfig::default()
        };
        process_and_infer_filtered(
            &wav,
            &pipeline_config,
            &mut registry,
            &[0, 1],
            &filters,
            &mut species_filter,
            None,
            location.map(|l| l.0),
            location.map(|l| l.1),
            "",
        )
        .expect("the file is processed")
    }

    const BERLIN: Option<(f64, f64)> = Some((52.52, 13.405));

    /// Review F13: one file through the pipeline, the tiny V2.4 model the only
    /// classifier (its first class `Pica pica`), and a geomodel whose magpie
    /// score depends on the week alone: `sigmoid(4 x week - 38)`, 0.12 in week
    /// 9 (held back at 0.5), 0.88 in week 10, about 0 in week 1 and about 1 in
    /// week 48 (`tiny_meta_week_test.onnx`, the same graph as
    /// `tiny_meta_test.onnx` with one weight set). Returns each magpie window's
    /// `(start, week, admitted)`.
    fn magpie_weeks(name: &str, secs: usize, mode: OccurrenceWeek) -> Vec<(f32, u32, bool)> {
        use crate::detection::corroboration::ConfirmationLevel;
        use crate::detection::noise::NoiseFilter;
        use crate::detection::privacy::PrivacyFilter;
        use crate::inference::labels::LabelSet;
        use crate::inference::model::{BirdNetModel, ModelConfig};
        use crate::inference::species_filter::SpeciesFilterConfig;
        const TINY_V24: &[u8] = include_bytes!("../../testdata/tiny_v24_test.onnx");
        const WEEKLY_META: &[u8] = include_bytes!("../../testdata/tiny_meta_week_test.onnx");
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("v24.onnx"), TINY_V24).unwrap();
        std::fs::write(dir.join("meta.onnx"), WEEKLY_META).unwrap();
        let labels = LabelSet::from_entries(
            std::iter::once(("Pica pica".to_owned(), "Eurasian Magpie".to_owned()))
                .chain((1..11).map(|i| (format!("Species{i} bird"), format!("Bird {i}"))))
                .collect(),
        );
        let model = BirdNetModel::load(
            &dir.join("v24.onnx"),
            labels.clone(),
            ModelConfig {
                confidence_threshold: 0.0,
                top_n: 100,
                ..ModelConfig::default()
            },
        )
        .unwrap();
        let mut registry = ClassifierRegistry::single("birdnet", model);
        let meta = LabelSet::from_entries(
            [
                ("Pica pica", "Eurasian Magpie"),
                ("Parus major", "Great Tit"),
                ("Corvus corax", "Common Raven"),
                ("Turdus merula", "Eurasian Blackbird"),
                ("Erithacus rubecula", "European Robin"),
            ]
            .iter()
            .map(|(s, c)| ((*s).to_owned(), (*c).to_owned()))
            .collect(),
        );
        let mut species_filter = SpeciesFilter::load_with_vocabulary(
            &dir.join("meta.onnx"),
            Some(meta),
            &labels,
            &std::collections::HashMap::new(),
            SpeciesFilterConfig {
                sf_thresh: 0.5,
                ..SpeciesFilterConfig::default()
            },
        )
        .expect("the weekly geomodel loads");
        let wav = dir.join(name);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        let mut x: u32 = 1;
        for _ in 0..48_000 * secs {
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
            occurrence_week: mode,
            ..PipelineConfig::default()
        };
        let file = process_and_infer_filtered(
            &wav,
            &pipeline_config,
            &mut registry,
            &[0],
            &filters,
            &mut species_filter,
            None,
            BERLIN.map(|l| l.0),
            BERLIN.map(|l| l.1),
            "",
        )
        .expect("the file is processed");
        file.events
            .iter()
            .filter(|e| e.detection.scientific_name == "Pica pica")
            .map(|e| {
                (
                    e.detection.start,
                    e.detection.week,
                    e.out_of_range.is_none(),
                )
            })
            .collect()
    }

    /// Review F13: a 15 s recording starting 2026-03-07 23:59:55 crosses into
    /// 03-08, and so from week bin 9 (days 1-7 of March) into bin 10. In the
    /// per-window mode the windows at 6, 9 and 12 s are scored in bin 10, and
    /// the occurrence filter, asked about bin 10 for them, admits the magpie
    /// it holds back in bin 9.
    ///
    /// Counterpart: the default, BirdNET-Pi's one week per file, scores every
    /// window in the file's bin 9 and holds the magpie back in all five.
    #[test]
    fn a_window_past_midnight_is_scored_in_its_own_week_only_when_asked() {
        let name = "2026-03-07-birdnet-23:59:55.wav";
        let per_window = magpie_weeks(name, 15, OccurrenceWeek::PerWindow);
        assert_eq!(
            per_window,
            [
                (0.0, 9, false),
                (3.0, 9, false),
                (6.0, 10, true),
                (9.0, 10, true),
                (12.0, 10, true)
            ],
            "per-window mode: each window's own capture date"
        );
        let per_file = magpie_weeks(name, 15, OccurrenceWeek::default());
        assert_eq!(
            per_file,
            [
                (0.0, 9, false),
                (3.0, 9, false),
                (6.0, 9, false),
                (9.0, 9, false),
                (12.0, 9, false)
            ],
            "the default is one week per file"
        );
    }

    /// The same at the year's end: 2026-12-31 23:59:55 is bin 48, and the
    /// windows after midnight are 2027-01-01, bin 1, where the weekly geomodel
    /// holds the magpie back.
    #[test]
    fn a_window_past_new_year_is_scored_in_week_one_only_when_asked() {
        let name = "2026-12-31-birdnet-23:59:55.wav";
        let per_window: Vec<(u32, bool)> = magpie_weeks(name, 15, OccurrenceWeek::PerWindow)
            .into_iter()
            .map(|(_, w, a)| (w, a))
            .collect();
        assert_eq!(
            per_window,
            [(48, true), (48, true), (1, false), (1, false), (1, false)]
        );
        let per_file: Vec<(u32, bool)> = magpie_weeks(name, 15, OccurrenceWeek::PerFile)
            .into_iter()
            .map(|(_, w, a)| (w, a))
            .collect();
        assert_eq!(per_file, [(48, true); 5]);
    }

    /// Each distinct week of a file is asked about once, in the order it first
    /// appears.
    #[test]
    fn each_distinct_week_is_asked_once_in_order() {
        assert_eq!(distinct_in_order(&[9, 9, 10, 10, 10]), [9, 10]);
        assert_eq!(distinct_in_order(&[48, 1, 1]), [48, 1]);
        assert_eq!(distinct_in_order(&[20; 5]), [20]);
        assert!(distinct_in_order(&[]).is_empty());
    }

    fn no_lists() -> crate::inference::species_filter::SpeciesFilterConfig {
        crate::inference::species_filter::SpeciesFilterConfig::default()
    }

    fn primary_species(set: &std::collections::HashSet<String>) -> usize {
        set.iter().filter(|n| n.starts_with("Species")).count()
    }

    /// ACC-01: a species the geomodel scores under the threshold here leaves
    /// the daemon as an out-of-range record carrying that score, for the
    /// "Unlikely here" quarantine, and is counted. It used to vanish.
    /// Counterparts in the same run: an admitted species is an ordinary
    /// detection, and species the geomodel does not score are counted and
    /// sent nowhere (the maintainer's decision on MMP-03).
    #[test]
    fn an_out_of_range_species_is_sent_for_quarantine_with_its_score() {
        let file = analysed_with(
            &["Turdus merula", "Pica pica"],
            &[],
            BERLIN,
            no_lists(),
            crate::detection::noise::NoiseFilter::new(0.0, Vec::new()),
        );
        let named = |sci: &str| {
            file.events
                .iter()
                .filter(|e| e.detection.scientific_name == sci)
                .collect::<Vec<_>>()
        };
        let blackbird = named("Turdus merula");
        assert_eq!(blackbird.len(), 1, "{:?}", file.events);
        let p = blackbird[0].out_of_range.expect("sent as out of range");
        assert!((p - 0.047).abs() < 1e-3, "the score that rejected it: {p}");
        let magpie = named("Pica pica");
        assert_eq!(magpie.len(), 1);
        assert_eq!(
            magpie[0].out_of_range, None,
            "admitted, an ordinary detection"
        );
        assert_eq!(
            file.events.len(),
            2,
            "the unscored placeholders are sent nowhere: {:?}",
            file.events
        );
        // One window; the blackbird, nine unscored primary names and eleven
        // unscored second-classifier names.
        assert_eq!(file.tally.windows, 1);
        assert_eq!(
            file.tally.occurrence_detections,
            1 + 9 + 11,
            "{:?}",
            file.tally
        );
        assert_eq!(file.tally.species_list_detections, 0);
        assert_eq!(file.tally.date, "2026-05-19");
    }

    /// The counterpart on the operator's side: an excluded species is counted
    /// under the lists and is not sent for quarantine, whatever its score.
    #[test]
    fn an_excluded_species_is_counted_and_never_quarantined() {
        let file = analysed_with(
            &["Turdus merula", "Pica pica"],
            &[],
            BERLIN,
            crate::inference::species_filter::SpeciesFilterConfig {
                exclude_list: vec!["Turdus merula".to_owned(), "Pica pica".to_owned()],
                ..no_lists()
            },
            crate::detection::noise::NoiseFilter::new(0.0, Vec::new()),
        );
        assert!(
            file.events.is_empty(),
            "excluded species went somewhere: {:?}",
            file.events
        );
        assert_eq!(file.tally.species_list_detections, 2, "{:?}", file.tally);
        assert_eq!(file.tally.occurrence_detections, 9 + 11);
    }

    /// The audit's gate: a species only the second classifier knows, which
    /// the geomodel scores below the threshold here, is not emitted. The
    /// counterpart in the same run: one it scores above the threshold is.
    #[test]
    fn a_species_only_a_second_classifier_knows_is_range_filtered() {
        let got = emitted(BERLIN, no_lists());
        assert!(
            !got.contains("Turdus merula"),
            "the geomodel scored it 0.047 against a bar of 0.5: {got:?}"
        );
        assert!(
            got.contains("Corvus corax"),
            "scored 0.80, so admitted: {got:?}"
        );
    }

    /// The maintainer's decision of 2026-11-23: a second classifier's species
    /// the geomodel has no score for is dropped while the occurrence filter is
    /// running, as the primary's own unscored species are, and the whitelist
    /// lets it in.
    #[test]
    fn a_species_the_geomodel_cannot_score_is_dropped_unless_whitelisted() {
        let got = emitted(
            BERLIN,
            crate::inference::species_filter::SpeciesFilterConfig {
                whitelist: std::iter::once("Bufo bufo".to_owned()).collect(),
                ..no_lists()
            },
        );
        assert!(!got.contains("Rana temporaria"), "{got:?}");
        assert!(got.contains("Bufo bufo"), "whitelisted: {got:?}");
    }

    /// Without coordinates the geomodel cannot run, and every species of
    /// every classifier is a candidate: the precondition for the list tests
    /// below, which would otherwise pass on a station that emitted nothing.
    #[test]
    fn without_coordinates_every_classifier_s_species_is_a_candidate() {
        let got = emitted(None, no_lists());
        assert_eq!(primary_species(&got), 11, "{got:?}");
        for name in SECOND_ONLY {
            assert!(got.contains(name), "{name}: {got:?}");
        }
    }

    /// An include list binds on every classifier: one that names only a
    /// primary species leaves the second classifier nothing to report. It
    /// used to be ignored there, as a typo, because it matched nothing in that
    /// classifier's vocabulary.
    #[test]
    fn an_include_list_binds_on_a_second_classifier_that_lacks_its_species() {
        let got = emitted(
            None,
            crate::inference::species_filter::SpeciesFilterConfig {
                include_list: vec!["Species0 bird".to_owned()],
                ..no_lists()
            },
        );
        assert_eq!(
            got,
            std::collections::HashSet::from(["Species0 bird".to_owned()])
        );
    }

    /// MMP-24, the mirror: an include list naming only a second classifier's
    /// species binds on the primary too.
    #[test]
    fn an_include_list_binds_on_the_primary_that_lacks_its_species() {
        let got = emitted(
            None,
            crate::inference::species_filter::SpeciesFilterConfig {
                include_list: vec!["Corvus corax".to_owned()],
                ..no_lists()
            },
        );
        assert_eq!(
            got,
            std::collections::HashSet::from(["Corvus corax".to_owned()])
        );
    }

    /// The counterpart: an include list that matches nothing on the station is
    /// still read as a typo, and ignored everywhere.
    #[test]
    fn an_include_list_matching_no_classifier_is_still_ignored() {
        let got = emitted(
            None,
            crate::inference::species_filter::SpeciesFilterConfig {
                include_list: vec!["Turdus merulaa".to_owned()],
                ..no_lists()
            },
        );
        assert_eq!(primary_species(&got), 11, "{got:?}");
        assert!(got.contains("Rana temporaria"), "{got:?}");
    }

    // A label that names a sound rather than a taxon is never stored as a
    // species (MMP-04, ACC-18; the maintainer's decision of 2026-11-23,
    // revised to "never store"). V2.4's (`Engine`, `Power tools`), Perch's
    // (`Speech`, `Dog`) and the human classes of all three (`Human vocal`,
    // `Homo sapiens`). No location, so the range filter cannot be what drops
    // them.
    const SOUNDS: [&str; 6] = [
        "Engine",
        "Power tools",
        "Speech",
        "Dog",
        "Human vocal",
        "Homo sapiens",
    ];

    #[test]
    fn a_sound_class_is_never_stored_as_a_species() {
        use crate::detection::noise::NoiseFilter;
        let got = emitted_with(
            &SOUNDS[..3],
            &SOUNDS[3..],
            None,
            no_lists(),
            NoiseFilter::new(0.0, Vec::new()),
        );
        let stored: Vec<&str> = SOUNDS
            .iter()
            .copied()
            .filter(|s| got.contains(*s))
            .collect();
        assert!(stored.is_empty(), "stored as species: {stored:?}");
        // The counterpart in the same run: the species beside them are kept.
        assert!(got.contains("Species3 bird"), "{got:?}");
        assert!(got.contains("Placeholder3 sp"), "{got:?}");
    }

    /// The classes still drive the chunk filters: a watched `Dog` discards its
    /// chunk, which here is the whole file. Green before the change too; it
    /// holds the drop after the filters, where it does not blind them.
    #[test]
    fn a_watched_sound_class_still_discards_its_chunk() {
        use crate::detection::noise::NoiseFilter;
        let heard = emitted_with(
            &["Dog"],
            &[],
            None,
            no_lists(),
            NoiseFilter::new(0.0, Vec::new()),
        );
        assert!(heard.contains("Species1 bird"), "precondition: {heard:?}");
        let barked = emitted_with(
            &["Dog"],
            &[],
            None,
            no_lists(),
            NoiseFilter::new(0.01, vec!["Dog".to_owned()]),
        );
        assert!(barked.is_empty(), "{barked:?}");
    }

    // ── ACC-04: only what the station could store is ranked and cut ────

    /// The tiny V3.0 classifier alone, loaded at `station` as the daemon loads
    /// its primary, its 11 classes named `Species0 bird` to `Species10 bird`.
    /// It reads the first 11 values of its window as their scores.
    fn one_classifier(station: f32) -> ClassifierRegistry {
        use crate::inference::labels::LabelSet;
        use crate::inference::model::{BirdNetModel, ModelConfig};
        const TINY_V30: &[u8] = include_bytes!("../../testdata/tiny_v30_test.onnx");
        let labels = LabelSet::from_entries(
            (0..11)
                .map(|i| (format!("Species{i} bird"), format!("Bird {i}")))
                .collect(),
        );
        let model = BirdNetModel::load_from_bytes(
            TINY_V30,
            labels,
            ModelConfig {
                confidence_threshold: station,
                ..ModelConfig::default()
            },
        )
        .expect("the tiny V3.0 model loads");
        ClassifierRegistry::single("birdnet", model)
    }

    /// A V3.0 window starting at `start_secs` whose classes score `head`.
    fn scored(head: &[f32], start_secs: f32) -> PreparedChunk {
        let mut chunk = chunk_of(96_000);
        chunk.spectrogram.data.fill(0.0);
        chunk.spectrogram.data[..head.len()].copy_from_slice(head);
        chunk.start_secs = start_secs;
        chunk.end_secs = start_secs + 3.0;
        chunk
    }

    /// What the processor publishes for a station bar of `station` and the
    /// per-species `overrides`, applied as the inference loop applies it before
    /// every file: the floor is the lowest threshold in force.
    fn publish(registry: &mut ClassifierRegistry, station: f32, overrides: &[(&str, f32)]) {
        use crate::detection::daemon::{
            SpeciesBar, StorableBars, ThresholdFloor, apply_threshold_floor, loaded_thresholds,
        };
        let floor = overrides.iter().fold(station, |low, &(_, t)| low.min(t));
        let published = ThresholdFloor::new(floor);
        published.publish_bars(StorableBars {
            station,
            species: overrides
                .iter()
                .map(|&(name, t)| {
                    (
                        name.to_owned(),
                        SpeciesBar {
                            threshold: Some(f64::from(t)),
                            adjustment: None,
                        },
                    )
                })
                .collect(),
        });
        let base = loaded_thresholds(registry);
        apply_threshold_floor(registry, &base, Some(&published));
        assert!(
            (registry.primary().model.config().confidence_threshold - floor).abs() < f32::EPSILON,
            "precondition: the model runs at the floor"
        );
    }

    fn names(detections: &[crate::detection::types::Detection]) -> Vec<&str> {
        detections
            .iter()
            .map(|d| d.scientific_name.as_str())
            .collect()
    }

    /// ACC-04's first gate. One species lowered to 0.4 under a 0.7 station
    /// drops the model's cut to 0.4 for every class. Ten classes at 0.5, which
    /// the station drops (no override, under 0.7), then filled the ten top-N
    /// slots, and the one class it would store — 0.45, over its own 0.4 — was
    /// truncated away inside the model.
    #[test]
    fn a_storable_species_is_not_cut_by_classes_the_station_would_drop() {
        let mut registry = one_classifier(0.7);
        publish(&mut registry, 0.7, &[("Species10 bird", 0.4)]);
        let mut head = [0.5_f32; 11];
        head[10] = 0.45;
        let out = infer_chunk(&mut registry, &[0], &scored(&head, 0.0), 20).expect("answers");
        let got = names(&out.detections);
        assert!(
            got.contains(&"Species10 bird"),
            "the storable species was truncated away: {got:?}"
        );
        assert_eq!(got, ["Species10 bird"], "only what the station can store");
    }

    /// ACC-04's second gate. At `strict` a species must be carried by its
    /// neighbours; neighbours that carry it only under the bar it is stored at
    /// are not evidence, because the station drops them. They were counted,
    /// since the model emitted every class over the floor another species set.
    /// The counterparts, in the same run: a species carried over its bar in
    /// every window is kept, and so is one carried over its own lowered bar.
    #[test]
    fn neighbours_under_the_storable_bar_do_not_corroborate() {
        use crate::detection::corroboration::ConfirmationLevel;
        use crate::detection::noise::NoiseFilter;
        use crate::detection::privacy::PrivacyFilter;
        let mut registry = one_classifier(0.7);
        publish(&mut registry, 0.7, &[("Species10 bird", 0.4)]);
        // Species0: 0.5 / 0.8 / 0.5 — over 0.7 only in the middle window.
        // Species1: 0.8 in all three. Species10: 0.45 / 0.8 / 0.45, its bar 0.4.
        let window = |s0: f32, s10: f32| {
            let mut head = [0.0_f32; 11];
            head[0] = s0;
            head[1] = 0.8;
            head[10] = s10;
            head
        };
        let starts = [0.0_f32, 3.0, 6.0];
        let heads = [window(0.5, 0.45), window(0.8, 0.8), window(0.5, 0.45)];
        let predictions: Vec<ChunkPrediction> = starts
            .iter()
            .zip(&heads)
            .map(|(&start, head)| {
                infer_chunk(&mut registry, &[0], &scored(head, start), 20).expect("answers")
            })
            .collect();
        let filters = ChunkFilters {
            privacy: PrivacyFilter::new(0.0),
            noise: NoiseFilter::new(0.0, Vec::new()),
            confirmation: ConfirmationLevel::Strict,
        };
        let kept = filters.apply(&starts, &predictions);
        let kept: Vec<Vec<&str>> = kept.iter().map(|chunk| names(chunk)).collect();
        assert!(
            kept.iter().all(|chunk| !chunk.contains(&"Species0 bird")),
            "corroborated by neighbours the station drops: {kept:?}"
        );
        assert!(
            kept.iter().all(|chunk| chunk.contains(&"Species1 bird")),
            "counterpart: a species over its bar throughout is kept: {kept:?}"
        );
        assert!(
            kept.iter().all(|chunk| chunk.contains(&"Species10 bird")),
            "counterpart: neighbours over a lowered bar still count: {kept:?}"
        );
    }

    /// ACC-04's counterpart: with no per-species bar, the storable bar is the
    /// floor, and the cut is the one the model made before the bars existed —
    /// the same candidates in the same order, top-N included (BirdNET-Pi's
    /// top-10-then-threshold equals threshold-then-top-10 when one bar
    /// applies). Two windows: eleven classes at or over 0.7, ties and the
    /// boundary included, so the top-N cut binds; and one with half under.
    #[test]
    fn with_no_species_bar_the_cut_is_unchanged() {
        let heads: [[f32; 11]; 2] = [
            [0.9, 0.75, 0.75, 0.71, 0.7, 0.8, 0.95, 0.8, 0.72, 0.85, 0.7],
            [0.1, 0.69, 0.7, 0.99, 0.3, 0.71, 0.0, 0.7, 0.5, 0.8, 0.65],
        ];
        let cut = |registry: &mut ClassifierRegistry, head: &[f32]| -> Vec<(String, u32)> {
            infer_chunk(registry, &[0], &scored(head, 0.0), 20)
                .expect("answers")
                .detections
                .into_iter()
                .map(|d| (d.scientific_name, d.confidence.to_bits()))
                .collect()
        };
        let mut before = one_classifier(0.7);
        let mut after = one_classifier(0.7);
        publish(&mut after, 0.7, &[]);
        for head in &heads {
            assert_eq!(cut(&mut after, head), cut(&mut before, head), "{head:?}");
        }
        assert_eq!(cut(&mut after, &heads[0]).len(), 10, "the top-N cut binds");
    }
}
