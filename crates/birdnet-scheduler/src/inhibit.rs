//! Night-inhibit logic.
//!
//! [`NightInhibit`] wraps sunrise/sunset times and optional offset margins
//! to produce a simple "is recording allowed right now?" answer.
//!
//! Unlike [`crate::RecordingWindow`] this type is constructed directly from solar
//! event minutes (as returned by [`crate::solar::SolarDay`]) with offset
//! minutes supplied by the user configuration.

use serde::{Deserialize, Serialize};

use crate::traits::RecordingGate;

/// Inhibit recording during darkness.
///
/// Recording is allowed between `(sunrise − pre_offset_min)` and
/// `(sunset + post_offset_min)`, both wrapped into `[0, 1440)`.
///
/// # The window may cross midnight, and usually does
///
/// The minutes this type carries are minutes of the **UTC** day, because
/// [`crate::solar::SolarDay`] reports solar events as absolute instants. One
/// local day's daylight only fits inside one UTC day near the Greenwich
/// meridian. Elsewhere it straddles UTC midnight, and the *wrapped* sunrise
/// minute is then larger than the wrapped sunset minute:
///
/// | Station         | sunrise UTC | sunset UTC |
/// |-----------------|-------------|------------|
/// | London, 21 Jun  | 03:42       | 20:21      |
/// | New York, 21 Jun| 09:24       | 00:30      |
/// | Auckland, 21 Jun| 19:33       | 05:11      |
///
/// So `allow_from_min > allow_until_min` is the *normal* case for most of the
/// world, not an error, and [`Self::is_recording_allowed`] reads it as a
/// window that wraps. Treating it as a plain `from <= m < until` interval —
/// which is what this did until the gate in
/// `tests/solar_window_worldwide.rs` was written — makes the window empty and
/// the station records nothing, all day, everywhere east of about 90° E or
/// west of about 75° W.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NightInhibit {
    /// Resolved recording start, minutes since UTC midnight, in `[0, 1440)`.
    pub allow_from_min: u32,
    /// Resolved recording end (exclusive), minutes since UTC midnight.
    ///
    /// In `[0, 1440)`, except for the sentinel `1440` that
    /// [`Self::disabled`] uses to mean "the whole day". Smaller than
    /// [`Self::allow_from_min`] whenever the window crosses UTC midnight.
    pub allow_until_min: u32,
}

impl NightInhibit {
    /// Create a new inhibit from pre-computed sunrise/sunset minutes.
    ///
    /// `pre_offset_min`  — extra minutes before sunrise to start recording (≥ 0).
    /// `post_offset_min` — extra minutes after sunset to keep recording (≥ 0).
    ///
    /// The offsets wrap rather than clamp. A sunrise at 00:05 UTC with a
    /// 30-minute pre-roll starts recording at 23:35 UTC the previous day, which
    /// is the half hour before sunrise the operator asked for; clamping it to
    /// 00:00 (which is what this did before) silently dropped 25 of those
    /// minutes, and did so hardest at exactly the longitudes where the window
    /// crosses midnight anyway.
    #[must_use]
    pub fn new(
        sunrise_min: u32,
        sunset_min: u32,
        pre_offset_min: u32,
        post_offset_min: u32,
    ) -> Self {
        // Signed, and computed before any wrapping, so a pre-roll larger than
        // the sunrise minute is a negative instant on the previous UTC day
        // rather than a `u32` underflow.
        let raw_from = i64::from(sunrise_min) - i64::from(pre_offset_min);
        let raw_until = i64::from(sunset_min) + i64::from(post_offset_min);
        // Offsets generous enough to cover the whole clock mean "always", which
        // a wrapped window cannot express: `from == until` is one instant, not
        // one day. Measure the span before wrapping and special-case it.
        if raw_until - raw_from >= 1440 {
            return Self::disabled();
        }
        Self {
            allow_from_min: u32::try_from(raw_from.rem_euclid(1440)).unwrap_or(0),
            allow_until_min: u32::try_from(raw_until.rem_euclid(1440)).unwrap_or(0),
        }
    }

    /// Convenience: always allow recording (disables night inhibit).
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            allow_from_min: 0,
            // 1440 so that `m < 1440` holds for all valid minutes (0..=1439).
            allow_until_min: 1440,
        }
    }

    /// Is recording currently permitted at `minutes_since_midnight` (UTC)?
    ///
    /// Half-open at both ends: sunrise is inside the window, sunset is not.
    /// When the window crosses UTC midnight — the common case away from
    /// Greenwich, see the type docs — the two halves are the union, not the
    /// intersection.
    #[must_use]
    pub const fn is_recording_allowed(&self, minutes_since_midnight: u32) -> bool {
        let m = minutes_since_midnight;
        if self.allow_from_min <= self.allow_until_min {
            m >= self.allow_from_min && m < self.allow_until_min
        } else {
            m >= self.allow_from_min || m < self.allow_until_min
        }
    }
}

impl RecordingGate for NightInhibit {
    fn is_allowed(&self, minutes_since_midnight: u32) -> bool {
        self.is_recording_allowed(minutes_since_midnight)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::RecordingGate;

    #[test]
    fn basic_inhibit() {
        // sunrise 06:00 (360), sunset 20:00 (1200), offsets 0
        let inhibit = NightInhibit::new(360, 1200, 0, 0);
        assert!(inhibit.is_recording_allowed(360));
        assert!(inhibit.is_recording_allowed(900));
        assert!(!inhibit.is_recording_allowed(359));
        assert!(!inhibit.is_recording_allowed(1200)); // sunset is exclusive end
    }

    #[test]
    fn with_offsets() {
        // sunrise 06:00, sunset 20:00, 30 min before/after
        let inhibit = NightInhibit::new(360, 1200, 30, 30);
        assert_eq!(inhibit.allow_from_min, 330);
        assert_eq!(inhibit.allow_until_min, 1230);
        assert!(inhibit.is_recording_allowed(330));
        assert!(!inhibit.is_recording_allowed(329));
    }

    #[test]
    fn disabled_allows_all() {
        let inhibit = NightInhibit::disabled();
        assert!(inhibit.is_allowed(0));
        assert!(inhibit.is_allowed(1439));
    }

    /// These two replace `pre_offset_clamps_to_zero` /
    /// `post_offset_clamps_to_1439`, which asserted the clamping that the
    /// wrap fix removed. The clamp was never the contract anyone wanted: it
    /// silently shortened the very pre-roll/post-roll the operator had asked
    /// for, and only at the two ends of the UTC day.
    #[test]
    fn a_pre_roll_reaching_past_utc_midnight_wraps_instead_of_clamping() {
        // Sunrise 00:05 UTC, 30 minutes of pre-roll: 23:35 the previous day.
        let inhibit = NightInhibit::new(5, 1200, 30, 0);
        assert_eq!(inhibit.allow_from_min, 1415);
        assert!(inhibit.is_recording_allowed(1415), "23:35 is inside");
        assert!(inhibit.is_recording_allowed(0), "so is UTC midnight");
        assert!(inhibit.is_recording_allowed(5), "and sunrise");
        assert!(!inhibit.is_recording_allowed(1414), "23:34 is not");
    }

    #[test]
    fn a_post_roll_reaching_past_utc_midnight_wraps_instead_of_clamping() {
        // Sunset 23:50 UTC, 30 minutes of post-roll: 00:20 the next day.
        let inhibit = NightInhibit::new(360, 1430, 0, 30);
        assert_eq!(inhibit.allow_until_min, 20);
        assert!(inhibit.is_recording_allowed(1439), "23:59 is still inside");
        assert!(inhibit.is_recording_allowed(19), "00:19 is the last minute");
        assert!(
            !inhibit.is_recording_allowed(20),
            "00:20 is the exclusive end"
        );
        assert!(
            !inhibit.is_recording_allowed(359),
            "and 05:59 is still night"
        );
    }

    /// Offsets wide enough to cover the clock mean "record always", which a
    /// wrapping window cannot express — `from == until` is one instant, not one
    /// day. Without the span check in `new` this collapses to a window that
    /// allows nothing, which is the opposite of what was asked for.
    #[test]
    fn offsets_wider_than_the_day_mean_always_not_never() {
        let inhibit = NightInhibit::new(360, 1200, 600, 600);
        for m in [0, 1, 359, 720, 1439] {
            assert!(
                inhibit.is_recording_allowed(m),
                "minute {m} should be allowed by a >24h window"
            );
        }
    }

    /// The band either side of the boundary, so the fix cannot be a
    /// blanket "allow everything when it wraps".
    #[test]
    fn a_wrapping_window_still_excludes_its_own_night() {
        // 19:33 UTC to 05:11 UTC — Auckland in June.
        let inhibit = NightInhibit::new(1173, 311, 0, 0);
        assert!(inhibit.is_recording_allowed(1173), "sunrise");
        assert!(inhibit.is_recording_allowed(0), "UTC midnight, mid-window");
        assert!(
            inhibit.is_recording_allowed(310),
            "last minute before sunset"
        );
        assert!(!inhibit.is_recording_allowed(311), "sunset, exclusive");
        assert!(
            !inhibit.is_recording_allowed(720),
            "midday UTC is its night"
        );
        assert!(
            !inhibit.is_recording_allowed(1172),
            "the minute before sunrise"
        );
    }
}
