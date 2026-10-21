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
    for &(key, min, max) in NUMERIC_RANGES {
        check_bounded(config, key, min, max, &mut out);
    }
    check_field_bounds(config, &mut out);
    check_confidence_floor(config, &mut out);
    check_confidence_ceiling(config, &mut out);
    check_sf_thresh_ceiling(config, &mut out);
    check_privacy_threshold_scale(config, &mut out);
    check_positive_int(
        config,
        "PRIVACY_RANK",
        PRIVACY_RANK_MIN,
        PRIVACY_RANK_MAX,
        &mut out,
    );
    check_positive_int(config, "RECORDING_LENGTH", 3, 60, &mut out);
    check_positive_int(
        config,
        "SEGMENT_DURATION",
        SEGMENT_DURATION_MIN,
        SEGMENT_DURATION_MAX,
        &mut out,
    );
    check_schedule(config, &mut out);
    check_audio_sources(config, &mut out);
    check_audio_format(config, &mut out);
    check_info_site(config, &mut out);
    check_full_disk(config, &mut out);
    check_notify_trigger(config, &mut out);
    check_lang(config, &mut out);
    check_private_mode(config, &mut out);
    check_public_access(config, &mut out);
    check_unknown_keys(config, &mut out);
    out
}

/// A key nothing reads is a setting the operator believes is in force and is
/// not (LC-7). A warning, not an error: the station runs, on the default.
fn check_unknown_keys(config: &Config, out: &mut Vec<Finding>) {
    for unknown in config.unknown_keys() {
        let fix = unknown.remedy();
        out.push(Finding::warn(unknown.key.clone(), unknown.to_string(), fix));
    }
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

/// A key's value, or `None` when it is absent **or blank**.
///
/// Every runtime reader treats `KEY=` as unset — `get_parsed(…).ok()` fails on
/// the empty string and falls back to the default — so a blank line is not an
/// error here either. It used to be one for eleven keys, and an error makes
/// the whole file unusable: `LATITUDE=` left over from an edit reverted every
/// other setting to the last-good copy, or ran the station web-only.
fn value_of<'a>(config: &'a Config, key: &str) -> Option<&'a str> {
    config.get(key).map(str::trim).filter(|v| !v.is_empty())
}

fn check_coords(config: &Config, out: &mut Vec<Finding>) {
    let lat_raw = value_of(config, "LATITUDE");
    let lon_raw = value_of(config, "LONGITUDE");

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

/// Every numeric setting the station reads as a bare number, with the range
/// outside which it cannot do its job.
///
/// # Why this is a table and not five literals
///
/// These bounds have two callers that must not disagree: [`validate`], which
/// `--doctor` runs against the config *file*, and the admin settings form,
/// which is where a value typed by a person actually arrives. Until 0.15.0
/// only the first existed, so `CONFIDENCE=75` — the percentage slip, typed
/// into a field labelled "Minimum Confidence (0-1)" — was saved, overlaid onto
/// the file config at startup **after** `validate` had already run, and
/// compared against a 0-to-1 model score for every three-second window. The
/// station recorded nothing, forever, and said nothing about why.
///
/// Only the keys [`validate`] already enforced are here. The two alert
/// thresholds the admin form also bounds (`APPRISE_MIN_CONFIDENCE`,
/// `EMAIL_MIN_CONFIDENCE`) are deliberately absent: an `Error` finding makes
/// `is_usable` false, and `startup_config::choose` then reverts the station to
/// its last-good configuration file. Turning a bad *alert* threshold into a
/// silent rollback of every setting is a far larger consequence than the
/// mistake, so the form bounds those two on its own and this list stays what
/// it was.
pub const NUMERIC_RANGES: &[(&str, f64, f64)] = &[
    ("CONFIDENCE", 0.0, 1.0),
    ("SF_THRESH", 0.0, 1.0),
    ("PRIVACY_THRESHOLD", 0.0, 1.0),
    ("SENSITIVITY", 0.5, 1.5),
    ("OVERLAP", 0.0, 2.9),
];

/// A number the settings pages and the configuration file both take, the range
/// the station can do something sensible with, and what it does outside it.
///
/// This is the other half of [`NUMERIC_RANGES`]. Those five are `Error`s
/// because outside them the station detects nothing. The fifteen below are not: the
/// daemon parses each one, and a value it cannot use is replaced by the default
/// or clamped, with a line in the log and nothing on screen. So here a value
/// outside the range is a **warning**, not an error — an `Error` makes
/// [`is_usable`] false, and `startup_config::choose` then reverts the *whole*
/// station to its last-good file, which for a mistyped retention period would
/// undo every other setting to punish it. The settings page and the settings API
/// are stricter, because they are where the number is typed: they refuse a
/// submission outside the range and say so ([`FieldBound::refusal`]).
///
/// What the daemon does outside the range is the `effect` column, from reading
/// its consumer: a claim here that the station "ignores" a value it in fact
/// applies would be worse than no claim.
#[derive(Debug, Clone, Copy)]
pub struct FieldBound {
    /// The settings-page field (and settings-table key).
    pub form_key: &'static str,
    /// The `birdnet.conf` key, or `""` for a setting only the settings table
    /// carries (the email port and cooldown, the rare-species window).
    pub config_key: &'static str,
    /// The label the page shows.
    pub label: &'static str,
    /// Smallest value the station can use.
    pub min: f64,
    /// Largest value the station can use.
    pub max: f64,
    /// Whether only whole numbers are meaningful.
    pub whole: bool,
    /// What the station does with a value outside the range.
    pub effect: &'static str,
}

impl FieldBound {
    /// Whether `value` is inside the range.
    #[must_use]
    pub fn contains(&self, value: f64) -> bool {
        value.is_finite() && value >= self.min && value <= self.max
    }

    /// The sentence the settings page gives for a refused submission of `raw`.
    #[must_use]
    pub fn refusal(&self, raw: &str) -> String {
        let kind = if self.whole {
            "a whole number"
        } else {
            "a number"
        };
        format!(
            "{} must be {kind} from {} to {}. {} You entered “{raw}”.",
            self.label,
            fmt_bound(self.min),
            fmt_bound(self.max),
            self.effect,
        )
    }

    /// Whether the text is acceptable for this field: blank (clears the
    /// setting), or a number in range of the right kind.
    #[must_use]
    pub fn accepts(&self, raw: &str) -> bool {
        let raw = raw.trim();
        if raw.is_empty() {
            return true;
        }
        let normalised = crate::config::locale::normalize_decimal(raw);
        if self.whole {
            // Digits, with one leading minus where the range goes negative.
            let body = normalised
                .strip_prefix('-')
                .filter(|_| self.min < 0.0)
                .unwrap_or(&normalised);
            if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
        }
        normalised.parse::<f64>().is_ok_and(|v| self.contains(v))
    }
}

fn fmt_bound(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

/// The fifteen bounded numeric settings. See [`FieldBound`].
pub const FIELD_BOUNDS: &[FieldBound] = &[
    FieldBound {
        form_key: "freq_shift_hz",
        config_key: "FREQ_SHIFT",
        label: "Frequency Shift",
        min: -12_000.0,
        max: 12_000.0,
        whole: true,
        effect: "Shifting by more than 12 kHz pushes the sound past what a clip's sample rate can carry; 0 turns it off.",
    },
    FieldBound {
        form_key: "pre_sunrise_offset",
        config_key: "PRE_SUNRISE_OFFSET",
        label: "Extra minutes before sunrise",
        min: 0.0,
        max: 120.0,
        whole: true,
        effect: "The station takes it as minutes of extra recording before sunrise; a value that does not parse is replaced by the twilight offset (30).",
    },
    FieldBound {
        form_key: "post_sunset_offset",
        config_key: "POST_SUNSET_OFFSET",
        label: "Extra minutes after sunset",
        min: 0.0,
        max: 120.0,
        whole: true,
        effect: "The station takes it as minutes of extra recording after sunset; a value that does not parse is replaced by the twilight offset (30).",
    },
    FieldBound {
        form_key: "notify_cooldown",
        config_key: "APPRISE_COOLDOWN",
        label: "Notification Cooldown",
        min: 0.0,
        max: 86_400.0,
        whole: true,
        effect: "It is seconds between alerts for the same species; a value that does not parse is replaced by the default (300), and more than a day silences a species for good.",
    },
    FieldBound {
        form_key: "deadman_hours",
        config_key: "DEADMAN_HOURS",
        label: "Silence alert after (hours)",
        min: 0.0,
        max: 8_760.0,
        whole: true,
        effect: "0 turns the alert off; a value that does not parse is replaced by the default (24), and more than a year can never fire.",
    },
    FieldBound {
        form_key: "clip_retention_days",
        config_key: "CLIP_RETENTION_DAYS",
        label: "Keep clips for (days)",
        min: 0.0,
        max: 36_500.0,
        whole: true,
        effect: "0 keeps clips for ever; any other value deletes the audio of every detection older than that many days, so a mistyped small number is not undoable.",
    },
    FieldBound {
        form_key: "max_files_per_species",
        config_key: "MAX_FILES_SPECIES",
        label: "Maximum clips per species",
        min: 0.0,
        max: 100_000.0,
        whole: true,
        effect: "0 keeps every clip; any other value deletes the oldest clips of a species beyond that many.",
    },
    FieldBound {
        form_key: "purge_threshold",
        config_key: "DISK_PURGE_THRESHOLD",
        label: "Disk-full threshold (percent)",
        min: 1.0,
        max: 100.0,
        whole: true,
        effect: "It is the percentage of the disk at which old recordings start being deleted: 0 would delete them continuously and anything above 100 never; the station uses 95 for a value outside 1 to 100.",
    },
    FieldBound {
        form_key: "extraction_length",
        config_key: "EXTRACTION_LENGTH",
        label: "Clip length (seconds)",
        min: 1.0,
        max: 60.0,
        whole: false,
        effect: "Seconds of audio saved around each detection; the station clamps it to 1 to 60 without saying so.",
    },
    FieldBound {
        form_key: "rare_species_days",
        config_key: "",
        label: "Rare-species window (days)",
        min: 0.0,
        max: 3_650.0,
        whole: true,
        effect: "Days without a sighting before a species counts as rare; the pages cap it at ten years.",
    },
    FieldBound {
        form_key: "stream_retention_secs",
        config_key: "STREAM_RETENTION_SECS",
        label: "Streaming Folder Retention (seconds)",
        min: 0.0,
        max: 604_800.0,
        whole: true,
        effect: "How long a raw segment waits in the temporary folder before being removed; 0 turns the age limit off, and more than a week keeps raw audio in a RAM-backed folder far longer than analysis needs.",
    },
    FieldBound {
        form_key: "stream_max_mb",
        config_key: "STREAM_MAX_MB",
        label: "Streaming Folder Limit (MB)",
        min: 0.0,
        max: 1_048_576.0,
        whole: true,
        effect: "A ceiling on the temporary folder; 0 turns it off. Above a terabyte it can never apply.",
    },
    FieldBound {
        form_key: "email_smtp_port",
        config_key: "",
        label: "SMTP Port",
        min: 1.0,
        max: 65_535.0,
        whole: true,
        effect: "A TCP port; a value that does not parse is replaced by 587.",
    },
    FieldBound {
        form_key: "email_cooldown_secs",
        config_key: "",
        label: "Email Cooldown (seconds)",
        min: 0.0,
        max: 86_400.0,
        whole: true,
        effect: "Seconds between emails for the same species; a value that does not parse is replaced by 300, and more than a day silences a species for good.",
    },
    FieldBound {
        form_key: "clip_target_lufs",
        config_key: "CLIP_TARGET_LUFS",
        label: "Even out clip volume (LUFS)",
        min: -40.0,
        max: -6.0,
        whole: false,
        effect: "A loudness target outside −40 to −6 is ignored and clips are saved as recorded.",
    },
];

/// A bounded setting present in the file with a value the station cannot use.
fn check_field_bounds(config: &Config, out: &mut Vec<Finding>) {
    for b in FIELD_BOUNDS.iter().filter(|b| !b.config_key.is_empty()) {
        let Some(raw) = value_of(config, b.config_key) else {
            continue;
        };
        if !b.accepts(raw) {
            out.push(Finding::warn(
                b.config_key,
                format!(
                    "{}={raw} is not {} from {} to {}: {}",
                    b.config_key,
                    if b.whole {
                        "a whole number"
                    } else {
                        "a number"
                    },
                    fmt_bound(b.min),
                    fmt_bound(b.max),
                    b.effect
                ),
                format!(
                    "set {} to a value from {} to {}, or remove the line to use the default",
                    b.config_key,
                    fmt_bound(b.min),
                    fmt_bound(b.max)
                ),
            ));
        }
    }
}

/// Threshold below which `CONFIDENCE` is treated as a probable mistake.
///
/// Not a hard floor — an operator who genuinely wants a firehose may set one —
/// but low enough that every ordinary configuration sits well above it.
const CONFIDENCE_FLOOR: f64 = 0.1;

/// Warn about a `CONFIDENCE` that is in range but implausibly low.
///
/// The [`NUMERIC_RANGES`] sweep already rejects the percentage mistake
/// (`CONFIDENCE=70`) and non-numeric junk as errors — it replaced the old
/// `check_unit_range` helper this line used to name. What a range check cannot
/// catch is the *decimal* slip —
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

/// Threshold above which `CONFIDENCE` is treated as a probable mistake.
///
/// The classifier's scores for a real, clear call sit mostly between 0.5 and
/// 0.9; a station asked for more than 0.95 records a few calls a day from a
/// dawn chorus of hundreds, and one asked for 0.99 records almost nothing at
/// all — which looks, from the dashboard, exactly like a dead microphone.
/// The floor has warned since the low-side slip was first seen; the high side
/// drew no finding (ON-5).
const CONFIDENCE_CEILING: f64 = 0.95;

/// Threshold above which `SF_THRESH` is treated as a probable mistake.
///
/// `SF_THRESH` is the occurrence probability below which the geomodel drops
/// a species from the candidate list. Its default is 0.03; even common
/// species sit under 0.3 for most weeks of the year, so a value above that
/// silences most of the list, and the station reports fewer species without
/// saying why.
const SF_THRESH_CEILING: f64 = 0.3;

/// Warn about a `CONFIDENCE` that is in range but implausibly high.
fn check_confidence_ceiling(config: &Config, out: &mut Vec<Finding>) {
    let Some(Ok(v)) = parse_float(config, "CONFIDENCE") else {
        return;
    };
    if v <= CONFIDENCE_CEILING {
        return;
    }
    out.push(Finding::warn(
        "CONFIDENCE",
        format!(
            "CONFIDENCE={v} is very high — only the clearest calls will be recorded, and a \
             quiet station is indistinguishable from a deaf one"
        ),
        format!(
            "unless this is deliberate, set CONFIDENCE={} (the default); review the \
             uncertain detections in the quarantine queue instead of raising the bar",
            super::DEFAULT_CONFIDENCE_THRESHOLD
        ),
    ));
}

/// Warn about an `SF_THRESH` that is in range but implausibly high.
fn check_sf_thresh_ceiling(config: &Config, out: &mut Vec<Finding>) {
    let Some(Ok(v)) = parse_float(config, "SF_THRESH") else {
        return;
    };
    if v <= SF_THRESH_CEILING {
        return;
    }
    out.push(Finding::warn(
        "SF_THRESH",
        format!(
            "SF_THRESH={v} is very high — the occurrence filter will drop most species from \
             the candidate list, including common ones outside their peak weeks"
        ),
        "unless this is deliberate, set SF_THRESH=0.03 (the default); the filter is meant to \
         remove species that never occur here, not to rank the ones that do",
    ));
}

/// `PRIVACY_THRESHOLD` above which the setting is treated as BirdNET-Pi's.
///
/// The key is shared and the scale is not. BirdNET-Pi reads it as a percentage
/// of the species list: a window is "human" if a human class ranks within the
/// top `max(10, 60 x value)` labels, and its install guidance uses values such
/// as 1 or 10. Here it is the model's own confidence for a human sound, where a
/// real voice at a distance scores 0.01-0.1 and typical settings are 0.01-0.03.
/// A BirdNET-Pi value carried across is in range (`1` is 100 %), validates
/// clean, and then requires a score of 1.0 before anything is masked: the
/// privacy filter the owner believes is on never fires.
const PRIVACY_THRESHOLD_CEILING: f64 = 0.5;

/// The smallest `PRIVACY_RANK` the station accepts: `0`, which switches the
/// rank rule off.
///
/// A constant of its own so that the two other places that bound the key — the
/// admin settings form and `--privacy-rank` — read the same number [`validate`]
/// does instead of typing a second `0`.
pub const PRIVACY_RANK_MIN: u64 = 0;

/// The largest `PRIVACY_RANK` the station accepts.
///
/// V2.4 has 6 522 classes and V3.0 has 11 560, so a rank past either masks
/// every window; BirdNET-Pi's own largest cutoff (`PRIVACY_THRESHOLD=100`)
/// is 6 000.
///
/// Read by [`validate`] for the config file and by the settings form for a
/// value typed on the page; `--privacy-rank` carries a clap range literal of
/// its own, which `the_privacy_rank_default_is_birdnet_pis_and_zero_switches_it_off`
/// holds to this.
pub const PRIVACY_RANK_MAX: u64 = 12_000;

/// The shortest `SEGMENT_DURATION` the station accepts, in seconds: the length
/// of one analysis window, since a chunk shorter than that cannot be analysed.
///
/// Constants of their own, like [`PRIVACY_RANK_MIN`], so that the settings form
/// and `PUT /api/v2/settings` read the numbers [`validate`] does instead of
/// typing a second `3` and `600`.
pub const SEGMENT_DURATION_MIN: u64 = 3;

/// The longest `SEGMENT_DURATION` the station accepts, in seconds.
///
/// Past ten minutes a chunk holds the station deaf for as long as it takes to
/// fill, and the stall watchdog's patience is derived from it. Read by
/// [`validate`] for the config file and by the settings form for a value typed
/// on the page; `--segment-duration` is a bare `u32` with no clap range.
pub const SEGMENT_DURATION_MAX: u64 = 600;

/// Warn about a `PRIVACY_THRESHOLD` that looks like BirdNET-Pi's scale.
fn check_privacy_threshold_scale(config: &Config, out: &mut Vec<Finding>) {
    let Some(Ok(v)) = parse_float(config, "PRIVACY_THRESHOLD") else {
        return;
    };
    if v <= PRIVACY_THRESHOLD_CEILING {
        return;
    }
    out.push(Finding::warn(
        "PRIVACY_THRESHOLD",
        format!(
            "PRIVACY_THRESHOLD={v} will almost never mask anything — it is the model's \
             confidence for a human sound, and a voice scores well below that. If this \
             value came from BirdNET-Pi, note that BirdNET-Pi reads the same key as a \
             percentage of the species list instead"
        ),
        "BirdNET-Pi's rule is PRIVACY_RANK here: for a BirdNET-Pi value p, set \
         PRIVACY_RANK=max(10, 60 x p) and PRIVACY_THRESHOLD=0. For a confidence-based \
         filter on top of it, use a small PRIVACY_THRESHOLD such as 0.02 (typical \
         range 0.01-0.03; lower masks more); 0 turns that rule off",
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
    let Some(raw) = value_of(config, key) else {
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
    if is_recording_schedule(raw) {
        // The grammar allows a window that ends at or before its start, which
        // the scheduler cannot express (`RecordingWindow::fixed`): it ignores
        // the schedule and the station records all day. Said here because
        // nothing else would: the daemon logs one line and the doctor, which
        // reads this, is where an owner who asked for a night-time window looks.
        if let Some((start, end)) = raw
            .trim()
            .strip_prefix("fixed:")
            .and_then(fixed_window_minutes)
            && start >= end
        {
            out.push(Finding::warn(
                "RECORDING_SCHEDULE",
                format!(
                    "RECORDING_SCHEDULE={:?} does not end after it starts: the scheduler cannot \
                     express a window that crosses midnight and ignores it, so the station \
                     records all day",
                    raw.trim()
                ),
                r#"use a window inside one day (fixed:06:00-20:00; the times are UTC), "solar", or "all-day""#
                    .to_string(),
            ));
        }
        return;
    }
    out.push(Finding::error(
        "RECORDING_SCHEDULE",
        format!(
            "RECORDING_SCHEDULE={:?} is not a recognised schedule",
            raw.trim()
        ),
        r#"use "all-day", "solar", or "fixed:HH:MM-HH:MM" (e.g. fixed:06:00-20:00)"#.to_string(),
    ));
}

/// Whether `raw` is a recording schedule this file accepts: `all-day`,
/// `solar`, blank (unset), or `fixed:HH:MM-HH:MM`.
///
/// Public so that the settings form and `PUT /api/v2/settings` ask the question
/// [`validate`] asks instead of keeping a second grammar. It is the *grammar*
/// only: the scheduler additionally refuses a fixed window that does not end
/// after it starts (`RecordingWindow::fixed`), which this accepts, so the form
/// checks that too.
#[must_use]
pub fn is_recording_schedule(raw: &str) -> bool {
    let raw = raw.trim();
    if matches!(raw, "all-day" | "solar" | "") {
        return true;
    }
    raw.strip_prefix("fixed:")
        .is_some_and(|window| fixed_window_minutes(window).is_some())
}

/// `HH:MM-HH:MM` as minutes since midnight, `(start, end)`, or `None` when it
/// is not two valid clock times.
///
/// Does not require `start < end`; see [`is_recording_schedule`].
#[must_use]
pub fn fixed_window_minutes(window: &str) -> Option<(u32, u32)> {
    let (start, end) = window.split_once('-')?;
    Some((parse_hhmm(start)?, parse_hhmm(end)?))
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let h: u8 = h.parse().ok()?;
    let m: u8 = m.parse().ok()?;
    (h < 24 && m < 60).then_some(u32::from(h) * 60 + u32::from(m))
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

/// The clip formats the station can write.
pub const AUDIO_FORMATS: [&str; 4] = ["wav", "mp3", "flac", "ogg"];

/// Whether `raw` names one of the [`AUDIO_FORMATS`], in any letter case.
#[must_use]
pub fn is_audio_format(raw: &str) -> bool {
    AUDIO_FORMATS.contains(&raw.trim().to_ascii_lowercase().as_str())
}

fn check_audio_format(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = value_of(config, "AUDIOFMT").or_else(|| value_of(config, "AUDIO_FORMAT"))
    else {
        return;
    };
    if !is_audio_format(raw) {
        out.push(Finding::error(
            "AUDIO_FORMAT",
            format!(
                "AUDIO_FORMAT={:?} is not supported",
                raw.trim().to_ascii_lowercase()
            ),
            r#"use "wav", "mp3", "flac", or "ogg" (non-WAV formats need ffmpeg or sox)"#
                .to_string(),
        ));
    }
}

/// The external sites a species page can link to, besides `none`.
pub const INFO_SITES: [&str; 3] = ["ebird", "allaboutbirds", "none"];

/// Whether `raw` is an [`INFO_SITES`] entry, in any letter case, or blank.
#[must_use]
pub fn is_info_site(raw: &str) -> bool {
    let raw = raw.trim().to_ascii_lowercase();
    raw.is_empty() || INFO_SITES.contains(&raw.as_str())
}

fn check_info_site(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = config.get("INFO_SITE") else {
        return;
    };
    if !is_info_site(raw) {
        out.push(Finding::error(
            "INFO_SITE",
            format!(
                "INFO_SITE={:?} is not recognised",
                raw.trim().to_ascii_lowercase()
            ),
            r#"use "ebird", "allaboutbirds", or "none""#.to_string(),
        ));
    }
}

/// What a full recordings disk may do (`FULL_DISK`).
pub const FULL_DISK_ACTIONS: [&str; 2] = ["purge", "keep"];

/// Whether `raw` is a [`FULL_DISK_ACTIONS`] entry, in any letter case.
#[must_use]
pub fn is_full_disk_action(raw: &str) -> bool {
    FULL_DISK_ACTIONS.contains(&raw.trim().to_ascii_lowercase().as_str())
}

/// A word the station does not know deletes the oldest clips, as when the key
/// is unset. A warning, not an error: it is a typo to fix, and an error here
/// would revert the whole station to its last-good file over it.
fn check_full_disk(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = value_of(config, "FULL_DISK") else {
        return;
    };
    if !is_full_disk_action(raw) {
        out.push(Finding::warn(
            "FULL_DISK",
            format!(
                "FULL_DISK={:?} is neither purge nor keep, so the station deletes the oldest \
                 clips when the disk is full, as when it is not set",
                raw.trim()
            ),
            r#"use "purge" or "keep" ("keep" stops recording and deletes nothing)"#.to_string(),
        ));
    }
}

/// The words `APPRISE_TRIGGER` (and the Notification trigger setting) takes,
/// alone or in a list: `each`, `new-species`, `new-species-daily`, or `off`.
/// The underscore spellings and the aliases `detection`, `none` and `never` are
/// read too.
pub const TRIGGER_WORDS: [&str; 9] = [
    "each",
    "detection",
    "new-species",
    "new_species",
    "new-species-daily",
    "new_species_daily",
    "off",
    "none",
    "never",
];

/// The words in a trigger list that are none of the [`TRIGGER_WORDS`].
#[must_use]
pub fn unknown_trigger_words(raw: &str) -> Vec<String> {
    raw.split([',', '+', ' ', ';'])
        .map(str::trim)
        .filter(|w| !w.is_empty() && !TRIGGER_WORDS.contains(&w.to_ascii_lowercase().as_str()))
        .map(str::to_owned)
        .collect()
}

/// A trigger word nothing reads is skipped, and a list of nothing but such
/// words notifies on every detection, which is not what a typo meant.
fn check_notify_trigger(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = value_of(config, "APPRISE_TRIGGER") else {
        return;
    };
    let unknown = unknown_trigger_words(raw);
    if !unknown.is_empty() {
        out.push(Finding::warn(
            "APPRISE_TRIGGER",
            format!(
                "APPRISE_TRIGGER has words nothing reads: {}",
                unknown.join(", ")
            ),
            "use each, new-species, new-species-daily, or off; several may be listed, \
             for example each,new-species"
                .to_string(),
        ));
    }
}

/// The names `PUBLIC_ACCESS` may carry.
///
/// The web crate owns the routes each one opens; its `private_mode` module
/// holds a gate that its list and this one agree, so a carve-out added there
/// is validated here.
pub const PUBLIC_ACCESS_NAMES: [&str; 3] = ["live_audio", "share", "metrics"];

/// The boolean spellings the config file and the settings form produce.
const BOOL_SPELLINGS: [&str; 8] = ["true", "false", "1", "0", "yes", "no", "on", "off"];

fn check_private_mode(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = config.get("PRIVATE_MODE") else {
        return;
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return;
    }
    if !BOOL_SPELLINGS.contains(&raw.to_ascii_lowercase().as_str()) {
        out.push(Finding::error(
            "PRIVATE_MODE",
            format!("PRIVATE_MODE={raw:?} is not a boolean; the station treats it as off"),
            "use true or false".to_string(),
        ));
    }
}

fn check_public_access(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = config.get("PUBLIC_ACCESS") else {
        return;
    };
    for name in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let normalised = name.to_ascii_lowercase().replace('-', "_");
        if !PUBLIC_ACCESS_NAMES.contains(&normalised.as_str()) {
            out.push(Finding::warn(
                "PUBLIC_ACCESS",
                format!(
                    "PUBLIC_ACCESS names {name:?}, which is not a carve-out; the station skips it"
                ),
                format!(
                    "use a comma-separated list of {}",
                    PUBLIC_ACCESS_NAMES.join(", ")
                ),
            ));
        }
    }
}

fn check_lang(config: &Config, out: &mut Vec<Finding>) {
    let Some(raw) = config.get("DATABASE_LANG").or_else(|| config.get("LANG")) else {
        return;
    };
    let raw = raw.trim();
    if raw.is_empty() || crate::i18n::find_language(raw).is_some() {
        return;
    }
    let codes: Vec<&str> = crate::i18n::LANGUAGES.iter().map(|l| l.code).collect();
    out.push(Finding::warn(
        "DATABASE_LANG",
        format!(
            "DATABASE_LANG={raw:?} is not a language species names are available in; \
             names are shown as the classifier gives them"
        ),
        format!("use one of: {}", codes.join(", ")),
    ));
}

fn parse_float(config: &Config, key: &str) -> Option<Result<f64, std::num::ParseFloatError>> {
    // Locale-tolerant — accepts both `42.36` and `42,36`.
    value_of(config, key).map(super::locale::parse_decimal)
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

    /// The grammar accepts a window that ends before it starts, and the
    /// scheduler (`RecordingWindow::fixed`) refuses it and records all day, which
    /// is the opposite of what someone asking for a night-time window wanted. A
    /// warning, not an error: an `Error` makes the file unusable and reverts the
    /// station to its last-good configuration, which is a worse answer than
    /// recording all day with the reason in `--doctor`.
    #[test]
    fn a_fixed_window_that_does_not_end_after_it_starts_is_a_warning() {
        for raw in ["fixed:20:00-06:00", "fixed:06:00-06:00"] {
            let findings = validate(&cfg(&[("RECORDING_SCHEDULE", raw)]));
            assert_eq!(findings.len(), 1, "{raw}: {findings:?}");
            assert_eq!(findings[0].severity, Severity::Warning, "{raw}");
            assert_eq!(findings[0].key, "RECORDING_SCHEDULE");
            assert!(
                findings[0].message.contains("all day"),
                "the message says what will happen: {}",
                findings[0].message
            );
            assert!(
                is_usable(&findings),
                "{raw} must not make the file unusable"
            );
        }
        // Counterparts: a window that ends after it starts, and the other
        // schedules, are clean.
        for raw in ["fixed:06:00-20:00", "fixed:00:00-23:59", "solar", "all-day"] {
            let findings = validate(&cfg(&[("RECORDING_SCHEDULE", raw)]));
            assert!(findings.is_empty(), "{raw}: {findings:?}");
        }
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

    /// ON-5: the high side warns as the low side does. `1.0` used to sit in
    /// the "normal" list below; a station at 1.0 records nothing, which is
    /// the case this exists for.
    #[test]
    fn implausibly_high_confidence_and_sf_thresh_are_warned() {
        for v in ["0.96", "0.99", "1.0"] {
            let findings = validate(&cfg(&[("CONFIDENCE", v)]));
            let hit = findings
                .iter()
                .find(|f| f.key == "CONFIDENCE")
                .unwrap_or_else(|| panic!("CONFIDENCE={v} must warn, got {findings:?}"));
            assert_eq!(
                hit.severity,
                Severity::Warning,
                "usable, so a warning: {hit:?}"
            );
            assert!(hit.message.contains("very high"), "{hit:?}");
        }
        let findings = validate(&cfg(&[("SF_THRESH", "0.5")]));
        assert!(
            findings
                .iter()
                .any(|f| f.key == "SF_THRESH" && f.severity == Severity::Warning),
            "{findings:?}"
        );
        // The boundaries themselves are inside the plausible range.
        assert!(validate(&cfg(&[("CONFIDENCE", "0.95"), ("SF_THRESH", "0.3")])).is_empty());
    }

    /// A BirdNET-Pi `PRIVACY_THRESHOLD` (a percentage of the species list: 1,
    /// 10) is in range here and means something else: nothing masks until the
    /// model is 100 % sure of a human sound. It warns, and the settings people
    /// really use here do not.
    #[test]
    fn a_birdnet_pi_privacy_threshold_is_warned_and_ordinary_ones_are_not() {
        for v in ["1", "1.0", "0.6"] {
            let findings = validate(&cfg(&[("PRIVACY_THRESHOLD", v)]));
            let hit = findings
                .iter()
                .find(|f| f.key == "PRIVACY_THRESHOLD")
                .unwrap_or_else(|| panic!("PRIVACY_THRESHOLD={v} must warn, got {findings:?}"));
            assert_eq!(
                hit.severity,
                Severity::Warning,
                "usable, so a warning: {hit:?}"
            );
            assert!(hit.message.contains("BirdNET-Pi"), "{hit:?}");
        }
        for v in ["0", "0.01", "0.02", "0.03", "0.05", "0.5"] {
            assert!(
                validate(&cfg(&[("PRIVACY_THRESHOLD", v)])).is_empty(),
                "PRIVACY_THRESHOLD={v} is a normal setting"
            );
        }
    }

    /// `PRIVACY_RANK` is a count of classes: 0 (off) to [`PRIVACY_RANK_MAX`],
    /// whole numbers only. The settings form refuses exactly what is refused
    /// here (`the_form_and_the_validator_agree_about_a_whole_number`), so this
    /// pins the edges that test compares against.
    #[test]
    fn privacy_rank_is_a_whole_number_from_zero_to_its_maximum() {
        let max = PRIVACY_RANK_MAX.to_string();
        let past = (PRIVACY_RANK_MAX + 1).to_string();
        for v in ["0", "10", "75", "600", max.as_str()] {
            assert!(
                validate(&cfg(&[("PRIVACY_RANK", v)])).is_empty(),
                "PRIVACY_RANK={v} is in range and must pass clean"
            );
        }
        for v in [past.as_str(), "-1", "0.5", "0,5", "10%", "abc"] {
            let findings = validate(&cfg(&[("PRIVACY_RANK", v)]));
            assert!(
                findings
                    .iter()
                    .any(|f| f.key == "PRIVACY_RANK" && f.severity == Severity::Error),
                "PRIVACY_RANK={v} must be an error, got {findings:?}"
            );
        }
        // A blank line is unset, as everywhere else in this file.
        assert!(validate(&cfg(&[("PRIVACY_RANK", "")])).is_empty());
        // And the documented range is the one the constants carry.
        assert_eq!((PRIVACY_RANK_MIN, PRIVACY_RANK_MAX), (0, 12_000));
    }

    /// `SEGMENT_DURATION` is a whole number of seconds from one analysis window
    /// to ten minutes. The settings form refuses exactly what is refused here
    /// (`the_form_and_the_validator_agree_about_a_whole_number`), so this pins
    /// the edges that test compares against.
    #[test]
    fn segment_duration_is_a_whole_number_of_seconds_from_three_to_six_hundred() {
        let past = (SEGMENT_DURATION_MAX + 1).to_string();
        for v in ["3", "4", "15", "600"] {
            assert!(
                validate(&cfg(&[("SEGMENT_DURATION", v)])).is_empty(),
                "SEGMENT_DURATION={v} is in range and must pass clean"
            );
        }
        for v in ["0", "2", past.as_str(), "-1", "15.5", "15,5", "15s", "abc"] {
            let findings = validate(&cfg(&[("SEGMENT_DURATION", v)]));
            assert!(
                findings
                    .iter()
                    .any(|f| f.key == "SEGMENT_DURATION" && f.severity == Severity::Error),
                "SEGMENT_DURATION={v} must be an error, got {findings:?}"
            );
        }
        // A blank line is unset, as everywhere else in this file.
        assert!(validate(&cfg(&[("SEGMENT_DURATION", "")])).is_empty());
        assert_eq!((SEGMENT_DURATION_MIN, SEGMENT_DURATION_MAX), (3, 600));
    }

    /// The settings form and the API ask [`is_recording_schedule`],
    /// [`is_audio_format`] and [`is_info_site`] instead of keeping a grammar of
    /// their own. `validate()` is built on the same three, so equality with it
    /// would hold for any grammar at all; what pins *this* grammar is the table
    /// of answers below, and the equality only shows the three checks still
    /// route through the predicates.
    #[test]
    fn the_shared_choice_predicates_say_what_validate_says() {
        let refused = |key: &str, v: &str| {
            validate(&cfg(&[(key, v)]))
                .iter()
                .any(|f| f.severity == Severity::Error)
        };
        // (value, accepted)
        let schedules = [
            ("all-day", true),
            ("solar", true),
            (" solar ", true),
            ("", true),
            ("fixed:06:00-20:00", true),
            ("fixed:00:00-23:59", true),
            // The grammar accepts a window that does not end after it starts;
            // the scheduler does not, so the settings form checks that too.
            ("fixed:20:00-06:00", true),
            ("fixed:6:0-7:0", true),
            ("SOLAR", false),
            ("Solar", false),
            ("allday", false),
            ("sunrise-sunset", false),
            ("fixed:", false),
            ("fixed:06:00", false),
            ("fixed:24:00-25:00", false),
            // Hour 24 on its own: the line above is refused for its 25 alone.
            ("fixed:24:00-23:00", false),
            ("fixed:00:00-24:00", false),
            ("fixed:06:60-07:00", false),
            ("fixed:06:00-07:60", false),
            ("fixed: 06:00-20:00", false),
            ("fixed:06:00 - 20:00", false),
            ("fixed:06:00-20:00-21:00", false),
            ("fixed:06:00-20:00 UTC", false),
        ];
        for (v, accepted) in schedules {
            assert_eq!(
                is_recording_schedule(v),
                accepted,
                "RECORDING_SCHEDULE={v:?}"
            );
            assert_eq!(
                !accepted,
                refused("RECORDING_SCHEDULE", v),
                "validate: {v:?}"
            );
        }
        let formats = [
            ("wav", true),
            ("mp3", true),
            ("flac", true),
            ("ogg", true),
            ("WAV", true),
            (" ogg ", true),
            ("aiff", false),
            ("S16_LE", false),
            ("mp4", false),
            ("wav,mp3", false),
        ];
        for (v, accepted) in formats {
            assert_eq!(is_audio_format(v), accepted, "AUDIOFMT={v:?}");
            assert_eq!(!accepted, refused("AUDIOFMT", v), "validate: {v:?}");
        }
        let sites = [
            ("ebird", true),
            ("allaboutbirds", true),
            ("none", true),
            ("eBird", true),
            (" none ", true),
            ("", true),
            ("wikipedia", false),
            ("all about birds", false),
        ];
        for (v, accepted) in sites {
            assert_eq!(is_info_site(v), accepted, "INFO_SITE={v:?}");
            assert_eq!(!accepted, refused("INFO_SITE", v), "validate: {v:?}");
        }
        // A window is its minutes since midnight, and does not have to end later.
        assert_eq!(fixed_window_minutes("06:30-20:00"), Some((390, 1200)));
        assert_eq!(fixed_window_minutes("20:00-06:00"), Some((1200, 360)));
        assert_eq!(fixed_window_minutes("25:00-06:00"), None);
        // The edges of the day: 23:59 is the last minute, and there is no hour 24.
        assert_eq!(fixed_window_minutes("00:00-23:59"), Some((0, 1439)));
        assert_eq!(fixed_window_minutes("00:00-24:00"), None);
        assert_eq!(fixed_window_minutes("24:00-06:00"), None);
        assert_eq!(fixed_window_minutes("00:00-06:60"), None);
    }

    #[test]
    fn ordinary_confidence_is_not_warned() {
        // Counter-test: neither the floor nor the ceiling may fire on real
        // configurations, including the default itself and both boundaries.
        for v in ["0.1", "0.25", "0.5", "0.7", "0,7", "0.95"] {
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
    fn database_lang_that_is_not_a_language_we_have_warns_and_names_the_choices() {
        for raw in ["eng", "xx", "zh", "english"] {
            let c = cfg(&[("DATABASE_LANG", raw)]);
            let findings = validate(&c);
            let f = findings
                .iter()
                .find(|f| f.key == "DATABASE_LANG")
                .unwrap_or_else(|| panic!("{raw:?} produced no finding"));
            assert_eq!(f.severity, Severity::Warning, "{raw:?}");
            assert!(
                f.remediation.contains("zh_CN") && f.remediation.contains("de"),
                "{}",
                f.remediation
            );
        }
    }

    #[test]
    fn database_lang_every_bundled_code_and_the_usual_spellings_are_clean() {
        for lang in crate::i18n::LANGUAGES {
            let c = cfg(&[("DATABASE_LANG", lang.code)]);
            assert!(is_clean(&validate(&c)), "{}", lang.code);
        }
        for raw in ["DE", "de_DE", "pt-BR", "zh-cn", " fr ", ""] {
            let c = cfg(&[("DATABASE_LANG", raw)]);
            assert!(is_clean(&validate(&c)), "{raw:?}");
        }
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

    /// `PRIVATE_MODE` takes the boolean spellings the file and the form
    /// produce, in any case, and nothing else. A value the station would
    /// silently read as "off" is an error here, because an operator who wrote
    /// `PRIVATE_MODE=enabled` believes the station is closed. The gate for
    /// the whole feature is end-to-end in the web crate; nothing in this
    /// crate read these two checks back, and cargo-mutants emptied both
    /// without a test noticing.
    #[test]
    fn private_mode_must_be_a_boolean_spelling() {
        for ok in [
            "true", "False", "1", "0", "YES", "no", "on", "Off", " true ",
        ] {
            let findings = validate(&cfg(&[("PRIVATE_MODE", ok)]));
            assert!(is_clean(&findings), "{ok:?}: {findings:?}");
        }
        let findings = validate(&cfg(&[("PRIVATE_MODE", "enabled")]));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[0].key, "PRIVATE_MODE");
        assert!(findings[0].message.contains("enabled"), "{findings:?}");
    }

    /// `PUBLIC_ACCESS` names only the carve-outs the web crate implements.
    /// Hyphens, case, spaces and a trailing comma are forgiven; an unknown
    /// name is a warning that quotes it, one finding per unknown name.
    ///
    /// A warning, not an error: the station skips the name, which closes that
    /// one surface — the safe direction. As an error it made the whole file
    /// unusable, so a misspelt carve-out reverted every other setting in the
    /// edit to the last-good copy, or ran the station web-only.
    #[test]
    fn public_access_names_only_the_carve_outs_that_exist() {
        let findings = validate(&cfg(&[(
            "PUBLIC_ACCESS",
            "live_audio, Share,METRICS, live-audio,",
        )]));
        assert!(is_clean(&findings), "{findings:?}");

        let findings = validate(&cfg(&[("PUBLIC_ACCESS", "share,live_audo,dashboard")]));
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert!(
            findings
                .iter()
                .all(|f| f.severity == Severity::Warning && f.key == "PUBLIC_ACCESS"),
            "{findings:?}"
        );
        assert!(
            is_usable(&findings),
            "a misspelt carve-out must not revert the file"
        );
        assert!(findings[0].message.contains("live_audo"), "{findings:?}");
        assert!(findings[1].message.contains("dashboard"), "{findings:?}");
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
        fn confidence_warning_tracks_the_floor_and_the_ceiling(c in 0.0_f64..=1.0_f64) {
            let cfg_ = cfg(&[("CONFIDENCE", &c.to_string())]);
            let findings = validate(&cfg_);
            let warned = findings.iter().any(|f| f.key == "CONFIDENCE");
            prop_assert_eq!(warned, c < CONFIDENCE_FLOOR || c > CONFIDENCE_CEILING, "c={}", c);
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

        /// Any well-formed "fixed:HH:MM-HH:MM" schedule is accepted: it is never
        /// an `Error`, and it carries a `Warning` exactly when the window does not
        /// end after it starts (the scheduler ignores such a window).
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
            let about_it: Vec<_> = findings
                .iter()
                .filter(|f| f.key == "RECORDING_SCHEDULE")
                .collect();
            prop_assert!(
                about_it.iter().all(|f| f.severity == Severity::Warning),
                "{raw} unexpectedly rejected: {findings:?}"
            );
            let start = u32::from(sh) * 60 + u32::from(sm);
            let end = u32::from(eh) * 60 + u32::from(em);
            prop_assert_eq!(
                !about_it.is_empty(),
                start >= end,
                "{}: a warning exactly when the window does not end after it starts: {:?}",
                raw,
                findings
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

    /// `FULL_DISK` is BirdNET-Pi's own key and is read as it is: `keep` and
    /// `purge` in any case are fine, anything else is a warning that says what
    /// the station does instead.
    #[test]
    fn full_disk_takes_purge_or_keep_and_warns_about_anything_else() {
        for ok in ["purge", "keep", "KEEP", " Purge "] {
            let cfg = Config::parse(&format!("FULL_DISK={ok}")).unwrap();
            assert!(
                validate(&cfg).iter().all(|f| f.key != "FULL_DISK"),
                "{ok:?} drew a finding"
            );
        }
        let cfg = Config::parse("FULL_DISK=erase").unwrap();
        let findings = validate(&cfg);
        let f = findings
            .iter()
            .find(|f| f.key == "FULL_DISK")
            .expect("a finding");
        assert_eq!(
            f.severity,
            Severity::Warning,
            "an error would revert the whole file"
        );
        assert!(
            f.message.contains("deletes the oldest clips"),
            "{}",
            f.message
        );
    }

    /// The trigger is a word or a list of words; a word nothing reads is named,
    /// as a warning (an error would revert every other setting).
    #[test]
    fn the_notification_trigger_takes_a_list_and_names_a_word_it_cannot_read() {
        for ok in [
            "each",
            "new-species,new-species-daily",
            "off",
            "each, new_species",
        ] {
            let cfg = Config::parse(&format!("APPRISE_TRIGGER={ok}")).unwrap();
            assert!(
                validate(&cfg).iter().all(|f| f.key != "APPRISE_TRIGGER"),
                "{ok:?} drew a finding"
            );
        }
        let cfg = Config::parse("APPRISE_TRIGGER=each,sometimes").unwrap();
        let findings = validate(&cfg);
        let f = findings
            .iter()
            .find(|f| f.key == "APPRISE_TRIGGER")
            .expect("a finding");
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.message.contains("sometimes"), "{}", f.message);
    }

    /// A key BirdNET-Pi writes is told where it went, or that it went nowhere:
    /// "not a setting this station reads" sent the operator nowhere.
    #[test]
    fn a_birdnet_pi_key_is_told_where_it_went() {
        let cfg = Config::parse(
            "BIRDWEATHER_ID=abc\nPURGE_THRESHOLD=90\nAPPRISE_ONLY_NOTIFY_SPECIES_NAMES=Crow\n\
             COLOR_SCHEME=dark\nRAW_SPECTROGRAM=1\nSITE_NAME=Home\nLATITUDE=42.36\nLONGITUDE=-71.06\n",
        )
        .unwrap();
        let findings = validate(&cfg);
        let of = |key: &str| {
            findings
                .iter()
                .find(|f| f.key == key)
                .unwrap_or_else(|| panic!("{key} drew no finding: {findings:?}"))
        };
        assert_eq!(
            of("BIRDWEATHER_ID").remediation,
            "rename it to BIRDWEATHER_TOKEN (without it BirdWeather uploads stop)"
        );
        assert_eq!(
            of("PURGE_THRESHOLD").remediation,
            "rename it to DISK_PURGE_THRESHOLD (without it the station purges at 95 %)"
        );
        // The name that misleads: it is the exclusion list, and says so.
        assert_eq!(
            of("APPRISE_ONLY_NOTIFY_SPECIES_NAMES").remediation,
            "rename it to APPRISE_WATCHLIST_EXCLUDE (BirdNET-Pi's key is the EXCLUSION list despite its name)"
        );
        assert!(
            of("COLOR_SCHEME")
                .remediation
                .starts_with("it has no equivalent here:"),
            "{}",
            of("COLOR_SCHEME").remediation
        );
        assert!(
            of("RAW_SPECTROGRAM")
                .remediation
                .contains("`raw_spectrogram`")
        );
        // A spelling suggestion would have sent SITE_NAME to SITENAME by luck of
        // edit distance; the BirdNET-Pi table says it on purpose.
        assert_eq!(of("SITE_NAME").remediation, "rename it to SITENAME");
        assert!(
            of("BIRDWEATHER_ID")
                .message
                .contains("it is a BirdNET-Pi key"),
            "{}",
            of("BIRDWEATHER_ID").message
        );
    }

    /// The gate for LC-7: a misspelt key is reported, named, with the key it
    /// was meant to be; a file of real keys draws no such finding.
    #[test]
    fn a_misspelt_key_is_reported_with_the_key_it_was_meant_to_be() {
        let cfg = Config::parse("CONFIDENC=0.90\nLATITUDE=42.36\nLONGITUDE=-71.06").unwrap();
        let findings = validate(&cfg);
        let f = findings
            .iter()
            .find(|f| f.key == "CONFIDENC")
            .unwrap_or_else(|| panic!("CONFIDENC drew no finding: {findings:?}"));
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(
            f.message,
            "CONFIDENC is not a setting this station reads; did you mean CONFIDENCE?"
        );
        assert_eq!(f.remediation, "rename it to CONFIDENCE");

        // Counterpart: the same file with the key spelt right draws nothing.
        let cfg = Config::parse("CONFIDENCE=0.90\nLATITUDE=42.36\nLONGITUDE=-71.06").unwrap();
        assert!(
            validate(&cfg).is_empty(),
            "a correct file drew findings: {:?}",
            validate(&cfg)
        );
    }

    /// A blank value is an unset one, for every key the station knows — the
    /// runtime's reading. Eleven keys used to answer `KEY=` with an error, and
    /// an error reverts the whole file.
    #[test]
    fn a_blank_value_is_unset_not_an_error() {
        let mut wrong = Vec::new();
        for key in crate::config::known_keys::KNOWN_CONFIG_KEYS {
            let config = Config::parse(&format!("{key}=\n")).unwrap();
            let errors: Vec<_> = validate(&config)
                .into_iter()
                .filter(|f| f.severity == Severity::Error)
                .map(|f| f.message)
                .collect();
            if !errors.is_empty() {
                wrong.push(format!("{key}= -> {errors:?}"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");

        // Counterpart: a value that is there and wrong is still an error.
        for (key, bad) in [
            ("LATITUDE", "abc"),
            ("CONFIDENCE", "x"),
            ("SEGMENT_DURATION", "-1"),
        ] {
            let config = Config::parse(&format!("{key}={bad}\n")).unwrap();
            assert!(!is_usable(&validate(&config)), "{key}={bad} passed");
        }
    }

    /// The fifteen bounded settings: each takes its own edges and refuses one
    /// step outside, in the file (a warning, never an error) and in the form.
    #[test]
    fn each_bounded_field_takes_its_edges_and_refuses_beyond_them() {
        assert_eq!(FIELD_BOUNDS.len(), 15);
        for b in FIELD_BOUNDS {
            let step = if b.whole { 1.0 } else { 0.5 };
            let at = |v: f64| format!("{v}");
            assert!(b.accepts(&at(b.min)), "{} refuses its minimum", b.form_key);
            assert!(b.accepts(&at(b.max)), "{} refuses its maximum", b.form_key);
            assert!(
                !b.accepts(&at(b.min - step)),
                "{} takes a value below its minimum",
                b.form_key
            );
            assert!(
                !b.accepts(&at(b.max + step)),
                "{} takes a value above its maximum",
                b.form_key
            );
            assert!(b.accepts(""), "{} refuses blank", b.form_key);
            assert!(!b.accepts("abc"), "{} takes text", b.form_key);
            assert!(!b.accepts("NaN"), "{} takes NaN", b.form_key);
            assert!(!b.accepts("inf"), "{} takes infinity", b.form_key);
            if b.whole {
                assert!(!b.accepts("1.5"), "{} takes a fraction", b.form_key);
            }
            let said = b.refusal("9999999");
            assert!(said.contains(b.label) && said.contains("9999999"), "{said}");
        }
    }

    /// The motivating slip: the percentage typed into a fraction-sized field.
    /// Out-of-range numbers in the file are warnings (an error would roll the
    /// whole station back to its last-good file), and the finding names the key.
    #[test]
    fn a_bounded_value_in_the_file_is_a_warning_and_never_an_error() {
        for b in FIELD_BOUNDS.iter().filter(|b| !b.config_key.is_empty()) {
            let bad = format!("{}", b.max + 1_000.0);
            let findings = validate(&cfg(&[(b.config_key, bad.as_str())]));
            let mine: Vec<&Finding> = findings.iter().filter(|f| f.key == b.config_key).collect();
            assert!(
                mine.iter().any(|f| f.severity == Severity::Warning),
                "{}={bad} raised no warning",
                b.config_key
            );
            assert!(
                findings.iter().all(|f| f.severity != Severity::Error),
                "{}={bad} must not make the station unusable: {findings:?}",
                b.config_key
            );
            assert!(is_usable(&findings));
            // The counterpart: the edge itself is clean.
            let ok = format!("{}", b.max);
            let findings = validate(&cfg(&[(b.config_key, ok.as_str())]));
            assert!(
                findings.iter().all(|f| f.key != b.config_key),
                "{}={ok} was flagged: {findings:?}",
                b.config_key
            );
        }
    }
}
