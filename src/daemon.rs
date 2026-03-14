//! Detection daemon startup and event processing bridge.
//!
//! Starts the background detection daemon and bridges its `std::mpsc` event
//! channel to WebSocket broadcasts and external integrations.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use crate::cli::Cli;
use crate::integrations::{AppriseHandle, EmailHandle};

/// Start the detection daemon in a background thread.
///
/// Returns the daemon handle, or `None` if the model/labels are not configured.
pub fn start_detection_daemon(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: birdnet_web::state::AppState,
    broadcast: birdnet_web::routes::websocket::DetectionBroadcast,
    apprise: Option<AppriseHandle>,
    birdweather: Option<birdnet_integrations::birdweather::Client>,
    email: Option<EmailHandle>,
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
        tracing::info!(
            "detection daemon not started: model, labels, or watch_dir not configured"
        );
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

    let daemon_config = birdnet_core::detection::daemon::DaemonConfig {
        watch_dir: watch_dir.clone(),
        model_path,
        labels_path,
        pipeline: birdnet_core::detection::pipeline::PipelineConfig {
            watch_dir,
            ..birdnet_core::detection::pipeline::PipelineConfig::default()
        },
        model: birdnet_core::inference::model::ModelConfig {
            sensitivity,
            confidence_threshold: confidence,
            ..birdnet_core::inference::model::ModelConfig::default()
        },
        process_existing: cli.process_existing,
    };

    let (event_tx, event_rx) = mpsc::channel();

    match birdnet_core::detection::daemon::run_daemon(&daemon_config, event_tx) {
        Ok(handle) => {
            tracing::info!("detection daemon started");
            let rt_handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                event_processor(event_rx, state, broadcast, apprise, birdweather, email, rt_handle);
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
#[allow(clippy::needless_pass_by_value)]
fn event_processor(
    event_rx: mpsc::Receiver<birdnet_core::detection::daemon::DetectionEvent>,
    state: birdnet_web::state::AppState,
    broadcast: birdnet_web::routes::websocket::DetectionBroadcast,
    apprise: Option<AppriseHandle>,
    birdweather: Option<birdnet_integrations::birdweather::Client>,
    email: Option<EmailHandle>,
    rt_handle: tokio::runtime::Handle,
) {
    tracing::debug!("event processor started");

    loop {
        let Ok(event) = event_rx.recv() else {
            tracing::info!("event channel closed, stopping event processor");
            break;
        };

        let detection = &event.detection;

        // Insert into SQLite.
        let week_str = detection.week.to_string();
        let file_str = event.source_file.to_string_lossy();
        let record = birdnet_db::sqlite::DetectionRecord {
            date: &detection.date,
            time: &detection.time,
            sci_name: &detection.scientific_name,
            com_name: &detection.common_name,
            confidence: f64::from(detection.confidence),
            lat: "",
            lon: "",
            cutoff: "",
            week: &week_str,
            sensitivity: "",
            overlap: "",
            file_name: &file_str,
        };

        if let Err(e) = state.with_db(|conn| birdnet_db::sqlite::insert_detection(conn, &record)) {
            tracing::warn!(error = %e, "failed to insert detection into database");
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
        };
        broadcast.send(&ws_event);

        // Apprise push notification.
        if let Some(ref apprise) = apprise {
            let should_send = apprise
                .blocking_lock()
                .should_notify(&detection.common_name, detection.confidence);

            if should_send {
                let species = detection.common_name.clone();
                let confidence = detection.confidence;
                let date = detection.date.clone();
                let time = detection.time.clone();
                let client = Arc::clone(apprise);

                rt_handle.spawn(async move {
                    let result = client
                        .lock()
                        .await
                        .notify_detection(&species, confidence, &date, &time)
                        .await;
                    if let Err(e) = result {
                        tracing::warn!(error = %e, species = %species, "Apprise notification failed");
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

        tracing::debug!(
            species = %detection.common_name,
            confidence = format!("{:.0}%", detection.confidence * 100.0),
            latency_ms = event.latency_ms,
            ws_clients = broadcast.client_count(),
            "event processed"
        );
    }
}
