//! Report rendering: human-readable text, machine-readable JSON, and the
//! worst-severity → exit-code summary.

use super::{Check, Status};

/// Render checks + summary as a single-line JSON object.
///
/// Hand-rolled rather than `serde_json::to_string` derive because the
/// shape is small, fixed, and we want to keep the diagnostic surface
/// free of macro magic that would obscure the contract. Strings are
/// escaped per RFC 8259 §7 (handles `\`, `"`, control chars, surrogate
/// pairs are not produced).
#[must_use]
pub(super) fn render_json(checks: &[Check], exit_code: i32) -> String {
    let (passed, warnings, errors, skipped) = tally(checks);
    let mut out = String::with_capacity(512 + checks.len() * 96);
    out.push_str("{\"summary\":{");
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!(
            "\"passed\":{passed},\"warnings\":{warnings},\"errors\":{errors},\"skipped\":{skipped},\"exit_code\":{exit_code}"
        ),
    );
    out.push_str("},\"checks\":[");
    for (i, c) in checks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let status = match c.status {
            Status::Pass => "pass",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Skip => "skip",
        };
        out.push_str("{\"status\":\"");
        out.push_str(status);
        out.push_str("\",\"name\":");
        push_json_str(&mut out, &c.name);
        out.push_str(",\"message\":");
        push_json_str(&mut out, &c.message);
        out.push_str(",\"remediation\":");
        if let Some(r) = &c.remediation {
            push_json_str(&mut out, r);
        } else {
            out.push_str("null");
        }
        out.push('}');
    }
    out.push_str("]}");
    out
}

fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Summarise check results into a single exit code.
#[must_use]
pub(super) fn summarise(checks: &[Check]) -> i32 {
    let mut worst = Status::Pass;
    for c in checks {
        if c.status > worst {
            worst = c.status;
        }
    }
    match worst {
        Status::Pass | Status::Skip => 0,
        Status::Warn => 1,
        Status::Fail => 2,
    }
}

/// Render the full diagnostic report as a single string of text.
///
/// Pure function with no I/O — every byte of the human-readable
/// `--doctor` output goes through here. Split out from the I/O wrapper
/// so it can be snapshot-tested against a golden file: a drift in the
/// user-facing format requires updating the snapshot, which has to be
/// reviewed in a PR.
#[must_use]
pub(super) fn render_text(checks: &[Check]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(checks.len() * 128 + 256);
    let _ = writeln!(out);
    let _ = writeln!(out, "BirdNet-Behavior preflight report");
    let _ = writeln!(out, "=================================");
    let _ = writeln!(out);
    for c in checks {
        let _ = write!(out, "{c}");
    }

    let (passes, warns, fails, skips) = tally(checks);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Summary: {passes} passed, {warns} warning(s), {fails} error(s), {skips} skipped."
    );
    if fails > 0 {
        let _ = writeln!(
            out,
            "Status:  NOT READY — fix the errors above before starting the detection daemon."
        );
    } else if warns > 0 {
        let _ = writeln!(
            out,
            "Status:  READY WITH WARNINGS — the daemon will start but some features are degraded."
        );
    } else {
        let _ = writeln!(
            out,
            "Status:  READY — start the daemon with `birdnet-behavior`."
        );
    }
    let _ = writeln!(out);
    out
}

fn tally(checks: &[Check]) -> (usize, usize, usize, usize) {
    let mut p = 0;
    let mut w = 0;
    let mut f = 0;
    let mut s = 0;
    for c in checks {
        match c.status {
            Status::Pass => p += 1,
            Status::Warn => w += 1,
            Status::Fail => f += 1,
            Status::Skip => s += 1,
        }
    }
    (p, w, f, s)
}

#[cfg(test)]
mod tests {
    use super::{render_json, render_text, summarise};
    use crate::doctor::Check;

    #[test]
    fn summarise_reports_worst_status() {
        let checks = vec![
            Check::pass("a", "ok"),
            Check::warn("b", "m", "fix"),
            Check::pass("c", "ok"),
        ];
        assert_eq!(summarise(&checks), 1);

        let mut with_fail = checks;
        with_fail.push(Check::fail("d", "broken", "fix"));
        assert_eq!(summarise(&with_fail), 2);

        let only_pass = vec![Check::pass("a", "ok"), Check::skip("b", "n/a")];
        assert_eq!(summarise(&only_pass), 0);
    }

    #[test]
    fn empty_checks_pass() {
        assert_eq!(summarise(&[]), 0);
    }

    // ── JSON rendering ─────────────────────────────────────────────────────

    #[test]
    fn json_summary_reflects_tally() {
        let checks = vec![
            Check::pass("a", "ok"),
            Check::warn("b", "m", "fix"),
            Check::fail("c", "broken", "fix"),
            Check::skip("d", "n/a"),
        ];
        let json = render_json(&checks, 2);
        assert!(json.contains("\"passed\":1"));
        assert!(json.contains("\"warnings\":1"));
        assert!(json.contains("\"errors\":1"));
        assert!(json.contains("\"skipped\":1"));
        assert!(json.contains("\"exit_code\":2"));
        assert!(json.contains("\"status\":\"pass\""));
        assert!(json.contains("\"status\":\"warn\""));
        assert!(json.contains("\"status\":\"fail\""));
        assert!(json.contains("\"status\":\"skip\""));
    }

    /// `--doctor-json` is documented for monitoring integrations (Nagios,
    /// Zabbix, a Prometheus textfile collector, a Home Assistant command
    /// sensor). Every one of those parses the output, so "looks escaped" is not
    /// the bar — it has to *parse*, for any check text the station can produce.
    ///
    /// The existing gate below asserts the document ends with `}`, which a
    /// malformed document also does. This one round-trips it through a real
    /// JSON parser with adversarial payloads.
    #[test]
    fn the_json_document_parses_for_hostile_check_text() {
        let nasty = [
            "quote\" and backslash\\",
            "newline\nreturn\rtab\t",
            "bell\u{7}nul-ish\u{1}vertical\u{b}",
            "unicode: Grünspecht 🐦 日本語",
            "}{\"]:,",
            "",
        ];
        let checks: Vec<Check> = nasty
            .iter()
            .map(|t| Check::warn(*t, *t, (*t).to_string()))
            .collect();
        let json = render_json(&checks, 1);
        let parsed: serde_json::Value =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("not valid JSON: {e}\n{json}"));

        // …and the text survives the round trip byte for byte, so a monitoring
        // system reads the message the operator would.
        let arr = parsed["checks"].as_array().expect("checks array");
        assert_eq!(arr.len(), nasty.len());
        for (got, want) in arr.iter().zip(nasty.iter()) {
            assert_eq!(got["name"].as_str(), Some(*want));
            assert_eq!(got["message"].as_str(), Some(*want));
            assert_eq!(got["remediation"].as_str(), Some(*want));
        }
        assert_eq!(parsed["summary"]["exit_code"].as_i64(), Some(1));
    }

    #[test]
    fn json_escapes_control_characters_and_quotes() {
        let c = Check::warn("name\"X", "line1\nline2\twith\\backslash", "fix\rme");
        let json = render_json(&[c], 1);
        // Must not contain unescaped specials in the string payload.
        assert!(json.contains("name\\\"X"), "{json}");
        assert!(json.contains("line1\\nline2\\twith\\\\backslash"), "{json}");
        assert!(json.contains("fix\\rme"), "{json}");
        // Must be parseable as a JSON object (last character is `}`).
        assert!(json.ends_with('}'));
    }

    #[test]
    fn json_omits_remediation_as_null() {
        let c = Check::pass("a", "ok");
        let json = render_json(&[c], 0);
        assert!(json.contains("\"remediation\":null"), "{json}");
    }

    #[test]
    fn json_empty_check_list_still_valid() {
        let json = render_json(&[], 0);
        // Empty arrays and zeroed summary.
        assert!(json.contains("\"checks\":[]"));
        assert!(json.contains("\"passed\":0"));
    }

    #[test]
    fn json_handles_low_codepoint_via_unicode_escape() {
        // U+0001 is below the 0x20 cut-off and must be -encoded.
        let c = Check::pass("a", "x\u{0001}y");
        let json = render_json(&[c], 0);
        assert!(json.contains("x\\u0001y"), "{json}");
    }

    // ── Snapshot tests for the human-readable report ─────────────────────
    //
    // Pin the exact bytes of the `--doctor` output against a golden file so
    // accidental wording or formatting drifts have to come through a PR. The
    // input is a hand-curated fixture (no live filesystem / network access)
    // so the snapshot is deterministic across hosts.
    //
    // To update after an intentional UX change:
    //   UPDATE_DOCTOR_SNAPSHOTS=1 cargo test -p birdnet-behavior --bin birdnet-behavior \
    //       -- doctor::render::tests::snapshot
    // and review the resulting diff against
    // `src/testdata/doctor_snapshots/*.txt`.

    const SNAPSHOT_DIR: &str = "src/testdata/doctor_snapshots";

    fn sample_all_pass() -> Vec<Check> {
        vec![
            Check::pass("CPU cores", "4 cores available for audio + inference"),
            Check::pass("Temp directory", "/tmp is writable"),
            Check::pass(
                "Configuration file",
                "loaded from /etc/birdnet/birdnet.conf",
            ),
            Check::pass(
                "Configuration values",
                "all settings are within valid ranges",
            ),
            Check::pass(
                "Web listen address",
                "127.0.0.1:8502 parses as a valid socket address",
            ),
            Check::pass("Database directory", "/var/lib/birdnet is writable"),
            Check::pass(
                "Database integrity",
                "/var/lib/birdnet/birds.db passes integrity check",
            ),
            Check::pass(
                "Recordings directory",
                "/var/lib/birdnet/recordings is writable",
            ),
            Check::pass("Audio source", "ALSA source configured"),
            Check::pass(
                "ALSA device probe",
                "plughw:1,0 matches an entry in `arecord -l`",
            ),
            Check::pass(
                "ONNX model file",
                "/usr/share/birdnet/model.onnx (541000000 bytes)",
            ),
            Check::pass(
                "Disk space",
                "120 GiB free on the volume containing /var/lib/birdnet/recordings",
            ),
        ]
    }

    fn sample_mixed() -> Vec<Check> {
        vec![
            Check::pass("CPU cores", "4 cores available for audio + inference"),
            Check::pass("Temp directory", "/tmp is writable"),
            Check::warn(
                "Configuration file",
                "/etc/birdnet/birdnet.conf not found — using built-in defaults",
                "copy .env.example to /etc/birdnet/birdnet.conf and edit before going to production",
            ),
            Check::pass(
                "Web listen address",
                "127.0.0.1:8502 parses as a valid socket address",
            ),
            Check::warn(
                "Database directory",
                "/var/lib/birdnet does not exist yet — will be created on first run",
                "no action needed unless you want to pre-create it with `mkdir -p`",
            ),
            Check::skip(
                "Database integrity",
                "no database file yet — will be created on first run",
            ),
            Check::skip(
                "Recordings directory",
                "no --watch-dir or RECS_DIR configured (file-watcher mode disabled)",
            ),
            Check::warn(
                "Audio source",
                "no audio source configured (no live detections will be produced)",
                "set one of: --alsa-device, --pipewire-device, --rtsp-url, --rtsp-urls, \
                 or the equivalent ALSA_CARD / RTSP_URL / PIPEWIRE_DEVICE config keys",
            ),
            Check::skip(
                "ONNX model file",
                "no --model / MODEL configured (will use the bundled default at startup)",
            ),
            Check::pass("Disk space", "120 GiB free on the volume containing /"),
        ]
    }

    fn sample_with_errors() -> Vec<Check> {
        vec![
            Check::pass("CPU cores", "4 cores available for audio + inference"),
            Check::pass("Temp directory", "/tmp is writable"),
            Check::pass(
                "Configuration file",
                "loaded from /etc/birdnet/birdnet.conf",
            ),
            Check::fail(
                "Config: LATITUDE",
                "latitude 200 is outside the valid range -90.0 to 90.0",
                "use decimal degrees, e.g. LATITUDE=42.3601 for Boston, MA",
            ),
            Check::fail(
                "Config: AUDIO_FORMAT",
                "AUDIO_FORMAT=\"aiff\" is not supported",
                "use \"wav\", \"mp3\", \"flac\", or \"ogg\" (non-WAV formats need ffmpeg or sox)",
            ),
            Check::fail(
                "Web listen address",
                "\"not-an-address\" is not a valid socket address: invalid socket address syntax",
                "use the form HOST:PORT, e.g. 127.0.0.1:8502 or 0.0.0.0:8502",
            ),
            Check::pass("Database directory", "/var/lib/birdnet is writable"),
            Check::skip(
                "Database integrity",
                "no database file yet — will be created on first run",
            ),
            Check::warn(
                "Audio source",
                "no audio source configured (no live detections will be produced)",
                "set one of: --alsa-device, --pipewire-device, --rtsp-url, --rtsp-urls, \
                 or the equivalent ALSA_CARD / RTSP_URL / PIPEWIRE_DEVICE config keys",
            ),
            Check::pass("Disk space", "120 GiB free on the volume containing /"),
        ]
    }

    fn check_snapshot(name: &str, actual: &str) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(SNAPSHOT_DIR)
            .join(format!("{name}.txt"));
        let update = std::env::var("UPDATE_DOCTOR_SNAPSHOTS").is_ok();
        if update || !path.exists() {
            std::fs::create_dir_all(path.parent().expect("snapshot dir"))
                .expect("create snapshot dir");
            std::fs::write(&path, actual).expect("write snapshot");
            return;
        }
        let expected = std::fs::read_to_string(&path).expect("read snapshot");
        assert_eq!(
            actual,
            expected,
            "Snapshot {name} drifted.\nRun with UPDATE_DOCTOR_SNAPSHOTS=1 to refresh.\n\
             Expected (from {}):\n{expected}\n\nActual:\n{actual}",
            path.display()
        );
    }

    #[test]
    fn snapshot_all_pass() {
        check_snapshot("all_pass", &render_text(&sample_all_pass()));
    }

    #[test]
    fn snapshot_mixed_warnings_and_skips() {
        check_snapshot("mixed", &render_text(&sample_mixed()));
    }

    #[test]
    fn snapshot_with_errors() {
        check_snapshot("with_errors", &render_text(&sample_with_errors()));
    }

    #[test]
    fn snapshot_empty_report() {
        check_snapshot("empty", &render_text(&[]));
    }
}

#[cfg(test)]
mod proptests {
    use super::{render_json, summarise};
    use crate::doctor::{Check, Status};
    use proptest::prelude::*;

    fn arb_status() -> impl Strategy<Value = Status> {
        prop_oneof![
            Just(Status::Pass),
            Just(Status::Warn),
            Just(Status::Fail),
            Just(Status::Skip),
        ]
    }

    fn arb_check() -> impl Strategy<Value = Check> {
        (
            arb_status(),
            ".{0,40}",
            ".{0,80}",
            proptest::option::of(".{0,60}"),
        )
            .prop_map(|(status, name, message, remediation)| Check {
                name,
                status,
                message,
                remediation,
            })
    }

    proptest! {
        /// JSON output is always parseable, and the parsed object has the
        /// documented schema (top-level object with `summary` and `checks`,
        /// every check entry has all four required fields).
        #[test]
        fn json_is_always_parseable(
            checks in proptest::collection::vec(arb_check(), 0..16),
            exit in -10_i32..=10,
        ) {
            let s = render_json(&checks, exit);
            let v: serde_json::Value = serde_json::from_str(&s)
                .expect("render_json must produce valid JSON");
            prop_assert!(v.is_object());
            let obj = v.as_object().unwrap();
            prop_assert!(obj.contains_key("summary"));
            prop_assert!(obj.contains_key("checks"));
            let arr = obj["checks"].as_array().expect("checks must be an array");
            prop_assert_eq!(arr.len(), checks.len());
            for entry in arr {
                let m = entry.as_object().unwrap();
                prop_assert!(m.contains_key("status"));
                prop_assert!(m.contains_key("name"));
                prop_assert!(m.contains_key("message"));
                prop_assert!(m.contains_key("remediation"));
            }
        }

        /// Summary counts always sum to the total number of checks — catches
        /// off-by-one bugs in `tally` regardless of input ordering.
        #[test]
        fn json_summary_sums_to_check_count(
            checks in proptest::collection::vec(arb_check(), 0..32),
        ) {
            let s = render_json(&checks, 0);
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            let sum = v["summary"]["passed"].as_u64().unwrap()
                + v["summary"]["warnings"].as_u64().unwrap()
                + v["summary"]["errors"].as_u64().unwrap()
                + v["summary"]["skipped"].as_u64().unwrap();
            prop_assert_eq!(sum, checks.len() as u64);
        }

        /// `summarise` and the JSON output's embedded summary always agree.
        #[test]
        fn summarise_matches_embedded_exit_code(
            checks in proptest::collection::vec(arb_check(), 0..32),
        ) {
            let code = summarise(&checks);
            let s = render_json(&checks, code);
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            prop_assert_eq!(v["summary"]["exit_code"].as_i64().unwrap(), i64::from(code));
        }

        /// `summarise` only returns one of the three documented exit codes.
        #[test]
        fn summarise_only_returns_documented_codes(
            checks in proptest::collection::vec(arb_check(), 0..32),
        ) {
            let code = summarise(&checks);
            prop_assert!(matches!(code, 0..=2), "unexpected exit code {code}");
        }
    }
}
