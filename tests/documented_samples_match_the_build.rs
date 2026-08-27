//! The manual quotes a live `/api/v2/health` response, and that sample carries
//! a version string. Nothing regenerates it, so it drifts silently — which is
//! exactly what happened: it sat at `0.13.1` through both the `0.14.0` and the
//! `0.15.0` version bumps, even though `RELEASING.md`'s pre-release checklist
//! names the line explicitly ("docs/book/reference/api.md: the sample
//! /api/v2/health response"). A checklist item that is only ever checked by a
//! human is one that gets missed twice without anyone noticing.
//!
//! `crates/birdnet-web/src/routes/openapi.rs` already gates `openapi.json`'s
//! `info.version` against `CARGO_PKG_VERSION` the same way. This is that gate
//! for the prose copy.
//!
//! Observed failing against the stale file before `api.md` was corrected:
//!
//! ```text
//! ---- the_manuals_health_sample_reports_this_builds_version stdout ----
//! assertion `left == right` failed: docs/book/reference/api.md's
//! /api/v2/health sample reports version "0.13.1", but this build is
//! "0.15.0" — RELEASING.md's checklist covers this line; update the sample
//!   left: "0.13.1"
//!  right: "0.15.0"
//! ```

/// The manual, compiled in — so the test moves with the file rather than
/// depending on the working directory a runner happens to use.
const API_DOC: &str = include_str!("../docs/book/reference/api.md");

/// The `"version": "..."` literal from the documented `/api/v2/health` body.
///
/// Returns `None` rather than panicking so the caller can tell "the sample
/// changed shape" apart from "the sample is stale" — a matcher that quietly
/// finds nothing would let this test pass for free, which is the failure mode
/// the whole file exists to prevent.
fn documented_health_version() -> Option<&'static str> {
    let after_path = API_DOC.split("/api/v2/health").nth(1)?;
    let field = after_path.find("\"version\":")?;
    let rest = &after_path[field + "\"version\":".len()..];
    let open = rest.find('"')? + 1;
    let close = rest[open..].find('"')? + open;
    Some(&rest[open..close])
}

#[test]
fn the_manuals_health_sample_reports_this_builds_version() {
    let documented = documented_health_version().expect(
        "docs/book/reference/api.md must still show a \"version\" field in its \
         /api/v2/health sample — if the sample moved or was reshaped, retarget \
         this gate rather than deleting it",
    );

    assert_eq!(
        documented,
        env!("CARGO_PKG_VERSION"),
        "docs/book/reference/api.md's /api/v2/health sample reports version \
         {documented:?}, but this build is {:?} — RELEASING.md's checklist \
         covers this line; update the sample",
        env!("CARGO_PKG_VERSION")
    );
}

/// The counterpart. The gate above can only be trusted if its matcher actually
/// locates the sample; an extractor that returns `None` on everything would
/// turn the real assertion into an `expect` that never runs and a test that is
/// green because it tested nothing.
///
/// Kept green rather than red: it asserts the matcher finds *a* plausible
/// version, without asserting *which* — so it stays passing across every future
/// bump while still failing the day the sample stops being found.
#[test]
fn the_version_matcher_actually_finds_the_sample() {
    let found = documented_health_version();
    assert!(
        found.is_some(),
        "the matcher found no version field in api.md's /api/v2/health sample, \
         so the gate beside it would pass vacuously"
    );
    let found = found.unwrap_or_default();
    assert!(
        found.split('.').count() == 3 && found.split('.').all(|p| !p.is_empty()),
        "expected a dotted version triple from the sample, got {found:?} — the \
         matcher is picking up the wrong string"
    );
}
