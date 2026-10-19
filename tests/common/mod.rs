//! Shared helpers for the model-gated end-to-end suites.
//!
//! `inference_e2e`, `pipeline_e2e` and `species_filter_e2e` need the real
//! ~541 MB BirdNET model, which is far too large to commit. They locate it
//! through `BIRDNET_TEST_MODEL` / `BIRDNET_TEST_LABELS` and skip when it is
//! absent, so an ordinary `cargo test` on a fresh clone still works.
//!
//! # Why a skip has to be loud
//!
//! Rust's test harness counts a test that returns early as **passed**. Measured
//! on this repo, the same suite reports the same summary line either way:
//!
//! ```text
//! without the model:  test result: ok. 2 passed ... finished in 0.00s
//! with the model:     test result: ok. 2 passed ... finished in 2.94s
//! ```
//!
//! Only the elapsed time distinguishes "verified the classifier against 11 000
//! species" from "did nothing". Nothing in `1942 passed` tells a reader which
//! happened, and these are the *only* tests that exercise the scientific core —
//! decode → resample → chunk → ONNX inference → confidence mapping.
//!
//! So when the model is known to be present, skipping is a failure, not a pass.
//! CI sets `BIRDNET_REQUIRE_MODEL=1` in the same step that successfully fetches
//! and sha256-verifies the model, which makes the two states distinguishable:
//!
//! * model fetched → `BIRDNET_REQUIRE_MODEL=1` → a skip **panics**;
//! * fetch failed (CDN outage) → variable unset → a skip is reported and
//!   tolerated, so an upstream outage cannot fail an unrelated build.

// Every test suite compiles this module, and each uses only the helper for the
// model it needs (V3.0 or V2.4), so the other is dead there.
#![allow(dead_code)]

use std::path::PathBuf;

/// Env var CI sets once the model has been fetched and checksum-verified.
const REQUIRE: &str = "BIRDNET_REQUIRE_MODEL";

/// Resolve the model and labels paths, or decide how to skip.
///
/// Returns `Some((model, labels))` when both env vars are set and both files
/// exist.
///
/// # Panics
///
/// Panics when the model is unavailable *and* `BIRDNET_REQUIRE_MODEL` is set to
/// anything other than `0`/empty — the signal that this environment promised a
/// model, so a silent skip would be a regression in the gate itself.
pub fn model_paths() -> Option<(PathBuf, PathBuf)> {
    paths_from_env(
        REQUIRE,
        "BIRDNET_TEST_MODEL",
        "BIRDNET_TEST_LABELS",
        "the scientific core (decode → resample → inference → confidence)",
    )
}

/// The same, for a BirdNET **V2.4** ONNX and its labels
/// (`BIRDNET_TEST_V24_MODEL` / `BIRDNET_TEST_V24_LABELS`, required by
/// `BIRDNET_REQUIRE_V24_MODEL`). `v24_matches_birdnet_pi` uses it: the V3.0
/// the other suites run is a 32 kHz probability model, so the 48 kHz logit path
/// and its sensitivity mapping — what a BirdNET-Pi user's detections depend on —
/// have no coverage from it.
pub fn v24_model_paths() -> Option<(PathBuf, PathBuf)> {
    paths_from_env(
        "BIRDNET_REQUIRE_V24_MODEL",
        "BIRDNET_TEST_V24_MODEL",
        "BIRDNET_TEST_V24_LABELS",
        "the V2.4 logit path (48 kHz raw waveform → sensitivity-scaled sigmoid)",
    )
}

/// [`v24_model_paths`] for a suite that needs the V2.4 range-filter ONNX too
/// (`BIRDNET_TEST_V24_MDATA`). The classifier model is not used by it, so only
/// the labels path is returned meaningfully.
pub fn v24_model_paths_with_mdata() -> Option<(PathBuf, PathBuf)> {
    let found = v24_model_paths()?;
    let required =
        std::env::var("BIRDNET_REQUIRE_V24_MODEL").is_ok_and(|v| !v.is_empty() && v != "0");
    let mdata_ok =
        std::env::var("BIRDNET_TEST_V24_MDATA").is_ok_and(|p| std::path::Path::new(&p).is_file());
    assert!(
        mdata_ok || !required,
        "BIRDNET_REQUIRE_V24_MODEL is set, so the V2.4 range-filter test must run, but \
         BIRDNET_TEST_V24_MDATA does not name a file"
    );
    if !mdata_ok {
        eprintln!("SKIP: BIRDNET_TEST_V24_MDATA does not name a file");
        return None;
    }
    Some(found)
}

fn paths_from_env(
    require_var: &str,
    model_var: &str,
    labels_var: &str,
    what: &str,
) -> Option<(PathBuf, PathBuf)> {
    let required = std::env::var(require_var).is_ok_and(|v| !v.is_empty() && v != "0");

    let missing = |reason: String| -> Option<(PathBuf, PathBuf)> {
        assert!(
            !required,
            "{require_var} is set, so the model-gated tests must run — but {reason}.\n\
             This means {what} \
             was NOT exercised, while the suite would still have reported `ok`. Fix the \
             model paths, or unset {require_var} if this environment genuinely has no model."
        );
        eprintln!("SKIP: {reason} (set {require_var}=1 to make this a failure)");
        None
    };

    let (Ok(model), Ok(labels)) = (std::env::var(model_var), std::env::var(labels_var)) else {
        return missing(format!("{model_var} / {labels_var} are not both set"));
    };

    let (model, labels) = (PathBuf::from(model), PathBuf::from(labels));
    if !model.is_file() {
        return missing(format!("no model file at {}", model.display()));
    }
    if !labels.is_file() {
        return missing(format!("no labels file at {}", labels.display()));
    }
    Some((model, labels))
}
