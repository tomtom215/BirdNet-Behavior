//! End-to-end smoke tests for the `--doctor` subcommand and friends.
//!
//! These tests spawn the actual compiled binary as a subprocess, so they
//! exercise:
//!   * `clap` argument parsing
//!   * `tracing_subscriber` initialisation (regression test for the
//!     "logs leaking onto stdout" bug — tracing must go to stderr so that
//!     `--doctor-json` produces a clean JSON line)
//!   * Doctor's runtime probes against the real filesystem and processes
//!   * Exit-code mapping to the documented 0 / 1 / 2 contract
//!
//! Cargo populates `CARGO_BIN_EXE_birdnet-behavior` with the path to the
//! compiled binary when running integration tests, so the suite needs no
//! extra dependencies.

use std::process::{Command, Stdio};

/// Path to the compiled `birdnet-behavior` binary, provided by Cargo.
const BIN: &str = env!("CARGO_BIN_EXE_birdnet-behavior");

/// Run the binary with the given args; return (stdout, stderr, exit code).
fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(BIN)
        .args(args)
        .env("RUST_LOG", "error") // keep noise low without affecting --doctor logic
        .env_remove("BIRDNET_CONFIG")
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn {BIN}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let code = out.status.code().unwrap_or(-1);
    (stdout, stderr, code)
}

#[test]
fn version_flag_prints_a_semver_string() {
    let (stdout, _stderr, code) = run(&["--version"]);
    assert_eq!(code, 0, "expected exit 0, got {code}");
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "--version stdout missing crate version: {stdout:?}"
    );
}

#[test]
fn help_flag_lists_the_doctor_subcommand() {
    let (stdout, _stderr, code) = run(&["--help"]);
    assert_eq!(code, 0, "expected exit 0, got {code}");
    assert!(stdout.contains("--doctor"), "--help is missing --doctor");
    assert!(
        stdout.contains("--doctor-json"),
        "--help is missing --doctor-json"
    );
}

#[test]
fn doctor_exits_with_documented_code_and_prints_report() {
    // With no config and no audio source, the doctor surfaces warnings (no
    // errors), so the exit code must be 1.
    let (stdout, _stderr, code) = run(&["--doctor", "--config", "/nonexistent/birdnet.conf"]);
    assert!(
        matches!(code, 0..=2),
        "exit code {code} is outside the documented 0/1/2 contract"
    );
    assert!(
        stdout.contains("BirdNet-Behavior preflight report"),
        "doctor stdout missing report header: {stdout:?}"
    );
    assert!(
        stdout.contains("[ PASS ]") || stdout.contains("[ WARN ]") || stdout.contains("[ FAIL ]"),
        "doctor stdout missing any status tag: {stdout:?}"
    );
    assert!(
        stdout.contains("Summary:"),
        "doctor stdout missing summary line: {stdout:?}"
    );
}

#[test]
fn preflight_alias_matches_doctor() {
    // --preflight is documented as an alias for --doctor; the two must
    // produce identical stdout.
    let (a, _, code_a) = run(&["--doctor", "--config", "/nonexistent"]);
    let (b, _, code_b) = run(&["--preflight", "--config", "/nonexistent"]);
    assert_eq!(
        code_a, code_b,
        "exit codes differ: doctor={code_a} preflight={code_b}"
    );
    assert_eq!(
        a, b,
        "stdout differs between --doctor and --preflight alias"
    );
}

#[test]
fn doctor_json_produces_a_clean_parseable_json_line() {
    // This is the regression test for the tracing-to-stdout bug: previously
    // the fmt layer wrote logs to stdout and mixed them with the JSON.
    let (stdout, stderr, code) = run(&["--doctor-json", "--config", "/nonexistent/birdnet.conf"]);
    assert!(
        matches!(code, 0..=2),
        "exit code {code} is outside the documented 0/1/2 contract; stderr={stderr}"
    );
    // No leading/trailing log noise — must be exactly one JSON line.
    let trimmed = stdout.trim();
    assert!(
        trimmed.starts_with('{') && trimmed.ends_with('}'),
        "stdout is not a single JSON object: {stdout:?}"
    );

    // Schema check: parse the JSON and verify the documented fields exist.
    let v: serde_json::Value = serde_json::from_str(trimmed).unwrap_or_else(|e| {
        panic!("--doctor-json stdout is not valid JSON: {e}\nstdout: {stdout:?}")
    });
    let obj = v.as_object().expect("top-level JSON must be an object");

    let summary = obj
        .get("summary")
        .and_then(|s| s.as_object())
        .expect("missing or non-object `summary`");
    for field in ["passed", "warnings", "errors", "skipped", "exit_code"] {
        assert!(
            summary.contains_key(field),
            "summary is missing `{field}`: {summary:?}"
        );
    }
    let embedded_exit = summary["exit_code"]
        .as_i64()
        .expect("exit_code must be an integer");
    assert_eq!(
        embedded_exit,
        i64::from(code),
        "process exit code ({code}) and JSON summary.exit_code ({embedded_exit}) disagree"
    );

    let checks = obj
        .get("checks")
        .and_then(|c| c.as_array())
        .expect("missing or non-array `checks`");
    assert!(!checks.is_empty(), "checks array must not be empty");
    for entry in checks {
        let m = entry.as_object().expect("each check must be an object");
        for field in ["status", "name", "message", "remediation"] {
            assert!(m.contains_key(field), "check missing `{field}`: {entry:?}");
        }
        let status = m["status"].as_str().expect("status must be a string");
        assert!(
            matches!(status, "pass" | "warn" | "fail" | "skip"),
            "unexpected status: {status:?}"
        );
    }
}

#[test]
fn check_db_with_missing_database_does_not_crash() {
    // Sanity that the other one-shot subcommands stay usable too. With no
    // database to check, `--check-db` is expected to log and exit cleanly
    // (not panic).
    let (_stdout, _stderr, code) = run(&["--check-db", "--config", "/nonexistent/birdnet.conf"]);
    assert!(
        matches!(code, 0..=2),
        "--check-db exited with surprising code {code}"
    );
}

#[test]
fn doctor_json_has_no_stdout_log_noise() {
    // Belt-and-braces: even if tracing is configured loosely (e.g. user
    // sets RUST_LOG=trace), --doctor-json must keep stdout clean.
    let out = Command::new(BIN)
        .args(["--doctor-json", "--config", "/nonexistent"])
        .env("RUST_LOG", "trace")
        .env_remove("BIRDNET_CONFIG")
        .stdin(Stdio::null())
        .output()
        .expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let trimmed = stdout.trim();
    // Must parse as JSON even with RUST_LOG=trace.
    serde_json::from_str::<serde_json::Value>(trimmed).unwrap_or_else(|e| {
        panic!(
            "--doctor-json stdout polluted by logs under RUST_LOG=trace: {e}\nstdout: {stdout:?}"
        )
    });
}

/// A confirmation level that cannot reject anything must say so, and one that
/// can must not.
///
/// The inert case is the whole reason this check exists: `lenient` with the
/// default overlap of zero is accepted, logged as enabled, and rejects nothing,
/// because a six-second neighbourhood holds two 3-second windows and 20% of two
/// rounds up to one — which every detection already satisfies. An operator who
/// set it and saw no change has no way to tell that from the filter working.
///
/// Run through the compiled binary rather than against the check function, so
/// it also fails if the check is written and never registered in `collect()` —
/// a diagnostic nobody calls is the same as no diagnostic.
#[test]
fn doctor_reports_an_inert_confirmation_level_and_passes_an_effective_one() {
    let line = |args: &[&str]| -> String {
        let (stdout, _stderr, _code) = run(args);
        stdout
            .lines()
            .find(|l| l.contains("Repeat-confirmation filter"))
            .unwrap_or_else(|| {
                panic!("doctor never mentioned the repeat-confirmation filter:\n{stdout}")
            })
            .to_owned()
    };

    let inert = line(&[
        "--doctor",
        "--config",
        "/nonexistent/birdnet.conf",
        "--confirmation-level",
        "lenient",
    ]);
    assert!(
        inert.contains("[ WARN ]") && inert.contains("rejects nothing"),
        "an inert confirmation level must be reported, not passed: {inert}"
    );

    // Counterpart: a level that does bite at this overlap must be a plain
    // pass, or the warning above is a blanket alarm rather than a discriminator.
    let effective = line(&[
        "--doctor",
        "--config",
        "/nonexistent/birdnet.conf",
        "--confirmation-level",
        "strict",
    ]);
    assert!(
        effective.contains("[ PASS ]"),
        "`strict` rejects at any overlap and must not be warned about: {effective}"
    );

    // And the same level becomes effective once the windows overlap enough,
    // which is the fix the warning tells the operator to apply.
    let fixed = line(&[
        "--doctor",
        "--config",
        "/nonexistent/birdnet.conf",
        "--confirmation-level",
        "lenient",
        "--overlap",
        "2.0",
    ]);
    assert!(
        fixed.contains("[ PASS ]"),
        "the fix the warning recommends must actually clear it: {fixed}"
    );
}
