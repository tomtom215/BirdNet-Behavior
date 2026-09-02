//! Resolving the windows the species-tracking flags are measured against.
//!
//! The flags themselves are `birdnet_db::species_tracking`, which takes the
//! windows as dates because neither of them belongs in SQL: the season
//! boundary depends on the station's latitude
//! (`birdnet_core::season`), and the tracking year's start is an operator
//! setting whose default is itself a northern convention. This is where those
//! two are turned into the dates the query wants.

use birdnet_db::species_tracking::TrackingWindows;

/// The windows for `on_date`, owned so a caller can hold them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWindows {
    /// First date of the tracking year containing `on_date`.
    pub year_start: String,
    /// First date of the season containing `on_date`.
    pub season_start: String,
    /// Name of that season, or `None` when the station has no latitude set.
    pub season: Option<&'static str>,
    /// Days of silence after which a detection counts as a return.
    pub absence_days: u32,
}

impl ResolvedWindows {
    /// Borrow these as the query's parameter type.
    #[must_use]
    pub fn as_windows(&self) -> TrackingWindows<'_> {
        TrackingWindows {
            year_start: &self.year_start,
            season_start: &self.season_start,
            absence_days: self.absence_days,
        }
    }
}

/// Default day of the month the tracking year resets on.
const DEFAULT_RESET_DAY: u32 = 1;

/// Default month the tracking year resets in.
///
/// January, which is the calendar convention and wrong for anyone who counts
/// their year from the start of the breeding season — hence the setting.
const DEFAULT_RESET_MONTH: u32 = 1;

/// Resolve the windows for `on_date` from the station's settings.
///
/// `on_date` must be `YYYY-MM-DD`. Falls back to the calendar year and to a
/// season window equal to the year window when the station has no latitude,
/// so a freshly installed station still gets a working year list rather than
/// an error — the season flag is simply never true, which
/// [`ResolvedWindows::season`] being `None` says out loud.
#[must_use]
pub fn resolve_windows(conn: &rusqlite::Connection, on_date: &str) -> ResolvedWindows {
    let absence_days = birdnet_db::settings::get_or(conn, "rare_species_days", "30")
        .unwrap_or_else(|_| "30".to_owned())
        .trim()
        .parse::<u32>()
        .unwrap_or(30)
        .min(3650);

    let reset_month =
        setting_u32(conn, "tracking_year_reset_month", DEFAULT_RESET_MONTH).clamp(1, 12);
    let reset_day = setting_u32(conn, "tracking_year_reset_day", DEFAULT_RESET_DAY).clamp(1, 31);

    let year_start = tracking_year_start(on_date, reset_month, reset_day)
        .unwrap_or_else(|| format!("{}-01-01", on_date.get(0..4).unwrap_or("1970")));

    let latitude: Option<f64> = birdnet_db::settings::get_or(conn, "latitude", "")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|v: &f64| v.is_finite() && v.abs() <= 90.0);

    let (season_start, season) = latitude
        .and_then(|lat| birdnet_core::season::season_on_date(lat, on_date))
        .map_or_else(
            || (year_start.clone(), None),
            |s| (s.start_date, Some(s.season)),
        );

    ResolvedWindows {
        year_start,
        season_start,
        season,
        absence_days,
    }
}

/// Read a small unsigned setting, falling back on anything unparseable.
fn setting_u32(conn: &rusqlite::Connection, key: &str, default: u32) -> u32 {
    birdnet_db::settings::get_or(conn, key, &default.to_string())
        .unwrap_or_else(|_| default.to_string())
        .trim()
        .parse::<u32>()
        .unwrap_or(default)
}

/// The first date of the tracking year containing `on_date`.
///
/// With the default 1 January this is just the calendar year. With a reset of,
/// say, 1 March, a date in February belongs to the year that began the
/// *previous* March — the same year-spanning problem the season table has, and
/// the same failure if it is got wrong: "first of the year" fires twice for
/// one bird in one tracking year.
///
/// Returns `None` for a date that does not parse, or a reset date that does not
/// exist in that year (29 February in a common year).
#[must_use]
pub fn tracking_year_start(on_date: &str, reset_month: u32, reset_day: u32) -> Option<String> {
    let year: u32 = on_date.get(0..4)?.parse().ok()?;
    let month: u32 = on_date.get(5..7)?.parse().ok()?;
    let day: u32 = on_date.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    if reset_day > birdnet_core::civil::days_in_month(year, reset_month) {
        return None;
    }
    let started_this_year = (month, day) >= (reset_month, reset_day);
    let start_year = if started_this_year {
        year
    } else {
        year.checked_sub(1)?
    };
    // The reset day must exist in the year the tracking year actually started
    // in, which is not necessarily the one checked above.
    if reset_day > birdnet_core::civil::days_in_month(start_year, reset_month) {
        return None;
    }
    Some(format!("{start_year:04}-{reset_month:02}-{reset_day:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (tempfile::TempDir, crate::state::AppState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = crate::state::AppState::new(dir.path().join("birds.db")).expect("state");
        (dir, state)
    }

    /// With the default reset the tracking year is the calendar year.
    #[test]
    fn the_default_tracking_year_is_the_calendar_year() {
        assert_eq!(
            tracking_year_start("2026-04-10", 1, 1).as_deref(),
            Some("2026-01-01")
        );
        assert_eq!(
            tracking_year_start("2026-01-01", 1, 1).as_deref(),
            Some("2026-01-01")
        );
        assert_eq!(
            tracking_year_start("2026-12-31", 1, 1).as_deref(),
            Some("2026-01-01")
        );
    }

    /// A March reset makes February belong to the previous tracking year.
    ///
    /// The year-spanning case. Without it, a bird recorded in October and
    /// again in February is "first of the year" twice inside one tracking
    /// year that runs March to March.
    #[test]
    fn a_march_reset_puts_february_in_the_previous_tracking_year() {
        assert_eq!(
            tracking_year_start("2026-02-15", 3, 1).as_deref(),
            Some("2025-03-01")
        );
        assert_eq!(
            tracking_year_start("2026-03-01", 3, 1).as_deref(),
            Some("2026-03-01"),
            "the reset day itself starts the new tracking year"
        );
        assert_eq!(
            tracking_year_start("2026-02-28", 3, 1).as_deref(),
            Some("2025-03-01"),
            "the day before does not"
        );
    }

    /// A reset date that does not exist is refused rather than guessed at.
    #[test]
    fn an_impossible_reset_date_is_refused() {
        assert_eq!(tracking_year_start("2026-06-01", 2, 30), None);
        assert_eq!(
            tracking_year_start("2028-06-01", 2, 29).as_deref(),
            Some("2028-02-29"),
            "29 February exists in 2028"
        );
        assert_eq!(
            tracking_year_start("2027-06-01", 2, 29),
            None,
            "and not in 2027"
        );
    }

    /// A malformed date yields nothing.
    #[test]
    fn a_malformed_date_yields_nothing() {
        assert_eq!(tracking_year_start("not-a-date", 1, 1), None);
        assert_eq!(tracking_year_start("2026-13-01", 1, 1), None);
    }

    /// With a latitude set, the season window is the station's season.
    #[test]
    fn a_northern_station_gets_northern_seasons() {
        let (_d, state) = state();
        state.with_db(|c| {
            birdnet_db::settings::set(
                c,
                "latitude",
                "52.2",
                birdnet_db::settings::SettingsCategory::Location,
            )
            .expect("set");
        });
        let w = state.with_db(|c| resolve_windows(c, "2026-04-10"));
        assert_eq!(w.season, Some("spring"));
        assert_eq!(w.season_start, "2026-03-20");
        assert_eq!(w.year_start, "2026-01-01");
    }

    /// And a southern one gets southern seasons on the same date.
    ///
    /// The counterpart: a resolver that ignored latitude would pass the test
    /// above and mislabel half the world.
    #[test]
    fn a_southern_station_gets_southern_seasons() {
        let (_d, state) = state();
        state.with_db(|c| {
            birdnet_db::settings::set(
                c,
                "latitude",
                "-33.9",
                birdnet_db::settings::SettingsCategory::Location,
            )
            .expect("set");
        });
        let w = state.with_db(|c| resolve_windows(c, "2026-04-10"));
        assert_eq!(w.season, Some("fall"));
        assert_eq!(w.season_start, "2026-03-20");
    }

    /// A station with no latitude still gets a year list.
    ///
    /// The season window collapses onto the year window, so the season flag
    /// can never be true on its own — and `season` is `None` so a page can say
    /// why rather than showing an unexplained empty badge.
    #[test]
    fn a_station_without_a_latitude_still_gets_a_year_window() {
        let (_d, state) = state();
        let w = state.with_db(|c| resolve_windows(c, "2026-04-10"));
        assert_eq!(w.season, None);
        assert_eq!(w.year_start, "2026-01-01");
        assert_eq!(
            w.season_start, w.year_start,
            "with no season the two windows must coincide, so nothing is 'new this season' \
             that is not also new this year"
        );
    }

    /// A nonsense latitude is treated as unset rather than as a location.
    #[test]
    fn an_impossible_latitude_is_treated_as_unset() {
        let (_d, state) = state();
        state.with_db(|c| {
            birdnet_db::settings::set(
                c,
                "latitude",
                "999",
                birdnet_db::settings::SettingsCategory::Location,
            )
            .expect("set");
        });
        assert_eq!(
            state.with_db(|c| resolve_windows(c, "2026-04-10")).season,
            None
        );
    }

    /// The absence threshold comes from the same setting the rare feeds use.
    ///
    /// Two settings for one concept would let the RSS feed and the day list
    /// disagree about which birds are notable, which is the sort of drift that
    /// is never noticed and never explicable.
    #[test]
    fn the_absence_threshold_comes_from_the_rare_species_setting() {
        let (_d, state) = state();
        state.with_db(|c| {
            birdnet_db::settings::set(
                c,
                "rare_species_days",
                "45",
                birdnet_db::settings::SettingsCategory::Detection,
            )
            .expect("set");
        });
        assert_eq!(
            state
                .with_db(|c| resolve_windows(c, "2026-04-10"))
                .absence_days,
            45
        );
    }
}
