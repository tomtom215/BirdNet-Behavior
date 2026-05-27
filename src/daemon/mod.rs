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

use std::path::PathBuf;
use std::sync::mpsc;

use birdnet_core::audio::extraction::Extractor;
use birdnet_integrations::notification::{NotificationFilter, NotificationTemplate};

use crate::cli::Cli;
use crate::integrations::{AppriseHandle, EmailHandle, HeartbeatHandle, MqttHandle};

use config::{
    build_extraction_config, build_model_config, build_pipeline_config,
    build_species_filter_config, resolve_f32_with_default, resolve_required_paths,
    species_thresholds_log_count,
};
use processor::event_processor;

mod config;
mod disposition;
mod processor;
mod webhook;

#[cfg(test)]
mod test_support;

/// Bound on the detection-event channel between the daemon's detection loop
/// (producer) and the [`event_processor`] (single consumer).
///
/// Bounding it means a stalled consumer — e.g. blocked on the shared SQLite
/// mutex behind a slow query — applies backpressure to the detection loop
/// instead of buffering unboundedly until the process is OOM-killed. When the
/// loop blocks on a full channel its heartbeat stops advancing, so the systemd
/// watchdog restarts a genuinely wedged daemon rather than letting memory grow.
/// The bound is generous enough to absorb a dawn-chorus burst while the
/// consumer keeps up.
const DETECTION_EVENT_CHANNEL_CAP: usize = 1024;

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
        tracing::warn!(
            "detection daemon NOT started: model, labels, or watch_dir not configured — the web \
             UI will run but NO audio will be analysed and NO detections recorded"
        );
        tracing::warn!(
            "configure --model, --labels, --watch-dir (or MODEL_PATH, LABELS_PATH, RECS_DIR in the config file) and restart"
        );
        return None;
    };

    // Ensure the watch directory exists before the file watcher attaches to it.
    // Under systemd the stream dir lives in the service's PrivateTmp /tmp, which
    // is a fresh, empty tmpfs on every start — so it is gone after each restart
    // even though the installer created it once. `notify` errors out when asked
    // to watch a missing directory, which would silently disable detection (web
    // UI up, nothing analysed). Create it up front so a freshly-(re)started
    // daemon always has somewhere to watch.
    if let Err(e) = std::fs::create_dir_all(&watch_dir) {
        tracing::warn!(
            watch_dir = %watch_dir.display(),
            error = %e,
            "could not create watch directory; the file watcher may fail to start"
        );
    }

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

    let (event_tx, event_rx) = mpsc::sync_channel(DETECTION_EVENT_CHANNEL_CAP);

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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

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
        include_bytes!("../../crates/birdnet-core/src/testdata/tiny_v24_test.onnx");

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
    async fn start_detection_daemon_creates_missing_watch_dir() {
        // The watch dir does NOT exist up front. start_detection_daemon must
        // create it (PrivateTmp wipes the service's /tmp on every systemd
        // restart) so the file watcher can attach instead of silently
        // disabling detection. Pins the create_dir_all against a mutant that
        // removes it — without the call `run_daemon`'s watcher would error on
        // the missing directory and the function would return None.
        let tmp = tempfile::tempdir().unwrap();
        let watch_dir = tmp.path().join("stream-not-created-yet");
        assert!(
            !watch_dir.exists(),
            "precondition: watch dir must be absent before the daemon starts"
        );

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
            watch_dir.exists(),
            "start_detection_daemon must create the missing watch dir before watching"
        );
        assert!(
            handle.is_some(),
            "daemon must start once the watch dir has been created"
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
}
