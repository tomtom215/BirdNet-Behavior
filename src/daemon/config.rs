//! Per-config builders and startup path/threshold resolution.
//!
//! Pure functions extracted from `start_detection_daemon` so every struct
//! literal and precedence rule is observable in a unit test rather than only
//! through the in-process integration harness.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use birdnet_core::audio::extraction::{AudioFormat, ExtractionConfig};
use birdnet_core::detection::corroboration::ConfirmationLevel;
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

/// The same precedence rule for `i64` flags.
///
/// `duplicate_interval_secs` and `night_margin_mins` were each written out
/// by hand at their use site, and cargo-mutants found the same hole in both:
/// `==` → `!=` survived, because no test exercised the one cell where the
/// two branches disagree — flag at its documented default *and* a config key
/// present. Sharing one function means one comparison to gate rather than a
/// new one per flag, which is the reason the `f32` sibling above exists.
///
/// No `EPSILON` dance here: integers compare exactly. The default is still
/// passed in rather than assumed, because "the operator left the flag alone"
/// is a claim about clap's default for *that* flag — `0` for one of these
/// and `60` for the other.
#[must_use]
pub(super) fn resolve_i64_with_default(
    cli_value: i64,
    cli_default: i64,
    config_value: Option<i64>,
) -> i64 {
    if cli_value == cli_default {
        config_value.unwrap_or(cli_value)
    } else {
        cli_value
    }
}

/// Build the taxon-aware daylight filter from flags, config and coordinates.
///
/// Returns a disabled filter unless the operator asked for it *and* the
/// station has coordinates: without a latitude there is no sunrise to compute,
/// and a filter that guessed one would quarantine the wrong half of the day.
#[must_use]
pub(super) fn build_daylight_filter(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> crate::daemon::daylight::DaylightFilter {
    let requested = cli.night_filter
        || config
            .and_then(|c| c.get_parsed::<bool>("NIGHT_FILTER").ok())
            .unwrap_or(false);

    let location = if requested {
        if let (Some(lat), Some(lon)) = (latitude, longitude) {
            birdnet_scheduler::solar::Location::new(lat, lon)
                .inspect_err(|e| {
                    tracing::warn!(
                        error = %e,
                        "night filter requested but the station coordinates are out of \
                         range; leaving it off"
                    );
                })
                .ok()
        } else {
            tracing::warn!(
                "night filter requested but the station has no coordinates; leaving it off \
                 — set a latitude and longitude to enable it"
            );
            None
        }
    } else {
        None
    };

    let margin_mins = resolve_i64_with_default(
        cli.night_margin_mins,
        60,
        config.and_then(|c| c.get_parsed::<i64>("NIGHT_MARGIN_MINS").ok()),
    );

    crate::daemon::daylight::DaylightFilter::new(
        location,
        margin_mins,
        birdnet_db::clock::local_utc_offset_secs(),
        resolve_extra_nocturnal(
            cli.night_extra_nocturnal.as_deref(),
            config.and_then(|c| c.get("NIGHT_EXTRA_NOCTURNAL")),
        ),
    )
}

/// Split the operator's extra-nocturnal list on commas.
#[must_use]
pub(super) fn resolve_extra_nocturnal(
    cli_value: Option<&str>,
    config_value: Option<&str>,
) -> Vec<String> {
    cli_value
        .or(config_value)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Resolve the noise filter's watch list from the flag and the config file.
///
/// An explicitly empty setting (`NOISE_CLASSES=`) means *watch nothing*, and
/// must not silently fall back to the default: an operator who wants the
/// threshold on but the dog off has no other way to say so, and a default that
/// reappeared would keep suppressing chunks they had asked to keep.
///
/// An absent setting takes the default. Absent and empty are different
/// answers, which is why this cannot be a `filter(|s| !s.is_empty())`.
#[must_use]
pub(super) fn resolve_noise_classes(
    cli_value: Option<&str>,
    config_value: Option<&str>,
) -> Vec<String> {
    let Some(raw) = cli_value.or(config_value) else {
        return birdnet_core::detection::noise::DEFAULT_NOISE_CLASSES
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
    };
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Resolve the repeat-confirmation level from the flag and the config file.
///
/// Same precedence as every other flag here — the config key is consulted only
/// when the flag was left at clap's default — but the value is a name rather
/// than a number, so it can be misspelled. A misspelling is reported and
/// treated as `off` rather than aborting startup: this filter's whole job is
/// to *discard* detections, so failing open records more than the operator
/// asked for, while failing closed would take a station off the air over a
/// typo in an optional tuning key.
///
/// Returns the level and, when something was rejected, the text to log. The
/// warning is returned rather than logged here so a unit test can see it; the
/// caller does the `tracing::warn!`.
#[must_use]
pub fn resolve_confirmation_level(
    cli_value: &str,
    cli_default: &str,
    config_value: Option<&str>,
) -> (ConfirmationLevel, Option<String>) {
    let chosen = if cli_value == cli_default {
        config_value.unwrap_or(cli_value)
    } else {
        cli_value
    };
    match ConfirmationLevel::parse(chosen) {
        Ok(level) => (level, None),
        Err(why) => (
            ConfirmationLevel::Off,
            Some(format!(
                "`{why}` is not a confirmation level; repeat-confirmation filtering \
                 is OFF and every detection that passes the other filters is recorded"
            )),
        ),
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

/// Read a yes/no setting written by hand in a config file.
///
/// Three outcomes, not two: `Some(true)`, `Some(false)`, and `None` for a value
/// that is neither. The caller falls back to its default on `None` *and* warns,
/// which is the whole reason the third case exists — an operator who writes
/// `BIRDNET_DYNAMIC_THRESHOLD=treu` currently gets a silently disabled feature
/// and nothing in the log to explain it.
///
/// A named function rather than a closure because the distinction is otherwise
/// unobservable: for a setting whose default is `false`, `Some(false)` and
/// `None` produce the same configuration, so cargo-mutants could delete the
/// entire off-arm and no test could tell. Here the two are different return
/// values and the arm can be gated directly.
#[must_use]
pub(super) fn parse_flag(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Resolve the dynamic-threshold configuration from the environment.
///
/// Every key is optional and off is the default. A malformed value falls back
/// to the default for that field rather than failing the start: a station that
/// mistyped a threshold should keep detecting birds.
#[must_use]
pub(super) fn resolve_dynamic_threshold(
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig {
    use birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig;
    let defaults = DynamicThresholdConfig::default();

    let flag = |key: &str| -> Option<bool> {
        let raw = std::env::var(key)
            .ok()
            .or_else(|| config.and_then(|c| c.get(key).map(str::to_owned)))?;
        let parsed = parse_flag(&raw);
        if parsed.is_none() {
            tracing::warn!(
                key,
                value = %raw,
                "not a yes/no value; ignoring it and using the default"
            );
        }
        parsed
    };
    let number = |key: &str| -> Option<f32> {
        std::env::var(key)
            .ok()
            .or_else(|| config.and_then(|c| c.get(key).map(str::to_owned)))
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
    };

    DynamicThresholdConfig {
        enabled: flag("BIRDNET_DYNAMIC_THRESHOLD").unwrap_or(defaults.enabled),
        trigger: number("BIRDNET_DYNAMIC_THRESHOLD_TRIGGER")
            .filter(|v| (0.0..=1.0).contains(v))
            .unwrap_or(defaults.trigger),
        min: number("BIRDNET_DYNAMIC_THRESHOLD_MIN")
            .filter(|v| (0.0..=1.0).contains(v))
            .unwrap_or(defaults.min),
        valid_hours: number("BIRDNET_DYNAMIC_THRESHOLD_HOURS")
            .filter(|v| *v >= 1.0 && *v <= 8760.0)
            .map_or(defaults.valid_hours, |v| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    v as u32
                }
            }),
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

    // Clamped rather than trusted: an extraction longer than the segment it is
    // cut from cannot be satisfied, and one at zero writes empty clips that
    // look like a broken microphone. 1–60 s brackets every sane answer.
    let extraction_length = resolve::setting::<f32>(
        cli,
        "extraction_length",
        cli.extraction_length.unwrap_or(6.0),
        config,
        "EXTRACTION_LENGTH",
    )
    .clamp(1.0, 60.0);

    // Extra lead-in beyond the symmetric spacer. Clamped rather than validated
    // away: a value longer than a whole segment cannot be satisfied even with
    // boundary spanning (it would need two predecessors), and 30 seconds is
    // already far beyond any call this is for.
    // No CLI flag: this is a per-station tuning choice rather than something
    // an operator flips per run, so it lives in the config file alone.
    let pre_capture_secs = config
        .and_then(|c| c.get_parsed::<f32>("PRE_CAPTURE_SECS").ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, 30.0);

    ExtractionConfig {
        extraction_length,
        target_format: AudioFormat::parse(&cli.audio_format),
        audio_format: cli.audio_format.clone(),
        output_dir: recordings_dir.to_path_buf(),
        recording_length: f32::from(u16::try_from(segment_duration).unwrap_or(u16::MAX)),
        freq_shift_hz,
        pre_capture_secs,
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

    // ── resolve_dynamic_threshold ──────────────────────────────────────
    //
    // Untested until cargo-mutants said so: six mutants on this function's
    // changed lines and nothing to catch any of them. Driven through `config`
    // rather than the environment — the function prefers an env var, but
    // `std::env::set_var` is `unsafe` in edition 2024 and `unsafe` is forbidden
    // workspace-wide, and setting a process-global from a concurrent test is
    // the race that made it `unsafe`. No env var is set in these tests, so the
    // config branch is the one taken.

    /// Off, with the documented defaults, when nothing is configured.
    #[test]
    fn the_dynamic_threshold_is_off_unless_asked_for() {
        let cfg = config_with(&[]);
        let resolved = resolve_dynamic_threshold(Some(&cfg));
        let defaults =
            birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig::default();
        assert!(!resolved.enabled, "the feature must default to off");
        assert_eq!(resolved, defaults, "and to its documented defaults");
        assert_eq!(
            resolve_dynamic_threshold(None),
            defaults,
            "no config at all is the same as an empty one"
        );
    }

    /// Every spelling of the on/off flag an operator might write.
    ///
    /// The two match arms are separate mutants: delete the true arm and the
    /// feature can never be switched on; delete the false arm and it can never
    /// be switched off again once written.
    #[test]
    fn every_spelling_of_the_flag_is_understood() {
        for on in ["1", "true", "yes", "on", "TRUE", " On "] {
            let cfg = config_with(&[("BIRDNET_DYNAMIC_THRESHOLD", on)]);
            assert!(
                resolve_dynamic_threshold(Some(&cfg)).enabled,
                "{on:?} should enable the feature"
            );
        }
        for off in ["0", "false", "no", "off", "OFF"] {
            let cfg = config_with(&[("BIRDNET_DYNAMIC_THRESHOLD", off)]);
            assert!(
                !resolve_dynamic_threshold(Some(&cfg)).enabled,
                "{off:?} should disable the feature"
            );
        }
        // A value that is neither falls back to the default rather than
        // guessing — a mistyped flag must not silently turn the feature on.
        let cfg = config_with(&[("BIRDNET_DYNAMIC_THRESHOLD", "maybe")]);
        assert!(!resolve_dynamic_threshold(Some(&cfg)).enabled);
    }

    /// The three outcomes of the yes/no parser, told apart.
    ///
    /// `resolve_dynamic_threshold` cannot distinguish `Some(false)` from
    /// `None` — its default is already `false`, so both produce the same
    /// config, and deleting the whole off-arm was invisible to every test.
    /// Here the two are different values, which is also what lets the caller
    /// warn about a typo instead of silently disabling the feature.
    #[test]
    fn a_yes_no_value_is_parsed_into_three_outcomes() {
        for on in ["1", "true", "yes", "on", "TRUE", " On "] {
            assert_eq!(parse_flag(on), Some(true), "{on:?}");
        }
        for off in ["0", "false", "no", "off", "OFF", " Off "] {
            assert_eq!(parse_flag(off), Some(false), "{off:?}");
        }
        for neither in ["treu", "", "  ", "2", "maybe", "y", "n"] {
            assert_eq!(
                parse_flag(neither),
                None,
                "{neither:?} is not a yes/no value and must be reported as such, \
                 not quietly read as off"
            );
        }
    }

    /// The lease length is bounded to 1 hour .. 1 year, and both ends of the
    /// range are asserted along with a value inside it.
    ///
    /// `*v >= 1.0 && *v <= 8760.0` carries three mutants — the `&&`, the `>=`
    /// and the `<=` — and each one admits a lease the station cannot use: zero
    /// hours makes every learned level expire instantly, and a decade-long one
    /// never expires at all.
    #[test]
    fn the_lease_length_is_bounded_at_both_ends() {
        let hours = |v: &str| {
            let cfg = config_with(&[("BIRDNET_DYNAMIC_THRESHOLD_HOURS", v)]);
            resolve_dynamic_threshold(Some(&cfg)).valid_hours
        };
        let default_hours =
            birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig::default()
                .valid_hours;

        assert_eq!(hours("48"), 48, "a value inside the range is taken");
        assert_eq!(hours("1"), 1, "1 hour is the lower bound, inclusive");
        assert_eq!(
            hours("8760"),
            8760,
            "8760 hours is the upper bound, inclusive"
        );

        for rejected in ["0", "0.5", "-5", "8761", "100000", "nonsense", "NaN"] {
            assert_eq!(
                hours(rejected),
                default_hours,
                "{rejected:?} is outside the usable range and must fall back"
            );
        }
    }

    /// `trigger` and `min` are confidences, so both are held to 0..=1.
    #[test]
    fn the_confidences_are_held_to_their_range() {
        let resolved = |k: &str, v: &str| {
            let cfg = config_with(&[(k, v)]);
            resolve_dynamic_threshold(Some(&cfg))
        };
        let defaults =
            birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig::default();

        assert!(
            (resolved("BIRDNET_DYNAMIC_THRESHOLD_TRIGGER", "0.85").trigger - 0.85).abs() < 1e-6
        );
        assert!((resolved("BIRDNET_DYNAMIC_THRESHOLD_MIN", "0.25").min - 0.25).abs() < 1e-6);

        for bad in ["1.5", "-0.1", "nonsense"] {
            assert!(
                (resolved("BIRDNET_DYNAMIC_THRESHOLD_TRIGGER", bad).trigger - defaults.trigger)
                    .abs()
                    < 1e-6,
                "trigger {bad:?} must fall back to the default"
            );
            assert!(
                (resolved("BIRDNET_DYNAMIC_THRESHOLD_MIN", bad).min - defaults.min).abs() < 1e-6,
                "min {bad:?} must fall back to the default"
            );
        }
    }
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

    // ── resolve_i64_with_default ────────────────────────────────────────
    //
    // The same four cells as above. These exist because cargo-mutants found
    // `==` → `!=` surviving at *both* hand-written i64 use sites (the night
    // margin and the duplicate interval); the comparison now lives here once,
    // and the config-present × CLI-at-default cell is what kills it.

    #[test]
    fn resolve_i64_uses_config_when_cli_at_default_and_config_present() {
        // `night_margin_mins`: flag left at clap's documented 60, config asks
        // for 30. Under `!=` the branches swap and this returns 60.
        assert_eq!(resolve_i64_with_default(60, 60, Some(30)), 30);
        // `duplicate_interval_secs`, whose default is 0 rather than non-zero:
        // the same rule has to hold for a zero default, which is the case the
        // old inline `== 0` form was written for.
        assert_eq!(resolve_i64_with_default(0, 0, Some(45)), 45);
    }

    #[test]
    fn resolve_i64_uses_cli_default_when_no_config() {
        // CLI at default, nothing in the config: the default stands. Kills
        // the "replace body with 0 / 1 / -1" mutants for the non-zero case.
        assert_eq!(resolve_i64_with_default(60, 60, None), 60);
        assert_eq!(resolve_i64_with_default(0, 0, None), 0);
    }

    #[test]
    fn resolve_i64_cli_override_wins_over_config() {
        // The operator passed a flag; the config key is ignored. This is the
        // documented precedence and the reason the default is a parameter —
        // "overridden" is only meaningful relative to *this* flag's default.
        assert_eq!(resolve_i64_with_default(15, 60, Some(30)), 15);
        assert_eq!(resolve_i64_with_default(90, 0, Some(45)), 90);
    }

    #[test]
    fn resolve_i64_uses_cli_when_overridden_and_no_config() {
        assert_eq!(resolve_i64_with_default(15, 60, None), 15);
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

    // ── repeat-confirmation level ───────────────────────────────────────

    #[test]
    fn the_config_key_is_read_only_when_the_flag_was_left_alone() {
        // All four cells of the precedence table, because the two that agree
        // hide the two that do not.
        let (level, warn) = resolve_confirmation_level("off", "off", Some("strict"));
        assert_eq!(level, ConfirmationLevel::Strict, "config key ignored");
        assert!(warn.is_none());

        assert_eq!(
            resolve_confirmation_level("moderate", "off", Some("strict")).0,
            ConfirmationLevel::Moderate,
            "the config key beat an explicit flag"
        );
        assert_eq!(
            resolve_confirmation_level("moderate", "off", None).0,
            ConfirmationLevel::Moderate
        );
        assert_eq!(
            resolve_confirmation_level("off", "off", None).0,
            ConfirmationLevel::Off
        );
    }

    #[test]
    fn a_misspelled_level_fails_open_and_says_so() {
        // Failing closed here would take a station off the air over a typo in
        // an optional tuning key; failing open silently would let an operator
        // believe a filter is running that is not. So: off, and a warning that
        // names the value.
        let (level, warn) = resolve_confirmation_level("off", "off", Some("strictt"));
        assert_eq!(level, ConfirmationLevel::Off);
        let warn = warn.expect("a rejected value must be reported");
        assert!(
            warn.contains("strictt"),
            "the warning must name the value that was rejected: {warn}"
        );
        assert!(
            warn.contains("OFF"),
            "the warning must say what happens instead: {warn}"
        );
    }

    #[test]
    fn an_accepted_level_is_reported_silently() {
        // Counterpart to the gate above: a warning on every startup is a
        // warning nobody reads.
        for token in ["off", "lenient", "moderate", "balanced", "strict"] {
            let (_, warn) = resolve_confirmation_level(token, "off", None);
            assert!(warn.is_none(), "`{token}` was rejected: {warn:?}");
        }
    }

    // ── noise-filter watch list ─────────────────────────────────────────

    #[test]
    fn an_absent_noise_class_setting_takes_the_default() {
        assert_eq!(resolve_noise_classes(None, None), ["Dog"]);
    }

    #[test]
    fn an_explicitly_empty_setting_watches_nothing() {
        // Counterpart, and the reason this is not `filter(|s| !s.is_empty())`:
        // absent and empty are different answers. An operator who wants the
        // threshold on but the dog off has no other way to say so, and a
        // default that reappeared would keep discarding chunks they asked to
        // keep — silently, since a suppressed chunk leaves no row.
        assert!(resolve_noise_classes(Some(""), None).is_empty());
        assert!(resolve_noise_classes(None, Some("")).is_empty());
        assert!(resolve_noise_classes(Some("  , ,"), None).is_empty());
    }

    #[test]
    fn a_list_is_split_on_commas_and_trimmed() {
        assert_eq!(
            resolve_noise_classes(Some(" Dog , Siren ,, Engine "), None),
            ["Dog", "Siren", "Engine"]
        );
    }

    #[test]
    fn the_flag_wins_over_the_config_file() {
        assert_eq!(resolve_noise_classes(Some("Siren"), Some("Dog")), ["Siren"]);
        assert_eq!(resolve_noise_classes(None, Some("Engine")), ["Engine"]);
    }

    // ── the daylight filter's preconditions ─────────────────────────────

    #[test]
    fn the_night_filter_stays_off_without_coordinates() {
        // Without a latitude there is no sunrise to compute. Enabling anyway
        // and failing open would work, but it would also tell the operator at
        // startup that a filter was running which could never fire.
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.night_filter = true;
        assert!(
            !build_daylight_filter(&cli, None, None, None).is_enabled(),
            "the night filter enabled itself without coordinates"
        );
        assert!(
            !build_daylight_filter(&cli, None, Some(51.48), None).is_enabled(),
            "a latitude alone was enough to enable the night filter"
        );
    }

    #[test]
    fn the_night_filter_turns_on_when_asked_and_locatable() {
        // Counterpart: a builder that always returned a disabled filter would
        // satisfy the gate above and the feature would never work at all.
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.night_filter = true;
        assert!(build_daylight_filter(&cli, None, Some(51.48), Some(0.0)).is_enabled());
    }

    #[test]
    fn the_night_filter_stays_off_unless_asked_for() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        assert!(!build_daylight_filter(&cli, None, Some(51.48), Some(0.0)).is_enabled());
    }

    #[test]
    fn out_of_range_coordinates_leave_the_night_filter_off() {
        // `Location::new` validates; a station with a corrupt latitude must
        // not get a filter built on a bogus sunrise.
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.night_filter = true;
        assert!(!build_daylight_filter(&cli, None, Some(120.0), Some(0.0)).is_enabled());
        assert!(!build_daylight_filter(&cli, None, Some(51.48), Some(999.0)).is_enabled());
    }

    #[test]
    fn extra_nocturnal_entries_are_split_and_trimmed() {
        assert_eq!(
            resolve_extra_nocturnal(Some(" Catharus , Vireo ,, "), None),
            ["Catharus", "Vireo"]
        );
        assert!(resolve_extra_nocturnal(None, None).is_empty());
        assert_eq!(
            resolve_extra_nocturnal(None, Some("Catharus")),
            ["Catharus"]
        );
        assert_eq!(
            resolve_extra_nocturnal(Some("Vireo"), Some("Catharus")),
            ["Vireo"],
            "the config file beat the flag"
        );
    }
}
