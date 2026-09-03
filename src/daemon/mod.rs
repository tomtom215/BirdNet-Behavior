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
use crate::integrations::{AppriseHandle, EmailHandle, MqttHandle};

use config::{
    build_extraction_config, build_model_config, build_pipeline_config,
    build_species_filter_config, resolve_required_paths, resolve_sensitivity,
    species_lists_log_counts, species_thresholds_log_count,
};
use processor::event_processor;

mod config;

/// Shared with `crate::doctor::config` and `crate::helpers::settings_overlay`
/// so the diagnostic, the settings bridge and the detection daemon resolve
/// these through one function each rather than their own copies. The doc on
/// `resolve_station_coords` records what a second copy cost the last time one
/// of these rules was duplicated: a diagnostic that read the setting the
/// runtime ignores reports on a station that does not exist.
pub use config::{
    resolve_confidence, resolve_confirmation_level, resolve_f32_with_default,
    resolve_station_coords,
};
mod daylight;
pub mod disposition;
mod duplicate;
mod processor;

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

/// Whether the dynamic-threshold feature is switched on but cannot change any
/// decision at this global confidence.
///
/// The station warns in exactly this case: the operator has enabled the
/// feature, and its floor sits at or above the global threshold, so every
/// level is clamped straight back to where it started and no species is ever
/// adjusted. Silent, and indistinguishable from "the birds just did not turn
/// up".
///
/// # Why this is a named function and not the `if` it used to be
///
/// It was written inline as `enabled && !is_effective_at(confidence)`, in a
/// branch whose only effect is a log line — so cargo-mutants could flip the
/// `&&` to `||` (warn always) or drop the `!` (warn exactly when the feature
/// *is* working) and no test could see it. Both survived a full CI run.
///
/// Naming it also removes a redundancy that the inline form hid.
/// `is_effective_at` is itself `enabled && min < threshold`, so the leading
/// `enabled &&` is doing less than it appears: the whole expression reduces to
/// `enabled && min >= threshold`. The truth table in the tests states all four
/// combinations rather than leaving that to be re-derived.
fn is_enabled_but_inert(
    config: birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig,
    global_confidence: f32,
) -> bool {
    config.enabled && !config.is_effective_at(global_confidence)
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

    let sensitivity = resolve_sensitivity(config);
    let confidence = resolve_confidence(config);

    // The classifier applies its own threshold before the pipeline sees a
    // detection, so a lowered per-species threshold reaches nothing unless the
    // model is told to run at the floor. Getting this wrong is silent: the
    // feature would be on, configured, and reach zero detections, because the
    // ones it exists to recover were discarded inside the model.
    let dynamic_config = crate::daemon::config::resolve_dynamic_threshold(config);
    let model_confidence = dynamic_config.model_floor(confidence);
    if is_enabled_but_inert(dynamic_config, confidence) {
        tracing::warn!(
            floor = dynamic_config.min,
            global = confidence,
            "dynamic thresholds are enabled but the floor is at or above the global \
             threshold, so no species can be adjusted. Lower BIRDNET_DYNAMIC_THRESHOLD_MIN \
             or raise the global confidence."
        );
    }

    let metadata_model_path = cli
        .metadata_model
        .clone()
        .or_else(|| config.and_then(|c| c.get("METADATA_MODEL_PATH").map(PathBuf::from)));

    let metadata_labels_path = cli
        .metadata_labels
        .clone()
        .or_else(|| config.and_then(|c| c.get("METADATA_LABELS_PATH").map(PathBuf::from)));

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

    // `0` is both the flag default and a meaningful value ("disabled"), so
    // the config key is consulted only when the flag was left alone. Passing
    // the default in rather than comparing here keeps the one comparison in
    // `resolve_i64_with_default`, where a unit test can reach it.
    let duplicate_interval_secs = crate::daemon::config::resolve_i64_with_default(
        cli.duplicate_interval_secs,
        0,
        config.and_then(|c| c.get_parsed::<i64>("DUPLICATE_INTERVAL_SECS").ok()),
    );

    let noise_threshold = resolve_f32_with_default(
        cli.noise_threshold,
        0.0,
        config.and_then(|c| c.get_parsed::<f32>("NOISE_THRESHOLD").ok()),
    );
    let noise_classes = crate::daemon::config::resolve_noise_classes(
        cli.noise_classes.as_deref(),
        config.and_then(|c| c.get("NOISE_CLASSES")),
    );

    let (confirmation, confirmation_warning) = resolve_confirmation_level(
        &cli.confirmation_level,
        "off",
        config.and_then(|c| c.get("CONFIRMATION_LEVEL")),
    );
    if let Some(warning) = confirmation_warning {
        tracing::warn!("{warning}");
    }

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

    // The operator's include/exclude lists, read through the same function the
    // /admin/species page uses so the two cannot drift.
    let species_lists =
        birdnet_web::routes::admin::species::handler::configured_species_lists(&state);
    if let Some((include, exclude)) = species_lists_log_counts(&species_lists) {
        tracing::info!(include, exclude, "loaded operator species filter lists");
    }

    // Re-read on a TTL inside the daemon loop so excluding a species takes
    // effect on the next processed file rather than the next restart.
    let species_lists_provider = {
        let state = state.clone();
        birdnet_core::inference::species_filter::SpeciesListsProvider::new(move || {
            birdnet_web::routes::admin::species::handler::configured_species_lists(&state)
        })
    };

    let (latitude, longitude) = resolve_station_coords(cli, config);

    let daylight = config::build_daylight_filter(cli, config, latitude, longitude);

    let daemon_config = birdnet_core::detection::daemon::DaemonConfig {
        watch_dir: watch_dir.clone(),
        model_path,
        labels_path,
        pipeline: build_pipeline_config(watch_dir, overlap),
        model: build_model_config(sensitivity, model_confidence),
        process_existing: cli.process_existing,
        metadata_model_path,
        metadata_labels_path,
        // Publish the occurrence filter's real state to Prometheus. This is
        // the number that would have made the inert-filter defect visible from
        // a dashboard instead of from reading the code: 0 means every species
        // the classifier knows is a candidate, wherever the station is.
        on_species_filter_state: Some(birdnet_core::detection::daemon::SpeciesFilterObserver::new(
            {
                let metrics = state.metrics();
                move |active, candidates| metrics.set_occurrence_filter(active, candidates)
            },
        )),
        species_filter: build_species_filter_config(sf_thresh, species_lists),
        species_lists_provider: Some(species_lists_provider),
        privacy_threshold,
        noise_threshold,
        noise_classes,
        confirmation,
        latitude,
        longitude,
        species_thresholds,
    };

    let (event_tx, event_rx) = mpsc::sync_channel(DETECTION_EVENT_CHANNEL_CAP);

    let thresholds_for_processor = daemon_config.species_thresholds.clone();
    let global_confidence = confidence;

    // Extract clips into the SAME dir the web serves recordings from
    // (AppState::recording_dir) — one source of truth — so clips persist on the
    // data disk and are found by the Recordings page and playback. They used to
    // land in watch_dir.parent()/Extracted (the transient tmpfs), which vanished
    // on every restart and never matched where the app reads (Bug B).
    let recordings_dir = state.recording_dir();
    let extractor = Extractor::new(build_extraction_config(cli, config, &recordings_dir));

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
                    mqtt,
                    notification_filter,
                    notification_template,
                    rt_handle,
                    thresholds_for_processor,
                    global_confidence,
                    extractor,
                    duplicate_interval_secs,
                    daylight,
                    birdnet_core::detection::dynamic_threshold::DynamicThresholds::new(
                        dynamic_config,
                    ),
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

    /// The full truth table for the "enabled but inert" warning.
    ///
    /// Both of this branch's mutants — `&&` to `||`, and dropping the `!` —
    /// survived a complete CI run, because the branch only writes a log line
    /// and nothing observed it. Stating all four combinations is what makes
    /// each one visible:
    ///
    /// | enabled | floor below the global threshold | warn? |
    /// |---------|----------------------------------|-------|
    /// | no      | —                                | no    |
    /// | yes     | yes (the feature works)          | no    |
    /// | yes     | no, equal                        | yes   |
    /// | yes     | no, above                        | yes   |
    ///
    /// `||` in place of `&&` makes the whole expression a tautology and warns
    /// on row 1; dropping the `!` inverts rows 2 and 3.
    #[test]
    fn the_inert_warning_fires_only_when_the_feature_is_on_and_cannot_act() {
        use birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig;
        const GLOBAL: f32 = 0.7;
        let cfg = |enabled: bool, min: f32| DynamicThresholdConfig {
            enabled,
            trigger: 0.9,
            min,
            valid_hours: 24,
        };

        assert!(
            !is_enabled_but_inert(cfg(false, 0.3), GLOBAL),
            "the feature is off; there is nothing to warn about"
        );
        assert!(
            !is_enabled_but_inert(cfg(false, 0.9), GLOBAL),
            "still off, even with a floor that would be inert if it were on"
        );
        assert!(
            !is_enabled_but_inert(cfg(true, 0.3), GLOBAL),
            "on, and the floor is below the global threshold — this is the \
             working configuration and must stay silent"
        );
        assert!(
            is_enabled_but_inert(cfg(true, GLOBAL), GLOBAL),
            "a floor exactly at the global threshold adjusts nothing: `min < \
             threshold` is false, so this must warn"
        );
        assert!(
            is_enabled_but_inert(cfg(true, 0.9), GLOBAL),
            "and a floor above it certainly must"
        );
    }
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
            &cli, None, state, broadcast, None, None, None, None, filter, template,
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
            &cli, None, state, broadcast, None, None, None, None, filter, template,
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
            &cli, None, state, broadcast, None, None, None, None, filter, template,
        );
        assert!(
            handle.is_none(),
            "daemon must return None when model/labels/watch_dir all unset"
        );
    }
}
