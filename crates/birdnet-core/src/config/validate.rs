//! Range and shape validation for configuration values.
//!
//! Catches malformed settings (out-of-range numbers, malformed schedule
//! strings, mutually-exclusive audio sources) at load time so the operator
//! sees a single clear error instead of a deep stack trace at first inference.
//!
//! Validation is intentionally *advisory*: missing keys are not failures
//! because BirdNet-Behavior runs with sensible built-in defaults. A key only
//! triggers a finding when it is *present* and its value is out of bounds.

use std::fmt;

use crate::config::Config;

/// A single validation finding produced by [`validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Severity of this finding.
    pub severity: Severity,
    /// Configuration key that triggered the finding (or a synthetic name).
    pub key: String,
    /// Human-readable description of what is wrong.
    pub message: String,
    /// Concrete suggestion the operator can act on.
    pub remediation: String,
}

/// Severity of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Misconfiguration that will cause functionality to be silently degraded.
    Warning,
    /// Misconfiguration that will prevent normal operation.
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warning => f.write_str("warning"),
            Self::Error => f.write_str("error"),
        }
    }
}

impl Finding {
    fn warn(key: impl Into<String>, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            key: key.into(),
            message: message.into(),
            remediation: fix.into(),
        }
    }

    fn error(key: impl Into<String>, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            key: key.into(),
            message: message.into(),
            remediation: fix.into(),
        }
    }
}

/// Run every validation check against `config` and return all findings.
///
/// An empty result means the configuration is well-formed. A non-empty
/// result with no [`Severity::Error`] entries means the configuration is
/// usable but the operator should review the warnings.
#[must_use]
pub fn validate(config: &Config) -> Vec<Finding> {
    let mut out = Vec::new();
    check_coords(config, &mut out);
    check_unit_range(config, "CONFIDENCE", 0.0, 1.0, &mut out);
    check_confidence_floor(config, &mut out);
    check_unit_range(config, "SF_THRESH", 0.0, 1.0, &mut out);
    check_unit_range(config, "PRIVACY_THRESHOLD", 0.0, 1.0, &mut out);
    check_bounded(config, "SENSITIVITY", 0.5, 1.5, &mut out);
    check_bounded(config, "OVERLAP", 0.0, 2.9, &mut out);
    check_positive_int(config, "RECORDING_LENGTH", 3, 60, &mut out);
    check_positive_int(config, "SEGMENT_DURATION", 3, 600, &mut out);
    check_schedule(config, &mut out);
    check_audio_sources(config, &mut out);
    check_audio_format(config, &mut out);
    check_info_site(config, &mut out);
    check_lang(config, &mut out);
    out
}

/// Convenience predicate: true when no findings are present.
#[must_use]
pub const fn is_clean(findings: &[Finding]) -> bool {
    findings.is_empty()
}

/// Convenience predicate: true when there are no [`Severity::Error`] findings.
#[must_use]
pub fn is_usable(findings: &[Finding]) -> bool {
    !findings.iter().any(|f| f.severity == Severity::Error)
}

fn check_coords(config: &Config, out: &mut Vec<Finding>) {
    let lat_raw = config.get("LATITUDE").map(str::trim);
    let lon_raw = config.get("LONGITUDE").map(str::trim);

    if let Some(raw) = lat_raw {
        // `parse_decimal` accepts both `.` and `,` as the decimal
        // separator so EU-formatted config values aren't rejected.
        match super::locale::parse_decimal(raw) {
            Ok(v) if !(-90.0..=90.0).contains(&v) => out.push(Finding::error(
                "LATITUDE",
                format!("latitude {v} is outside the valid range -90.0 to 90.0"),
                "use decimal degrees, e.g. LATITUDE=42.3601 for Boston, MA".to_string(),
            )),
            Err(e) => out.push(Finding::error(
                "LATITUDE",
                format!("latitude is not a number: {e}"),
                "use decimal degrees, e.g. LATITUDE=42.3601 or 42,3601".to_string(),
            )),
            Ok(_) => {}
        }
    }

    if let Some(raw) = lon_raw {
        match super::locale::parse_decimal(raw) {
            Ok(v) if !(-180.0..=180.0).contains(&v) => out.push(Finding::error(
                "LONGITUDE",
                format!("longitude {v} is outside the valid range -180.0 to 180.0"),
                "use decimal degrees, e.g. LONGITUDE=-71.0589 for Boston, MA".to_string(),
            )),
            Err(e) => out.push(Finding::error(
                "LONGITUDE",
                format!("longitude is not a number: {e}"),
                "use decimal degrees, e.g. LONGITUDE=-71.0589 or -71,0589".to_string(),
            )),
            Ok(_) => {}
        }
    }

    let lat_present = lat_raw.is_some_and(|s| !s.is_empty());
    let lon_present = lon_raw.is_some_and(|s| !s.is_empty());
    if lat_present && !lon_present {
        out.push(Finding::warn(
            "LONGITUDE",
            "LATITUDE is set but LONGITUDE is missing — species frequency filter and solar schedule will be disabled".to_string(),
            "set LONGITUDE to your station longitude in decimal degrees".to_string(),
        ));
    } else if lon_present && !lat_present {
        out.push(Finding::warn(
            "LATITUDE",
            "LONGITUDE is set but LATITUDE is missing — species frequency filter and solar schedule will be disabled".to_string(),
            "set LATITUDE to your station latitude in decimal degrees".to_string(),
        ));
    }
}

fn check_unit_range(config: &Config, key: &str, min: f64, max: f64, out: &mut Vec<Finding>) {
    check_bounded(config, key, min, max, out);
}

/// Threshold below which `CONFIDENCE` is treated as a probable mistake.
///
/// Not a hard floor — an operator who genuinely wants a firehose may set one —
/// but low enough that every ordinary configuration sits well above it.
const CONFIDENCE_FLOOR: f64 = 0.1;

/// Warn about a `CONFIDENCE` that is in range but implausibly low.
///
/// [`check_unit_range`] already rejects the percentage mistake (`CONFIDENCE=70`)
/// and non-numeric junk as errors. What it cannot catch is the *decimal* slip —
/// `0.075` for `0.75` — or a `0` copied from `SF_THRESH`, where "0 = disabled"
/// is the documented meaning. Those parse, sit inside 0–1, validate clean,
/// but they make the station record whatever the model's best guess was for
/// every three-second window: the disk fills, the species list fills with
/// noise, and nothing anywhere says why. A warning keeps the value usable
/// while making the choice visible in `--doctor`.
fn check_confidence_floor(config: &Config, out: &mut Vec<Finding>) {
    let Some(Ok(v)) = parse_float(config, "CONFIDENCE") else {
        return;
    };
    if !(0.0..CONFIDENCE_FLOOR).contains(&v) {
        return;
    }
    out.push(Finding::warn(
        "CONFIDENCE",
        format!(
            "CONFIDENCE={v} is very low — nearly every 3-second window will be \
             recorded as a detection, filling the disk with false positives"
        ),
        format!(
            "unless this is deliberate, set CONFIDENCE={} (the default); a \
             value this small is usually a slipped decimal point, and unlike \
             SF_THRESH a CONFIDENCE of 0 does not mean \"disabled\"",
            super::DEFAULT_CONFIDENCE_THRESHOLD
        ),
    ));
}

fn check_bounded(config: &Config, key: &str, min: f64, max: f64, out: &mut Vec<Finding>) {
    let Some(parsed) = parse_float(config, key) else {
        return;
    };
    match parsed {
        Err(e) => out.push(Finding::error(
            key,
            format!("{key} is not a number: {e}"),
            format!("set {key} to a value between {min} and {max}"),
        )),
        Ok(v) if !(min..=max).contains(&v) => out.push(Finding::error(
            key,
            format!("{key}={v} is outside the valid range {min} to {max}"),
            format!("set {key} to a value between {min} and {max}"),
        )),
        Ok(_) => {}
    }
}

fn check_positive_int(config: &Config, key: &str, min: u64, max: u64, out: &mut Vec<Finding>) {
    let Some(raw) = config.get(key) else {
        return;
    };
    match raw.parse::<u64>() {
        Err(e) => out.push(Finding::error(
            key,
            format!("{key} is not a non-negative integer: {e}"),
            format!("set {key} to a whole number between {min} and {max}"),
        )),
        Ok(v) if !(min..=max).contains(&v) => out.push(Finding::error(
            key,
            format!("{key}={v} is outside the valid range {min} to {max}"),
            format!("set {key} to a whole number between {min} and {max}"),
        )),
        Ok(_) => {}
    }
}

fn check_schedule(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = config.get("RECORDING_SCHEDULE") else {
        return;
    };
    let raw = raw.trim();
    if matches!(raw, "all-day" | "solar" | "") {
        return;
    }
    if let Some(window) = raw.strip_prefix("fixed:")
        && parse_time_window(window).is_some()
    {
        return;
    }
    out.push(Finding::error(
        "RECORDING_SCHEDULE",
        format!("RECORDING_SCHEDULE={raw:?} is not a recognised schedule"),
        r#"use "all-day", "solar", or "fixed:HH:MM-HH:MM" (e.g. fixed:06:00-20:00)"#.to_string(),
    ));
}

fn parse_time_window(window: &str) -> Option<()> {
    let (start, end) = window.split_once('-')?;
    parse_hhmm(start)?;
    parse_hhmm(end)?;
    Some(())
}

fn parse_hhmm(s: &str) -> Option<()> {
    let (h, m) = s.split_once(':')?;
    let h: u8 = h.parse().ok()?;
    let m: u8 = m.parse().ok()?;
    (h < 24 && m < 60).then_some(())
}

fn check_audio_sources(config: &Config, out: &mut Vec<Finding>) {
    let sources: Vec<&str> = ["ALSA_CARD", "RTSP_URL", "PIPEWIRE_DEVICE"]
        .into_iter()
        .filter(|k| config.get(k).is_some_and(|v| !v.trim().is_empty()))
        .collect();
    if sources.len() > 1 {
        out.push(Finding::warn(
            "AUDIO_SOURCE",
            format!(
                "multiple audio sources are set ({}); only one will be used",
                sources.join(", ")
            ),
            "set exactly one of ALSA_CARD, RTSP_URL, or PIPEWIRE_DEVICE in the config".to_string(),
        ));
    }
}

fn check_audio_format(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = config
        .get("AUDIOFMT")
        .or_else(|| config.get("AUDIO_FORMAT"))
    else {
        return;
    };
    let raw = raw.trim().to_ascii_lowercase();
    if !["wav", "mp3", "flac", "ogg"].contains(&raw.as_str()) {
        out.push(Finding::error(
            "AUDIO_FORMAT",
            format!("AUDIO_FORMAT={raw:?} is not supported"),
            r#"use "wav", "mp3", "flac", or "ogg" (non-WAV formats need ffmpeg or sox)"#
                .to_string(),
        ));
    }
}

fn check_info_site(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = config.get("INFO_SITE") else {
        return;
    };
    let raw = raw.trim().to_ascii_lowercase();
    if !["ebird", "allaboutbirds", "none", ""].contains(&raw.as_str()) {
        out.push(Finding::error(
            "INFO_SITE",
            format!("INFO_SITE={raw:?} is not recognised"),
            r#"use "ebird", "allaboutbirds", or "none""#.to_string(),
        ));
    }
}

fn check_lang(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = config.get("DATABASE_LANG").or_else(|| config.get("LANG")) else {
        return;
    };
    let raw = raw.trim();
    let is_iso639_1 = raw.len() == 2 && raw.chars().all(|c| c.is_ascii_lowercase());
    if !is_iso639_1 {
        out.push(Finding::warn(
            "DATABASE_LANG",
            format!("DATABASE_LANG={raw:?} is not a two-letter ISO-639-1 code"),
            r#"use a code like "en", "de", "fr", "es", "pt", "it", "ja", or "zh""#.to_string(),
        ));
    }
}

fn parse_float(config: &Config, key: &str) -> Option<Result<f64, std::num::ParseFloatError>> {
    // Locale-tolerant — accepts both `42.36` and `42,36`.
    config.get(key).map(super::locale::parse_decimal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pairs: &[(&str, &str)]) -> Config {
        use std::fmt::Write as _;
        let mut body = String::with_capacity(pairs.len() * 32);
        for (k, v) in pairs {
            let _ = writeln!(body, "{k}={v}");
        }
        Config::parse(&body).expect("parse")
    }

    #[test]
    fn empty_config_is_clean() {
        assert!(is_clean(&validate(&cfg(&[]))));
    }

    #[test]
    fn valid_settings_are_clean() {
        let c = cfg(&[
            ("LATITUDE", "42.3601"),
            ("LONGITUDE", "-71.0589"),
            ("CONFIDENCE", "0.7"),
            ("SF_THRESH", "0.03"),
            ("OVERLAP", "0.0"),
            ("SENSITIVITY", "1.0"),
            ("RECORDING_LENGTH", "15"),
            ("RECORDING_SCHEDULE", "solar"),
            ("AUDIO_FORMAT", "wav"),
            ("INFO_SITE", "ebird"),
            ("DATABASE_LANG", "en"),
        ]);
        assert!(is_clean(&validate(&c)), "{:?}", validate(&c));
    }

    #[test]
    fn fixed_window_schedule_accepted() {
        let c = cfg(&[("RECORDING_SCHEDULE", "fixed:06:00-20:00")]);
        assert!(is_clean(&validate(&c)));
    }

    #[test]
    fn malformed_schedule_rejected() {
        let c = cfg(&[("RECORDING_SCHEDULE", "fixed:25:99-20:00")]);
        let findings = validate(&c);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[0].key, "RECORDING_SCHEDULE");
    }

    #[test]
    fn out_of_range_latitude_is_error() {
        let c = cfg(&[("LATITUDE", "200.0"), ("LONGITUDE", "0")]);
        let findings = validate(&c);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "LATITUDE" && f.severity == Severity::Error)
        );
    }

    #[test]
    fn out_of_range_longitude_is_error() {
        let c = cfg(&[("LATITUDE", "0"), ("LONGITUDE", "200.0")]);
        let findings = validate(&c);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "LONGITUDE" && f.severity == Severity::Error)
        );
    }

    #[test]
    fn lat_without_lon_warns() {
        let c = cfg(&[("LATITUDE", "42.0")]);
        let findings = validate(&c);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "LONGITUDE" && f.severity == Severity::Warning)
        );
    }

    /// The exact values a live `--doctor` run accepted without comment before
    /// `check_confidence_floor` existed. Each one silently turns the station
    /// into a false-positive firehose.
    #[test]
    fn implausibly_low_confidence_is_warned() {
        for v in ["0", "0.0", "0.001", "0.075", "0.07", "0,07"] {
            let findings = validate(&cfg(&[("CONFIDENCE", v)]));
            assert!(
                findings
                    .iter()
                    .any(|f| f.key == "CONFIDENCE" && f.severity == Severity::Warning),
                "CONFIDENCE={v} should warn, got {findings:?}"
            );
            assert!(
                !findings
                    .iter()
                    .any(|f| f.key == "CONFIDENCE" && f.severity == Severity::Error),
                "CONFIDENCE={v} is usable — it must warn, not block startup"
            );
        }
    }

    #[test]
    fn ordinary_confidence_is_not_warned() {
        // Counter-test: the floor must not fire on real configurations,
        // including the default itself and the boundary value.
        for v in ["0.1", "0.25", "0.5", "0.7", "0,7", "0.95", "1.0"] {
            let findings = validate(&cfg(&[("CONFIDENCE", v)]));
            assert!(
                !findings.iter().any(|f| f.key == "CONFIDENCE"),
                "CONFIDENCE={v} is a normal setting and must pass clean, got {findings:?}"
            );
        }
    }

    #[test]
    fn absent_confidence_is_not_warned() {
        // The common case: nothing in birdnet.conf, daemon applies the default.
        let findings = validate(&cfg(&[("LATITUDE", "42.36"), ("LONGITUDE", "-71.06")]));
        assert!(
            !findings.iter().any(|f| f.key == "CONFIDENCE"),
            "{findings:?}"
        );
    }

    #[test]
    fn confidence_above_one_is_error() {
        let c = cfg(&[("CONFIDENCE", "2.5")]);
        let findings = validate(&c);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "CONFIDENCE" && f.severity == Severity::Error)
        );
    }

    #[test]
    fn sf_thresh_negative_is_error() {
        let c = cfg(&[("SF_THRESH", "-0.1")]);
        let findings = validate(&c);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "SF_THRESH" && f.severity == Severity::Error)
        );
    }

    #[test]
    fn overlap_too_high_is_error() {
        let c = cfg(&[("OVERLAP", "5.0")]);
        let findings = validate(&c);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "OVERLAP" && f.severity == Severity::Error)
        );
    }

    #[test]
    fn sensitivity_out_of_range_is_error() {
        let c = cfg(&[("SENSITIVITY", "3.0")]);
        let findings = validate(&c);
        assert!(findings.iter().any(|f| f.key == "SENSITIVITY"));
    }

    #[test]
    fn recording_length_zero_is_error() {
        let c = cfg(&[("RECORDING_LENGTH", "0")]);
        let findings = validate(&c);
        assert!(findings.iter().any(|f| f.key == "RECORDING_LENGTH"));
    }

    #[test]
    fn audio_format_unknown_is_error() {
        let c = cfg(&[("AUDIO_FORMAT", "aiff")]);
        let findings = validate(&c);
        assert!(findings.iter().any(|f| f.key == "AUDIO_FORMAT"));
    }

    #[test]
    fn info_site_unknown_is_error() {
        let c = cfg(&[("INFO_SITE", "google")]);
        let findings = validate(&c);
        assert!(findings.iter().any(|f| f.key == "INFO_SITE"));
    }

    #[test]
    fn database_lang_three_letters_warns() {
        let c = cfg(&[("DATABASE_LANG", "eng")]);
        let findings = validate(&c);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "DATABASE_LANG" && f.severity == Severity::Warning)
        );
    }

    #[test]
    fn multiple_audio_sources_warns() {
        let c = cfg(&[("ALSA_CARD", "plughw:1,0"), ("RTSP_URL", "rtsp://x/y")]);
        let findings = validate(&c);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "AUDIO_SOURCE" && f.severity == Severity::Warning)
        );
    }

    #[test]
    fn non_numeric_confidence_is_error() {
        let c = cfg(&[("CONFIDENCE", "high")]);
        let findings = validate(&c);
        assert!(findings.iter().any(|f| f.key == "CONFIDENCE"));
    }

    #[test]
    fn empty_audio_source_value_does_not_count() {
        let c = cfg(&[("ALSA_CARD", ""), ("RTSP_URL", "rtsp://x/y")]);
        let findings = validate(&c);
        assert!(!findings.iter().any(|f| f.key == "AUDIO_SOURCE"));
    }

    #[test]
    fn is_usable_distinguishes_warnings_from_errors() {
        let warnings_only = vec![Finding::warn("X", "m", "r")];
        let with_error = vec![Finding::error("X", "m", "r")];
        assert!(is_usable(&warnings_only));
        assert!(!is_usable(&with_error));
    }

    #[test]
    fn is_clean_distinguishes_empty_from_populated() {
        // The counter-direction matters as much as the positive case: a
        // predicate that answers "clean" unconditionally would let every
        // caller above skip reporting real findings.
        assert!(is_clean(&[]));
        assert!(!is_clean(&[Finding::warn("X", "m", "r")]));
        assert!(!is_clean(&[Finding::error("X", "m", "r")]));
    }

    #[test]
    fn severity_displays_as_a_lowercase_word() {
        // Operators grep logs and the doctor output for these exact words,
        // so the rendering is contract, not cosmetics — an empty or
        // defaulted `Display` must not pass.
        assert_eq!(Severity::Warning.to_string(), "warning");
        assert_eq!(Severity::Error.to_string(), "error");
    }

    // ── Property-based tests ──────────────────────────────────────────────
    //
    // proptest generates randomised inputs across the entire reachable space
    // so we don't have to enumerate every edge case by hand. Each property
    // is a quantified statement that should hold for ALL inputs in the
    // range, not just the examples we thought of.

    use proptest::prelude::*;

    proptest! {
        /// Any in-range latitude/longitude pair must not produce a coordinate
        /// finding. Generated values cover the full valid surface.
        #[test]
        fn valid_coords_never_produce_coord_findings(
            lat in -90.0_f64..=90.0_f64,
            lon in -180.0_f64..=180.0_f64,
        ) {
            let c = cfg(&[
                ("LATITUDE", &lat.to_string()),
                ("LONGITUDE", &lon.to_string()),
            ]);
            let findings = validate(&c);
            prop_assert!(!findings.iter().any(|f| f.key == "LATITUDE" || f.key == "LONGITUDE"));
        }

        /// Any out-of-range latitude must yield a `LATITUDE` Error finding.
        /// Filter excludes the valid band so we test the failure mode only.
        #[test]
        fn out_of_range_latitude_always_errors(
            lat in prop_oneof![-1e6_f64..=-90.001_f64, 90.001_f64..=1e6_f64],
        ) {
            let c = cfg(&[("LATITUDE", &lat.to_string()), ("LONGITUDE", "0.0")]);
            let findings = validate(&c);
            prop_assert!(
                findings.iter().any(|f| f.key == "LATITUDE" && f.severity == Severity::Error),
                "lat={lat} did not produce a LATITUDE error; findings: {findings:?}"
            );
        }

        /// CONFIDENCE in [0, 1] is always accepted — it may draw the
        /// implausibly-low warning, but never an error, so a station with any
        /// in-range threshold always starts.
        #[test]
        fn valid_confidence_never_errors(c in 0.0_f64..=1.0_f64) {
            let cfg_ = cfg(&[("CONFIDENCE", &c.to_string())]);
            let findings = validate(&cfg_);
            prop_assert!(
                !findings.iter().any(|f| f.key == "CONFIDENCE" && f.severity == Severity::Error)
            );
        }

        /// The only CONFIDENCE finding in range is the low-threshold warning,
        /// and it fires exactly below the floor — never at or above it.
        #[test]
        fn confidence_warning_tracks_the_floor(c in 0.0_f64..=1.0_f64) {
            let cfg_ = cfg(&[("CONFIDENCE", &c.to_string())]);
            let findings = validate(&cfg_);
            let warned = findings.iter().any(|f| f.key == "CONFIDENCE");
            prop_assert_eq!(warned, c < CONFIDENCE_FLOOR, "c={}", c);
        }

        /// CONFIDENCE outside [0, 1] always errors. Restrict to a finite,
        /// non-NaN range so the assertion is well-defined.
        #[test]
        fn invalid_confidence_always_errors(
            c in prop_oneof![-1e6_f64..-0.0001_f64, 1.0001_f64..=1e6_f64],
        ) {
            let cfg_ = cfg(&[("CONFIDENCE", &c.to_string())]);
            let findings = validate(&cfg_);
            prop_assert!(
                findings.iter().any(|f| f.key == "CONFIDENCE" && f.severity == Severity::Error),
                "c={c} did not produce CONFIDENCE error"
            );
        }

        /// Any well-formed "fixed:HH:MM-HH:MM" schedule is accepted.
        #[test]
        fn well_formed_fixed_schedule_is_accepted(
            sh in 0_u8..24,
            sm in 0_u8..60,
            eh in 0_u8..24,
            em in 0_u8..60,
        ) {
            let raw = format!("fixed:{sh:02}:{sm:02}-{eh:02}:{em:02}");
            let c = cfg(&[("RECORDING_SCHEDULE", &raw)]);
            let findings = validate(&c);
            prop_assert!(
                !findings.iter().any(|f| f.key == "RECORDING_SCHEDULE"),
                "{raw} unexpectedly rejected: {findings:?}"
            );
        }

        /// Any HH:MM with an out-of-range hour is rejected.
        #[test]
        fn out_of_range_hour_rejected(
            sh in 24_u8..=99,
            sm in 0_u8..60,
            eh in 0_u8..24,
            em in 0_u8..60,
        ) {
            let raw = format!("fixed:{sh:02}:{sm:02}-{eh:02}:{em:02}");
            let c = cfg(&[("RECORDING_SCHEDULE", &raw)]);
            let findings = validate(&c);
            prop_assert!(
                findings.iter().any(|f| f.key == "RECORDING_SCHEDULE"),
                "{raw} unexpectedly accepted"
            );
        }

        /// Any HH:MM with an out-of-range minute (>= 60) is rejected.
        /// Pins the strict-less-than boundary check in `parse_hhmm` — caught
        /// by `cargo-mutants` when this case wasn't covered.
        #[test]
        fn out_of_range_minute_rejected(
            sh in 0_u8..24,
            sm in 60_u8..=99,
            eh in 0_u8..24,
            em in 0_u8..60,
        ) {
            let raw = format!("fixed:{sh:02}:{sm:02}-{eh:02}:{em:02}");
            let c = cfg(&[("RECORDING_SCHEDULE", &raw)]);
            let findings = validate(&c);
            prop_assert!(
                findings.iter().any(|f| f.key == "RECORDING_SCHEDULE"),
                "{raw} unexpectedly accepted"
            );
        }

        /// Validation should never panic, regardless of input contents.
        /// Generates arbitrary string pairs (within reasonable size) to make
        /// sure the validator survives whatever malformed user input arrives.
        #[test]
        fn validation_never_panics(
            keys in proptest::collection::vec("[A-Z_]{1,20}", 0..10),
            values in proptest::collection::vec(".{0,40}", 0..10),
        ) {
            let n = keys.len().min(values.len());
            let pairs: Vec<(&str, &str)> = (0..n)
                .map(|i| (keys[i].as_str(), values[i].as_str()))
                .collect();
            // Build a config from random pairs and assert validation returns
            // without panicking. Output content is unconstrained.
            let c = cfg(&pairs);
            let _ = validate(&c);
        }
    }
}
