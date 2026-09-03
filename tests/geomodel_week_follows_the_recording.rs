//! The species-occurrence filter must be asked about the season the audio was
//! recorded in.
//!
//! # What was wrong
//!
//! `BirdNET`'s metadata model — the "geomodel" — takes
//! `(latitude, longitude, week)` and was trained on a 48-week year, so its
//! input domain is `1..=48`. The daemon passed a literal `0` at both of its
//! call sites:
//!
//! ```text
//! crates/birdnet-core/src/detection/daemon/run.rs:351
//!     0, // week will be computed by caller
//! crates/birdnet-core/src/detection/daemon/run.rs:429
//!     0,
//! ```
//!
//! and there was no caller that computed it — `run.rs` *is* the caller.
//! `sf_thresh` defaults to `0.03`, so the filter is on by default: every
//! station with coordinates was filtering its species list against a point
//! outside the model's domain, identically in June and December, for the whole
//! life of the project. Every `detections.Week` ever written is `0`.
//!
//! It was invisible because the one end-to-end test over that function,
//! `tests/species_filter_e2e.rs`, passed a real week of its own (`20`) and so
//! exercised the parameter rather than the daemon's use of it — and because
//! week 0 does not error, it just returns a different, plausible-looking
//! occurrence vector.
//!
//! # What this gate holds
//!
//! Two halves, neither needing the 541 MB model:
//!
//! 1. **The input the derivation reads is real and date-dependent.** The
//!    recording date survives the audio pipeline intact, and two recordings six
//!    months apart resolve to different weeks — which is precisely the question
//!    nobody asked of the daemon.
//! 2. **The arithmetic is `BirdNET`'s**, checked against the reference
//!    implementation's stated contract rather than against itself.
//!
//! The third half is not a test: `process_and_infer_filtered` no longer takes
//! a `week` parameter at all, so there is no longer an argument position a
//! constant can be passed in. That is enforced by the compiler, and the fact
//! that this file and `tests/species_filter_e2e.rs` compile is the evidence.
//!
//! Observed failing before the fix: with `civil::birdnet_week` stubbed to
//! return `0` — which is exactly what `run.rs` passed — every assertion below
//! about a specific week fails, and
//! `january_and_july_are_not_the_same_week_to_the_geomodel` fails on the
//! inequality.

use std::path::Path;

use birdnet_core::civil::{birdnet_week, birdnet_week_from_date};
use birdnet_core::detection::daemon::process_file_pipeline_only;
use birdnet_core::detection::pipeline::PipelineConfig;

/// The bundled 30-second Eurasian Magpie recording, also used by
/// `tests/pipeline_e2e.rs` and `tests/species_filter_e2e.rs`.
const PICA_PICA_WAV: &str = "tests/testdata/Pica_pica_30s.wav";

/// Stage the bundled recording under a date the daemon will parse, and return
/// the recording date the audio pipeline reports for it.
///
/// This goes through `process_file_pipeline_only` — the same
/// `pipeline::process_file` the inference path calls — rather than through
/// `RecordingFile::parse` directly, so that a change to how the pipeline
/// derives a chunk's recording is caught here too.
fn recording_date_through_the_pipeline(dir: &Path, stem: &str) -> String {
    let staged = dir.join(format!("{stem}.wav"));
    std::fs::copy(PICA_PICA_WAV, &staged).expect("stage the bundled recording");

    let chunks = process_file_pipeline_only(&staged, &PipelineConfig::default())
        .expect("the bundled recording must go through the pipeline");
    assert!(
        !chunks.is_empty(),
        "a 30 s recording must produce at least one chunk, or this gate proves nothing"
    );

    let date = chunks[0].recording.date.clone();
    assert!(
        chunks.iter().all(|c| c.recording.date == date),
        "every chunk of one file must carry one recording date; the daemon derives \
         a single week per file on that basis"
    );
    date
}

#[test]
fn january_and_july_are_not_the_same_week_to_the_geomodel() {
    let dir = tempfile::tempdir().expect("tempdir");

    let january = recording_date_through_the_pipeline(dir.path(), "2026-01-05-birdnet-06:30:00");
    let july = recording_date_through_the_pipeline(dir.path(), "2026-07-05-birdnet-06:30:00");

    assert_eq!(
        january, "2026-01-05",
        "the pipeline must carry the filename's date"
    );
    assert_eq!(july, "2026-07-05");

    let january_week = birdnet_week_from_date(&january).expect("a staged date must parse");
    let july_week = birdnet_week_from_date(&july).expect("a staged date must parse");

    // The discrimination, not just the alarm: a derivation that returns any
    // constant — 0, as the daemon did, or 20, as the e2e test passed — satisfies
    // "is in range" and fails this.
    assert_ne!(
        january_week, july_week,
        "two recordings six months apart must not be scored against the same season"
    );

    assert_eq!(
        january_week, 1,
        "5 January is the first week of the 48-week year"
    );
    assert_eq!(july_week, 25, "5 July is week 25: (7-1)*4 + 1");
}

/// The week the daemon derives is a property of the recording, not of the
/// clock at the moment of analysis.
///
/// A station that loses power for three days and then drains its backlog must
/// score that backlog against the season it was recorded in. This is why the
/// derivation reads `chunk.recording.date` and never `SystemTime::now()`.
#[test]
fn the_week_comes_from_the_recording_not_from_analysis_time() {
    let dir = tempfile::tempdir().expect("tempdir");

    // A recording from the far side of the year from any plausible "now" in
    // the window this test could run in. Whatever today is, one of these two
    // is not today's week.
    let a = recording_date_through_the_pipeline(dir.path(), "2026-02-10-birdnet-05:00:00");
    let b = recording_date_through_the_pipeline(dir.path(), "2026-08-10-birdnet-05:00:00");

    assert_eq!(birdnet_week_from_date(&a), Some(6));
    assert_eq!(birdnet_week_from_date(&b), Some(30));
}

/// Every date the calendar can produce lands inside the domain the model was
/// trained on.
///
/// `tphakala/birdnet-go` records an un-clamped copy of this formula returning
/// week 49 for 29–31 December, fed live into its range filter
/// (`internal/classifier/range_filter.go`, `getWeekForFilter`). The clamp is
/// the fix for that, and this asserts it end to end rather than only in the
/// unit tests beside the function.
#[test]
fn no_calendar_date_can_leave_the_models_domain() {
    for month in 1..=12u32 {
        for day in 1..=31u32 {
            let w = birdnet_week(month, day);
            assert!(
                (1..=48).contains(&w),
                "{month:02}-{day:02} gave week {w}, outside BirdNET's 1..=48 domain"
            );
        }
    }
}
