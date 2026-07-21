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
use birdnet_core::inference::species_filter::SpeciesFilterConfig;

use crate::cli::Cli;

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
#[must_use]
pub(super) fn build_species_filter_config(sf_thresh: f32) -> SpeciesFilterConfig {
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
pub(super) fn extraction_output_dir(watch_dir: &Path) -> PathBuf {
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
pub(super) fn build_extraction_config(cli: &Cli, watch_dir: &Path) -> ExtractionConfig {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::test_support::thresholds;
    use clap::Parser;

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

    // ── Bug B repro (hardware-free) ─────────────────────────────────────────
    //
    // On a DEFAULT systemd install, extracted detection clips are written to the
    // transient RAM tmpfs, NOT to the persistent recordings dir the web UI and
    // backups read. This test uses the real production path logic to prove it
    // without any hardware or audio device.
    //
    // Installer defaults (installer/lib/{10-config,30-platform,65-service}.sh):
    //   systemd ExecStart:  --watch-dir /tmp/birdnet-stream   (transient tmpfs,
    //                       wiped on every restart under PrivateTmp=yes)
    //   config:             DB_PATH  = <DATA_DIR>/birds.db      (persistent disk)
    //                       RECS_DIR = <DATA_DIR>/recordings    (persistent disk)
    // birdnet-web `state.rs` derives recording_dir = db_path.parent()/recordings,
    // which equals RECS_DIR — so the ONLY divergence is the extractor's target.
    #[test]
    fn bug_b_extracted_clips_land_on_tmpfs_not_where_web_reads() {
        let data_dir = Path::new("/home/pi/BirdNet-Behavior");
        let watch_dir = Path::new("/tmp/birdnet-stream"); // systemd --watch-dir
        let db_path = data_dir.join("birds.db"); // config DB_PATH

        // Where the daemon writes extracted clips today (real production fn).
        let extraction_dir = extraction_output_dir(watch_dir);
        // Where birdnet-web reads them (state.rs: db_path.parent()/recordings).
        let web_recording_dir = db_path.parent().unwrap().join("recordings");

        // Clips are written onto the RAM tmpfs — wiped on every restart.
        assert_eq!(extraction_dir, Path::new("/tmp/Extracted"));
        assert!(
            extraction_dir.starts_with("/tmp"),
            "extracted clips sit on the transient tmpfs"
        );
        // The web UI + backups read the persistent disk, which nothing populates.
        assert!(
            !web_recording_dir.starts_with("/tmp"),
            "the recordings dir the app reads is on the persistent disk"
        );
        assert_ne!(
            extraction_dir, web_recording_dir,
            "BUG B (location): extraction target diverges from the web recordings \
             dir, so clips vanish on restart and never appear where the app looks"
        );
        // NOTE: there is also a *structure* mismatch (layer 2): the extractor
        // nests clips under `Extracted/By_Date/<date>/<species>/`, while the web
        // serve route (`/api/v2/recordings/{name}`, is_safe_filename rejects `/`)
        // resolves `recording_dir.join(basename)` FLAT — so even once the
        // location is repointed, playback needs the serve/list path reconciled.
        // Both layers are addressed by the Bug B fix.
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
}
