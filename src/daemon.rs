//! Detection daemon startup and event processing bridge.
//!
//! Starts the background detection daemon and bridges its `std::mpsc` event
//! channel to WebSocket broadcasts and external integrations. Now also supports
//! heartbeat pings, notification templates, species filters, and trigger modes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use birdnet_core::audio::extraction::{AudioFormat, ExtractionConfig, Extractor};
use birdnet_integrations::notification::{
    NotificationContext, NotificationFilter, NotificationTemplate,
};

use crate::cli::Cli;
use crate::integrations::{AppriseHandle, EmailHandle, HeartbeatHandle, MqttHandle};

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
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
    let model_path = cli
        .model
        .clone()
        .or_else(|| config?.get("MODEL_PATH").map(PathBuf::from));

    let labels_path = cli
        .labels
        .clone()
        .or_else(|| config?.get("LABELS_PATH").map(PathBuf::from));

    let watch_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from));

    let (Some(model_path), Some(labels_path), Some(watch_dir)) =
        (model_path, labels_path, watch_dir)
    else {
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

    // Resolve metadata model path from CLI or config
    let metadata_model_path = cli
        .metadata_model
        .clone()
        .or_else(|| config?.get("METADATA_MODEL_PATH").map(PathBuf::from));

    // Resolve species filter threshold
    let sf_thresh = if (cli.sf_thresh - 0.03).abs() < f32::EPSILON {
        // CLI default; check config file
        config
            .and_then(|c| c.get_parsed::<f32>("SF_THRESH").ok())
            .unwrap_or(cli.sf_thresh)
    } else {
        cli.sf_thresh
    };

    // Resolve privacy threshold
    let privacy_threshold = if cli.privacy_threshold.abs() < f32::EPSILON {
        config
            .and_then(|c| c.get_parsed::<f32>("PRIVACY_THRESHOLD").ok())
            .unwrap_or(0.0)
    } else {
        cli.privacy_threshold
    };

    let species_filter_config = birdnet_core::inference::species_filter::SpeciesFilterConfig {
        sf_thresh,
        ..birdnet_core::inference::species_filter::SpeciesFilterConfig::default()
    };

    // Resolve overlap from CLI or config
    let overlap = if cli.overlap.abs() < f32::EPSILON {
        config
            .and_then(|c| c.get_parsed::<f32>("OVERLAP").ok())
            .unwrap_or(0.0)
    } else {
        cli.overlap
    };

    // Load per-species confidence thresholds from database
    let species_thresholds = state
        .with_db(|conn| birdnet_db::sqlite::get_species_threshold_map(conn).unwrap_or_default());

    if !species_thresholds.is_empty() {
        tracing::info!(
            count = species_thresholds.len(),
            "loaded per-species confidence thresholds"
        );
    }

    let daemon_config = birdnet_core::detection::daemon::DaemonConfig {
        watch_dir: watch_dir.clone(),
        model_path,
        labels_path,
        pipeline: birdnet_core::detection::pipeline::PipelineConfig {
            watch_dir,
            chunk_overlap_secs: overlap,
            ..birdnet_core::detection::pipeline::PipelineConfig::default()
        },
        model: birdnet_core::inference::model::ModelConfig {
            sensitivity,
            confidence_threshold: confidence,
            ..birdnet_core::inference::model::ModelConfig::default()
        },
        process_existing: cli.process_existing,
        metadata_model_path,
        species_filter: species_filter_config,
        privacy_threshold,
        latitude: cli.latitude,
        longitude: cli.longitude,
        species_thresholds,
    };

    let (event_tx, event_rx) = mpsc::channel();

    let thresholds_for_processor = daemon_config.species_thresholds.clone();
    let global_confidence = confidence;

    // Build audio extraction config from CLI args.
    let audio_format = AudioFormat::parse(&cli.audio_format);
    let extraction_output_dir = daemon_config.watch_dir.parent().map_or_else(
        || PathBuf::from("BirdSongs/Extracted"),
        |p| p.join("Extracted"),
    );
    let extraction_config = ExtractionConfig {
        target_format: audio_format,
        audio_format: cli.audio_format.clone(),
        output_dir: extraction_output_dir,
        #[allow(clippy::cast_precision_loss)]
        recording_length: cli.segment_duration as f32,
        freq_shift_hz: cli.freq_shift_hz,
        ..ExtractionConfig::default()
    };
    let extractor = Extractor::new(extraction_config);

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
        #[allow(clippy::cast_precision_loss)]
        metrics.observe_inference_seconds((event.latency_ms as f64) / 1000.0);

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
                            dispatch_webhook(&url, &method, body.as_deref(), &rule_name).await;
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
            // If count is 1, the row we just inserted is the only one today.
            today_count <= 1
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
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let confidence_pct = (detection.confidence * 100.0) as u32;
        let notify_ctx = NotificationContext {
            sci_name: detection.scientific_name.clone(),
            com_name: detection.common_name.clone(),
            confidence: detection.confidence,
            confidence_pct,
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
        let passes_filter =
            !rule_suppressed && notification_filter.should_notify(&detection.scientific_name, None);

        // Apprise push notification (with filter and template).
        if let Some(ref apprise) = apprise {
            let should_send = passes_filter
                && apprise
                    .blocking_lock()
                    .should_notify(&detection.common_name, detection.confidence);

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
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let payload = birdnet_integrations::mqtt::DetectionPayload {
                timestamp: format!("{}T{}", detection.date, detection.time),
                scientific_name: detection.scientific_name.clone(),
                common_name: detection.common_name.clone(),
                confidence: detection.confidence,
                confidence_pct: (detection.confidence * 100.0).round() as u32,
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

/// Fire an alert-rule webhook request.
///
/// Dispatched as a background tokio task; errors are logged but not propagated.
async fn dispatch_webhook(url: &str, method: &str, body: Option<&str>, rule_name: &str) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build();

    let Ok(client) = client else {
        tracing::warn!(rule = rule_name, "failed to build HTTP client for webhook");
        return;
    };

    let request = if method.eq_ignore_ascii_case("GET") {
        client.get(url)
    } else {
        let builder = client.post(url);
        if let Some(b) = body {
            builder
                .header("Content-Type", "application/json")
                .body(b.to_owned())
        } else {
            builder
                .header("Content-Type", "application/json")
                .body("{}")
        }
    };

    match request.send().await {
        Ok(resp) => {
            tracing::debug!(
                rule = rule_name,
                url,
                status = resp.status().as_u16(),
                "webhook dispatched"
            );
        }
        Err(e) => {
            tracing::warn!(
                rule = rule_name,
                url,
                error = %e,
                "webhook dispatch failed"
            );
        }
    }
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
        // f64::from(f32) is exact for representable values; pin the contract.
        let t = thresholds(&[("Pica pica", 0.8)]);
        let d = decide_disposition(0.8, "Pica pica", &t, 0.5);
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
}
