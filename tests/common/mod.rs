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
    let required = std::env::var(REQUIRE).is_ok_and(|v| !v.is_empty() && v != "0");

    let missing = |reason: String| -> Option<(PathBuf, PathBuf)> {
        assert!(
            !required,
            "{REQUIRE} is set, so the model-gated tests must run — but {reason}.\n\
             This means the scientific core (decode → resample → inference → confidence) \
             was NOT exercised, while the suite would still have reported `ok`. Fix the \
             model paths, or unset {REQUIRE} if this environment genuinely has no model."
        );
        eprintln!("SKIP: {reason} (set {REQUIRE}=1 to make this a failure)");
        None
    };

    let (Ok(model), Ok(labels)) = (
        std::env::var("BIRDNET_TEST_MODEL"),
        std::env::var("BIRDNET_TEST_LABELS"),
    ) else {
        return missing("BIRDNET_TEST_MODEL / BIRDNET_TEST_LABELS are not both set".to_string());
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
