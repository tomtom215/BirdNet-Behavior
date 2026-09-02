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

// ── The tuning guide's confirmation-level table ──────────────────────────────
//
// `docs/book/guides/tuning.md` prints, for each confirmation level, the
// fraction it demands, the span it looks across, and the smallest overlap at
// which it demands anything at all. Every one of those numbers is derived —
// `ConfirmationLevel::minimum_overlap` searches for it by asking the filter the
// same question the filter will ask at run time — so the table is a transcript
// of an answer that can change without anyone editing the prose.
//
// That is the failure this repository has had twice: a doc comment on
// `load_icu()` asserting a linkage that did not exist, and `aligned_sum`
// documented as summing when it averaged. Both misled the next reader. Here the
// next reader is an operator deciding what to put in their config, and a stale
// row would tell them to set an overlap that does nothing — the exact defect
// the whole feature warns about.

/// The tuning guide, compiled in.
const TUNING_DOC: &str = include_str!("../docs/book/guides/tuning.md");

/// One parsed row of the confirmation-level table.
struct LevelRow<'a> {
    level: &'a str,
    percent: u32,
    span_secs: u32,
    /// `None` for "any" — the level bites at every overlap.
    minimum_overlap: Option<f32>,
    /// `(needs, of)` from an "already N of M" note, when the row carries one.
    already: Option<(usize, usize)>,
}

/// Pull the rows out of the table under the confirmation-level heading.
///
/// Returns an empty vector if the table moved or was reshaped, which the caller
/// treats as a failure rather than a pass — a matcher that quietly finds
/// nothing is how a gate like this becomes decorative.
fn documented_levels() -> Vec<LevelRow<'static>> {
    let Some(section) = TUNING_DOC.split("BIRDNET_CONFIRMATION_LEVEL=").nth(1) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 4 {
            continue;
        }
        let level = cells[0].trim_matches('`');
        if !matches!(level, "lenient" | "moderate" | "balanced" | "strict") {
            continue;
        }
        let Ok(percent) = cells[1].trim_end_matches('%').parse::<u32>() else {
            continue;
        };
        let Ok(span_secs) = cells[2].trim_end_matches(" s").parse::<u32>() else {
            continue;
        };
        let last = cells[3];
        let minimum_overlap = if last.starts_with("any") {
            None
        } else {
            match last.trim_matches('`').parse::<f32>() {
                Ok(v) => Some(v),
                Err(_) => continue,
            }
        };
        let already = last.split("already ").nth(1).and_then(|rest| {
            let mut parts = rest.trim_end_matches(')').split(" of ");
            Some((
                parts.next()?.trim().parse().ok()?,
                parts.next()?.trim().parse().ok()?,
            ))
        });
        out.push(LevelRow {
            level,
            percent,
            span_secs,
            minimum_overlap,
            already,
        });
    }
    out
}

#[test]
fn the_tuning_guides_confirmation_table_matches_the_filter() {
    use birdnet_core::detection::corroboration::{
        ConfirmationLevel, REFERENCE_SPAN, required_confirmations,
    };

    let rows = documented_levels();
    assert_eq!(
        rows.len(),
        4,
        "docs/book/guides/tuning.md must still carry a four-row table of \
         confirmation levels under the BIRDNET_CONFIRMATION_LEVEL example — if \
         the table moved or was reshaped, retarget this gate rather than \
         deleting it. Parsed {} row(s).",
        rows.len()
    );

    // The window length the table's numbers are written for. Not operator
    // tunable at the config layer, so a literal here is honest; it is the same
    // value `check_confirmation_filter` reports against.
    let chunk_secs =
        birdnet_core::detection::pipeline::PipelineConfig::default().chunk_duration_secs;

    for row in rows {
        let level = ConfirmationLevel::parse(row.level).expect("filtered to the four names above");

        #[allow(clippy::cast_precision_loss)]
        let documented_fraction = row.percent as f32 / 100.0;
        assert!(
            (documented_fraction - level.required_fraction()).abs() < 1e-6,
            "the guide says `{}` demands {}%, the filter demands {}%",
            row.level,
            row.percent,
            level.required_fraction() * 100.0
        );

        #[allow(clippy::cast_precision_loss)]
        let documented_span = row.span_secs as f32;
        assert!(
            (documented_span - REFERENCE_SPAN).abs() < 1e-6,
            "the guide says `{}` looks across {} s, the filter uses {REFERENCE_SPAN} s",
            row.level,
            row.span_secs
        );

        let actual = level
            .minimum_overlap(chunk_secs)
            .expect("an enabled level has one");
        match row.minimum_overlap {
            Some(documented) => assert!(
                (documented - actual).abs() < 1e-6,
                "the guide tells operators `{}` needs at least {documented}s of \
                 overlap; the filter starts demanding a second opinion at {actual}s",
                row.level
            ),
            None => assert!(
                actual == 0.0,
                "the guide says `{}` bites at any overlap; the filter needs {actual}s",
                row.level
            ),
        }

        if let Some((needs, of)) = row.already {
            assert_eq!(
                level.required_confirmations_at(0.0, chunk_secs),
                needs,
                "the guide says `{}` is already {needs} of {of} at zero overlap",
                row.level
            );
            // And the other half of "N of M": M is the number of windows the
            // filter actually sees at zero overlap, so feeding M back through
            // the same arithmetic must reproduce N. A row claiming "2 of 5"
            // would pass the check above and fail this one.
            assert_eq!(
                required_confirmations(level, of),
                needs,
                "the guide says `{}` is {needs} of {of} at zero overlap, but {of} \
                 windows at that level is {}",
                row.level,
                required_confirmations(level, of)
            );
        }
    }
}
