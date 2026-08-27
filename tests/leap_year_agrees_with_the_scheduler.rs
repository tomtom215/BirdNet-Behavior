//! The two surviving leap-year implementations must agree.
//!
//! `birdnet_core::civil::is_leap_year` is the workspace's copy, and the calendar
//! helpers in `birdnet-web` now go through it. One more survives, in
//! `birdnet-scheduler`, and it survives on purpose: that crate depends on
//! `serde` and nothing else, deliberately, so the solar arithmetic stays a
//! pure-computation crate. Taking `birdnet-core` for two `const fn`s would pull
//! ONNX Runtime, `symphonia` and `rubato` into it.
//!
//! (A third copy, in `src/capture/schedule.rs`, is the *oracle* its own
//! conversion is checked against; its comment says so, and an oracle that calls
//! the implementation under test proves nothing. It stays too.)
//!
//! What "on purpose" has to buy is a check that the two cannot drift. The
//! scheduler's predicate is private, but it is load-bearing: `SolarDay::for_date`
//! rejects a day past the end of its month, so whether `2024-02-29` is accepted
//! *is* the scheduler's answer to "is 2024 a leap year". This drives that and
//! compares it against `civil`, for every February from 1800 to 2400 — which
//! exercises all three Gregorian rules (the /4, the /100 exception, and the /400
//! exception to the exception) many times over.
//!
//! This test lives in the binary crate because it is the only place that
//! depends on both.

use birdnet_core::civil::{days_in_month, is_leap_year};
use birdnet_scheduler::solar::{Location, SolarDay};

/// Somewhere unremarkable, so nothing is ever rejected for a reason other than
/// the calendar.
fn london() -> Location {
    Location::new(51.5, -0.12).expect("valid location")
}

fn scheduler_accepts(year: u32, month: u32, day: u32) -> bool {
    SolarDay::for_date(london(), year, month, day).is_ok()
}

#[test]
fn february_29_is_accepted_exactly_when_civil_says_it_is_a_leap_year() {
    let mut leaps = 0_u32;
    for year in 1800..=2400 {
        let civil_says = is_leap_year(year);
        let scheduler_says = scheduler_accepts(year, 2, 29);
        assert_eq!(
            civil_says,
            scheduler_says,
            "{year}-02-29: civil::is_leap_year says {civil_says}, the scheduler {} it",
            if scheduler_says { "accepts" } else { "rejects" }
        );
        leaps += u32::from(civil_says);
    }

    // The counterpart. If both implementations said "no" to everything, the
    // loop above would pass without noticing. 1800..=2400 is 601 years and
    // contains 146 leap years: one in four, less 1800, 1900, 2100, 2200 and
    // 2300, plus 2000 and 2400 which the /400 rule restores.
    assert_eq!(leaps, 146, "the leap-year count itself is wrong");
}

#[test]
fn the_last_day_of_every_month_is_accepted_and_the_next_is_not() {
    for year in [1899_u32, 1900, 1996, 2000, 2024, 2025, 2100, 2400] {
        for month in 1..=12 {
            let last = days_in_month(year, month);
            assert!(
                scheduler_accepts(year, month, last),
                "{year}-{month:02}-{last} should be a real date"
            );
            assert!(
                !scheduler_accepts(year, month, last + 1),
                "{year}-{month:02}-{} should not be",
                last + 1
            );
        }
    }
}
