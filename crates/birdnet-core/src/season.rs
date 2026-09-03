//! Which season a date falls in, at this station's latitude.
//!
//! # Why latitude is a parameter
//!
//! Half the places a station can stand are south of the equator, where March
//! is autumn. A season table with northern dates baked in is wrong by six
//! months for all of them, and the error is invisible: "new this spring"
//! simply labels the wrong three months, and nothing looks broken.
//!
//! Nearer the equator neither set applies. There is no meaningful winter at
//! 3° N; what there is instead is a wet season and a dry one, twice a year in
//! much of the equatorial belt, and bird activity tracks those far better than
//! it tracks a solstice.
//!
//! # The boundary
//!
//! ±10°, not the ±23.5° of the tropics. That is a convention rather than a
//! fact — it is the same one birdnet-go uses, and the reasoning is that the
//! seasonal signal a bird station actually sees fades well before the
//! astronomical tropic. An operator whose site disagrees can say so; the
//! classification is a default, not a claim about their weather.
//!
//! # The year a season belongs to
//!
//! Northern winter starts on 21 December and ends in March, so it spans a year
//! boundary and a January date belongs to the winter that started the *previous*
//! December. Getting this wrong splits one winter into two, which makes "first
//! of the season" fire twice for the same bird in the same winter — in January,
//! for a species already recorded in December.
//!
//! [`SeasonOccurrence`] is therefore a season *and* the year it started in, and
//! that pair is the identity anything comparing seasons should use.

use crate::civil::days_from_civil;

/// Latitude above which the northern-hemisphere table applies.
pub const NORTHERN_THRESHOLD_DEG: f64 = 10.0;

/// Latitude below which the southern-hemisphere table applies.
pub const SOUTHERN_THRESHOLD_DEG: f64 = -10.0;

/// Which of the three season tables a latitude selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hemisphere {
    /// Above [`NORTHERN_THRESHOLD_DEG`].
    Northern,
    /// Below [`SOUTHERN_THRESHOLD_DEG`].
    Southern,
    /// Between the two: wet and dry rather than four temperate seasons.
    Equatorial,
}

impl Hemisphere {
    /// The hemisphere `latitude_deg` falls in.
    #[must_use]
    pub fn from_latitude(latitude_deg: f64) -> Self {
        if latitude_deg > NORTHERN_THRESHOLD_DEG {
            Self::Northern
        } else if latitude_deg < SOUTHERN_THRESHOLD_DEG {
            Self::Southern
        } else {
            Self::Equatorial
        }
    }

    /// The season boundaries for this hemisphere, in calendar order.
    ///
    /// Each entry is `(name, month, day)` and names the season that *starts*
    /// on that date. The list is ordered by date within the year, which is
    /// what [`season_on`] relies on.
    #[must_use]
    pub const fn boundaries(self) -> &'static [(&'static str, u32, u32)] {
        match self {
            // Astronomical starts, to the day the solstices and equinoxes
            // most often fall on. They wander by a day either side across the
            // leap cycle; pinning them would mean a table that is right for
            // one year in four, and a bird does not know which day it is.
            Self::Northern => &[
                ("spring", 3, 20),
                ("summer", 6, 21),
                ("fall", 9, 22),
                ("winter", 12, 21),
            ],
            Self::Southern => &[
                ("fall", 3, 20),
                ("winter", 6, 21),
                ("spring", 9, 22),
                ("summer", 12, 21),
            ],
            // Month starts rather than astronomical dates: equatorial wet and
            // dry seasons are driven by the ITCZ, which does not care about
            // the solstice, and no single set of dates fits the whole belt.
            // These are the conventional quarters and are meant to be
            // overridable rather than authoritative.
            Self::Equatorial => &[
                ("wet1", 3, 1),
                ("dry1", 6, 1),
                ("wet2", 9, 1),
                ("dry2", 12, 1),
            ],
        }
    }
}

/// A season, and the calendar year it began in.
///
/// The pair, not the name alone: a northern winter spans a year boundary, so
/// `("winter", 2025)` runs from 21 December 2025 into March 2026 and every day
/// of it is the same occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonOccurrence {
    /// Season name, from [`Hemisphere::boundaries`].
    pub season: &'static str,
    /// The calendar year the season started in.
    pub start_year: u32,
    /// The date it started, as `YYYY-MM-DD`.
    pub start_date: String,
}

/// The season occurrence containing `(year, month, day)` at `latitude_deg`.
///
/// Returns `None` only for a date the civil calendar rejects.
#[must_use]
pub fn season_on(latitude_deg: f64, year: u32, month: u32, day: u32) -> Option<SeasonOccurrence> {
    if !(1..=12).contains(&month) || day == 0 || day > crate::civil::days_in_month(year, month) {
        return None;
    }
    let hemisphere = Hemisphere::from_latitude(latitude_deg);
    let boundaries = hemisphere.boundaries();
    let target = days_from_civil(year, month, day);

    // Walk backwards through this year's boundaries for the latest one that
    // has already passed.
    for &(name, bm, bd) in boundaries.iter().rev() {
        if days_from_civil(year, bm, bd) <= target {
            return Some(SeasonOccurrence {
                season: name,
                start_year: year,
                start_date: format!("{year:04}-{bm:02}-{bd:02}"),
            });
        }
    }

    // Before the first boundary of this year, so the season began last year —
    // the year-spanning case, and the one a naive implementation gets wrong.
    // Every January in the northern table lands here.
    let (name, bm, bd) = *boundaries.last()?;
    let prev = year.checked_sub(1)?;
    Some(SeasonOccurrence {
        season: name,
        start_year: prev,
        start_date: format!("{prev:04}-{bm:02}-{bd:02}"),
    })
}

/// Parse `YYYY-MM-DD` and return its season occurrence.
#[must_use]
pub fn season_on_date(latitude_deg: f64, date: &str) -> Option<SeasonOccurrence> {
    let mut parts = date.split('-');
    let year: u32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    season_on(latitude_deg, year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn season(lat: f64, date: &str) -> SeasonOccurrence {
        season_on_date(lat, date).unwrap_or_else(|| panic!("no season for {date}"))
    }

    #[test]
    fn latitude_selects_the_table() {
        assert_eq!(Hemisphere::from_latitude(52.0), Hemisphere::Northern);
        assert_eq!(Hemisphere::from_latitude(-33.9), Hemisphere::Southern);
        assert_eq!(Hemisphere::from_latitude(0.0), Hemisphere::Equatorial);
        assert_eq!(Hemisphere::from_latitude(9.9), Hemisphere::Equatorial);
        assert_eq!(Hemisphere::from_latitude(10.1), Hemisphere::Northern);
        assert_eq!(Hemisphere::from_latitude(-10.1), Hemisphere::Southern);
    }

    /// March is spring in the north and autumn in the south.
    ///
    /// The single fact this module exists for. A station at Cape Town running
    /// a northern table labels its autumn "spring" for three months and
    /// nothing looks broken.
    #[test]
    fn the_same_date_is_a_different_season_in_each_hemisphere() {
        assert_eq!(season(52.0, "2026-04-15").season, "spring");
        assert_eq!(season(-33.9, "2026-04-15").season, "fall");
        assert_eq!(season(52.0, "2026-10-15").season, "fall");
        assert_eq!(season(-33.9, "2026-10-15").season, "spring");
    }

    /// Near the equator neither temperate table applies.
    #[test]
    fn an_equatorial_station_gets_wet_and_dry() {
        assert_eq!(season(1.3, "2026-04-15").season, "wet1");
        assert_eq!(season(1.3, "2026-07-15").season, "dry1");
        assert_eq!(season(1.3, "2026-10-15").season, "wet2");
        assert_eq!(season(1.3, "2026-01-15").season, "dry2");
    }

    /// A January date belongs to the winter that started in December.
    ///
    /// The year-spanning case. Getting it wrong splits one winter in two, so
    /// "first of the season" fires again in January for a bird already
    /// recorded in December — which is the kind of wrong that looks like a
    /// feature working.
    #[test]
    fn january_belongs_to_the_previous_december_winter() {
        let jan = season(52.0, "2026-01-15");
        assert_eq!(jan.season, "winter");
        assert_eq!(jan.start_year, 2025, "the winter began in December 2025");
        assert_eq!(jan.start_date, "2025-12-21");

        let dec = season(52.0, "2025-12-25");
        assert_eq!(
            dec, jan,
            "25 December and the following 15 January are the same winter"
        );
    }

    /// And the counterpart: two consecutive Januaries are different winters.
    ///
    /// Without this, an implementation that mapped every winter to one
    /// occurrence would satisfy the test above and "first of the season" would
    /// fire once ever.
    #[test]
    fn consecutive_winters_are_different_occurrences() {
        assert_ne!(season(52.0, "2026-01-15"), season(52.0, "2027-01-15"));
        assert_eq!(season(52.0, "2027-01-15").start_year, 2026);
    }

    /// The southern summer spans the year boundary in the same way.
    #[test]
    fn the_southern_summer_also_spans_the_year_boundary() {
        let jan = season(-33.9, "2026-01-15");
        assert_eq!(jan.season, "summer");
        assert_eq!(jan.start_year, 2025);
        assert_eq!(jan, season(-33.9, "2025-12-25"));
    }

    /// A boundary day belongs to the season it starts, not the one before.
    #[test]
    fn a_boundary_day_starts_its_own_season() {
        assert_eq!(season(52.0, "2026-03-20").season, "spring");
        assert_eq!(season(52.0, "2026-03-19").season, "winter");
        assert_eq!(season(52.0, "2026-12-21").season, "winter");
        assert_eq!(season(52.0, "2026-12-20").season, "fall");
    }

    /// Every day of a year lands in exactly one season, and the four seasons
    /// partition it.
    ///
    /// A table with a gap or an overlap would show up as a day with no season
    /// or a run that jumps backwards; neither is visible from spot checks.
    #[test]
    fn every_day_of_a_year_has_exactly_one_season() {
        for lat in [52.0_f64, -33.9, 1.3] {
            let mut seen: Vec<(&str, u32)> = Vec::new();
            for month in 1..=12 {
                for day in 1..=crate::civil::days_in_month(2026, month) {
                    let s = season_on(lat, 2026, month, day)
                        .unwrap_or_else(|| panic!("no season at {lat} on 2026-{month}-{day}"));
                    let key = (s.season, s.start_year);
                    if seen.last() != Some(&key) {
                        assert!(
                            !seen.contains(&key),
                            "at {lat}, season {key:?} recurs after another one \
                             — the boundaries are out of order"
                        );
                        seen.push(key);
                    }
                }
            }
            assert_eq!(
                seen.len(),
                5,
                "at {lat} a calendar year should cross four boundaries and so touch five \
                 season occurrences (the one it starts in, then four), got {seen:?}"
            );
        }
    }

    /// A leap year's 29 February is a real date with a season.
    #[test]
    fn a_leap_day_has_a_season() {
        assert_eq!(season(52.0, "2028-02-29").season, "winter");
        assert!(
            season_on(52.0, 2026, 2, 29).is_none(),
            "2026 is not a leap year"
        );
    }

    /// Malformed dates are rejected rather than guessed at.
    #[test]
    fn a_malformed_date_is_rejected() {
        assert!(season_on_date(52.0, "2026-13-01").is_none());
        assert!(season_on_date(52.0, "2026-00-01").is_none());
        assert!(season_on_date(52.0, "2026-01-32").is_none());
        assert!(season_on_date(52.0, "not-a-date").is_none());
        assert!(season_on_date(52.0, "2026-01-01-extra").is_none());
        assert!(season_on_date(52.0, "2026-01").is_none());
    }
}
