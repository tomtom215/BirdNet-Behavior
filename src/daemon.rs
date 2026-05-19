//! Detection daemon startup and event processing bridge.
//!
//! Starts the background detection daemon and bridges its `std::mpsc` event
//! channel to WebSocket broadcasts and external integrations. Now also supports
//! heartbeat pings, notification templates, species filters, and trigger modes.
//!
//! ## Pure helpers
//!
//! Every non-trivial decision or struct-literal in this file is split into a
//! tiny `pub(crate)`- or module-private helper at the top of the file so each
//! decision boundary is observable in a unit test rather than only via an
//! integration harness. This is the same pattern PR #50 used to bring
//! `crates/birdnet-db/src/sqlite/queries/detections.rs` to a `missed = 0`
//! cargo-mutants score (`parse_search_term` / `strip_not_prefix`): rewrite
//! the obstacle out of existence rather than lifting a threshold.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use birdnet_core::audio::extraction::{AudioFormat, ExtractionConfig, Extractor};
use birdnet_core::detection::pipeline::PipelineConfig;
use birdnet_core::inference::model::ModelConfig;
use birdnet_core::inference::species_filter::SpeciesFilterConfig;
use birdnet_integrations::notification::{
    NotificationContext, NotificationFilter, NotificationTemplate,
};

use crate::cli::Cli;
use crate::integrations::{AppriseHandle, EmailHandle, HeartbeatHandle, MqttHandle};

// ── Pure builder helpers ─────────────────────────────────────────────────
//
// Each of the four `build_*_config` helpers below encapsulates a struct
// literal that was previously inline in `start_detection_daemon`. The
// inline-literal form was responsible for ~10 cargo-mutants survivors of
// the "delete field" family: every field's source is now individually
// testable through the dedicated unit tests in the `#[cfg(test)]` mod
// below, and the orchestrator's only job is to thread arguments
// through.

/// CLI-default vs. config-file precedence resolver for `f32` flags.
///
/// **Why this exists.** A common pattern in `start_detection_daemon` was:
///
/// ```text
/// let v = if (cli.flag - DEFAULT).abs() < f32::EPSILON {
///     config.and_then(...).unwrap_or(cli.flag)
/// } else {
///     cli.flag
/// };
/// ```
///
/// The `(cli - DEFAULT).abs() < f32::EPSILON` boundary check produced a
/// family of cargo-mutants (`<` → `==` / `>` / `<=`, `-` → `+` / `/`) that
/// no unit test could observe without an exact-epsilon-difference value
/// the f32 representation doesn't actually produce in practice.
///
/// Replacing the dance with bit-exact equality on the CLI default
/// eliminates every one of those mutants. clap parses the documented
/// default string ("0.03", "0.0") into the same f32 bit pattern the Rust
/// literal compares to, so `==` is the contract — `EPSILON`-tolerance is
/// a workaround for a problem clap doesn't have.
///
/// Returns the resolved value: when the operator did *not* override the
/// CLI default, the config value (if present) wins; otherwise the CLI
/// value (the operator's override) wins.
#[must_use]
#[allow(clippy::float_cmp)] // see docs above — `==` is the contract here.
pub fn resolve_f32_with_default(
    cli_value: f32,
    cli_default: f32,
    config_value: Option<f32>,
) -> f32 {
    if cli_value == cli_default {
        config_value.unwrap_or(cli_value)
    } else {
        cli_value
    }
}

/// Build the [`PipelineConfig`] used by the daemon's audio pipeline.
///
/// The pipeline's only operator-tunable knobs at this layer are the
/// recording watch directory and the chunk overlap (in seconds);
/// everything else comes from the model's own contract (sample rate,
/// raw-vs-mel input) and is auto-adjusted by `run_daemon` based on the
/// loaded model. Tests pin each field individually so a future field
/// addition can't silently revert to `Default::default()`.
#[must_use]
pub fn build_pipeline_config(watch_dir: PathBuf, chunk_overlap_secs: f32) -> PipelineConfig {
    PipelineConfig {
        watch_dir,
        chunk_overlap_secs,
        ..PipelineConfig::default()
    }
}

/// Build the [`ModelConfig`] used by ML inference.
///
/// Pins `sensitivity` and `confidence_threshold` from CLI/config; every
/// other knob (`top_n`, `num_threads`) comes from the model's defaults.
#[must_use]
pub fn build_model_config(sensitivity: f32, confidence_threshold: f32) -> ModelConfig {
    ModelConfig {
        sensitivity,
        confidence_threshold,
        ..ModelConfig::default()
    }
}

/// Build the [`SpeciesFilterConfig`] used by the metadata-model species filter.
#[must_use]
pub fn build_species_filter_config(sf_thresh: f32) -> SpeciesFilterConfig {
    SpeciesFilterConfig {
        sf_thresh,
        ..SpeciesFilterConfig::default()
    }
}

/// Derive the audio-clip extraction output directory from a watch dir.
///
/// Per BirdNET-Pi convention: extracted clips land in a sibling
/// `Extracted/` directory next to the recordings watch dir. If the
/// watch dir has no parent (e.g. `/` or a relative bare filename), we
/// fall back to the well-known `BirdSongs/Extracted` path so the
/// extractor always has *somewhere* to write.
#[must_use]
pub fn extraction_output_dir(watch_dir: &Path) -> PathBuf {
    watch_dir.parent().map_or_else(
        || PathBuf::from("BirdSongs/Extracted"),
        |p| p.join("Extracted"),
    )
}

/// Build the [`ExtractionConfig`] for the detection-clip extractor.
///
/// `recording_length` is the CLI's `--segment-duration` (an integer
/// seconds value) cast to `f32`; the cast cannot lose precision in the
/// practical range (1–3600 s).
#[must_use]
pub fn build_extraction_config(cli: &Cli, watch_dir: &Path) -> ExtractionConfig {
    ExtractionConfig {
        target_format: AudioFormat::parse(&cli.audio_format),
        audio_format: cli.audio_format.clone(),
        output_dir: extraction_output_dir(watch_dir),
        recording_length: f32::from(u16::try_from(cli.segment_duration).unwrap_or(u16::MAX)),
        freq_shift_hz: cli.freq_shift_hz,
        ..ExtractionConfig::default()
    }
}

/// Resolve the three "must be configured for the daemon to run" paths.
///
/// Returns `Some((model, labels, watch_dir))` when all three resolve from
/// CLI args or config file; `None` otherwise (the daemon won't start in
/// that case). Extracting this from `start_detection_daemon` makes the
/// triple-Option dance unit-testable without standing up the rest of
/// the runtime (`AppState`, broadcast channel, integration handles).
#[must_use]
pub fn resolve_required_paths(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let model = cli
        .model
        .clone()
        .or_else(|| config.and_then(|c| c.get("MODEL_PATH").map(PathBuf::from)))?;
    let labels = cli
        .labels
        .clone()
        .or_else(|| config.and_then(|c| c.get("LABELS_PATH").map(PathBuf::from)))?;
    let watch_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config.and_then(|c| c.get("RECS_DIR").map(PathBuf::from)))?;
    Some((model, labels, watch_dir))
}

/// "Should we log the loaded per-species threshold count, and what is it?"
///
/// Returns `Some(count)` when there's at least one per-species threshold
/// configured; `None` when the map is empty. Extracted from
/// `start_detection_daemon` because the inline `if !thresholds.is_empty()`
/// produced an unattackable `delete !` cargo-mutants survivor (the only
/// observable side effect is a log line, which the test suite doesn't
/// capture). The helper returns a value tests can assert against
/// directly.
#[must_use]
pub fn species_thresholds_log_count(thresholds: &HashMap<String, f64>) -> Option<usize> {
    if thresholds.is_empty() {
        None
    } else {
        Some(thresholds.len())
    }
}

/// Truncating cast of a probability in `[0, 1]` to a 0–100 percentage.
///
/// Matches the historical behaviour of the inline `(confidence * 100.0)
/// as u32` in the notification-context builder. Extracted so the
/// `*` arithmetic mutant has a unit-testable surface.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn confidence_pct_trunc(confidence: f32) -> u32 {
    (confidence * 100.0) as u32
}

/// Rounding cast of a probability in `[0, 1]` to a 0–100 percentage.
///
/// Matches the historical behaviour of the inline `(confidence *
/// 100.0).round() as u32` in the MQTT payload builder. Extracted for
/// the same reason as [`confidence_pct_trunc`].
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn confidence_pct_round(confidence: f32) -> u32 {
    (confidence * 100.0).round() as u32
}

/// Convert daemon-reported per-event latency (ms) to seconds.
///
/// Extracted so the `/ 1000.0` arithmetic mutant has a unit-testable
/// surface independent of the rest of `event_processor`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn latency_ms_to_seconds(latency_ms: u64) -> f64 {
    (latency_ms as f64) / 1000.0
}

/// "Is the row we just inserted the only one for this species today?"
///
/// Used to power the rare-species celebration in the dashboard. Takes the
/// count returned by `detection_count_for_species_date` *after* the
/// current row's insert; `<= 1` covers both "exactly the row we just
/// inserted" and the defensive `0` case where the query failed and the
/// caller fell back to a sentinel. Extracted from `event_processor`
/// because the inline `<= 1` had a `<=` → `>` mutant with no covering
/// test.
#[must_use]
pub const fn is_first_detection_today(count_after_insert: i64) -> bool {
    count_after_insert <= 1
}

/// Combine the "Suppress alert-rule fired?" gate with the trigger-mode
/// filter into a single notification-eligibility verdict.
///
/// Returns `true` only when no Suppress rule matched *and* the trigger
/// filter says notify. Extracted from `event_processor` to make the
/// `!rule_suppressed && filter_says_notify` chain observable in a
/// unit test — the inline form produced `&&` → `||` and `delete !`
/// cargo-mutants that no test could catch without standing up the
/// integration stack.
#[must_use]
pub const fn passes_filter(rule_suppressed: bool, filter_says_notify: bool) -> bool {
    !rule_suppressed && filter_says_notify
}

/// Combine the upstream `passes_filter` verdict with an integration's
/// own send-policy verdict into a final dispatch decision.
///
/// Apprise (and historically other integrations) has its own
/// per-species / confidence-threshold gate that fires *after* the
/// global filter. Pulled into a pure helper for the same reason as
/// [`passes_filter`].
#[must_use]
pub const fn should_dispatch_notification(
    passes_filter: bool,
    integration_says_notify: bool,
) -> bool {
    passes_filter && integration_says_notify
}

/// What to do with a detection after the threshold gates run.
///
/// Extracted from the inline gate logic in [`event_processor`] so the
/// dispatch decision is unit-testable without spinning up a database,
/// broadcast channel, or notification stack. The actual side effects
/// (DB insert, quarantine row, audio extraction, broadcasts) still live
/// in the processor — this enum just pins the decision boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DispositionDecision {
    /// Detection passes all gates — persist, extract clip, broadcast.
    Accept,
    /// Below an operator-set per-species threshold — quarantine for review.
    Quarantine {
        /// The threshold that gated this detection, for the quarantine row.
        threshold: f64,
    },
    /// Below the global threshold and no per-species override — drop silently.
    DropBelowGlobal,
}

/// Pure helper: decide what to do with a detection based on its confidence
/// and the configured thresholds.
///
/// * A per-species threshold (if present) wins over the global threshold
///   and triggers quarantine (not silent drop) when missed — this preserves
///   the detection for manual review.
/// * Without a per-species override, the global confidence threshold is
///   the gate. Detections below it are dropped silently because the model
///   already applied the same gate; this is a belt-and-braces check.
///
/// Comparisons are done in `f64` because per-species thresholds come from
/// SQLite REAL columns (f64) and we don't want a single-precision rounding
/// step to flip a `==`-on-boundary case.
fn decide_disposition(
    confidence: f32,
    sci_name: &str,
    per_species_thresholds: &HashMap<String, f64>,
    global_confidence: f32,
) -> DispositionDecision {
    if let Some(&threshold) = per_species_thresholds.get(sci_name) {
        if f64::from(confidence) < threshold {
            return DispositionDecision::Quarantine { threshold };
        }
        return DispositionDecision::Accept;
    }
    if confidence < global_confidence {
        return DispositionDecision::DropBelowGlobal;
    }
    DispositionDecision::Accept
}

/// Derive a stable audio-source label from a recording's filename.
///
/// Recording filenames follow `YYYY-MM-DD-birdnet[-RTSP_ID]-HH:MM:SS.wav`.
/// The optional `RTSP_ID` segment (`RTSP_1`, `RTSP_2`, …) names the
/// per-stream supervisor that produced the file; its absence means the
/// file came from the local microphone (ALSA / PulseAudio / PipeWire).
/// We collapse all microphone sources to a single `local` label because
/// the supervisor doesn't currently expose finer-grained per-mic IDs.
///
/// Used to populate the `birdnet_audio_source_up{source}` gauge as a
/// best-effort liveness signal. A proper supervisor → metrics path is
/// the right long-term fix, but a per-event freshness gauge is already
/// useful: stations with one source going dark while another stays up
/// show that immediately in Prometheus.
#[must_use]
fn derive_source_label(source_file: &std::path::Path) -> String {
    let name = source_file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    birdnet_core::detection::types::RecordingFile::parse(name)
        .and_then(|rf| rf.rtsp_id)
        .unwrap_or_else(|| "local".to_owned())
}

/// Start the detection daemon in a background thread.
///
/// Returns the daemon handle, or `None` if the model/labels are not configured.
///
/// The body is almost entirely thread-the-arguments-through-builders glue;
/// every non-trivial decision lives in a pure helper at the top of this
/// file with dedicated unit-test coverage. See the module docs for the
/// rationale.
#[allow(clippy::too_many_arguments)]
pub fn start_detection_daemon(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: birdnet_web::state::AppState,
    broadcast: birdnet_web::routes::websocket::DetectionBroadcast,
    apprise: Option<AppriseHandle>,
    birdweather: Option<birdnet_integrations::birdweather::Client>,
    email: Option<EmailHandle>,
    heartbeat: Option<HeartbeatHandle>,
    mqtt: Option<MqttHandle>,
    notification_filter: NotificationFilter,
    notification_template: NotificationTemplate,
) -> Option<birdnet_core::detection::daemon::DaemonHandle> {
    let Some((model_path, labels_path, watch_dir)) = resolve_required_paths(cli, config) else {
        tracing::info!("detection daemon not started: model, labels, or watch_dir not configured");
        tracing::info!(
            "use --model, --labels, --watch-dir flags or set MODEL_PATH, LABELS_PATH, RECS_DIR in config"
        );
        return None;
    };

    let sensitivity = config
        .and_then(|c| c.get_parsed::<f32>("SENSITIVITY").ok())
        .unwrap_or(1.0);

    let confidence = config
        .and_then(|c| c.get_parsed::<f32>("CONFIDENCE").ok())
        .unwrap_or(0.25);

    let metadata_model_path = cli
        .metadata_model
        .clone()
        .or_else(|| config.and_then(|c| c.get("METADATA_MODEL_PATH").map(PathBuf::from)));

    let sf_thresh = resolve_f32_with_default(
        cli.sf_thresh,
        0.03,
        config.and_then(|c| c.get_parsed::<f32>("SF_THRESH").ok()),
    );

    let privacy_threshold = resolve_f32_with_default(
        cli.privacy_threshold,
        0.0,
        config.and_then(|c| c.get_parsed::<f32>("PRIVACY_THRESHOLD").ok()),
    );

    let overlap = resolve_f32_with_default(
        cli.overlap,
        0.0,
        config.and_then(|c| c.get_parsed::<f32>("OVERLAP").ok()),
    );

    let species_thresholds = state
        .with_db(|conn| birdnet_db::sqlite::get_species_threshold_map(conn).unwrap_or_default());

    if let Some(count) = species_thresholds_log_count(&species_thresholds) {
        tracing::info!(count, "loaded per-species confidence thresholds");
    }

    let daemon_config = birdnet_core::detection::daemon::DaemonConfig {
        watch_dir: watch_dir.clone(),
        model_path,
        labels_path,
        pipeline: build_pipeline_config(watch_dir, overlap),
        model: build_model_config(sensitivity, confidence),
        process_existing: cli.process_existing,
        metadata_model_path,
        species_filter: build_species_filter_config(sf_thresh),
        privacy_threshold,
        latitude: cli.latitude,
        longitude: cli.longitude,
        species_thresholds,
    };

    let (event_tx, event_rx) = mpsc::channel();

    let thresholds_for_processor = daemon_config.species_thresholds.clone();
    let global_confidence = confidence;

    let extractor = Extractor::new(build_extraction_config(cli, &daemon_config.watch_dir));

    match birdnet_core::detection::daemon::run_daemon(&daemon_config, event_tx) {
        Ok(handle) => {
            tracing::info!("detection daemon started");
            let rt_handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                event_processor(
                    event_rx,
                    state,
                    broadcast,
                    apprise,
                    birdweather,
                    email,
                    heartbeat,
                    mqtt,
                    notification_filter,
                    notification_template,
                    rt_handle,
                    thresholds_for_processor,
                    global_confidence,
                    extractor,
                );
            });
            Some(handle)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to start detection daemon");
            None
        }
    }
}

/// Bridge detection events from the daemon to database inserts and WebSocket broadcasts.
#[allow(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
fn event_processor(
    event_rx: mpsc::Receiver<birdnet_core::detection::daemon::DetectionEvent>,
    state: birdnet_web::state::AppState,
    broadcast: birdnet_web::routes::websocket::DetectionBroadcast,
    apprise: Option<AppriseHandle>,
    birdweather: Option<birdnet_integrations::birdweather::Client>,
    email: Option<EmailHandle>,
    heartbeat: Option<HeartbeatHandle>,
    mqtt: Option<MqttHandle>,
    notification_filter: NotificationFilter,
    notification_template: NotificationTemplate,
    rt_handle: tokio::runtime::Handle,
    species_thresholds: std::collections::HashMap<String, f64>,
    global_confidence: f32,
    extractor: Extractor,
) {
    tracing::debug!("event processor started");

    loop {
        let Ok(event) = event_rx.recv() else {
            tracing::info!("event channel closed, stopping event processor");
            break;
        };

        let detection = &event.detection;
        let correlation_id = event.correlation_id.as_str();

        // Apply per-species confidence threshold.
        // Detections that pass the global threshold but fail a stricter
        // per-species threshold are quarantined for manual review rather
        // than silently dropped.
        match decide_disposition(
            detection.confidence,
            &detection.scientific_name,
            &species_thresholds,
            global_confidence,
        ) {
            DispositionDecision::Quarantine { threshold } => {
                tracing::debug!(
                    correlation_id,
                    species = %detection.scientific_name,
                    confidence = detection.confidence,
                    threshold,
                    "detection below per-species threshold — quarantining for review"
                );
                let week_str = detection.week.to_string();
                let file_str = event.source_file.to_string_lossy();
                let q_record = birdnet_db::sqlite::QuarantineRecord {
                    date: &detection.date,
                    time: &detection.time,
                    sci_name: &detection.scientific_name,
                    com_name: &detection.common_name,
                    confidence: f64::from(detection.confidence),
                    sf_probability: None,
                    reason: birdnet_db::sqlite::QuarantineReason::LowConfidence,
                    file_name: if file_str.is_empty() {
                        None
                    } else {
                        Some(file_str.as_ref())
                    },
                    lat: None,
                    lon: None,
                    week: week_str.parse::<i32>().ok(),
                };
                if let Err(e) =
                    state.with_db(|conn| birdnet_db::sqlite::insert_quarantine(conn, &q_record))
                {
                    tracing::warn!(
                        correlation_id,
                        error = %e,
                        species = %detection.scientific_name,
                        "failed to quarantine detection"
                    );
                }
                continue;
            }
            DispositionDecision::DropBelowGlobal => continue,
            DispositionDecision::Accept => {}
        }

        // Insert into SQLite. Numeric columns receive Option<f64> /
        // Option<i64> so missing values become SQLite NULLs (the schema
        // declares Lat/Lon/Cutoff/Sens/Overlap as REAL and Week as
        // INTEGER). Previously the daemon passed empty strings here,
        // which SQLite silently stored as TEXT and every subsequent
        // typed read returned "Invalid column type Text at index N".
        let file_str = event.source_file.to_string_lossy();
        let record = birdnet_db::sqlite::DetectionRecord {
            date: &detection.date,
            time: &detection.time,
            sci_name: &detection.scientific_name,
            com_name: &detection.common_name,
            confidence: f64::from(detection.confidence),
            lat: None,
            lon: None,
            cutoff: None,
            week: Some(i64::from(detection.week)),
            sensitivity: None,
            overlap: None,
            file_name: &file_str,
            // Without this, every chunk of one recording shares the same
            // UNIQUE key (Date, Time, Sci_Name, File_Name) and only the
            // first chunk's detection is kept. See migration 11.
            chunk_offset_secs: Some(f64::from(detection.start)),
            // The correlation_id is the per-file id stamped on every
            // event; persisting it closes the log→row round trip so an
            // admin clicking a suspicious detection in the web UI can
            // grep the journal for the exact decode/infer/notify
            // slice that produced it. Empty string means "no id"
            // (forward-compat with non-daemon write paths) — store as
            // NULL so the DB doesn't try to index empty strings.
            correlation_id: if correlation_id.is_empty() {
                None
            } else {
                Some(correlation_id)
            },
        };

        let metrics = state.metrics();
        // Liveness signal: a detection here proves the source produced a
        // file the watcher picked up. Stamp the audio-source label
        // accordingly. We parse the source label from the filename
        // because the source supervisor doesn't currently feed liveness
        // updates upstream; the filename's RTSP prefix is the only
        // per-event tag the daemon sees.
        let source_label = derive_source_label(&event.source_file);
        metrics.set_source_up(&source_label, true);

        let db_start = std::time::Instant::now();
        let insert_result =
            state.with_db(|conn| birdnet_db::sqlite::insert_detection(conn, &record));
        metrics.observe_db_write_seconds(db_start.elapsed().as_secs_f64());
        if let Err(e) = insert_result {
            tracing::warn!(
                correlation_id,
                error = %e,
                "failed to insert detection into database"
            );
        } else {
            metrics.inc_detection(&detection.scientific_name, detection.start);
        }
        // event.latency_ms covers decode + inference; surface as a histogram
        // so the dashboard can flag rising p95s before they catch the eye.
        metrics.observe_inference_seconds(latency_ms_to_seconds(event.latency_ms));

        // Extract audio clip to disk.
        match extractor.extract_detection(&event.source_file, detection) {
            Ok(path) => tracing::debug!(
                species = %detection.common_name,
                path = %path.display(),
                "audio clip extracted"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                species = %detection.common_name,
                "audio clip extraction failed"
            ),
        }

        // Also insert into DuckDB analytics (if enabled).
        #[cfg(feature = "analytics")]
        if state.has_analytics() {
            let insert_result = state.with_analytics(|adb| {
                adb.insert_detection(
                    &detection.date,
                    &detection.time,
                    &detection.scientific_name,
                    &detection.common_name,
                    f64::from(detection.confidence),
                    &file_str,
                )
            });
            if let Some(Err(e)) = insert_result {
                tracing::warn!(error = %e, "failed to insert detection into DuckDB");
            }
        }

        // Evaluate alert rules (loaded fresh for each detection to reflect UI changes).
        let rule_suppressed = state.with_db(|conn| {
            let rules = birdnet_db::alert_rules::list_rules(conn).unwrap_or_default();
            let matched = birdnet_db::alert_rules::evaluate_rules(
                &rules,
                &detection.common_name,
                f64::from(detection.confidence),
                &detection.time,
            );

            let mut suppressed = false;
            for rule in matched {
                match &rule.action {
                    birdnet_db::alert_rules::AlertAction::Log => {
                        tracing::info!(
                            rule = %rule.name,
                            species = %detection.common_name,
                            sci_name = %detection.scientific_name,
                            confidence = detection.confidence,
                            date = %detection.date,
                            time = %detection.time,
                            "alert rule matched (log action)"
                        );
                    }
                    birdnet_db::alert_rules::AlertAction::Suppress => {
                        tracing::debug!(
                            rule = %rule.name,
                            species = %detection.common_name,
                            "alert rule suppressing notifications"
                        );
                        suppressed = true;
                    }
                    birdnet_db::alert_rules::AlertAction::Webhook {
                        url,
                        method,
                        body_template,
                    } => {
                        let body = body_template.as_deref().map(|tmpl| {
                            birdnet_db::alert_rules::render_webhook_body(
                                tmpl,
                                &detection.common_name,
                                &detection.scientific_name,
                                f64::from(detection.confidence),
                                &detection.date,
                                &detection.time,
                            )
                        });
                        let url = url.clone();
                        let method = method.clone();
                        let rule_name = rule.name.clone();
                        rt_handle.spawn(async move {
                            match dispatch_webhook(&url, &method, body.as_deref()).await {
                                Ok(status) => tracing::debug!(
                                    rule = %rule_name,
                                    url,
                                    status,
                                    "webhook dispatched"
                                ),
                                Err(e) => tracing::warn!(
                                    rule = %rule_name,
                                    url,
                                    error = %e,
                                    "webhook dispatch failed"
                                ),
                            }
                        });
                    }
                }
            }
            suppressed
        });

        // Check if this is the first detection of this species today
        // (to power the rare-species celebration in the dashboard).
        let is_new_today = state.with_db(|conn| {
            let today_count = birdnet_db::sqlite::detection_count_for_species_date(
                conn,
                &detection.date,
                &detection.scientific_name,
            )
            .unwrap_or(1);
            is_first_detection_today(today_count)
        });

        // Broadcast to WebSocket clients.
        let ws_event = birdnet_web::routes::websocket::WsDetectionEvent {
            event: "detection",
            common_name: detection.common_name.clone(),
            scientific_name: detection.scientific_name.clone(),
            confidence: detection.confidence,
            date: detection.date.clone(),
            time: detection.time.clone(),
            start: detection.start,
            stop: detection.stop,
            is_new_today,
        };
        broadcast.send(&ws_event);

        // Build notification context for template rendering.
        let notify_ctx = NotificationContext {
            sci_name: detection.scientific_name.clone(),
            com_name: detection.common_name.clone(),
            confidence: detection.confidence,
            confidence_pct: confidence_pct_trunc(detection.confidence),
            date: detection.date.clone(),
            time: detection.time.clone(),
            week: detection.week,
            latitude: 0.0,
            longitude: 0.0,
            reason: String::new(),
            listen_url: None,
            image_url: None,
            station_url: None,
        };

        // Check notification filter (trigger mode + species filter).
        // Also respect Suppress alert rules.
        let filter_says_notify =
            notification_filter.should_notify(&detection.scientific_name, None);
        let dispatch_allowed = passes_filter(rule_suppressed, filter_says_notify);

        // Apprise push notification (with filter and template).
        if let Some(ref apprise) = apprise {
            let apprise_says_notify = apprise
                .blocking_lock()
                .should_notify(&detection.common_name, detection.confidence);
            let should_send = should_dispatch_notification(dispatch_allowed, apprise_says_notify);

            if should_send {
                let (title, body) = notification_template.render(&notify_ctx);
                let client = Arc::clone(apprise);

                rt_handle.spawn(async move {
                    let result = client
                        .lock()
                        .await
                        .send_notification(
                            &title,
                            &body,
                            birdnet_integrations::apprise::NotifyType::Info,
                        )
                        .await;
                    if let Err(e) = result {
                        tracing::warn!(error = %e, "Apprise notification failed");
                    }
                });
            }
        }

        // BirdWeather upload.
        if let Some(ref bw) = birdweather {
            let post = birdnet_integrations::birdweather::DetectionPost {
                timestamp: format!("{}T{}Z", detection.date, detection.time),
                common_name: detection.common_name.clone(),
                scientific_name: detection.scientific_name.clone(),
                confidence: detection.confidence,
                lat: bw.coordinates().0,
                lon: bw.coordinates().1,
            };
            let client = bw.clone();
            rt_handle.spawn(async move {
                if let Err(e) = client.post_detection(&post).await {
                    tracing::warn!(error = %e, species = %post.common_name, "BirdWeather post failed");
                }
            });
        }

        // Email alert.
        if let Some(ref notifier) = email {
            let notifier = std::sync::Arc::clone(notifier);
            let alert = birdnet_integrations::email::DetectionEmail {
                common_name: detection.common_name.clone(),
                scientific_name: detection.scientific_name.clone(),
                confidence: f64::from(detection.confidence),
                date: detection.date.clone(),
                time: detection.time.clone(),
                station_name: None,
                detection_url: None,
            };
            rt_handle.spawn(async move {
                match notifier.notify(&alert).await {
                    Ok(true) => tracing::debug!(species = %alert.common_name, "email alert sent"),
                    Ok(false) => {}
                    Err(e) => tracing::warn!(error = %e, species = %alert.common_name, "email alert failed"),
                }
            });
        }

        // Heartbeat ping after processing.
        if let Some(ref hb) = heartbeat {
            let hb = Arc::clone(hb);
            rt_handle.spawn(async move {
                if let Err(e) = hb.ping().await {
                    tracing::debug!(error = %e, "heartbeat ping failed");
                }
            });
        }

        // MQTT publish (blocking I/O, handled in spawn_blocking thread).
        if let Some(ref mqtt_client) = mqtt {
            let payload = birdnet_integrations::mqtt::DetectionPayload {
                timestamp: format!("{}T{}", detection.date, detection.time),
                scientific_name: detection.scientific_name.clone(),
                common_name: detection.common_name.clone(),
                confidence: detection.confidence,
                confidence_pct: confidence_pct_round(detection.confidence),
                file_name: detection.file_name_extr.clone(),
                rtsp_id: event
                    .source_file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| {
                        birdnet_core::detection::types::RecordingFile::parse(n)
                            .and_then(|rf| rf.rtsp_id)
                    }),
            };
            let client = Arc::clone(mqtt_client);
            if let Err(e) = client.publish_detection(&payload) {
                tracing::debug!(
                    error = %e,
                    species = %detection.common_name,
                    "MQTT publish failed (broker may be offline)"
                );
            }
        }

        tracing::debug!(
            correlation_id,
            species = %detection.common_name,
            confidence = format!("{:.0}%", detection.confidence * 100.0),
            latency_ms = event.latency_ms,
            ws_clients = broadcast.client_count(),
            "event processed"
        );
    }
}

/// The wire-level shape of an outbound webhook request: method, body, and
/// content-type. Built by [`build_webhook_spec`] from operator-supplied
/// rule config, then handed to [`dispatch_webhook`] for the actual send.
///
/// Returned as a value (rather than wired into `reqwest::RequestBuilder`
/// directly) so the request shape can be tested without building a
/// reqwest client or hitting the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookSpec {
    /// HTTP verb. `Get` carries no body; `Post` carries a JSON body.
    pub method: WebhookMethod,
    /// JSON body sent with `Post`. Defaults to `"{}"` when the operator
    /// supplies no body template — matching the historical behaviour
    /// expected by alert-rule sinks that require valid JSON.
    pub body: String,
}

/// Webhook HTTP method picked from the operator's alert-rule config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookMethod {
    /// `GET` — used when the operator's rule has `method = "GET"`
    /// (case-insensitive). The body is ignored.
    Get,
    /// `POST` — the default for everything else. Carries the JSON body.
    Post,
}

/// Pure helper: build the [`WebhookSpec`] for an alert-rule webhook.
///
/// The previous inline form in `dispatch_webhook` produced a chain of
/// mutants on the method-comparison + body-default that no test could
/// catch without running an HTTP server. Returning a comparable value
/// makes the dispatch decision unit-testable: tests assert each cell of
/// the (method, body-present) decision matrix.
///
/// Method is case-insensitively matched: `"get"`, `"Get"`, and `"GET"`
/// all produce [`WebhookMethod::Get`]. Anything else (including
/// nonsense like `"PATCH"`) falls through to `Post` because the
/// alert-rule schema documents only `GET` and `POST` and we want the
/// safe default for misconfigured rules.
#[must_use]
pub fn build_webhook_spec(method: &str, body: Option<&str>) -> WebhookSpec {
    if method.eq_ignore_ascii_case("GET") {
        WebhookSpec {
            method: WebhookMethod::Get,
            body: String::new(),
        }
    } else {
        WebhookSpec {
            method: WebhookMethod::Post,
            body: body.map_or_else(|| "{}".to_owned(), str::to_owned),
        }
    }
}

/// Error type for the webhook dispatcher.
///
/// Returning a typed error (rather than `()` plus tracing-only diagnostics)
/// has two benefits:
/// * The function's body-replacement cargo-mutants become unviable —
///   `Result<u16, WebhookError>` can't be substituted with `()`.
/// * The caller can react to specific failure modes if it ever wants
///   to (today it just logs, but the surface is there).
#[derive(Debug)]
pub enum WebhookError {
    /// Building the reqwest client failed (TLS init, system DNS, etc.).
    ClientBuild(String),
    /// The request was sent but the network or the remote rejected it.
    Send(String),
}

impl std::fmt::Display for WebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClientBuild(e) => write!(f, "failed to build HTTP client for webhook: {e}"),
            Self::Send(e) => write!(f, "webhook dispatch failed: {e}"),
        }
    }
}

impl std::error::Error for WebhookError {}

/// Fire an alert-rule webhook request and return the HTTP status on success.
///
/// `dispatch_webhook` is the only non-pure step in the alert-rule dispatch
/// chain: every decision *about* the request is delegated to
/// [`build_webhook_spec`], which is unit-tested. This function's body is
/// dominated by the network call; returning `Result<u16, WebhookError>`
/// makes the body-replacement cargo-mutants unviable.
pub async fn dispatch_webhook(
    url: &str,
    method: &str,
    body: Option<&str>,
) -> Result<u16, WebhookError> {
    let spec = build_webhook_spec(method, body);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| WebhookError::ClientBuild(e.to_string()))?;

    let request = match spec.method {
        WebhookMethod::Get => client.get(url),
        WebhookMethod::Post => client
            .post(url)
            .header("Content-Type", "application/json")
            .body(spec.body),
    };

    request
        .send()
        .await
        .map(|resp| resp.status().as_u16())
        .map_err(|e| WebhookError::Send(e.to_string()))
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pure-logic helpers extracted from `event_processor`.
    //!
    //! These exist because the carryover from PR #35 noted that the daemon's
    //! event-processing path was at ~0 % unit coverage — the bugs we found
    //! lived there. Unit tests for the dispatch decision protect the
    //! threshold-gate contract going forward without needing a live model
    //! or database.

    use super::*;
    use clap::Parser;

    fn thresholds(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    #[test]
    fn no_per_species_threshold_accepts_above_global() {
        // global=0.5, detection=0.7 → accept (no per-species).
        let d = decide_disposition(0.7, "Pica pica", &thresholds(&[]), 0.5);
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn no_per_species_threshold_drops_below_global() {
        // global=0.5, detection=0.4 → drop (no per-species; model already
        // gated, but the double-check fires here too).
        let d = decide_disposition(0.4, "Pica pica", &thresholds(&[]), 0.5);
        assert_eq!(d, DispositionDecision::DropBelowGlobal);
    }

    #[test]
    fn per_species_threshold_accepts_when_met() {
        // per-species=0.8, detection=0.85 → accept.
        let t = thresholds(&[("Pica pica", 0.8)]);
        let d = decide_disposition(0.85, "Pica pica", &t, 0.5);
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn per_species_threshold_quarantines_when_missed() {
        // per-species=0.8, detection=0.6 → quarantine, not drop.
        // The whole point of the quarantine workflow is to keep these
        // detections around for review rather than silently dropping them.
        let t = thresholds(&[("Pica pica", 0.8)]);
        let d = decide_disposition(0.6, "Pica pica", &t, 0.5);
        assert_eq!(d, DispositionDecision::Quarantine { threshold: 0.8 });
    }

    #[test]
    fn per_species_threshold_overrides_global_when_below_both() {
        // global=0.5, per-species=0.8, detection=0.4 → quarantine (NOT
        // drop). The per-species override wins even when the detection
        // would also have failed the global gate — the operator-configured
        // threshold is the gate that decides quarantine vs. drop.
        let t = thresholds(&[("Pica pica", 0.8)]);
        let d = decide_disposition(0.4, "Pica pica", &t, 0.5);
        assert_eq!(d, DispositionDecision::Quarantine { threshold: 0.8 });
    }

    #[test]
    fn per_species_threshold_only_applies_to_named_species() {
        // Threshold for Pica pica; detection for Corvus corax → no override.
        let t = thresholds(&[("Pica pica", 0.95)]);
        let d = decide_disposition(0.6, "Corvus corax", &t, 0.5);
        // 0.6 > 0.5 global, no override → accept.
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn boundary_at_threshold_is_accept_for_global() {
        // The check uses `<` so equality passes. Pin the contract.
        let d = decide_disposition(0.5, "Pica pica", &thresholds(&[]), 0.5);
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn boundary_at_threshold_is_accept_for_per_species() {
        // Use exactly-representable f32 boundary value 0.5 so the
        // `<` → `<=` mutation is observable. The naive choice of 0.8
        // would leave both `<` and `<=` returning false — `0.8_f32`
        // rounds up to ~0.80000001 in f64, while `0.8_f64` is
        // ~0.79999999... so `f64::from(0.8_f32) <= 0.8_f64` is
        // already false. 0.5 is a power of two and round-trips
        // exactly between f32 and f64, so `0.5 < 0.5` is false and
        // `0.5 <= 0.5` is true — the assertion below catches the
        // boundary mutation.
        let t = thresholds(&[("Pica pica", 0.5)]);
        let d = decide_disposition(0.5, "Pica pica", &t, 0.25);
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn empty_string_species_is_treated_like_unknown() {
        // Defensive: a malformed detection with no species name should
        // hit the no-override path; whether it accepts or drops depends
        // on confidence.
        let d = decide_disposition(0.9, "", &thresholds(&[]), 0.5);
        assert_eq!(d, DispositionDecision::Accept);
        let d2 = decide_disposition(0.1, "", &thresholds(&[]), 0.5);
        assert_eq!(d2, DispositionDecision::DropBelowGlobal);
    }

    // ── derive_source_label ────────────────────────────────────────────

    #[test]
    fn source_label_is_local_for_no_rtsp_prefix() {
        let p = std::path::Path::new("/tmp/2026-05-19-birdnet-09:00:00.wav");
        assert_eq!(derive_source_label(p), "local");
    }

    #[test]
    fn source_label_picks_up_rtsp_id() {
        let p = std::path::Path::new("/tmp/2026-05-19-birdnet-RTSP_1-09:00:00.wav");
        assert_eq!(derive_source_label(p), "RTSP_1");
        let p2 = std::path::Path::new("/tmp/2026-05-19-birdnet-RTSP_42-12:34:56.flac");
        assert_eq!(derive_source_label(p2), "RTSP_42");
    }

    #[test]
    fn source_label_falls_back_to_local_on_unparseable_filename() {
        // Filename that doesn't match the canonical schema.
        let p = std::path::Path::new("/tmp/random-file.wav");
        assert_eq!(derive_source_label(p), "local");
        let p2 = std::path::Path::new("/tmp/");
        assert_eq!(derive_source_label(p2), "local");
    }

    // ── resolve_f32_with_default ────────────────────────────────────────
    //
    // Four cells: (CLI at default | CLI overridden) × (config present | absent).
    // The `==` → `!=` cargo-mutants mutation flips the choice of branch; the
    // CLI-at-default × config-present test catches it (the only cell where
    // the two paths return different values).

    #[test]
    fn resolve_f32_uses_config_when_cli_at_default_and_config_present() {
        // CLI flag at its documented default (0.03) and config has a value:
        // the resolver should pick the config value. This is the cell that
        // catches the `==` → `!=` mutant — flipping the branch returns the
        // CLI default instead of the config override.
        let v = resolve_f32_with_default(0.03, 0.03, Some(0.5));
        assert!((v - 0.5).abs() < f32::EPSILON, "got {v}");
    }

    #[test]
    fn resolve_f32_uses_cli_default_when_no_config() {
        // CLI at default, no config: fall back to the CLI value (which is
        // the documented default). Catches "replace ... with 0.0" body
        // mutations.
        let v = resolve_f32_with_default(0.03, 0.03, None);
        assert!((v - 0.03).abs() < f32::EPSILON, "got {v}");
    }

    #[test]
    fn resolve_f32_uses_cli_when_overridden_and_no_config() {
        // CLI overridden, no config: use the operator's CLI value.
        let v = resolve_f32_with_default(0.5, 0.03, None);
        assert!((v - 0.5).abs() < f32::EPSILON, "got {v}");
    }

    #[test]
    fn resolve_f32_cli_override_wins_over_config() {
        // CLI explicitly overridden, config also present: the operator's
        // CLI override wins. Pins the documented precedence.
        let v = resolve_f32_with_default(0.5, 0.03, Some(0.9));
        assert!((v - 0.5).abs() < f32::EPSILON, "got {v}");
    }

    #[test]
    fn resolve_f32_handles_zero_default() {
        // privacy_threshold defaults to 0.0. Pin both branches against the
        // zero default so the `== 0.0` boundary surfaces if the helper
        // gets refactored back to a `< EPSILON` form by mistake.
        assert!((resolve_f32_with_default(0.0, 0.0, Some(0.02)) - 0.02).abs() < f32::EPSILON);
        assert!((resolve_f32_with_default(0.0, 0.0, None) - 0.0).abs() < f32::EPSILON);
        assert!((resolve_f32_with_default(0.01, 0.0, Some(0.02)) - 0.01).abs() < f32::EPSILON);
    }

    // ── build_pipeline_config ───────────────────────────────────────────
    //
    // The struct-literal field-source mutations cargo-mutants surfaces on
    // the inline form (e.g. "delete field watch_dir from PipelineConfig
    // literal") are caught here by asserting each field individually.

    #[test]
    fn build_pipeline_config_pins_watch_dir_and_overlap() {
        let cfg = build_pipeline_config(PathBuf::from("/var/lib/birdnet/recs"), 1.5);
        assert_eq!(cfg.watch_dir, PathBuf::from("/var/lib/birdnet/recs"));
        assert!((cfg.chunk_overlap_secs - 1.5).abs() < f32::EPSILON);
        // Other fields fall through to defaults — assert that contract
        // explicitly so a mistaken `..` change surfaces.
        let default = PipelineConfig::default();
        assert_eq!(cfg.target_sample_rate, default.target_sample_rate);
        assert!((cfg.chunk_duration_secs - default.chunk_duration_secs).abs() < f32::EPSILON);
        assert!((cfg.confidence_threshold - default.confidence_threshold).abs() < f32::EPSILON);
        assert_eq!(cfg.raw_audio_input, default.raw_audio_input);
    }

    #[test]
    fn build_pipeline_config_zero_overlap_is_distinct_from_default_dir() {
        // Edge: a literal with overlap=0.0 should still set watch_dir
        // correctly. Catches "delete field watch_dir" mutants that
        // would leave the default `/tmp/StreamData`.
        let cfg = build_pipeline_config(PathBuf::from("/other/dir"), 0.0);
        assert_eq!(cfg.watch_dir, PathBuf::from("/other/dir"));
        assert!((cfg.chunk_overlap_secs - 0.0).abs() < f32::EPSILON);
    }

    // ── build_model_config ──────────────────────────────────────────────

    #[test]
    fn build_model_config_pins_sensitivity_and_threshold() {
        let cfg = build_model_config(1.25, 0.42);
        assert!((cfg.sensitivity - 1.25).abs() < f32::EPSILON);
        assert!((cfg.confidence_threshold - 0.42).abs() < f32::EPSILON);
        let default = ModelConfig::default();
        assert_eq!(cfg.top_n, default.top_n);
        assert_eq!(cfg.num_threads, default.num_threads);
    }

    #[test]
    fn build_model_config_distinct_inputs_produce_distinct_fields() {
        // Asserts the two fields don't accidentally cross-wire: sensitivity
        // != threshold means a "swap fields" mutant would be caught.
        let cfg = build_model_config(0.5, 0.9);
        assert!((cfg.sensitivity - 0.5).abs() < f32::EPSILON);
        assert!((cfg.confidence_threshold - 0.9).abs() < f32::EPSILON);
    }

    // ── build_species_filter_config ─────────────────────────────────────

    #[test]
    fn build_species_filter_config_pins_sf_thresh() {
        let cfg = build_species_filter_config(0.07);
        assert!((cfg.sf_thresh - 0.07).abs() < f32::EPSILON);
        let default = SpeciesFilterConfig::default();
        assert_eq!(cfg.whitelist, default.whitelist);
        assert_eq!(cfg.include_list, default.include_list);
        assert_eq!(cfg.exclude_list, default.exclude_list);
    }

    // ── extraction_output_dir ───────────────────────────────────────────

    #[test]
    fn extraction_output_dir_uses_parent_when_present() {
        // /var/lib/birdnet/recs has parent /var/lib/birdnet, so the
        // output dir is /var/lib/birdnet/Extracted.
        let p = extraction_output_dir(Path::new("/var/lib/birdnet/recs"));
        assert_eq!(p, PathBuf::from("/var/lib/birdnet/Extracted"));
    }

    #[test]
    fn extraction_output_dir_falls_back_when_no_parent() {
        // A relative bare path has parent = Some("") which still joins;
        // a root path's parent is None and falls back to the well-known
        // default.
        let p = extraction_output_dir(Path::new("/"));
        assert_eq!(p, PathBuf::from("BirdSongs/Extracted"));
    }

    // ── build_extraction_config ─────────────────────────────────────────

    #[test]
    fn build_extraction_config_pins_every_field() {
        let cli = Cli::parse_from([
            "birdnet-behavior",
            "--audio-format",
            "flac",
            "--segment-duration",
            "20",
            "--freq-shift-hz",
            "1500",
        ]);
        let cfg = build_extraction_config(&cli, Path::new("/var/lib/birdnet/recs"));
        assert_eq!(cfg.audio_format, "flac");
        assert_eq!(cfg.target_format, AudioFormat::Flac);
        assert_eq!(cfg.output_dir, PathBuf::from("/var/lib/birdnet/Extracted"));
        assert!((cfg.recording_length - 20.0).abs() < f32::EPSILON);
        assert_eq!(cfg.freq_shift_hz, 1500);
        // Default still applies to extraction_length:
        let default = ExtractionConfig::default();
        assert!((cfg.extraction_length - default.extraction_length).abs() < f32::EPSILON);
    }

    #[test]
    fn build_extraction_config_defaults() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let cfg = build_extraction_config(&cli, Path::new("/tmp/StreamData"));
        // Default audio format is wav.
        assert_eq!(cfg.audio_format, "wav");
        assert_eq!(cfg.target_format, AudioFormat::Wav);
        // Default segment_duration is 15.
        assert!((cfg.recording_length - 15.0).abs() < f32::EPSILON);
        assert_eq!(cfg.freq_shift_hz, 0);
        assert_eq!(cfg.output_dir, PathBuf::from("/tmp/Extracted"));
    }

    // ── resolve_required_paths ──────────────────────────────────────────

    #[test]
    fn resolve_required_paths_returns_none_with_no_flags_no_config() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        // No CLI flags, no config: must return None so the daemon refuses
        // to start. Catches the `Some(Default::default())` mutant on the
        // function body — it would return three empty paths instead.
        assert!(resolve_required_paths(&cli, None).is_none());
    }

    #[test]
    fn resolve_required_paths_returns_some_when_all_cli_present() {
        let cli = Cli::parse_from([
            "birdnet-behavior",
            "--model",
            "/m/birdnet.onnx",
            "--labels",
            "/m/labels.txt",
            "--watch-dir",
            "/m/recs",
        ]);
        let (model, labels, watch) =
            resolve_required_paths(&cli, None).expect("all three CLI flags set");
        assert_eq!(model, PathBuf::from("/m/birdnet.onnx"));
        assert_eq!(labels, PathBuf::from("/m/labels.txt"));
        assert_eq!(watch, PathBuf::from("/m/recs"));
    }

    #[test]
    fn resolve_required_paths_returns_none_if_any_missing() {
        // model + labels but not watch_dir → None.
        let cli = Cli::parse_from([
            "birdnet-behavior",
            "--model",
            "/m/birdnet.onnx",
            "--labels",
            "/m/labels.txt",
        ]);
        assert!(resolve_required_paths(&cli, None).is_none());
    }

    #[test]
    fn resolve_required_paths_resolves_from_config_fallback() {
        // Use a real Config built from a string snippet so the
        // CLI-or-config precedence is exercised end-to-end.
        let mut tmpcfg = std::env::temp_dir();
        tmpcfg.push(format!("birdnet-resolve-test-{}.conf", std::process::id()));
        std::fs::write(
            &tmpcfg,
            "MODEL_PATH=/cfg/model.onnx\nLABELS_PATH=/cfg/labels.txt\nRECS_DIR=/cfg/recs\n",
        )
        .unwrap();
        let cfg = birdnet_core::config::Config::load_from(&tmpcfg).unwrap();
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let (model, labels, watch) =
            resolve_required_paths(&cli, Some(&cfg)).expect("config-only resolution");
        assert_eq!(model, PathBuf::from("/cfg/model.onnx"));
        assert_eq!(labels, PathBuf::from("/cfg/labels.txt"));
        assert_eq!(watch, PathBuf::from("/cfg/recs"));
        let _ = std::fs::remove_file(&tmpcfg);
    }

    // ── species_thresholds_log_count ────────────────────────────────────

    #[test]
    fn species_thresholds_log_count_none_for_empty() {
        let empty = HashMap::<String, f64>::new();
        assert_eq!(species_thresholds_log_count(&empty), None);
    }

    #[test]
    fn species_thresholds_log_count_some_for_nonempty() {
        let m = thresholds(&[("Pica pica", 0.8), ("Corvus corax", 0.85)]);
        assert_eq!(species_thresholds_log_count(&m), Some(2));
    }

    // ── confidence_pct_trunc / confidence_pct_round ────────────────────
    //
    // Both helpers wrap an arithmetic mutant on `*`. The tests below pin
    // a value that catches `*` → `+` (would produce 100.7) and `*` → `/`
    // (would produce ~0.0095).

    #[test]
    fn confidence_pct_trunc_basic() {
        assert_eq!(confidence_pct_trunc(0.0), 0);
        assert_eq!(confidence_pct_trunc(0.5), 50);
        assert_eq!(confidence_pct_trunc(0.954), 95);
        assert_eq!(confidence_pct_trunc(1.0), 100);
    }

    #[test]
    fn confidence_pct_trunc_truncates_not_rounds() {
        // 0.999 * 100 = 99.9 → truncates to 99 (round would give 100).
        // Pinning this catches an accidental swap to round semantics.
        assert_eq!(confidence_pct_trunc(0.999), 99);
    }

    #[test]
    fn confidence_pct_round_basic() {
        assert_eq!(confidence_pct_round(0.0), 0);
        assert_eq!(confidence_pct_round(0.5), 50);
        assert_eq!(confidence_pct_round(0.954), 95);
        assert_eq!(confidence_pct_round(1.0), 100);
    }

    #[test]
    fn confidence_pct_round_rounds_not_truncates() {
        // 0.999 * 100 = 99.9 → rounds to 100. Pins the round semantic.
        assert_eq!(confidence_pct_round(0.999), 100);
        // 0.955 rounds to 96 (vs trunc 95): distinguishes the two helpers
        // and catches a `*` arithmetic mutation that would skew the value.
        assert_eq!(confidence_pct_round(0.955), 96);
    }

    // ── latency_ms_to_seconds ───────────────────────────────────────────

    #[test]
    fn latency_ms_to_seconds_basic() {
        assert!((latency_ms_to_seconds(0) - 0.0).abs() < 1e-9);
        assert!((latency_ms_to_seconds(1_000) - 1.0).abs() < 1e-9);
        assert!((latency_ms_to_seconds(250) - 0.25).abs() < 1e-9);
        assert!((latency_ms_to_seconds(1_500) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn latency_ms_to_seconds_division_distinct_from_modulo() {
        // 1500 ms = 1.5 s. The `/ 1000.0` → `% 1000.0` mutant would
        // produce 500.0; the `/ 1000.0` → `* 1000.0` mutant would
        // produce 1_500_000. Either is caught by the previous test;
        // pin a non-round case here to double-cover.
        let v = latency_ms_to_seconds(2_750);
        assert!((v - 2.75).abs() < 1e-9, "got {v}");
    }

    // ── is_first_detection_today ────────────────────────────────────────

    #[test]
    fn is_first_detection_today_boundary() {
        // The `<=` boundary: 0 and 1 are "first", 2 is not. Pins both sides.
        assert!(is_first_detection_today(0));
        assert!(is_first_detection_today(1));
        assert!(!is_first_detection_today(2));
        assert!(!is_first_detection_today(100));
    }

    #[test]
    fn is_first_detection_today_handles_negative_defensively() {
        // The query returns i64 and could in principle be negative if the
        // upstream sentinel changes. <= 1 still classifies as "first".
        assert!(is_first_detection_today(-1));
    }

    // ── passes_filter ───────────────────────────────────────────────────
    //
    // Four-cell truth table for `!suppressed && filter`. The mutations
    // cargo-mutants generates on the inline form are:
    //   - `&&` → `||`: changes (T,F) and (F,T) results
    //   - `delete !`: changes (T,T) and (F,F)
    //   - replace body with `true`/`false`

    #[test]
    fn passes_filter_truth_table() {
        // (suppressed, filter) → expected
        assert!(passes_filter(false, true)); // green light
        assert!(!passes_filter(false, false)); // filter says no
        assert!(!passes_filter(true, true)); // rule says suppress
        assert!(!passes_filter(true, false)); // both negative
    }

    // ── should_dispatch_notification ────────────────────────────────────

    #[test]
    fn should_dispatch_notification_truth_table() {
        // (dispatch_allowed, integration_says_notify) → expected
        assert!(should_dispatch_notification(true, true));
        assert!(!should_dispatch_notification(true, false));
        assert!(!should_dispatch_notification(false, true));
        assert!(!should_dispatch_notification(false, false));
    }

    // ── build_webhook_spec ──────────────────────────────────────────────
    //
    // The four cells of the decision matrix:
    //   (method ∈ {GET, POST}) × (body ∈ {Some, None})
    // The pure helper makes every cell observable in a unit test.

    #[test]
    fn build_webhook_spec_get_ignores_body() {
        // GET → method=Get, body empty (per the contract docstring).
        let s = build_webhook_spec("GET", Some("{\"hello\": \"world\"}"));
        assert_eq!(s.method, WebhookMethod::Get);
        assert_eq!(s.body, "");
        // Case insensitivity:
        let s2 = build_webhook_spec("get", None);
        assert_eq!(s2.method, WebhookMethod::Get);
        let s3 = build_webhook_spec("Get", None);
        assert_eq!(s3.method, WebhookMethod::Get);
    }

    #[test]
    fn build_webhook_spec_post_uses_supplied_body() {
        let s = build_webhook_spec("POST", Some("{\"k\": 1}"));
        assert_eq!(s.method, WebhookMethod::Post);
        assert_eq!(s.body, "{\"k\": 1}");
    }

    #[test]
    fn build_webhook_spec_post_defaults_body_to_empty_object() {
        // No body supplied: default to "{}" so the recipient sees valid
        // JSON. Pins the contract for alert-rule sinks that require
        // application/json bodies.
        let s = build_webhook_spec("POST", None);
        assert_eq!(s.method, WebhookMethod::Post);
        assert_eq!(s.body, "{}");
    }

    #[test]
    fn build_webhook_spec_unknown_method_falls_back_to_post() {
        // Operator misconfigures method as "PATCH"? Fall back to POST
        // because the safe default is "send something with a body" rather
        // than "send a GET with no body".
        let s = build_webhook_spec("PATCH", Some("{\"a\":1}"));
        assert_eq!(s.method, WebhookMethod::Post);
        assert_eq!(s.body, "{\"a\":1}");
    }

    // ── dispatch_webhook ────────────────────────────────────────────────
    //
    // The function is async and makes a real network call, so the test
    // exercises a deliberately-failing URL (TEST-NET-2, RFC 5737) and
    // asserts the Err arm fires. This is enough to catch:
    //   - "replace dispatch_webhook -> Result<u16, WebhookError> with Ok(0)" — would
    //     produce Ok(0), the test asserts Err.
    //   - "replace … with Err(WebhookError::ClientBuild(\"xyzzy\".into()))" — would
    //     produce the wrong error variant, the test asserts the Send
    //     variant (because client builds fine on a healthy CI host).
    //   - "replace dispatch_webhook -> Result<u16, WebhookError> with ()" — unviable
    //     by return type, no longer counted as missed.

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_webhook_returns_send_error_on_unreachable() {
        // TEST-NET-2 (198.51.100.0/24) — RFC 5737 reserved range, will
        // not route. The 10-second timeout caps the test runtime.
        let r = dispatch_webhook("http://198.51.100.1:1/", "POST", None).await;
        assert!(r.is_err(), "expected Err on unreachable host, got {r:?}");
        // Confirm it's the Send variant, not ClientBuild — the client
        // builds fine; only the network call fails.
        match r {
            Err(WebhookError::Send(_)) => {}
            Err(WebhookError::ClientBuild(e)) => {
                panic!("expected Send error, got ClientBuild({e})")
            }
            Ok(s) => panic!("expected Err, got Ok({s})"),
        }
    }

    #[test]
    fn webhook_error_display_distinguishes_variants() {
        // Pin the Display impls so the log messages from the call site
        // remain searchable. Catches "delete fmt arm" mutations.
        assert!(
            WebhookError::ClientBuild("tls-init failed".into())
                .to_string()
                .contains("client")
        );
        assert!(
            WebhookError::Send("connect refused".into())
                .to_string()
                .contains("dispatch")
        );
    }

    // ── start_detection_daemon: in-process happy path ───────────────────
    //
    // The four-layer testing standard (ADR-16) requires more than unit
    // tests for any change touching audio / inference / persistence. This
    // is the unit-level equivalent of Layer 3 (subprocess integration):
    // we stand the daemon up *in-process* against the tiny V2.4 ONNX
    // model bundled at `crates/birdnet-core/src/testdata/tiny_v24_test.onnx`
    // (the same one used by `crates/birdnet-core/src/inference/model.rs`
    // tests) and assert the function returns `Some(handle)`. Without
    // this test, the cargo-mutants substitution
    // `replace start_detection_daemon -> Option<...> with None` survives
    // every other test in the suite (no other test calls the function).

    /// The 11-label fixture string matching `tiny_v24_test.onnx`'s
    /// 11-class output head. Format is the V2.4 underscore-separated
    /// "`Scientific_Common`" each line carries.
    fn tiny_labels_text() -> String {
        (0..11)
            .map(|i| format!("Species{i}_Bird {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Tiny V2.4-shaped ONNX model included as bytes; same artifact the
    /// inference model unit tests use. Writing to disk so the daemon's
    /// file-based `BirdNetModel::load` can pick it up like the real
    /// model on a deployed station.
    const TINY_V24_MODEL_BYTES: &[u8] =
        include_bytes!("../crates/birdnet-core/src/testdata/tiny_v24_test.onnx");

    #[tokio::test(flavor = "current_thread")]
    async fn start_detection_daemon_returns_some_with_valid_inputs() {
        let tmp = tempfile::tempdir().unwrap();
        let watch_dir = tmp.path().join("recs");
        std::fs::create_dir_all(&watch_dir).unwrap();

        let model_path = tmp.path().join("model.onnx");
        std::fs::write(&model_path, TINY_V24_MODEL_BYTES).unwrap();

        let labels_path = tmp.path().join("labels.txt");
        std::fs::write(&labels_path, tiny_labels_text()).unwrap();

        let cli = Cli::parse_from([
            "birdnet-behavior",
            "--model",
            model_path.to_str().unwrap(),
            "--labels",
            labels_path.to_str().unwrap(),
            "--watch-dir",
            watch_dir.to_str().unwrap(),
        ]);

        let db_path = tmp.path().join("birds.db");
        let state = birdnet_web::state::AppState::new(db_path).unwrap();
        let broadcast = state.detection_broadcast();

        let filter = birdnet_integrations::notification::NotificationFilter {
            trigger: birdnet_integrations::notification::TriggerMode::EachDetection,
            species_filter: birdnet_integrations::notification::SpeciesFilter::new(None, None),
        };
        let template = birdnet_integrations::notification::NotificationTemplate::default();

        let handle = start_detection_daemon(
            &cli, None, state, broadcast, None, None, None, None, None, filter, template,
        );

        assert!(
            handle.is_some(),
            "daemon must return Some(DaemonHandle) when model+labels+watch_dir all resolve"
        );
        if let Some(h) = handle {
            h.stop();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn start_detection_daemon_returns_none_when_inputs_missing() {
        // Paired test: with no CLI args and no config, the function must
        // return None. Catches the inverse `replace with Some(Default::default())`
        // mutant (which doesn't apply directly to this return type, but
        // any test asserting None pins the early-return contract).
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("birds.db");
        let state = birdnet_web::state::AppState::new(db_path).unwrap();
        let broadcast = state.detection_broadcast();
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let filter = birdnet_integrations::notification::NotificationFilter {
            trigger: birdnet_integrations::notification::TriggerMode::EachDetection,
            species_filter: birdnet_integrations::notification::SpeciesFilter::new(None, None),
        };
        let template = birdnet_integrations::notification::NotificationTemplate::default();

        let handle = start_detection_daemon(
            &cli, None, state, broadcast, None, None, None, None, None, filter, template,
        );
        assert!(
            handle.is_none(),
            "daemon must return None when model/labels/watch_dir all unset"
        );
    }

    // ── event_processor: one-shot in-process test ───────────────────────
    //
    // Sends a single `DetectionEvent` through the channel, then drops the
    // sender so `event_processor` exits its loop after consuming the one
    // event. Asserts a row was inserted into the DB. This catches
    // `replace event_processor with ()` because the mutant produces no
    // DB write at all (the row count stays 0).
    //
    // Lives inside the daemon module so the private `event_processor`
    // function is directly callable.

    #[tokio::test(flavor = "current_thread")]
    async fn event_processor_inserts_row_for_accepted_event() {
        use birdnet_core::audio::extraction::ExtractionConfig;
        use birdnet_core::detection::daemon::DetectionEvent;
        use birdnet_core::detection::types::Detection;

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("birds.db");
        let state = birdnet_web::state::AppState::new(db_path).unwrap();
        let broadcast = state.detection_broadcast();

        // Send exactly one event, then close the channel.
        let (event_tx, event_rx) = mpsc::channel::<DetectionEvent>();
        let detection = Detection {
            date: "2026-05-19".into(),
            time: "09:00:00".into(),
            scientific_name: "Pica pica".into(),
            common_name: "Eurasian Magpie".into(),
            confidence: 0.95,
            start: 0.0,
            stop: 3.0,
            week: 20,
            file_name_extr: None,
        };
        event_tx
            .send(DetectionEvent {
                detection,
                source_file: tmp.path().join("nonexistent.wav"),
                latency_ms: 100,
                correlation_id: "test-corr-abc".into(),
            })
            .unwrap();
        drop(event_tx);

        let filter = birdnet_integrations::notification::NotificationFilter {
            trigger: birdnet_integrations::notification::TriggerMode::EachDetection,
            species_filter: birdnet_integrations::notification::SpeciesFilter::new(None, None),
        };
        let template = birdnet_integrations::notification::NotificationTemplate::default();
        let extractor = Extractor::new(ExtractionConfig::default());
        let rt_handle = tokio::runtime::Handle::current();

        let state_for_processor = state.clone();
        tokio::task::spawn_blocking(move || {
            super::event_processor(
                event_rx,
                state_for_processor,
                broadcast,
                None,
                None,
                None,
                None,
                None,
                filter,
                template,
                rt_handle,
                HashMap::new(),
                0.25,
                extractor,
            );
        })
        .await
        .unwrap();

        let count = state.with_db(|conn| birdnet_db::sqlite::detection_count(conn).unwrap_or(0));
        assert_eq!(
            count, 1,
            "event_processor must persist accepted events; got {count} rows"
        );

        // And the correlation id round-trips.
        let recent = state
            .with_db(|conn| birdnet_db::sqlite::recent_detections(conn, 1).unwrap_or_default());
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].correlation_id.as_deref(), Some("test-corr-abc"));
    }
}
