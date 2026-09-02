//! The taxon-aware daylight filter: is this bird plausible at this hour?
//!
//! # The problem
//!
//! A blue tit "detected" at 02:30 is almost always the classifier hearing
//! something else and reaching for the nearest bird. Those detections spread
//! across the whole night and distort exactly the analytics people build
//! stations to look at — the dawn-chorus curve, per-species activity
//! histograms, sessionisation.
//!
//! A blanket "no birds at night" rule is worse than the problem: owls,
//! nightjars, rails and bitterns call at night on purpose and are the
//! detections an operator most wants. So the question is *who*, not *when* —
//! which is what [`birdnet_core::detection::nocturnal`] answers.
//!
//! # This module answers *when*
//!
//! "Night" here is not "after sunset". Birds sing through dusk and start again
//! well before sunrise, so a window that began at sunset would quarantine the
//! evening chorus. The window is `[sunset + margin, sunrise - margin]`, and the
//! margin defaults to an hour.
//!
//! # It fails open
//!
//! Every step can legitimately fail — no coordinates configured, a timestamp
//! that names no civil time, a polar summer where the sun does not set. Each
//! one returns "not night", so a station that cannot compute the window keeps
//! every detection rather than quarantining the lot.

use birdnet_core::detection::nocturnal::{NightVerdict, night_verdict};
use birdnet_scheduler::solar::{Location, SolarDay};

/// Minutes in a day, for wrapping local-time arithmetic.
const MINUTES_PER_DAY: i64 = 1440;

/// What the filter decided about one detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DaylightVerdict {
    /// Not in the night window, or the species belongs there. Keep it.
    Keep,
    /// A species not known to be nocturnal, in the middle of the night.
    Quarantine,
}

/// The station's night window and the operator's exemptions.
#[derive(Debug, Clone)]
pub(super) struct DaylightFilter {
    /// `None` disables the filter — including when no coordinates are set,
    /// since without them there is no sunrise to compute.
    location: Option<Location>,
    /// Minutes after sunset / before sunrise before the window opens.
    margin_mins: i64,
    /// The station's offset from UTC, in seconds.
    utc_offset_secs: i64,
    /// Genera or scientific names the operator always allows at night.
    extra_nocturnal: Vec<String>,
}

impl DaylightFilter {
    /// A filter for a station at `location`.
    ///
    /// `location` of `None`, or a non-positive `margin_mins`, disables it.
    pub(super) const fn new(
        location: Option<Location>,
        margin_mins: i64,
        utc_offset_secs: i64,
        extra_nocturnal: Vec<String>,
    ) -> Self {
        Self {
            location,
            margin_mins,
            utc_offset_secs,
            extra_nocturnal,
        }
    }

    /// Whether the filter will quarantine anything.
    pub(super) const fn is_enabled(&self) -> bool {
        self.location.is_some() && self.margin_mins >= 0
    }

    /// Decide on one detection.
    pub(super) fn verdict(&self, sci_name: &str, date: &str, time: &str) -> DaylightVerdict {
        if !self.is_enabled() {
            return DaylightVerdict::Keep;
        }
        // Ask the cheap question first: a nocturnal species needs no solar
        // computation at all, and on a station listening for owls that is most
        // of what arrives after dark.
        if night_verdict(sci_name, &self.extra_nocturnal) == NightVerdict::Nocturnal {
            return DaylightVerdict::Keep;
        }
        match self.is_deep_night(date, time) {
            Some(true) => DaylightVerdict::Quarantine,
            // `None` is "cannot tell" and `Some(false)` is "not night". Both
            // keep the detection: a station that cannot compute its own
            // sunrise must not quarantine everything it hears.
            _ => DaylightVerdict::Keep,
        }
    }

    /// Whether `date`/`time` (station-local) falls in the night window.
    ///
    /// `None` when the window cannot be computed: an unparseable timestamp, a
    /// date the solar routine rejects, or a polar day/night where the sun does
    /// not cross the horizon at all.
    fn is_deep_night(&self, date: &str, time: &str) -> Option<bool> {
        let location = self.location?;
        let civil = birdnet_core::civil::parse_civil(date, time)?;
        let day = SolarDay::for_date(location, civil.year, civil.month, civil.day).ok()?;

        // `SolarDay` reports UTC minutes; the detection's timestamp is
        // station-local. Convert the solar events rather than the detection so
        // the comparison stays in the operator's own clock, which is the one
        // the margin was chosen against.
        let offset_min = self.utc_offset_secs / 60;
        let sunrise = i64::from(day.sunrise_utc_min?) + offset_min;
        let sunset = i64::from(day.sunset_utc_min?) + offset_min;

        let minute_of_day = i64::from(civil.hour) * 60 + i64::from(civil.minute);
        Some(is_within_night(
            minute_of_day,
            sunset + self.margin_mins,
            sunrise - self.margin_mins,
        ))
    }
}

/// Whether `minute_of_day` lies in the window that opens at `night_start` and
/// closes at `night_end`, both in local minutes and both allowed to fall
/// outside `[0, 1440)`.
///
/// The window wraps midnight, which is what makes this worth extracting: the
/// obvious `start <= m && m < end` is false for every minute of every night,
/// because `night_start` (evening) is always the larger number.
///
/// A window that has collapsed — margins so wide that the "night" would end
/// before it began, which happens near the solstice at high latitude — is
/// reported as no night at all rather than as an inverted window covering the
/// whole day.
#[must_use]
pub(super) const fn is_within_night(minute_of_day: i64, night_start: i64, night_end: i64) -> bool {
    let m = minute_of_day.rem_euclid(MINUTES_PER_DAY);
    let start = night_start.rem_euclid(MINUTES_PER_DAY);
    let end = night_end.rem_euclid(MINUTES_PER_DAY);

    // Length of the window walking forward from start to end, mod one day.
    //
    // A collapsed window — margins so wide that "night" would end before it
    // began, which happens near the solstice at high latitude — gives a length
    // of zero, and nothing is `< 0`. That falls out of the arithmetic and
    // needs no special case: an earlier version had an explicit `length == 0`
    // early return, and removing it changed no test because it could not.
    // What the comparison must stay is strict: with `<=`, a collapsed window
    // would match at exactly its own start minute.
    let length = (end - start).rem_euclid(MINUTES_PER_DAY);
    (m - start).rem_euclid(MINUTES_PER_DAY) < length
}

#[cfg(test)]
mod tests {
    use super::{DaylightFilter, DaylightVerdict, is_within_night};
    use birdnet_scheduler::solar::Location;

    /// Local minute-of-day for `HH:MM`.
    fn m(hh: i64, mm: i64) -> i64 {
        hh * 60 + mm
    }

    // ── the wrap-around window ──────────────────────────────────────────
    //
    // Night always spans midnight, so `night_start` (evening) is the larger
    // number. The obvious `start <= x && x < end` is false for every minute of
    // every night — which is why this is a named function with its own gates
    // rather than an inline comparison.

    #[test]
    fn the_small_hours_are_inside_a_window_that_spans_midnight() {
        // 22:00 → 05:00, the shape of every real night.
        let (start, end) = (m(22, 0), m(5, 0));
        for (hh, mm) in [(22, 0), (23, 30), (0, 0), (2, 30), (4, 59)] {
            assert!(
                is_within_night(m(hh, mm), start, end),
                "{hh:02}:{mm:02} was read as outside the night"
            );
        }
    }

    #[test]
    fn daytime_is_outside_a_window_that_spans_midnight() {
        // Counterpart: a window covering the whole day would satisfy the gate
        // above, and would quarantine every detection a station ever made.
        let (start, end) = (m(22, 0), m(5, 0));
        for (hh, mm) in [(5, 0), (6, 0), (12, 0), (18, 0), (21, 59)] {
            assert!(
                !is_within_night(m(hh, mm), start, end),
                "{hh:02}:{mm:02} was read as the middle of the night"
            );
        }
    }

    #[test]
    fn the_window_boundaries_are_start_inclusive_and_end_exclusive() {
        let (start, end) = (m(22, 0), m(5, 0));
        assert!(
            is_within_night(m(22, 0), start, end),
            "the window did not open"
        );
        assert!(
            !is_within_night(m(5, 0), start, end),
            "the window did not close"
        );
    }

    #[test]
    fn a_window_given_in_minutes_outside_a_day_still_works() {
        // `sunset + margin` overruns 1440 whenever the sun sets after 23:00,
        // and `sunrise - margin` goes negative for a sunrise before the
        // margin — both routine at high latitude in summer.
        // Window 00:30 → 01:00, given as start 1470 (= 24:30).
        assert!(is_within_night(m(0, 45), 1440 + 30, 60));
        assert!(!is_within_night(m(12, 0), 1440 + 30, 60));
        // Window 23:45 → 00:30, given as start -15.
        assert!(is_within_night(m(23, 50), -15, 30));
        assert!(is_within_night(m(0, 15), -15, 30));
        assert!(!is_within_night(m(12, 0), -15, 30));
    }

    #[test]
    fn a_collapsed_window_is_no_night_rather_than_a_whole_day() {
        // Near the solstice at high latitude the margins can push the start
        // past the end. Read as an inverted window that is the whole day, and
        // every detection in June would be quarantined.
        assert!(!is_within_night(m(2, 0), m(3, 0), m(3, 0)));
        assert!(!is_within_night(m(12, 0), m(3, 0), m(3, 0)));
        // The start minute itself, which is the one a `<=` comparison would
        // wrongly admit — and the only minute that distinguishes the two.
        assert!(!is_within_night(m(3, 0), m(3, 0), m(3, 0)));
    }

    // ── the filter as a whole ───────────────────────────────────────────

    /// A filter at Greenwich, UTC, with an hour of margin.
    fn greenwich(extra: &[&str]) -> DaylightFilter {
        DaylightFilter::new(
            Some(Location::new_unchecked(51.48, 0.0)),
            60,
            0,
            extra.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    #[test]
    fn a_songbird_in_the_small_hours_is_quarantined() {
        // Greenwich, 15 January, from `SolarDay`: sunrise 07:59 UTC (minute
        // 479), sunset 16:18 (minute 978) — read off the real routine, not
        // assumed. 02:30 is deep night by any margin.
        assert_eq!(
            greenwich(&[]).verdict("Cyanistes caeruleus", "2026-01-15", "02:30:00"),
            DaylightVerdict::Quarantine
        );
    }

    #[test]
    fn the_same_songbird_in_the_afternoon_is_kept() {
        // Counterpart: a filter that quarantined regardless of hour would
        // satisfy the gate above and silently empty the station.
        assert_eq!(
            greenwich(&[]).verdict("Cyanistes caeruleus", "2026-01-15", "13:00:00"),
            DaylightVerdict::Keep
        );
    }

    #[test]
    fn an_owl_in_the_small_hours_is_kept() {
        // The taxon half. Without it this is a blanket night filter, and the
        // detections it discards are the ones an operator most wants.
        for (sci, hhmmss) in [
            ("Strix aluco", "02:30:00"),
            ("Tyto alba", "01:00:00"),
            ("Caprimulgus europaeus", "23:45:00"),
            ("Botaurus stellaris", "03:15:00"),
            ("Rallus aquaticus", "02:00:00"),
        ] {
            assert_eq!(
                greenwich(&[]).verdict(sci, "2026-01-15", hhmmss),
                DaylightVerdict::Keep,
                "{sci} was quarantined for calling at night"
            );
        }
    }

    #[test]
    fn the_dusk_and_dawn_chorus_is_inside_the_margin() {
        // Birds sing through dusk and start again well before sunrise. A
        // window that opened at sunset would quarantine the evening chorus,
        // which is a large part of what a garden station records.
        //
        // Greenwich, 15 January: sunset 16:18, sunrise 07:59. With an hour of
        // margin the window is 17:18 → 06:59, so 16:20 (two minutes after
        // sunset) and 07:30 (twenty-nine before sunrise) are both inside the
        // margin and outside the window — which is the whole point of having
        // one.
        let f = greenwich(&[]);
        assert_eq!(
            f.verdict("Cyanistes caeruleus", "2026-01-15", "16:20:00"),
            DaylightVerdict::Keep,
            "the evening chorus was quarantined"
        );
        assert_eq!(
            f.verdict("Cyanistes caeruleus", "2026-01-15", "07:30:00"),
            DaylightVerdict::Keep,
            "the pre-dawn chorus was quarantined"
        );
    }

    /// The same station, with its clock set to a whole-hour offset.
    ///
    /// Greenwich's solar minutes are the known ones either way — only the
    /// conversion into the operator's clock changes.
    fn greenwich_at_offset(utc_offset_secs: i64) -> DaylightFilter {
        DaylightFilter::new(
            Some(Location::new_unchecked(51.48, 0.0)),
            60,
            utc_offset_secs,
            Vec::new(),
        )
    }

    #[test]
    fn the_solar_events_are_shifted_into_the_station_clock_not_away_from_it() {
        // Every other test here runs at UTC, where `+ offset_min` and
        // `- offset_min` are the same expression — which is why cargo-mutants
        // found both of them surviving. A station away from UTC is where the
        // sign is load-bearing: get it wrong and the window moves by twice the
        // offset, so a station at UTC+3 quarantines its afternoon and records
        // its small hours.
        //
        // Greenwich, 15 January: sunrise minute 479 (07:59 UTC), sunset 978
        // (16:18). At UTC+3 those are 10:59 and 19:18 local, so with an hour
        // of margin the night window is 20:18 → 09:59.
        let east = greenwich_at_offset(3 * 3600);
        assert_eq!(
            east.verdict("Cyanistes caeruleus", "2026-01-15", "05:00:00"),
            DaylightVerdict::Quarantine,
            "05:00 is four hours before local sunrise and should be night"
        );
        assert_eq!(
            east.verdict("Cyanistes caeruleus", "2026-01-15", "16:00:00"),
            DaylightVerdict::Keep,
            "16:00 is three hours before local sunset and should be day"
        );

        // Westward, to pin the direction rather than merely "some shift
        // happens": at UTC-3 sunrise is 04:59 and sunset 13:18 local, so the
        // window is 14:18 -> 03:59 and the same two clock times swap sides.
        let west = greenwich_at_offset(-3 * 3600);
        assert_eq!(
            west.verdict("Cyanistes caeruleus", "2026-01-15", "05:00:00"),
            DaylightVerdict::Keep,
            "05:00 is after local sunrise at UTC-3 and should be day"
        );
        assert_eq!(
            west.verdict("Cyanistes caeruleus", "2026-01-15", "16:00:00"),
            DaylightVerdict::Quarantine,
            "16:00 is after local sunset at UTC-3 and should be night"
        );
    }

    #[test]
    fn the_operator_can_exempt_a_species_or_a_whole_genus() {
        // The genus table cannot be complete, and a station that hears a
        // particular bird at night every week should not keep approving it.
        assert_eq!(
            greenwich(&["Cyanistes caeruleus"]).verdict(
                "Cyanistes caeruleus",
                "2026-01-15",
                "02:30:00"
            ),
            DaylightVerdict::Keep
        );
        assert_eq!(
            greenwich(&["Catharus"]).verdict("Catharus ustulatus", "2026-01-15", "02:30:00"),
            DaylightVerdict::Keep,
            "a genus-level exemption did not cover its species"
        );
        // ...and an exemption for something else does not cover this bird.
        assert_eq!(
            greenwich(&["Catharus"]).verdict("Cyanistes caeruleus", "2026-01-15", "02:30:00"),
            DaylightVerdict::Quarantine
        );
    }

    // ── failing open ────────────────────────────────────────────────────

    #[test]
    fn a_station_with_no_coordinates_quarantines_nothing() {
        let f = DaylightFilter::new(None, 60, 0, Vec::new());
        assert!(!f.is_enabled());
        assert_eq!(
            f.verdict("Cyanistes caeruleus", "2026-01-15", "02:30:00"),
            DaylightVerdict::Keep
        );
    }

    #[test]
    fn an_unreadable_timestamp_keeps_the_detection() {
        // `Date`/`Time` are free-form text. Quarantining a real detection
        // because its filename was odd is the worse failure.
        let f = greenwich(&[]);
        for (date, time) in [
            ("", ""),
            ("not-a-date", "02:30:00"),
            ("2026-01-15", "2:30:00"),
        ] {
            assert_eq!(
                f.verdict("Cyanistes caeruleus", date, time),
                DaylightVerdict::Keep,
                "{date:?} {time:?} was quarantined"
            );
        }
    }

    #[test]
    fn a_polar_summer_quarantines_nothing() {
        // Above the Arctic circle in June the sun does not set, so there is no
        // sunrise or sunset minute to compare against. The filter must keep
        // everything rather than quarantine a whole season.
        let f = DaylightFilter::new(
            Some(Location::new_unchecked(78.22, 15.65)), // Longyearbyen
            60,
            3600,
            Vec::new(),
        );
        assert_eq!(
            f.verdict("Cyanistes caeruleus", "2026-06-21", "02:30:00"),
            DaylightVerdict::Keep
        );
    }

    #[test]
    fn the_station_timezone_moves_the_window() {
        // The solar routine reports UTC; the detection's timestamp is local.
        // Getting this wrong shifts the night by the whole offset, which at
        // UTC+12 means quarantining the middle of the afternoon.
        let sydney = |offset_secs| {
            DaylightFilter::new(
                Some(Location::new_unchecked(-33.87, 151.21)),
                60,
                offset_secs,
                Vec::new(),
            )
        };
        // Sydney is UTC+11 in January: sunrise 1138 UTC → 05:58 local, sunset
        // 548 UTC → 20:08 local, so the window is 21:08 → 04:58 and 02:30
        // local is deep night.
        assert_eq!(
            sydney(11 * 3600).verdict("Cyanistes caeruleus", "2026-01-15", "02:30:00"),
            DaylightVerdict::Quarantine
        );
        // With the offset wrongly left at UTC the window becomes 10:08 → 17:58
        // and 02:30 falls outside it, so the filter keeps a detection it
        // should have caught — which is the bug this pins.
        assert_eq!(
            sydney(0).verdict("Cyanistes caeruleus", "2026-01-15", "02:30:00"),
            DaylightVerdict::Keep
        );
    }
}
