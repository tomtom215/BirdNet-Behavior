//! Integration tests for the `birdnet_core::config::validate` public API.
//!
//! Unit tests inside the module exercise internal helpers; these tests
//! exercise the validator through its published surface only, so that an
//! accidental privatisation or signature change here is caught even when
//! every internal test still passes.

use birdnet_core::config::Config;
use birdnet_core::config::validate::{self, Severity};

/// Build a Config from an inline INI-style body.
fn parse(body: &str) -> Config {
    Config::parse(body).expect("test fixture should parse")
}

#[test]
fn realistic_user_config_validates_clean() {
    // Mirrors the worked .env.example values exactly so this test fails if
    // the example ever drifts out of sync with the validator's expectations.
    let body = "\
LATITUDE=42.3601
LONGITUDE=-71.0589
CONFIDENCE=0.7
SENSITIVITY=1.0
SF_THRESH=0.03
PRIVACY_THRESHOLD=0.0
OVERLAP=0.0
RECORDING_LENGTH=15
SEGMENT_DURATION=15
RECORDING_SCHEDULE=solar
AUDIO_FORMAT=wav
INFO_SITE=ebird
DATABASE_LANG=en
ALSA_CARD=plughw:1,0
";
    let cfg = parse(body);
    let findings = validate::validate(&cfg);
    assert!(
        validate::is_clean(&findings),
        "realistic user config produced findings: {findings:?}"
    );
    assert!(validate::is_usable(&findings));
}

#[test]
fn broken_user_config_reports_each_issue_with_actionable_remediation() {
    // A pathological config that violates several rules at once. Each
    // finding must carry both a message AND a non-empty remediation string —
    // a finding with no remediation is a bug in the validator.
    let body = "\
LATITUDE=200.0
LONGITUDE=300.0
CONFIDENCE=5.0
SF_THRESH=-0.1
OVERLAP=10.0
RECORDING_SCHEDULE=fixed:25:99-99:99
AUDIO_FORMAT=aiff
INFO_SITE=google
ALSA_CARD=plughw:1,0
RTSP_URL=rtsp://x/y
PIPEWIRE_DEVICE=default
";
    let cfg = parse(body);
    let findings = validate::validate(&cfg);

    // Expect at least these failure keys.
    let keys: Vec<&str> = findings.iter().map(|f| f.key.as_str()).collect();
    for expected in [
        "LATITUDE",
        "LONGITUDE",
        "CONFIDENCE",
        "SF_THRESH",
        "OVERLAP",
        "RECORDING_SCHEDULE",
        "AUDIO_FORMAT",
        "INFO_SITE",
        "AUDIO_SOURCE",
    ] {
        assert!(
            keys.contains(&expected),
            "expected to see a finding for {expected}; got {keys:?}"
        );
    }

    // Every finding must have non-empty message and remediation strings.
    for f in &findings {
        assert!(
            !f.message.is_empty(),
            "finding {:?} has empty message",
            f.key
        );
        assert!(
            !f.remediation.is_empty(),
            "finding {:?} has no remediation — users would be stuck",
            f.key
        );
    }

    // The aggregate must be unusable (at least one Error finding).
    assert!(!validate::is_usable(&findings));
}

#[test]
fn warnings_alone_do_not_make_config_unusable() {
    // LATITUDE set but LONGITUDE missing is a warning, not an error,
    // because the daemon still starts (just with solar schedule disabled).
    let body = "\
LATITUDE=42.0
CONFIDENCE=0.7
";
    let cfg = parse(body);
    let findings = validate::validate(&cfg);

    // Should have exactly one finding: a LONGITUDE warning.
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].key, "LONGITUDE");
    assert_eq!(findings[0].severity, Severity::Warning);

    // Config is usable because all findings are warnings only.
    assert!(validate::is_usable(&findings));
    assert!(!validate::is_clean(&findings)); // not "clean" (findings exist)
}

#[test]
fn severity_displays_as_lowercase_word() {
    assert_eq!(Severity::Warning.to_string(), "warning");
    assert_eq!(Severity::Error.to_string(), "error");
}

#[test]
fn empty_config_is_both_clean_and_usable() {
    let cfg = parse("");
    let findings = validate::validate(&cfg);
    assert!(findings.is_empty());
    assert!(validate::is_clean(&findings));
    assert!(validate::is_usable(&findings));
}

#[test]
fn comments_and_whitespace_do_not_confuse_validator() {
    let body = "\
# This is a comment
   # indented comment

LATITUDE=42.0   # comments after values are NOT parsed; the trailing ' # comment' becomes the value
LONGITUDE = -71.0
";
    let cfg = parse(body);
    // The point of this test is to ensure the validator doesn't panic on
    // weird-but-syntactically-valid input — not to assert specific findings.
    let _ = validate::validate(&cfg);
}
