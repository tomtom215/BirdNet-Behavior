//! Per-file processing: decode → pipeline → inference → (optional) filtering.

use std::path::Path;
use std::time::Instant;

use crate::detection::ChunkFilters;
use crate::detection::pipeline::{self, PipelineConfig, PreparedChunk};
use crate::detection::types::ChunkPrediction;
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

/// Process a single audio file and run inference.
///
/// Returns all detections found in the file, or an empty vec if
/// nothing meets the confidence threshold.
///
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
    let mut verdicts = Vec::with_capacity(route.len());
    let mut human_score = 0.0_f32;

    for &idx in route {
        let Some(registered) = registry.model_mut(idx) else {
            // Unreachable: routes are resolved against the loaded models at
            // startup and refused if they name one that does not exist.
            continue;
        };
        let model_id = registered.id.clone();
        let prediction = registered.model.predict_chunk(
            &chunk.spectrogram.data,
            &chunk.recording.date,
            &chunk.recording.time,
            chunk.start_secs,
            chunk.end_secs,
            week,
        )?;
        human_score = human_score.max(prediction.human_score);
        verdicts.push(crate::detection::merge::ModelVerdict {
            model_id,
            detections: prediction.detections,
        });
    }

    Ok(ChunkPrediction {
        detections: crate::detection::merge::merge_verdicts(&verdicts),
        human_score,
    })
}

/// `correlation_id`, if non-empty, is stamped on every event emitted for
/// this file and surfaced in every log line — see [`DetectionEvent::correlation_id`].
///
/// # Errors
///
/// Returns `DaemonError` if any stage fails.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_arguments
)]
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
    let allowed_species =
        species_filter.filter_species(lat.zip(lon), week, registry.primary().model.labels())?;
    if let Some(observer) = filter_observer {
        observer.report(
            species_filter.has_model(),
            Some(allowed_species.len() as u64),
        );
    }

    // Collect events, applying species filter
    let mut events = Vec::new();
    let total_ms = start.elapsed().as_millis() as u64;

    for (chunk, detections) in chunks.iter().zip(filtered_predictions.iter()) {
        for detection in detections {
            if !allowed_species.contains(&detection.scientific_name) {
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
}
