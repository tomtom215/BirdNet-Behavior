//! Per-config builders and startup path/threshold resolution.
//!
//! Pure functions extracted from `start_detection_daemon` so every struct
//! literal and precedence rule is observable in a unit test rather than only
//! through the in-process integration harness.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use birdnet_core::audio::extraction::{AudioFormat, ExtractionConfig};
use birdnet_core::detection::pipeline::PipelineConfig;
use birdnet_core::inference::model::ModelConfig;
use birdnet_core::inference::species_filter::{SpeciesFilterConfig, SpeciesLists};

use crate::cli::Cli;
use crate::helpers::resolve;

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
pub(super) fn resolve_f32_with_default(
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
pub(super) fn build_pipeline_config(watch_dir: PathBuf, chunk_overlap_secs: f32) -> PipelineConfig {
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
pub(super) fn build_model_config(sensitivity: f32, confidence_threshold: f32) -> ModelConfig {
    ModelConfig {
        sensitivity,
        confidence_threshold,
        ..ModelConfig::default()
    }
}

/// Build the [`SpeciesFilterConfig`] used by the metadata-model species filter.
///
/// `lists` are the operator's include/exclude entries from `/admin/species`.
/// They used to be left at `SpeciesFilterConfig::default()` — empty — and
/// nothing anywhere else populated them, so the two lists the page maintains,
/// confirms every addition to, and offers a preview page for had no effect on a
/// single detection. An excluded species kept being recorded, counted, notified
/// on and uploaded.
#[must_use]
pub(super) fn build_species_filter_config(
    sf_thresh: f32,
    lists: SpeciesLists,
) -> SpeciesFilterConfig {
    SpeciesFilterConfig {
        sf_thresh,
        include_list: lists.include,
        exclude_list: lists.exclude,
        ..SpeciesFilterConfig::default()
    }
}

/// Resolve the station's coordinates for the detection daemon.
///
/// CLI flag first, then the config — which the settings overlay has already
/// layered `/admin/settings` onto. The daemon read `cli.latitude` alone, so a
/// station configured the normal way (the installer writes `LATITUDE` /
/// `LONGITUDE` into `birdnet.conf`, and the web form writes the settings table)
/// handed the daemon `None` and never ran the metadata model at all — making
/// `sf_thresh`, a headline species-frequency feature, silently inert on most
/// real installs. `crate::capture::schedule::resolve_location` has always done
/// this correctly for the recording schedule; this is the same rule.
///
/// Returns `(latitude, longitude)`, each `None` when unresolvable.
#[must_use]
pub fn resolve_station_coords(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> (Option<f64>, Option<f64>) {
    let lat = cli
        .latitude
        .or_else(|| config.and_then(|c| c.get_parsed::<f64>("LATITUDE").ok()));
    let lon = cli
        .longitude
        .or_else(|| config.and_then(|c| c.get_parsed::<f64>("LONGITUDE").ok()));
    (lat, lon)
}

/// Resolve the minimum-confidence threshold the daemon will enforce.
///
/// `CONFIDENCE` from the config — which the settings overlay has already
/// layered `/admin/settings` onto — otherwise
/// [`DEFAULT_CONFIDENCE_THRESHOLD`](birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD) —
/// the same value the admin form advertises and the onboarding wizard
/// pre-selects.
///
/// A `CONFIDENCE` that does not parse falls back to the default rather than to
/// zero: an unusable value must not silently turn the station into a
/// false-positive firehose. Out-of-range and non-numeric values are separately
/// reported as errors by `birdnet_core::config::validate`, which `--doctor`
/// runs from `ExecStartPre` — so in practice the daemon never starts on one.
///
/// Extracted from `start_detection_daemon` so the precedence rule and the
/// default are observable in a unit test rather than only through a live run.
#[must_use]
pub fn resolve_confidence(config: Option<&birdnet_core::config::Config>) -> f32 {
    config
        .and_then(|c| c.get_parsed::<f32>("CONFIDENCE").ok())
        .unwrap_or(birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD)
}

/// Resolve the detection sensitivity the daemon will apply.
///
/// Same precedence and fallback rule as [`resolve_confidence`], against
/// `SENSITIVITY` / [`DEFAULT_SENSITIVITY`](birdnet_core::config::DEFAULT_SENSITIVITY).
#[must_use]
pub(super) fn resolve_sensitivity(config: Option<&birdnet_core::config::Config>) -> f32 {
    config
        .and_then(|c| c.get_parsed::<f32>("SENSITIVITY").ok())
        .unwrap_or(birdnet_core::config::DEFAULT_SENSITIVITY)
}

/// Build the [`ExtractionConfig`] for the detection-clip extractor.
///
/// `recordings_dir` is the persistent directory the web server serves
/// recordings from (`AppState::recording_dir`). Extracted clips are written
/// there directly and FLAT, so they survive restarts and are found by the
/// Recordings page, playback, and backups. (Bug B: clips previously landed in a
/// sibling `Extracted/By_Date/…` next to the transient tmpfs watch dir — wiped
/// on every restart under `PrivateTmp`, and never where the app reads.)
///
/// `recording_length` is the resolved segment duration (an integer seconds
/// value) cast to `f32`; the cast cannot lose precision in the practical range
/// (1–3600 s).
///
/// `segment_duration` and `freq_shift_hz` are resolved rather than read
/// straight off the `Cli`, so the values an operator sets on `/admin/settings`
/// reach the extractor. Both flags carry a clap `default_value`, so reading
/// `cli.*` directly meant the default always won and the settings-page fields
/// were editable but inert.
#[must_use]
pub(super) fn build_extraction_config(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    recordings_dir: &Path,
) -> ExtractionConfig {
    let segment_duration = resolve::setting::<u32>(
        cli,
        "segment_duration",
        cli.segment_duration,
        config,
        "SEGMENT_DURATION",
    );
    let freq_shift_hz = resolve::setting::<i32>(
        cli,
        "freq_shift_hz",
        cli.freq_shift_hz,
        config,
        "FREQ_SHIFT",
    );

    ExtractionConfig {
        target_format: AudioFormat::parse(&cli.audio_format),
        audio_format: cli.audio_format.clone(),
        output_dir: recordings_dir.to_path_buf(),
        recording_length: f32::from(u16::try_from(segment_duration).unwrap_or(u16::MAX)),
        freq_shift_hz,
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
pub(super) fn resolve_required_paths(
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
pub(super) fn species_thresholds_log_count(thresholds: &HashMap<String, f64>) -> Option<usize> {
    if thresholds.is_empty() {
        None
    } else {
        Some(thresholds.len())
    }
}

/// "Should we log the operator's species lists, and how long are they?"
///
/// Returns `Some((include_len, exclude_len))` when *either* list has an entry;
/// `None` when the operator has configured neither. Same reason as
/// [`species_thresholds_log_count`] one function up, and the same shape: the
/// inline `if !include.is_empty() || !exclude.is_empty()` guard left three
/// cargo-mutants survivors — the `||`, and each of the two `!` — because its
/// only observable effect is a log line the suite doesn't capture.
///
/// A one-list station is the case that matters: it is both the common
/// configuration and the one the `||`→`&&` mutant silently breaks.
#[must_use]
pub(super) const fn species_lists_log_counts(lists: &SpeciesLists) -> Option<(usize, usize)> {
    if lists.include.is_empty() && lists.exclude.is_empty() {
        None
    } else {
        Some((lists.include.len(), lists.exclude.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::test_support::thresholds;
    use crate::helpers::test_support::{cli_with_explicit, config_with};
    use clap::Parser;

    // ── settings reach the extractor ────────────────────────────────────
    //
    // These pin *effect*, not just mapping: an operator sets the value on
    // `/admin/settings`, the overlay lands it in the config, and the extractor
    // must be built with it. Both flags carry a clap `default_value`, so
    // reading `cli.*` straight — which is what the code did — meant the field
    // was editable, persisted, and silently ignored.

    #[test]
    fn segment_duration_from_settings_reaches_the_extractor() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let cfg = config_with(&[("SEGMENT_DURATION", "30")]);
        let extraction = build_extraction_config(&cli, Some(&cfg), Path::new("/tmp/recs"));
        assert!(
            (extraction.recording_length - 30.0).abs() < f32::EPSILON,
            "settings-page segment duration must reach the extractor, got {}",
            extraction.recording_length
        );
    }

    #[test]
    fn explicit_segment_duration_flag_beats_the_settings() {
        let mut cli = cli_with_explicit(&["segment_duration"]);
        cli.segment_duration = 20;
        let cfg = config_with(&[("SEGMENT_DURATION", "30")]);
        let extraction = build_extraction_config(&cli, Some(&cfg), Path::new("/tmp/recs"));
        assert!((extraction.recording_length - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn freq_shift_from_settings_reaches_the_extractor() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let cfg = config_with(&[("FREQ_SHIFT", "2500")]);
        let extraction = build_extraction_config(&cli, Some(&cfg), Path::new("/tmp/recs"));
        assert_eq!(extraction.freq_shift_hz, 2500);
    }

    #[test]
    fn without_a_config_the_cli_defaults_still_apply() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let extraction = build_extraction_config(&cli, None, Path::new("/tmp/recs"));
        assert!((extraction.recording_length - 15.0).abs() < f32::EPSILON);
        assert_eq!(extraction.freq_shift_hz, 0);
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
        let cfg = build_species_filter_config(0.07, SpeciesLists::default());
        assert!((cfg.sf_thresh - 0.07).abs() < f32::EPSILON);
        let default = SpeciesFilterConfig::default();
        assert_eq!(cfg.whitelist, default.whitelist);
        assert_eq!(cfg.include_list, default.include_list);
        assert_eq!(cfg.exclude_list, default.exclude_list);
    }

    #[test]
    fn build_species_filter_config_carries_the_operator_lists() {
        // The defect: this took everything but `sf_thresh` from `Default`, so
        // the two lists `/admin/species` maintains arrived empty and no
        // detection was ever filtered by them.
        let lists = SpeciesLists {
            include: vec!["Eurasian Blackbird".into()],
            exclude: vec!["Human".into(), "Turdus merula".into()],
        };
        let cfg = build_species_filter_config(0.03, lists);
        assert_eq!(cfg.include_list, vec!["Eurasian Blackbird".to_string()]);
        assert_eq!(
            cfg.exclude_list,
            vec!["Human".to_string(), "Turdus merula".to_string()]
        );
    }

    // ── resolve_station_coords ──────────────────────────────────────────

    #[test]
    fn station_coords_fall_back_to_the_config() {
        // The bug this closes: the daemon read `cli.latitude` alone, so a
        // station configured the normal way — the installer writes LATITUDE /
        // LONGITUDE into birdnet.conf, and /admin/settings writes the settings
        // table the overlay layers onto it — handed the daemon `None` and never
        // ran the metadata model, making `sf_thresh` inert on most real
        // installs.
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let cfg = config_with(&[("LATITUDE", "42.3601"), ("LONGITUDE", "-71.0589")]);
        let (lat, lon) = resolve_station_coords(&cli, Some(&cfg));
        assert_eq!(lat, Some(42.3601));
        assert_eq!(lon, Some(-71.0589));
    }

    #[test]
    fn station_coords_prefer_the_cli() {
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.latitude = Some(51.5);
        cli.longitude = Some(-0.13);
        let cfg = config_with(&[("LATITUDE", "42.3601"), ("LONGITUDE", "-71.0589")]);
        let (lat, lon) = resolve_station_coords(&cli, Some(&cfg));
        assert_eq!(lat, Some(51.5));
        assert_eq!(lon, Some(-0.13));
    }

    #[test]
    fn station_coords_are_none_when_nothing_is_configured() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        assert_eq!(resolve_station_coords(&cli, None), (None, None));
    }

    #[test]
    fn station_coords_resolve_each_axis_independently() {
        // A half-configured station (latitude only) must not silently produce a
        // (lat, 0.0) location; the filter treats a missing axis as no location.
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.latitude = Some(51.5);
        let (lat, lon) = resolve_station_coords(&cli, None);
        assert_eq!(lat, Some(51.5));
        assert_eq!(lon, None);
        assert!(lat.zip(lon).is_none(), "an incomplete pair is no location");
    }

    // ── Bug B fix: extraction target == web recording dir (hardware-free) ───
    //
    // On a default systemd install, extracted detection clips used to be written
    // to /tmp/Extracted (a transient tmpfs wiped on every restart under
    // PrivateTmp) while the web reads recordings from <DATA_DIR>/recordings on
    // the persistent disk — so clips never persisted and playback 404'd. The fix
    // passes AppState::recording_dir (the exact dir the web serves) straight to
    // the extractor, so the two can never diverge. Proven here with no hardware.
    #[test]
    fn bug_b_extraction_target_matches_web_recording_dir() {
        let data_dir = Path::new("/home/pi/BirdNet-Behavior");
        let db_path = data_dir.join("birds.db"); // config DB_PATH
        // birdnet-web state.rs derives recording_dir = db_path.parent()/recordings;
        // the daemon passes that same PathBuf (AppState::recording_dir) here.
        let web_recording_dir = db_path.parent().unwrap().join("recordings");

        let cli = Cli::parse_from(["birdnet-behavior"]);
        let cfg = build_extraction_config(&cli, None, &web_recording_dir);

        assert_eq!(
            cfg.output_dir, web_recording_dir,
            "extraction writes exactly where the web serves recordings from"
        );
        assert!(
            !cfg.output_dir.starts_with("/tmp"),
            "clips persist on the data disk, not the transient tmpfs"
        );
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
        let cfg = build_extraction_config(&cli, None, Path::new("/var/lib/birdnet/recordings"));
        assert_eq!(cfg.audio_format, "flac");
        assert_eq!(cfg.target_format, AudioFormat::Flac);
        // output_dir is the recordings dir passed in, verbatim (clips land there
        // flat) — no longer a derived sibling `Extracted/`.
        assert_eq!(cfg.output_dir, PathBuf::from("/var/lib/birdnet/recordings"));
        assert!((cfg.recording_length - 20.0).abs() < f32::EPSILON);
        assert_eq!(cfg.freq_shift_hz, 1500);
        // Default still applies to extraction_length:
        let default = ExtractionConfig::default();
        assert!((cfg.extraction_length - default.extraction_length).abs() < f32::EPSILON);
    }

    #[test]
    fn build_extraction_config_defaults() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let cfg = build_extraction_config(
            &cli,
            None,
            Path::new("/home/pi/BirdNet-Behavior/recordings"),
        );
        // Default audio format is wav.
        assert_eq!(cfg.audio_format, "wav");
        assert_eq!(cfg.target_format, AudioFormat::Wav);
        // Default segment_duration is 15.
        assert!((cfg.recording_length - 15.0).abs() < f32::EPSILON);
        assert_eq!(cfg.freq_shift_hz, 0);
        assert_eq!(
            cfg.output_dir,
            PathBuf::from("/home/pi/BirdNet-Behavior/recordings")
        );
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

    // ── species_lists_log_counts ────────────────────────────────────────
    //
    // Four cases, because the guard this replaces had three distinct
    // mutants. The two single-list cases are what kill the `||`→`&&`
    // survivor; the empty case kills both `delete !` survivors.

    fn lists(include: &[&str], exclude: &[&str]) -> SpeciesLists {
        SpeciesLists {
            include: include.iter().map(|s| (*s).to_owned()).collect(),
            exclude: exclude.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn species_lists_log_counts_none_when_the_operator_configured_neither() {
        assert_eq!(species_lists_log_counts(&lists(&[], &[])), None);
    }

    #[test]
    fn species_lists_log_counts_some_for_an_include_only_station() {
        assert_eq!(
            species_lists_log_counts(&lists(&["Pica pica"], &[])),
            Some((1, 0))
        );
    }

    #[test]
    fn species_lists_log_counts_some_for_an_exclude_only_station() {
        assert_eq!(
            species_lists_log_counts(&lists(&[], &["Corvus corax"])),
            Some((0, 1))
        );
    }

    #[test]
    fn species_lists_log_counts_reports_both_lengths() {
        assert_eq!(
            species_lists_log_counts(&lists(&["Pica pica", "Turdus merula"], &["Corvus corax"])),
            Some((2, 1))
        );
    }

    // ── minimum-confidence resolution ───────────────────────────────────
    //
    // The threshold decides whether anything is recorded at all, and three
    // places have to agree on it: the daemon (enforces), the admin form
    // (displays), and the onboarding wizard (offers). These pin the daemon's
    // half of that contract.

    #[test]
    fn confidence_defaults_to_the_shared_constant_when_unset() {
        // The overwhelmingly common install: birdnet.conf carries no
        // CONFIDENCE line (the generated file has it commented out).
        let cfg = config_with(&[("LATITUDE", "42.36")]);
        assert!(
            (resolve_confidence(Some(&cfg)) - birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD)
                .abs()
                < f32::EPSILON
        );
        // …and with no config file at all.
        assert!(
            (resolve_confidence(None) - birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn confidence_default_is_075_not_the_old_025() {
        // Counter-test for the drift the shared constant was introduced to
        // kill: the daemon recorded at 0.25 while the UI advertised 0.70.
        let got = resolve_confidence(None);
        assert!(
            (got - 0.75).abs() < f32::EPSILON,
            "daemon default must be the shipped 0.75, got {got}"
        );
    }

    #[test]
    fn configured_confidence_wins_over_the_default() {
        let cfg = config_with(&[("CONFIDENCE", "0.85")]);
        assert!((resolve_confidence(Some(&cfg)) - 0.85).abs() < f32::EPSILON);
    }

    // The wizard→overlay→daemon chain is pinned in `helpers::settings_overlay`,
    // where `apply_setting_overrides` is in scope: see
    // `wizard_written_confidence_reaches_the_daemon`.

    #[test]
    fn unparseable_confidence_falls_back_to_the_default_not_zero() {
        // `--doctor` blocks startup on this, but if it is ever reached the
        // fallback must not be a firehose.
        let cfg = config_with(&[("CONFIDENCE", "not-a-number")]);
        assert!(
            (resolve_confidence(Some(&cfg)) - birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn sensitivity_defaults_and_overrides_follow_the_same_rule() {
        assert!(
            (resolve_sensitivity(None) - birdnet_core::config::DEFAULT_SENSITIVITY).abs()
                < f32::EPSILON
        );
        let cfg = config_with(&[("SENSITIVITY", "1.4")]);
        assert!((resolve_sensitivity(Some(&cfg)) - 1.4).abs() < f32::EPSILON);
    }
}
