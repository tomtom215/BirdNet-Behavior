//! Per-file processing: decode → pipeline → inference → (optional) filtering.

use std::path::Path;
use std::time::Instant;

use crate::detection::pipeline::{self, PipelineConfig, PreparedChunk};
use crate::detection::privacy::PrivacyFilter;
use crate::detection::types::Detection;
use crate::inference::model::BirdNetModel;
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

/// Process a single audio file and run inference.
///
/// Returns all detections found in the file, or an empty vec if
/// nothing meets the confidence threshold.
///
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
    model: &mut BirdNetModel,
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

        let detections = model.predict(
            &chunk.spectrogram.data,
            &chunk.recording.date,
            &chunk.recording.time,
            chunk.start_secs,
            chunk.end_secs,
            0, // week will be computed by caller
        )?;

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
    tracing::info!(
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
/// After running inference, applies the privacy filter (suppressing chunks
/// with human voice) and the species occurrence filter (only keeping species
/// that are likely present at the given location and time of year).
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
    model: &mut BirdNetModel,
    privacy_filter: &PrivacyFilter,
    species_filter: &mut SpeciesFilter,
    filter_observer: Option<&crate::detection::daemon::SpeciesFilterObserver>,
    lat: Option<f64>,
    lon: Option<f64>,
    week: u32,
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

    // Run inference on all chunks first to collect raw predictions
    let mut all_predictions: Vec<Vec<Detection>> = Vec::with_capacity(chunks.len());

    for chunk in &chunks {
        let detections = model.predict(
            &chunk.spectrogram.data,
            &chunk.recording.date,
            &chunk.recording.time,
            chunk.start_secs,
            chunk.end_secs,
            week,
        )?;
        all_predictions.push(detections);
    }

    // Apply privacy filter
    let filtered_predictions = privacy_filter.filter_predictions(&all_predictions);

    // Build the allowed species set from the species filter.
    //
    // Always consulted, even with no coordinates: only the metadata model needs
    // to know where the station is, and `filter_species` skips just that stage
    // when `location` is `None`. The operator's include/exclude lists are an
    // explicit instruction and still apply — gating the whole filter on
    // coordinates, as this used to, meant a station that never set a latitude
    // kept recording every species its operator had asked to suppress.
    let allowed_species = species_filter.filter_species(lat.zip(lon), week, model.labels())?;
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
    tracing::info!(
        correlation_id,
        file = %path.display(),
        detections = events.len(),
        total_ms = total.as_millis(),
        privacy = privacy_filter.is_enabled(),
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
