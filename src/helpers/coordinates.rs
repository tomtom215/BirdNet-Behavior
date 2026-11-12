//! How a station's position is written anywhere it can leave the machine.
//!
//! One place, because two places is how the support bundle came to promise one
//! decimal while `--doctor`, staged into it, printed four: the bundle's
//! journal and settings were coarsened, and the doctor's report, written by
//! other code, was not.
//!
//! The rounding is done where the report is *written*, not by whatever copies
//! it afterwards. The report is what the bug template asks people to paste into
//! a public issue and what the bundle stages as `doctor.txt` and `doctor.json`;
//! the bundle's own scrubbing hunts the coordinates in the *config file*, so it
//! cannot know a position typed on the dashboard, which lives only in the
//! settings table. A report that never contains the digits cannot leak them
//! through a path nobody thought of. The exact value stays in the settings page
//! and the config file, where the operator already controls who sees it.

/// A coordinate as a shared report shows it: one decimal place, about 11 km.
///
/// The support bundle and the doctor report are written to be pasted into a
/// public issue, and four decimals of a station's latitude and longitude are a
/// house. One decimal still says which hemisphere, which season's daylight and
/// which region's species the station was configured for, which is everything a
/// support question about the scheduler or the occurrence filter needs.
///
/// `None` for a value that is not a finite number.
#[must_use]
pub fn shared(value: f64) -> Option<String> {
    value.is_finite().then(|| format!("{value:.1}"))
}

/// A position as the doctor report prints it: `47.4, -122.3 (shown to 0.1°)`.
///
/// The note is there so an operator who sees a different number in their
/// settings knows the report rounded it on purpose. `unreadable` when either
/// number is not finite, because a position that is not a number is not one to
/// print digits of.
#[must_use]
pub fn label(lat: f64, lon: f64) -> String {
    match (shared(lat), shared(lon)) {
        (Some(lat), Some(lon)) => format!("{lat}, {lon} (shown to 0.1°)"),
        _ => "unreadable".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_is_shown_to_one_decimal_with_its_note() {
        assert_eq!(label(47.38291, -122.31746), "47.4, -122.3 (shown to 0.1°)");
        assert_eq!(shared(47.38291).as_deref(), Some("47.4"));
    }

    #[test]
    fn a_position_that_is_not_a_number_prints_no_digits() {
        assert_eq!(label(f64::NAN, 1.0), "unreadable");
        assert_eq!(label(1.0, f64::INFINITY), "unreadable");
        assert_eq!(shared(f64::NAN), None);
    }
}
