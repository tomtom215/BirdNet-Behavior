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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use birdnet_core::audio::extraction::Extractor;
use birdnet_core::inference::model_state::{ClassifierState, DetectorState, FilterState};
use birdnet_core::inference::registry::ModelSpec;
use birdnet_integrations::notification::{NotificationFilter, NotificationTemplate};

use crate::cli::Cli;
use crate::integrations::{AppriseHandle, EmailHandle, MqttHandle};

use config::{
    build_extraction_config, build_model_config, build_pipeline_config,
    build_species_filter_config, resolve_required_paths, resolve_sensitivity,
    species_lists_log_counts, species_thresholds_log_count, startup_species_thresholds,
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
    clip_format, resolve_confidence, resolve_confirmation_level, resolve_f32_with_default,
    resolve_i64_with_default, resolve_station_coords,
};
mod alert_cooldown;
mod alert_rules;
mod daylight;
pub mod disposition;
mod duplicate;
mod local_offset;
mod occurrence;
mod processor;
pub mod repair;
mod run;
mod spool;

// Under `tests/` so the repository's source scans read it as test code.
#[cfg(test)]
#[path = "tests/replay.rs"]
mod replay_tests;
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

/// `(loaded, floor)`: the threshold the classifiers are loaded with, and the
/// floor they run at.
///
/// `loaded` becomes each classifier's "own bar" when it has no
/// `MODEL_n_THRESHOLD`, and it is the station's: the merge ranks "cleared"
/// detections by it, the Models board shows it, an answer's "below its own
/// bar" is judged against it, and acceptance uses the station's too. It was the
/// dynamic floor, so all three disagreed with acceptance (audit MMP-07).
/// `floor` is lower with dynamic thresholds on; the processor publishes it as
/// the [`ThresholdFloor`] and the inference loop lowers every classifier to it
/// before each file, so the model still runs where a learned species can reach.
///
/// [`ThresholdFloor`]: birdnet_core::detection::daemon::ThresholdFloor
const fn classifier_thresholds(
    dynamic: birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig,
    station: f32,
) -> (f32, f32) {
    (station, dynamic.model_floor(station))
}

/// What the daemon reports about its own throughput (OB-12, PR-1, PR-2),
/// mapped onto the metrics registry. The source label is derived here
/// because `derive_source_label` is the one place that knows the convention.
fn throughput_observer(
    state: &birdnet_web::state::AppState,
) -> birdnet_core::detection::daemon::ThroughputObserver {
    birdnet_core::detection::daemon::ThroughputObserver::new({
                let state = state.clone();
                move |path| {
                    let metrics = state.metrics();
                    let source = crate::daemon::disposition::derive_source_label(path);
                    metrics.inc_file_analysed(&source);
                    // From the header, not by decoding again: the file is
                    // still claimed here, and this is the data rate.
                    if let Some(secs) = birdnet_core::audio::decode::probe_duration_secs(path) {
                        metrics.add_audio_analysed(&source, secs);
                    }
                }
            })
            // A segment gone before the pipeline read it (PR-1 / S-3): the
            // counter that used to be a log line identical to the healthy case.
            .with_dropped({
                let metrics = state.metrics();
                move |path| {
                    metrics.inc_segment_dropped(&crate::daemon::disposition::derive_source_label(
                        path,
                    ));
                }
            })
            // The queue's depth and what the shed policy skipped (PR-2).
            .with_queue_depth({
                let metrics = state.metrics();
                move |depth| metrics.set_analysis_queue_depth(depth)
            })
            .with_shed({
                let metrics = state.metrics();
                move |_path, reason| metrics.inc_segment_shed(reason.as_str())
            })
            // Each classifier's run of failed windows (`e853776`), for the
            // `classifiers` condition, the health endpoint and the exposition.
            .with_model_health({
                let metrics = state.metrics();
                move |health| metrics.set_model_health(health)
            })
            // What each classifier is, once they have loaded: the Models board
            // on Station Health states these instead of leaving them to the
            // config file.
            .with_model_info({
                let metrics = state.metrics();
                move |info| metrics.set_model_info(info)
            })
            // What each file's filters discarded, by reason (ACC-02): the
            // counters, and the day's tally per source in `filter_discards`.
            .with_filtered({
                let state = state.clone();
                move |path, tally| {
                    // The effort ledger's denominator: only audio analysed is
                    // listening (RES-01), and a window no classifier answered
                    // was not analysed (review F18).
                    // The audio that decoded, not what the header claims
                    // (review F10); the header only where that is not known.
                    let secs = if tally.decoded_ms > 0 {
                        Some(decoded_secs(tally))
                    } else {
                        birdnet_core::audio::decode::probe_duration_secs(path)
                    };
                    if let Some(secs) = secs {
                        crate::integrations::effort::note_analysed(
                            &state,
                            path,
                            secs * answered_share(tally),
                        );
                    }
                    record_discards(&state, path, tally);
                }
            })
}

/// The share of a file's windows some classifier answered: `1.0` for a file
/// with none failed or no windows counted, which is every file the daemon
/// analysed before failed windows were kept (review F18).
#[allow(
    clippy::cast_precision_loss,
    reason = "window counts are far below 2^52"
)]
fn answered_share(tally: &birdnet_core::detection::FilterTally) -> f64 {
    if tally.windows == 0 || tally.failed_windows == 0 {
        return 1.0;
    }
    tally.windows.saturating_sub(tally.failed_windows) as f64 / tally.windows as f64
}

/// [`birdnet_core::detection::FilterTally::decoded_ms`] in seconds.
#[allow(
    clippy::cast_precision_loss,
    reason = "milliseconds of one recording are far below 2^52"
)]
fn decoded_secs(tally: &birdnet_core::detection::FilterTally) -> f64 {
    tally.decoded_ms as f64 / 1000.0
}

/// The segments whose speech the human-sound rule masked (RES-28): marked
/// here as each file is analysed, read by the stream drain, which keeps no
/// raw copy of them. One set for the process, because the daemon and the disk
/// manager are started apart.
pub fn speech_marks() -> &'static birdnet_core::audio::capture::SpeechMarks {
    static MARKS: std::sync::OnceLock<birdnet_core::audio::capture::SpeechMarks> =
        std::sync::OnceLock::new();
    MARKS.get_or_init(birdnet_core::audio::capture::SpeechMarks::default)
}

/// Count and keep what the filters discarded from one analysed file (ACC-02),
/// and mark it for the raw keep if speech was masked in it (RES-28).
fn record_discards(
    state: &birdnet_web::state::AppState,
    path: &std::path::Path,
    tally: &birdnet_core::detection::FilterTally,
) {
    if tally.privacy_windows > 0 {
        speech_marks().mark(path);
    }
    if !tally.discarded_anything() {
        return;
    }
    let metrics = state.metrics();
    let rows = tally.by_reason();
    for (reason, windows, detections) in rows {
        metrics.add_windows_masked(reason, windows);
        metrics.add_detections_dropped(reason, detections);
    }
    if tally.date.is_empty() {
        return;
    }
    let source = crate::daemon::disposition::derive_source_label(path);
    let written = state.with_ingest_db(|conn| {
        for (reason, windows, detections) in rows {
            birdnet_db::sqlite::add_discards(
                conn,
                &tally.date,
                &source,
                reason,
                windows,
                detections,
            )?;
        }
        Ok::<(), birdnet_db::sqlite::DbError>(())
    });
    if let Some(Err(e)) = written {
        tracing::warn!(error = %e, file = %path.display(), "could not record the filters' discards");
    }
}

/// Whether the board is at the limit the shed policy backs off at: the CPU
/// at or above the thermal-alert line, or the firmware throttling now.
///
/// The sensor reads stay in [`shed_policy`]'s closure; the decision is here
/// so it can be stated as a table. Both of its operators survived a full
/// mutation run inside the closure (`>=` for `<`, `||` for `&&`) because
/// nothing could call a closure that reads the real sensor.
fn thermal_limit_reached(cpu_temp_c: Option<f32>, throttled_now: bool) -> bool {
    cpu_temp_c.is_some_and(|t| t >= crate::integrations::THERMAL_ALERT_C) || throttled_now
}

/// One segment in two while the queue is deep or the board is at its thermal
/// limit (PR-2); the thermal condition and this read the same sensor and the
/// same line, so the notification and the shed agree.
fn shed_policy() -> birdnet_core::detection::daemon::ShedPolicy {
    birdnet_core::detection::daemon::ShedPolicy::new(
        birdnet_core::detection::daemon::DEFAULT_SHED_BACKLOG_ABOVE,
    )
    .with_thermal(|| {
        thermal_limit_reached(
            birdnet_web::system_info::cpu_temperature(),
            birdnet_web::system_info::pi_throttled()
                .is_some_and(birdnet_web::system_info::PiThrottle::throttled_now),
        )
    })
}

/// The plan's classifiers split for the daemon: the primary (always first),
/// the rest, and each one's own threshold by id — the processor holds a
/// detection to the threshold of the classifier that made it where one is set.
fn split_plan(
    specs: Vec<ModelSpec>,
    model_path: &std::path::Path,
    labels_path: &std::path::Path,
) -> (
    ModelSpec,
    Vec<ModelSpec>,
    std::collections::HashMap<String, f32>,
) {
    let thresholds = specs
        .iter()
        .filter_map(|s| s.threshold.map(|t| (s.id.clone(), t)))
        .collect();
    let mut specs = specs.into_iter();
    // `plan` always puts the primary first; an empty plan is the defaults.
    let primary = specs.next().unwrap_or_else(|| ModelSpec {
        id: "birdnet".to_owned(),
        model_path: model_path.to_path_buf(),
        labels_path: labels_path.to_path_buf(),
        threshold: None,
        sample_rate: None,
        output_scale: None,
    });
    (primary, specs.collect(), thresholds)
}

/// Where the daemon's judged detections go: the live broadcast, and the
/// clients that deliver and upload them.
pub struct Delivery {
    /// The dashboard's live detection broadcast.
    pub broadcast: birdnet_web::routes::websocket::DetectionBroadcast,
    /// Apprise notifications.
    pub apprise: Option<AppriseHandle>,
    /// `BirdWeather` uploads.
    pub birdweather: Option<birdnet_integrations::birdweather::Client>,
    /// Email notifications.
    pub email: Option<EmailHandle>,
    /// MQTT publishing.
    pub mqtt: Option<MqttHandle>,
    /// Which detections notify.
    pub notification_filter: NotificationFilter,
    /// What a notification says.
    pub notification_template: NotificationTemplate,
}

/// Start the detection daemon in a background thread.
///
/// Returns the daemon handle, or `None` if the model/labels are not configured.
///
/// Every non-trivial decision lives in a pure helper at the top of this file
/// with dedicated unit-test coverage, and each stage of the start-up is a
/// function of its own below: the settings the detector runs at, the
/// classifiers this machine can run, the occurrence source the web layer
/// reads, the daemon's configuration, and the processor that consumes it.
pub fn start_detection_daemon(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: birdnet_web::state::AppState,
    delivery: Delivery,
    in_flight: birdnet_core::detection::daemon::InFlight,
) -> Option<birdnet_core::detection::daemon::DaemonHandle> {
    // Told to the web layer before anything can stop the daemon from starting:
    // the Station Health page scans this folder for recordings whose names the
    // daemon cannot read, and the Capture tab says where it is, which matters
    // most on a station whose model is not installed yet.
    if let Some(dir) = cli
        .watch_dir
        .clone()
        .or_else(|| config.and_then(|c| c.get("RECS_DIR").map(std::path::PathBuf::from)))
    {
        state.set_watch_dir(dir);
    }
    let Some((model_path, labels_path, watch_dir)) = resolve_required_paths(cli, config) else {
        state
            .metrics()
            .set_detector_state(DetectorState::NotConfigured {
                why: not_configured_why(cli, config),
            });
        tracing::warn!(
            "detection daemon NOT started: model, labels, or watch_dir not configured — the web \
             UI will run but NO audio will be analysed and NO detections recorded"
        );
        tracing::warn!(
            "open the station in a browser and install the classifier from the setup page \
             (Settings → Capture), then restart it from there; or set --model, --labels and \
             --watch-dir (MODEL_PATH, LABELS_PATH and RECS_DIR in the config file)"
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

    let settings = Settings::resolve(cli, config, &state);
    let planned = plan_classifiers(config, &state, &model_path, &labels_path);
    let paths = Paths {
        watch_dir,
        model_path,
        labels_path,
    };
    let extraction = extraction_config(cli, config, &state, &settings, &planned.privacy_rules);
    let model_thresholds = planned.model_thresholds.clone();
    let daemon_config = daemon_config(
        cli,
        &state,
        paths,
        &settings,
        planned,
        in_flight,
        extraction.clip_reach(),
    );
    run_and_process(
        state,
        daemon_config,
        settings,
        delivery,
        Processing {
            extractor: Extractor::new(extraction),
            model_thresholds,
        },
    )
}

/// Where the daemon reads from: the watch folder, and the primary
/// classifier's model and labels.
struct Paths {
    watch_dir: PathBuf,
    model_path: PathBuf,
    labels_path: PathBuf,
}

/// The settings the detector runs at, each resolved once from the command
/// line, the configuration file and the settings table.
struct Settings {
    sensitivity: f32,
    /// The station's confidence threshold.
    confidence: f32,
    dynamic: birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig,
    /// The threshold the classifiers are loaded with ([`classifier_thresholds`]).
    load_confidence: f32,
    /// The floor they run at.
    model_confidence: f32,
    metadata_model_path: Option<PathBuf>,
    metadata_labels_path: Option<PathBuf>,
    species_aliases_path: Option<PathBuf>,
    sf_thresh: f32,
    privacy_threshold: f32,
    privacy_rank: usize,
    duplicate_interval_secs: i64,
    noise_threshold: f32,
    noise_classes: Vec<String>,
    noise_remember_secs: f32,
    confirmation: birdnet_core::detection::corroboration::ConfirmationLevel,
    overlap: f32,
    /// `OCCURRENCE_WEEK` (review F13).
    occurrence_week: birdnet_core::detection::pipeline::OccurrenceWeek,
    /// `ANALYSIS_CHANNELS` (review F12).
    channels: birdnet_core::detection::pipeline::ChannelAnalysis,
    species_thresholds: std::collections::HashMap<String, f64>,
    species_lists: birdnet_core::inference::species_filter::SpeciesLists,
    species_lists_provider: birdnet_core::inference::species_filter::SpeciesListsProvider,
    latitude: Option<f64>,
    longitude: Option<f64>,
    daylight: crate::daemon::daylight::DaylightFilter,
}

impl Settings {
    /// Resolve every setting, logging what an operator should know about.
    fn resolve(
        cli: &Cli,
        config: Option<&birdnet_core::config::Config>,
        state: &birdnet_web::state::AppState,
    ) -> Self {
        let confidence = resolve_confidence(config);
        // The classifier applies its own threshold before the pipeline sees a
        // detection, so a lowered per-species threshold reaches nothing unless
        // the model is told to run at the floor. Getting this wrong is silent:
        // the feature would be on, configured, and reach zero detections,
        // because the ones it exists to recover were discarded inside the model.
        let dynamic = crate::daemon::config::resolve_dynamic_threshold(config);
        let (load_confidence, model_confidence) = classifier_thresholds(dynamic, confidence);
        if is_enabled_but_inert(dynamic, confidence) {
            tracing::warn!(
                floor = dynamic.min,
                global = confidence,
                "dynamic thresholds are enabled but the floor is at or above the global \
                 threshold, so no species can be adjusted. Lower BIRDNET_DYNAMIC_THRESHOLD_MIN \
                 or raise the global confidence."
            );
        }
        let (confirmation, confirmation_warning) = resolve_confirmation_level(
            &cli.confirmation_level,
            "off",
            config.and_then(|c| c.get("CONFIRMATION_LEVEL")),
        );
        if let Some(warning) = confirmation_warning {
            tracing::warn!("{warning}");
        }
        let (species_thresholds, species_lists, species_lists_provider) = species_settings(state);
        let (latitude, longitude) = resolve_station_coords(cli, config);
        Self {
            sensitivity: resolve_sensitivity(config),
            confidence,
            dynamic,
            load_confidence,
            model_confidence,
            metadata_model_path: cli
                .metadata_model
                .clone()
                .or_else(|| config.and_then(|c| c.get("METADATA_MODEL_PATH").map(PathBuf::from))),
            metadata_labels_path: cli
                .metadata_labels
                .clone()
                .or_else(|| config.and_then(|c| c.get("METADATA_LABELS_PATH").map(PathBuf::from))),
            // No CLI flag: this is a file an operator curates over months, not
            // something worth typing at a one-off invocation.
            species_aliases_path: config
                .and_then(|c| c.get("SPECIES_ALIASES_PATH").map(PathBuf::from)),
            sf_thresh: resolve_f32_with_default(
                cli.sf_thresh,
                0.03,
                config.and_then(|c| c.get_parsed::<f32>("SF_THRESH").ok()),
            ),
            privacy_threshold: resolve_f32_with_default(
                cli.privacy_threshold,
                0.0,
                config.and_then(|c| c.get_parsed::<f32>("PRIVACY_THRESHOLD").ok()),
            ),
            privacy_rank: privacy_rank(cli, config),
            // `0` is both the flag default and a meaningful value ("disabled"),
            // so the config key is consulted only when the flag was left alone.
            // Passing the default in rather than comparing here keeps the one
            // comparison in `resolve_i64_with_default`, where a unit test can
            // reach it.
            duplicate_interval_secs: crate::daemon::config::resolve_i64_with_default(
                cli.duplicate_interval_secs,
                0,
                config.and_then(|c| c.get_parsed::<i64>("DUPLICATE_INTERVAL_SECS").ok()),
            ),
            noise_threshold: resolve_f32_with_default(
                cli.noise_threshold,
                0.0,
                config.and_then(|c| c.get_parsed::<f32>("NOISE_THRESHOLD").ok()),
            ),
            noise_classes: crate::daemon::config::resolve_noise_classes(
                cli.noise_classes.as_deref(),
                config.and_then(|c| c.get("NOISE_CLASSES")),
            ),
            // Off unless asked for: the window removes real detections whenever
            // a real bird happens to be the species a dog resembles, and that is
            // a trade an operator should make knowingly.
            noise_remember_secs: config
                .and_then(|c| c.get_parsed::<f32>("NOISE_REMEMBER_SECS").ok())
                .unwrap_or(0.0),
            confirmation,
            overlap: resolve_f32_with_default(
                cli.overlap,
                0.0,
                config.and_then(|c| c.get_parsed::<f32>("OVERLAP").ok()),
            ),
            occurrence_week: config::resolve_occurrence_week(config),
            channels: config::resolve_channel_analysis(config),
            species_thresholds,
            species_lists,
            species_lists_provider,
            latitude,
            longitude,
            daylight: config::build_daylight_filter(cli, config, latitude, longitude),
        }
    }
}

/// `PRIVACY_RANK`: `10` is the flag default and `0` ("off") is a meaningful
/// value, so the config key is consulted only when the flag was left alone.
fn privacy_rank(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> usize {
    usize::try_from(
        crate::daemon::config::resolve_i64_with_default(
            i64::from(cli.privacy_rank),
            i64::try_from(birdnet_core::config::DEFAULT_PRIVACY_RANK).unwrap_or(10),
            config.and_then(|c| c.get_parsed::<i64>("PRIVACY_RANK").ok()),
        )
        .max(0),
    )
    .unwrap_or(birdnet_core::config::DEFAULT_PRIVACY_RANK)
}

/// The per-species thresholds, the operator's include/exclude lists, and the
/// provider the daemon re-reads those lists through.
fn species_settings(
    state: &birdnet_web::state::AppState,
) -> (
    std::collections::HashMap<String, f64>,
    birdnet_core::inference::species_filter::SpeciesLists,
    birdnet_core::inference::species_filter::SpeciesListsProvider,
) {
    let (species_thresholds, thresholds_warning) =
        startup_species_thresholds(state.with_db(birdnet_db::sqlite::get_species_threshold_map));
    if let Some(warning) = thresholds_warning {
        tracing::warn!("{warning}");
    }
    if let Some(count) = species_thresholds_log_count(&species_thresholds) {
        tracing::info!(count, "loaded per-species confidence thresholds");
    }
    // The operator's include/exclude lists, read through the same function the
    // /admin/species page uses so the two cannot drift.
    let species_lists =
        birdnet_web::routes::admin::species::handler::configured_species_lists(state);
    if let Some((include, exclude, whitelist)) = species_lists_log_counts(&species_lists) {
        tracing::info!(
            include,
            exclude,
            whitelist,
            "loaded operator species filter lists"
        );
    }
    // Re-read on a TTL inside the daemon loop so excluding a species takes
    // effect on the next processed file rather than the next restart.
    let provider = {
        let state = state.clone();
        birdnet_core::inference::species_filter::SpeciesListsProvider::new(move || {
            birdnet_web::routes::admin::species::handler::configured_species_lists(&state)
        })
    };
    (species_thresholds, species_lists, provider)
}

/// The classifiers this machine can run, split for the daemon.
struct Planned {
    primary: ModelSpec,
    extra_models: Vec<ModelSpec>,
    model_thresholds: std::collections::HashMap<String, f32>,
    routes: std::collections::HashMap<String, Vec<String>>,
    observer: birdnet_core::detection::daemon::ModelStateObserver,
    /// Each classifier's own privacy rule (review F09).
    privacy_rules: Vec<(
        String,
        birdnet_core::detection::privacy::ClassifierPrivacyRule,
    )>,
    /// Each classifier's own noise threshold (review F09).
    noise_rules: Vec<(String, f32)>,
}

/// Which classifiers this machine can actually afford (`G-10` Stage 2). The
/// primary is always in the plan; extras are admitted only while they
/// demonstrably fit, and every refusal is logged with its arithmetic — an
/// unattended station must say why it is running one model, not just quietly
/// run one.
fn plan_classifiers(
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
    model_path: &std::path::Path,
    labels_path: &std::path::Path,
) -> Planned {
    let model_plan = crate::helpers::models::plan(
        config,
        model_path.to_path_buf(),
        labels_path.to_path_buf(),
        birdnet_behavioral::memory::detect_ceiling().map(|(mib, _)| mib),
    );
    for note in &model_plan.notes {
        tracing::info!("{note}");
    }
    // Said at WARN, not INFO (MDL-06): the station is running on fewer
    // classifiers than it was given, and the journal is where that is read.
    for left in &model_plan.left_out {
        tracing::warn!(classifier = %left.id, "{}", left.message());
    }
    let observer = model_state_observer(state, model_plan.left_out, model_plan.declared);
    let (primary, extra_models, model_thresholds) =
        split_plan(model_plan.specs, model_path, labels_path);
    Planned {
        primary,
        extra_models,
        model_thresholds,
        routes: model_plan.routes,
        observer,
        privacy_rules: model_plan.privacy_rules,
        noise_rules: model_plan.noise_rules,
    }
}

/// The daemon's configuration, and the occurrence source the web layer reads
/// published on the way.
fn daemon_config(
    cli: &Cli,
    state: &birdnet_web::state::AppState,
    paths: Paths,
    settings: &Settings,
    planned: Planned,
    in_flight: birdnet_core::detection::daemon::InFlight,
    privacy_clip_reach: birdnet_core::detection::privacy::ClipReach,
) -> birdnet_core::detection::daemon::DaemonConfig {
    // The floor the inference loop runs every classifier at or below: the
    // model confidence, lowered to any per-species threshold under it, and kept
    // current by the processor as the operator changes thresholds. Seeded from
    // the thresholds already loaded, so the first files — analysed before the
    // processor thread is up to publish — are not judged by the model alone.
    let threshold_floor =
        std::sync::Arc::new(birdnet_core::detection::daemon::ThresholdFloor::new(
            processor::lowest_threshold(settings.model_confidence, &settings.species_thresholds),
        ));
    publish_occurrence_source(state, settings, &paths.labels_path, &planned);
    let Paths {
        watch_dir,
        model_path,
        labels_path,
    } = paths;
    let Planned {
        primary,
        extra_models,
        routes,
        observer,
        privacy_rules,
        noise_rules,
        ..
    } = planned;
    birdnet_core::detection::daemon::DaemonConfig {
        watch_dir: watch_dir.clone(),
        model_path,
        labels_path,
        // The primary is built by the daemon from `model_path`/`labels_path`
        // above and the plan's four settings for it below.
        extra_models,
        primary_id: primary.id,
        primary_threshold: primary.threshold,
        primary_sample_rate: primary.sample_rate,
        primary_output_scale: primary.output_scale,
        model_routes: routes,
        pipeline: birdnet_core::detection::pipeline::PipelineConfig {
            occurrence_week: settings.occurrence_week,
            channels: settings.channels,
            ..build_pipeline_config(watch_dir, settings.overlap)
        },
        model: build_model_config(settings.sensitivity, settings.load_confidence),
        process_existing: cli.process_existing,
        metadata_model_path: settings.metadata_model_path.clone(),
        metadata_labels_path: settings.metadata_labels_path.clone(),
        species_aliases_path: settings.species_aliases_path.clone(),
        // What became of each classifier and of the occurrence filter, into
        // the station's published model state (MDL-01 to MDL-16).
        on_model_state: Some(observer),
        // Count what the pipeline actually gets through. Nothing did, and
        // without it a station whose model answers nothing is indistinguishable
        // from one whose pipeline is not running: every other series is
        // downstream of a prediction the model made, so both are flat and
        // empty. The label is derived here rather than in `birdnet-core`
        // because `derive_source_label` is the one place that knows the
        // convention.
        on_file_analysed: Some(throughput_observer(state)),
        // The stream directory's purge honours these claims (PR-1 / S-3).
        in_flight: Some(in_flight),
        // One segment in two while the queue is deep or the board is at its
        // thermal limit (PR-2); the thermal condition and this read the same
        // sensor and the same line, so the notification and the shed agree.
        shed: Some(shed_policy()),
        species_filter: build_species_filter_config(
            settings.sf_thresh,
            settings.species_lists.clone(),
        ),
        species_lists_provider: Some(settings.species_lists_provider.clone()),
        privacy_threshold: settings.privacy_threshold,
        privacy_rank: settings.privacy_rank,
        privacy_clip_reach,
        noise_threshold: settings.noise_threshold,
        noise_classes: settings.noise_classes.clone(),
        classifier_privacy: privacy_rules,
        classifier_noise: noise_rules,
        noise_remember_secs: settings.noise_remember_secs,
        confirmation: settings.confirmation,
        latitude: settings.latitude,
        longitude: settings.longitude,
        species_thresholds: settings.species_thresholds.clone(),
        threshold_floor: Some(threshold_floor),
    }
}

/// How clips are cut. Extracted into the SAME dir the web serves recordings
/// from (`AppState::recording_dir`) — one source of truth — so clips persist on
/// the data disk and are found by the Recordings page and playback (Bug B).
/// With the privacy filter on, a clip stays inside its own segment, and the
/// filter is told how far a clip reaches so it can clear the whole of it
/// (PIPE7); "on" is the filter's own rule, so clips are kept in their segment
/// exactly when the filter runs: a classifier's own rule (review F09) turns it
/// on as the station's does, so it counts here too.
fn extraction_config(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
    settings: &Settings,
    privacy_rules: &[(
        String,
        birdnet_core::detection::privacy::ClassifierPrivacyRule,
    )],
) -> birdnet_core::audio::extraction::ExtractionConfig {
    let mut extraction = build_extraction_config(cli, config, &state.recording_dir());
    extraction.own_segment_only = privacy_rules
        .iter()
        .fold(
            birdnet_core::detection::privacy::PrivacyFilter::new(settings.privacy_threshold)
                .with_rank_cutoff(settings.privacy_rank),
            |filter, (id, rule)| filter.with_classifier_rule(id.clone(), *rule),
        )
        .is_enabled();
    extraction
}

/// The web crate cannot reach the daemon's own occurrence filter: it lives on
/// the detection thread, behind an ONNX session that needs `&mut` to run. It
/// is given a source that builds the same filter, lazily, from the same
/// files, threshold and lists, so the Species page and the API can say why a
/// bird is or is not admitted. Nothing is loaded until somebody asks.
fn publish_occurrence_source(
    state: &birdnet_web::state::AppState,
    settings: &Settings,
    labels_path: &std::path::Path,
    planned: &Planned,
) {
    state.set_occurrence_source(std::sync::Arc::new(
        occurrence::DaemonOccurrence::new(
            settings.latitude.zip(settings.longitude),
            settings
                .metadata_model_path
                .clone()
                .filter(|m| !birdnet_core::inference::model_state::occurrence_filter_turned_off(m))
                .map(|model| occurrence::RangeModelFiles {
                    model,
                    labels: settings.metadata_labels_path.clone(),
                    aliases: settings.species_aliases_path.clone(),
                }),
            labels_path.to_path_buf(),
            build_species_filter_config(settings.sf_thresh, settings.species_lists.clone()),
            settings.species_lists_provider.clone(),
        )
        .with_classifiers(
            &planned.primary.id,
            planned
                .extra_models
                .iter()
                .map(|m| (m.id.clone(), m.labels_path.clone()))
                .collect(),
        ),
    ));
}

/// Start the daemon on `daemon_config` and, once it runs, the event processor
/// that consumes what it detects; publish which happened.
fn run_and_process(
    state: birdnet_web::state::AppState,
    daemon_config: birdnet_core::detection::daemon::DaemonConfig,
    settings: Settings,
    delivery: Delivery,
    processing: Processing,
) -> Option<birdnet_core::detection::daemon::DaemonHandle> {
    let (event_tx, event_rx) = mpsc::sync_channel(DETECTION_EVENT_CHANNEL_CAP);
    match birdnet_core::detection::daemon::run_daemon(&daemon_config, event_tx) {
        Ok(handle) => {
            tracing::info!("detection daemon started");
            state.metrics().set_detector_state(DetectorState::Running);
            let manifest = run_manifest(
                &state,
                &daemon_config,
                &settings,
                processing.model_thresholds,
            );
            let floor = daemon_config.threshold_floor.clone();
            let extractor = processing.extractor;
            let rt_handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                // No run, no rows: returning here drops the receiver, the
                // daemon stops on its next send, and `?strict=1` says so.
                let Some(provenance) = run::provenance_for(&state, &manifest) else {
                    return;
                };
                event_processor(
                    event_rx,
                    state,
                    delivery.broadcast,
                    delivery.apprise,
                    delivery.birdweather,
                    delivery.email,
                    delivery.mqtt,
                    delivery.notification_filter,
                    delivery.notification_template,
                    rt_handle,
                    daemon_config.species_thresholds,
                    settings.confidence,
                    extractor,
                    settings.duplicate_interval_secs,
                    settings.daylight,
                    birdnet_core::detection::dynamic_threshold::DynamicThresholds::new(
                        settings.dynamic,
                    ),
                    provenance,
                    floor.map(|floor| processor::PublishedFloor {
                        floor,
                        model: settings.model_confidence,
                        station: settings.confidence,
                    }),
                );
            });
            Some(handle)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to start detection daemon");
            state
                .metrics()
                .set_detector_state(DetectorState::Failed { why: e.to_string() });
            None
        }
    }
}

/// What the processor needs besides the daemon's configuration: how it cuts
/// clips, and each classifier's own threshold for the run record.
struct Processing {
    extractor: Extractor,
    model_thresholds: std::collections::HashMap<String, f32>,
}

/// What every row this run writes will say about the station, the settings
/// (R-2 / UP-1) and the model (R-1) that produced it. The settings are the
/// same resolved values the model and the daylight filter use, so the row and
/// the run cannot disagree; the model is hashed and written to
/// `analysis_runs` on the processor thread before it consumes an event —
/// paths only here, because the hashing reads 541 MB and does not belong on
/// the async runtime.
///
/// The run records the geomodel it filters with, and only that one: a filter
/// that is off — its model missing or damaged, its labels missing, the station
/// without a position — filters nothing, and a missing file made the run
/// impossible to register, which stopped every detection being recorded
/// (MDL-05). So it is read after the daemon has said whether the filter runs.
fn run_manifest(
    state: &birdnet_web::state::AppState,
    daemon_config: &birdnet_core::detection::daemon::DaemonConfig,
    settings: &Settings,
    model_thresholds: std::collections::HashMap<String, f32>,
) -> run::RunManifest {
    run::RunManifest {
        model_path: daemon_config.model_path.clone(),
        labels_path: daemon_config.labels_path.clone(),
        geomodel_path: state
            .models_state()
            .occurrence_filter
            .is_active()
            .then(|| daemon_config.metadata_model_path.clone())
            .flatten(),
        app_version: env!("CARGO_PKG_VERSION"),
        confidence: f64::from(settings.confidence),
        sensitivity: f64::from(settings.sensitivity),
        overlap: f64::from(settings.overlap),
        sf_thresh: f64::from(settings.sf_thresh),
        lat: settings.latitude,
        lon: settings.longitude,
        model_thresholds,
        primary_id: daemon_config.primary_id.clone(),
        extra_classifiers: daemon_config
            .extra_models
            .iter()
            .map(|m| (m.id.clone(), m.model_path.clone(), m.labels_path.clone()))
            .collect(),
    }
}

/// Why the detector is not set up, naming each setting that is missing.
fn not_configured_why(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> String {
    let set = |flag: Option<&PathBuf>, key: &str| {
        flag.is_some() || config.and_then(|c| c.get(key)).is_some()
    };
    let missing: Vec<&str> = [
        (
            set(cli.model.as_ref(), "MODEL_PATH"),
            "the classifier (MODEL_PATH)",
        ),
        (
            set(cli.labels.as_ref(), "LABELS_PATH"),
            "its labels (LABELS_PATH)",
        ),
        (
            set(cli.watch_dir.as_ref(), "RECS_DIR"),
            "the recordings folder (RECS_DIR)",
        ),
    ]
    .into_iter()
    .filter_map(|(present, what)| (!present).then_some(what))
    .collect();
    let (list, verb) = match missing.as_slice() {
        [one] => ((*one).to_owned(), "is"),
        [rest @ .., last] => (format!("{} and {last}", rest.join(", ")), "are"),
        // Not reached: this is asked only when one of them is missing.
        [] => ("one of them".to_owned(), "is"),
    };
    format!(
        "{list} {verb} not configured. Install the classifier from the setup page (Settings → \
         Capture) and restart from there, or set them in the configuration file"
    )
}

/// Where the daemon's report of its models goes: the station's published
/// model state (MDL-01 to MDL-16).
///
/// The registry reports the classifiers it was handed; `planned_out` holds
/// the ones the plan did not hand it (no labels setting, an unreadable file,
/// no memory to spare). Each report is the two together, in the order the
/// configuration declares them, so the published list is every classifier
/// the configuration names, whichever step left it out.
fn model_state_observer(
    state: &birdnet_web::state::AppState,
    planned_out: Vec<ClassifierState>,
    declared: Vec<String>,
) -> birdnet_core::detection::daemon::ModelStateObserver {
    let classifiers = {
        let metrics = state.metrics();
        move |loaded: &[ClassifierState]| {
            metrics.set_classifier_states(declared_order(loaded, &planned_out, &declared));
        }
    };
    let filter = {
        let metrics = state.metrics();
        move |filter: &FilterState| metrics.set_occurrence_filter(filter.clone())
    };
    birdnet_core::detection::daemon::ModelStateObserver::new(classifiers, filter)
}

/// The registry's states and the plan's left-out ones, in declaration order.
fn declared_order(
    loaded: &[ClassifierState],
    planned_out: &[ClassifierState],
    declared: &[String],
) -> Vec<ClassifierState> {
    let mut states: Vec<ClassifierState> = loaded.iter().chain(planned_out).cloned().collect();
    states.sort_by_key(|s| declared.iter().position(|id| *id == s.id));
    states
}

/// Keep the health endpoint's daemon flag true only while the loop thread is
/// alive (`PR-5`, `OP-2`).
///
/// `source` is [`birdnet_core::detection::daemon::DaemonHandle::running_flag`],
/// cleared by the loop thread as it returns; `target` is
/// `AppState::detection_status_flag`, which `/api/v2/health?strict=1` reads.
/// Before this the target was stored once at start-up and cleared by nothing,
/// so a daemon that died after boot went on reporting itself running for the
/// life of the process — on the one endpoint a container healthcheck or a
/// monitor is told to point at.
///
/// Polled rather than pushed because the loop thread is `birdnet-core`'s and
/// the flag is the web crate's; a poll every few seconds is far inside the
/// 15-minute alerting debounce and costs one atomic load. The task ends once
/// it has recorded the exit: a daemon is not restarted in-process, so there
/// is nothing further to mirror.
///
/// The published model state follows it down too: the detector becomes
/// [`DetectorState::Stopped`], and the classifiers and filter it ran are
/// published as not running.
pub fn mirror_liveness(
    source: Arc<AtomicBool>,
    target: Arc<AtomicBool>,
    metrics: birdnet_web::metrics::SharedMetrics,
    every: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        loop {
            tick.tick().await;
            if !source.load(Ordering::Acquire) {
                target.store(false, Ordering::Relaxed);
                metrics.set_detector_state(DetectorState::Stopped);
                tracing::error!(
                    "detection daemon thread has exited; /api/v2/health?strict=1 now reports it \
                     stopped. The web UI keeps serving; restart the service to resume detection"
                );
                return;
            }
        }
    })
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use birdnet_core::inference::model_state::ClassifierStatus;
    use birdnet_web::occurrence::Unavailable;
    use clap::Parser as _;

    fn cli(args: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("birdnet-behavior").chain(args.iter().copied()))
    }

    fn config(text: &str) -> birdnet_core::config::Config {
        birdnet_core::config::Config::parse(text).unwrap()
    }

    /// `PRIVACY_RANK`: a station nobody has customised runs BirdNET-Pi's
    /// default of 10; the config file's value is read when the flag was left
    /// alone, `0` included (it turns the rule off); a flag that was set wins;
    /// and a negative value in the file is off, not a wrapped huge rank.
    #[test]
    fn the_privacy_rank_is_the_flag_then_the_file_then_ten() {
        assert_eq!(privacy_rank(&cli(&[]), None), 10);
        assert_eq!(
            privacy_rank(&cli(&[]), Some(&config("PRIVACY_RANK=0\n"))),
            0
        );
        assert_eq!(
            privacy_rank(&cli(&[]), Some(&config("PRIVACY_RANK=25\n"))),
            25
        );
        assert_eq!(
            privacy_rank(
                &cli(&["--privacy-rank", "3"]),
                Some(&config("PRIVACY_RANK=25\n"))
            ),
            3
        );
        assert_eq!(
            privacy_rank(&cli(&[]), Some(&config("PRIVACY_RANK=-4\n"))),
            0
        );
    }

    /// Clips are cut into the folder the web serves recordings from, and stay
    /// inside their own segment exactly when either privacy rule runs (PIPE7):
    /// the default rank of 10 runs it, a rank of 0 with no score threshold
    /// does not, and a score threshold alone does.
    #[test]
    fn clips_stay_in_their_segment_exactly_when_the_privacy_filter_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        for (args, own_segment) in [
            (&[][..], true),
            (&["--privacy-rank", "0"][..], false),
            (
                &["--privacy-rank", "0", "--privacy-threshold", "0.02"][..],
                true,
            ),
        ] {
            let cli = cli(args);
            let settings = Settings::resolve(&cli, None, &state);
            let extraction = extraction_config(&cli, None, &state, &settings, &[]);
            assert_eq!(extraction.own_segment_only, own_segment, "{args:?}");
            assert_eq!(extraction.output_dir, state.recording_dir(), "{args:?}");
        }
        // Review F09: with the station's rules off, one classifier's own rule
        // runs the filter, so its clips stay in their segment too.
        let cli = cli(&["--privacy-rank", "0"]);
        let settings = Settings::resolve(&cli, None, &state);
        let rule = birdnet_core::detection::privacy::ClassifierPrivacyRule {
            threshold: Some(0.4),
            rank_cutoff: None,
        };
        let extraction =
            extraction_config(&cli, None, &state, &settings, &[("perch".to_owned(), rule)]);
        assert!(
            extraction.own_segment_only,
            "a classifier's own privacy rule runs the filter"
        );
    }

    /// The configuration's per-window week, chosen channel and per-classifier
    /// privacy and noise rules (review F09, F12, F13) reach the configuration
    /// the daemon runs with, not only the settings they are read into.
    #[test]
    fn the_review_s_analysis_settings_reach_the_daemon() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let model = tmp.path().join("model.onnx");
        let labels = tmp.path().join("labels.txt");
        let config = config(
            "STATION_PROFILE=research\nANALYSIS_CHANNELS=right\nMODEL_ID=v24\n\
             MODEL_PRIVACY_THRESHOLD=0.3\nMODEL_NOISE_THRESHOLD=0.6\n",
        );
        let cli = cli(&[]);
        let settings = Settings::resolve(&cli, Some(&config), &state);
        let planned = plan_classifiers(Some(&config), &state, &model, &labels);
        let daemon = daemon_config(
            &cli,
            &state,
            Paths {
                watch_dir: tmp.path().join("recs"),
                model_path: model,
                labels_path: labels,
            },
            &settings,
            planned,
            birdnet_core::detection::daemon::InFlight::new(),
            birdnet_core::detection::privacy::ClipReach::default(),
        );
        assert_eq!(
            daemon.pipeline.occurrence_week,
            birdnet_core::detection::pipeline::OccurrenceWeek::PerWindow
        );
        assert_eq!(
            daemon.pipeline.channels,
            birdnet_core::detection::pipeline::ChannelAnalysis::Only(1)
        );
        assert_eq!(daemon.classifier_privacy.len(), 1);
        assert_eq!(daemon.classifier_privacy[0].0, "v24");
        assert_eq!(daemon.classifier_noise, [("v24".to_owned(), 0.6)]);
    }

    /// The Species page is handed the daemon's range model, so a configured
    /// one that will not load is reported as that failure; a range model
    /// turned off (`off`) is reported as no model at all, not as a file named
    /// "off" that failed.
    #[test]
    fn the_species_page_is_given_the_range_model_unless_it_is_turned_off() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let model = tmp.path().join("model.onnx");
        let labels = tmp.path().join("labels.txt");
        let ask = |metadata: &str| {
            let cli = cli(&[
                "--latitude",
                "52.5",
                "--longitude",
                "13.4",
                "--metadata-model",
                metadata,
            ]);
            let settings = Settings::resolve(&cli, None, &state);
            let planned = plan_classifiers(None, &state, &model, &labels);
            publish_occurrence_source(&state, &settings, &labels, &planned);
            state
                .occurrence_source()
                .expect("the daemon published its occurrence source")
                .report(22)
                .unwrap_err()
        };
        let configured = ask("/nonexistent/range.onnx");
        assert!(
            matches!(configured, Unavailable::Failed(_)),
            "{configured:?}"
        );
        assert_eq!(ask("off"), Unavailable::NoModel);
    }

    fn classifier(id: &str) -> ClassifierState {
        ClassifierState {
            id: id.to_owned(),
            primary: id == "primary",
            model_path: PathBuf::from(format!("/m/{id}.onnx")),
            labels_path: None,
            status: ClassifierStatus::Judging {
                sources: Vec::new(),
                unrouted_sources: id == "primary",
            },
        }
    }

    /// The published classifiers are every one the configuration names, the
    /// loaded and the left-out together, in the order the configuration
    /// declares them, whichever list each came from.
    #[test]
    fn the_published_classifiers_are_in_declaration_order() {
        let declared = ["primary", "bats", "owls"].map(str::to_owned);
        let loaded = [classifier("owls"), classifier("primary")];
        let planned_out = [classifier("bats")];
        let ids: Vec<String> = declared_order(&loaded, &planned_out, &declared)
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, ["primary", "bats", "owls"]);
    }
}

#[cfg(test)]
mod not_configured_tests {
    use super::not_configured_why;
    use clap::Parser as _;

    /// What a station with nothing configured is told reads as a sentence,
    /// and names each missing setting; one with only the folder missing
    /// names that alone.
    #[test]
    fn the_reason_names_what_is_missing_in_a_sentence() {
        let bare = crate::cli::Cli::parse_from(["birdnet-behavior"]);
        let why = not_configured_why(&bare, None);
        assert!(!why.contains("no the"), "{why}");
        assert!(
            why.starts_with(
                "the classifier (MODEL_PATH), its labels (LABELS_PATH) and the recordings \
                 folder (RECS_DIR) are not configured."
            ),
            "{why}"
        );
        let some = crate::cli::Cli::parse_from([
            "birdnet-behavior",
            "--model",
            "/m/a.onnx",
            "--labels",
            "/m/a.txt",
        ]);
        let why = not_configured_why(&some, None);
        assert!(
            why.starts_with("the recordings folder (RECS_DIR) is not configured."),
            "{why}"
        );
    }
}

#[cfg(test)]
mod liveness_mirror_tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(10);

    /// Eight ticks. Both tests run on a paused clock (`start_paused`), which
    /// the runtime advances only when every task is idle, so the mirror always
    /// takes each tick that falls inside this sleep before the test resumes.
    /// On the real clock it did not: under load the test thread could be
    /// starved past both the mirror's first tick and this sleep, the runtime
    /// then woke both in one turn, and the test was polled first and read the
    /// flag the mirror had not yet cleared (`Tests (x86_64)` on `fa59a8d`).
    async fn settle() {
        tokio::time::sleep(TICK * 8).await;
    }

    /// A daemon that exits is reported as stopped — and one that is still
    /// running is not, which is the counterpart that stops "always clear"
    /// from passing.
    ///
    /// Observed failing with the mirror's `store(false)` removed: the
    /// transition assertion reports `the daemon exited but the health flag
    /// still says running`.
    #[tokio::test(start_paused = true)]
    async fn the_health_flag_follows_the_loop_thread_down_and_not_before() {
        let source = Arc::new(AtomicBool::new(true));
        let target = Arc::new(AtomicBool::new(true));
        let metrics = birdnet_web::metrics::new_shared();
        metrics.set_detector_state(DetectorState::Running);
        let task = mirror_liveness(
            Arc::clone(&source),
            Arc::clone(&target),
            Arc::clone(&metrics),
            TICK,
        );

        settle().await;
        assert_eq!(metrics.models_state().detector, DetectorState::Running);
        assert!(
            target.load(Ordering::Relaxed),
            "a running daemon must keep reading as running"
        );
        assert!(
            !task.is_finished(),
            "the mirror must keep watching a live daemon"
        );

        source.store(false, Ordering::Release);
        settle().await;
        assert!(
            !target.load(Ordering::Relaxed),
            "the daemon exited but the health flag still says running"
        );
        assert!(
            task.is_finished(),
            "nothing is left to mirror once the exit is recorded"
        );
        assert_eq!(
            metrics.models_state().detector,
            DetectorState::Stopped,
            "the published state follows the loop thread down"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_daemon_that_never_came_up_is_reported_at_the_first_tick() {
        let source = Arc::new(AtomicBool::new(false));
        let target = Arc::new(AtomicBool::new(true));
        let task = mirror_liveness(
            Arc::clone(&source),
            Arc::clone(&target),
            birdnet_web::metrics::new_shared(),
            TICK,
        );
        settle().await;
        assert!(!target.load(Ordering::Relaxed));
        assert!(task.is_finished());
    }
}

#[cfg(test)]
mod tests {
    /// A 15 s, 48 kHz mono silent WAV at `path`.
    fn write_segment(path: &std::path::Path) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..(15 * 48_000) {
            w.write_sample(0_i16).unwrap();
        }
        w.finalize().unwrap();
    }

    /// RES-01 / MSC-16. Effort is the audio the pipeline analysed, credited to
    /// the hour it was recorded in, not the time a source's gauge read "up".
    /// Twenty 15 s segments, half shed under a thermal limit, in one five-minute
    /// tick with the source up the whole time: 150 s of effort, not 300.
    #[test]
    fn a_file_whose_windows_partly_failed_is_credited_the_share_answered() {
        // Review F18: a file of five windows, two of which no classifier
        // answered, is 9 s of listening of its 15, not 15.
        use birdnet_db::audio_sources::{AudioSourceStore as _, NewAudioSource, SourceKind};
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        // The ledger credits a source the station has.
        state
            .with_db(|c| {
                c.insert(&NewAudioSource::defaults(
                    "src_a",
                    SourceKind::UsbAlsa,
                    "hw:1,0",
                ))
            })
            .unwrap();
        let observer = super::throughput_observer(&state);
        let path = tmp.path().join("2026-05-19-birdnet-src_a-09:00:00.wav");
        write_segment(&path);
        observer.analysed(&path);
        observer.filtered(
            &path,
            &birdnet_core::detection::FilterTally {
                windows: 5,
                failed_windows: 2,
                ..birdnet_core::detection::FilterTally::default()
            },
        );
        // Credits are held until the ledger's tick writes them.
        crate::integrations::effort::tick(&state, 0.0);
        let total: f64 = state.with_db(|c| {
            c.query_row(
                "SELECT COALESCE(SUM(seconds), 0) FROM recording_effort WHERE source = 'src_a'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert!((total - 9.0).abs() < 1e-6, "credited {total} s, not 9 s");
        // Counterpart: a file with none failed is credited whole.
        let whole = tmp.path().join("2026-05-19-birdnet-src_a-09:00:15.wav");
        write_segment(&whole);
        observer.analysed(&whole);
        observer.filtered(
            &whole,
            &birdnet_core::detection::FilterTally {
                windows: 5,
                ..birdnet_core::detection::FilterTally::default()
            },
        );
        // Credits are held until the ledger's tick writes them.
        crate::integrations::effort::tick(&state, 0.0);
        let total: f64 = state.with_db(|c| {
            c.query_row(
                "SELECT COALESCE(SUM(seconds), 0) FROM recording_effort WHERE source = 'src_a'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert!(
            (total - 24.0).abs() < 1e-6,
            "credited {total} s, not 9 + 15"
        );
        // Review F10: a file whose header claims 15 s and of which 1 s decoded
        // (a WAV cut short) is 1 s of listening.
        let cut = tmp.path().join("2026-05-19-birdnet-src_a-09:00:30.wav");
        write_segment(&cut);
        observer.analysed(&cut);
        observer.filtered(
            &cut,
            &birdnet_core::detection::FilterTally {
                windows: 1,
                decoded_ms: 1000,
                ..birdnet_core::detection::FilterTally::default()
            },
        );
        crate::integrations::effort::tick(&state, 0.0);
        let total: f64 = state.with_db(|c| {
            c.query_row(
                "SELECT COALESCE(SUM(seconds), 0) FROM recording_effort WHERE source = 'src_a'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert!(
            (total - 25.0).abs() < 1e-6,
            "credited {total} s: the header's 15 s, not the 1 s that decoded"
        );
    }

    #[test]
    fn effort_is_the_audio_analysed_not_the_time_a_source_was_up() {
        use birdnet_db::audio_sources::{AudioSourceStore as _, NewAudioSource, SourceKind};
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        state
            .with_db(|c| {
                c.insert(&NewAudioSource::defaults(
                    "src_a",
                    SourceKind::UsbAlsa,
                    "hw:1,0",
                ))
            })
            .unwrap();
        state.metrics().set_source_up("src_a", true);
        let observer = super::throughput_observer(&state);
        for i in 0..20 {
            let path = tmp.path().join(format!(
                "2026-05-19-birdnet-src_a-09:{:02}:{:02}.wav",
                i / 4,
                (i % 4) * 15
            ));
            write_segment(&path);
            if i % 2 == 0 {
                // As `run.rs` reports an analysed file: analysed, then what its
                // filters did.
                observer.analysed(&path);
                observer.filtered(&path, &birdnet_core::detection::FilterTally::default());
            } else {
                observer.shed(&path, birdnet_core::detection::daemon::ShedReason::Thermal);
            }
        }
        crate::integrations::effort::tick(&state, 300.0);
        let total: f64 = state.with_db(|c| {
            c.query_row(
                "SELECT COALESCE(SUM(seconds), 0) FROM recording_effort WHERE source = 'src_a'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert!(
            (total - 150.0).abs() < 1e-6,
            "effort credited {total} s for 150 s of analysed audio (300 s of capture, half shed)"
        );
        let day: String = state.with_db(|c| {
            c.query_row(
                "SELECT date FROM recording_effort WHERE source = 'src_a'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert_eq!(
            day, "2026-05-19",
            "effort belongs to the day the audio was recorded"
        );
        // RES-02: the same audio by the hour it was recorded in. The uptime sample
        // is the clock's: it lands in the hour the tick ran in, not 09:00.
        let (hour, analysed): (i64, f64) = state.with_db(|c| {
            c.query_row(
                "SELECT hour, seconds_analysed FROM recording_effort_hourly
                  WHERE date = '2026-05-19' AND source = 'src_a'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        });
        assert_eq!(hour, 9);
        assert!((analysed - 150.0).abs() < 1e-6, "{analysed}");
        let up: f64 = state.with_db(|c| {
            c.query_row(
                "SELECT COALESCE(SUM(seconds_up), 0) FROM recording_effort_hourly WHERE source = 'src_a'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert!(
            (up - 300.0).abs() < 1e-6,
            "the five-minute sample still records the source as up: {up}"
        );
    }

    /// ACC-02's exposition gate: a file whose privacy rule masked windows and
    /// removed detections reaches `birdnet_detections_dropped_total` and
    /// `birdnet_windows_masked_total` under `reason="privacy"`, and the day's
    /// tally for its source is kept. Nothing called the metric with
    /// `privacy` before. Counterpart: a file nothing was discarded from
    /// writes no series and no row.
    #[test]
    fn a_file_s_privacy_discards_are_exported_and_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let observer = super::throughput_observer(&state);
        let clean = birdnet_core::detection::FilterTally {
            date: "2026-05-19".to_owned(),
            windows: 5,
            ..Default::default()
        };
        observer.filtered(
            std::path::Path::new("/s/2026-05-19-birdnet-09:00:00.wav"),
            &clean,
        );
        let snap = state.metrics().snapshot();
        assert!(snap.detections_dropped.is_empty() && snap.windows_masked.is_empty());

        let voiced = birdnet_core::detection::FilterTally {
            privacy_windows: 3,
            privacy_detections: 2,
            occurrence_detections: 1,
            ..clean
        };
        observer.filtered(
            std::path::Path::new("/s/2026-05-19-birdnet-cam1-09:00:15.wav"),
            &voiced,
        );
        let out = birdnet_web::metrics::render_runtime_metrics(&state.metrics().snapshot());
        assert!(
            out.contains(r#"birdnet_detections_dropped_total{reason="privacy"} 2"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_windows_masked_total{reason="privacy"} 3"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_detections_dropped_total{reason="occurrence"} 1"#),
            "{out}"
        );
        // RES-28: the voiced segment is marked for the raw keep; the clean
        // one is not.
        assert!(
            super::speech_marks()
                .take(std::ffi::OsStr::new("2026-05-19-birdnet-cam1-09:00:15.wav"))
        );
        assert!(
            !super::speech_marks().take(std::ffi::OsStr::new("2026-05-19-birdnet-09:00:00.wav"))
        );
        let rows = state
            .with_db(|c| birdnet_db::sqlite::discards_on(c, "2026-05-19"))
            .unwrap();
        let got: Vec<(&str, &str, u64, u64)> = rows
            .iter()
            .map(|r| {
                (
                    r.source.as_str(),
                    r.reason.as_str(),
                    r.windows,
                    r.detections,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![("cam1", "occurrence", 0, 1), ("cam1", "privacy", 3, 2)]
        );
    }

    use super::*;

    /// The shed policy's thermal input, as a table.
    ///
    /// | CPU temperature        | throttled now | at the limit? |
    /// |------------------------|---------------|---------------|
    /// | unknown                | no            | no            |
    /// | below the alert line   | no            | no            |
    /// | exactly the alert line | no            | yes           |
    /// | above it               | no            | yes           |
    /// | unknown                | yes           | yes           |
    /// | below the alert line   | yes           | yes           |
    ///
    /// The exact-line row separates `>=` from `>` and from `<`; the two
    /// throttled rows with a cool or unknown CPU separate `||` from `&&`.
    #[test]
    fn the_thermal_limit_is_the_alert_line_or_the_firmware_throttling() {
        let line = crate::integrations::THERMAL_ALERT_C;
        assert!(!thermal_limit_reached(None, false));
        assert!(!thermal_limit_reached(Some(line - 0.5), false));
        assert!(thermal_limit_reached(Some(line), false));
        assert!(thermal_limit_reached(Some(line + 10.0), false));
        assert!(thermal_limit_reached(None, true));
        assert!(thermal_limit_reached(Some(line - 20.0), true));
    }

    /// Audit MMP-07: with dynamic thresholds on (floor 0.3) under a 0.7
    /// station, a classifier without a threshold of its own is loaded with the
    /// station's 0.7, so its "own bar" is the one acceptance applies. The floor
    /// still reaches the model at run time through `ThresholdFloor`; that half
    /// is held by `a_lowered_threshold_reaches_a_model_loaded_above_it`
    /// (`tests/a_lowered_species_threshold_reaches_the_model.rs`).
    #[test]
    fn classifiers_are_loaded_at_the_station_bar_not_the_dynamic_floor() {
        use birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig;
        let on = DynamicThresholdConfig {
            enabled: true,
            trigger: 0.8,
            min: 0.3,
            valid_hours: 24,
        };
        let (loaded, floor) = classifier_thresholds(on, 0.7);
        assert!((loaded - 0.7).abs() < f32::EPSILON, "loaded at {loaded}");
        // Counterpart: the floor the processor publishes is still the low one.
        assert!((floor - 0.3).abs() < f32::EPSILON, "floor {floor}");
    }

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
            &cli,
            None,
            state,
            Delivery {
                broadcast,
                apprise: None,
                birdweather: None,
                email: None,
                mqtt: None,
                notification_filter: filter,
                notification_template: template,
            },
            birdnet_core::detection::daemon::InFlight::new(),
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
            &cli,
            None,
            state,
            Delivery {
                broadcast,
                apprise: None,
                birdweather: None,
                email: None,
                mqtt: None,
                notification_filter: filter,
                notification_template: template,
            },
            birdnet_core::detection::daemon::InFlight::new(),
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
            &cli,
            None,
            state,
            Delivery {
                broadcast,
                apprise: None,
                birdweather: None,
                email: None,
                mqtt: None,
                notification_filter: filter,
                notification_template: template,
            },
            birdnet_core::detection::daemon::InFlight::new(),
        );
        assert!(
            handle.is_none(),
            "daemon must return None when model/labels/watch_dir all unset"
        );
    }
}
