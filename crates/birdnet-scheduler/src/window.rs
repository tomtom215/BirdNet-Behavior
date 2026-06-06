//! Recording time window types.
//!
//! A [`RecordingWindow`] defines when the system is allowed to record.
//! Windows can be:
//! - **Fixed** — absolute clock times (e.g. 06:00–22:00 every day)
//! - **Solar** — relative to sunrise/sunset (e.g. 30 min before sunrise
//!   to 30 min after sunset)
//! - **`AllDay`** — no restriction (always record)

use serde::{Deserialize, Serialize};

use crate::error::SchedulerError;
use crate::traits::RecordingGate;

/// A clock time represented as minutes since midnight (0–1439).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TimeOfDay(u32);

impl TimeOfDay {
    /// Create from hours and minutes.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidWindow`] if hours ≥ 24 or minutes ≥ 60.
    pub fn from_hm(hours: u32, minutes: u32) -> Result<Self, SchedulerError> {
        if hours >= 24 || minutes >= 60 {
            return Err(SchedulerError::InvalidWindow(format!(
                "{hours:02}:{minutes:02} is not a valid time"
            )));
        }
        Ok(Self(hours * 60 + minutes))
    }

    /// Minutes since midnight.
    #[must_use]
    pub const fn as_minutes(&self) -> u32 {
        self.0
    }

    /// Format as `HH:MM`.
    #[must_use]
    pub fn as_hm_string(&self) -> String {
        format!("{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

impl TryFrom<u32> for TimeOfDay {
    type Error = SchedulerError;
    fn try_from(minutes: u32) -> Result<Self, Self::Error> {
        if minutes >= 1440 {
            return Err(SchedulerError::InvalidWindow(format!(
                "{minutes} minutes exceeds 1440"
            )));
        }
        Ok(Self(minutes))
    }
}

/// The kind of time window to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WindowKind {
    /// Always record.
    AllDay,
    /// Fixed clock window.
    Fixed {
        /// Start time (minutes since midnight).
        start_min: u32,
        /// End time (minutes since midnight).
        end_min: u32,
    },
    /// Solar-relative window.
    Solar {
        /// Minutes before sunrise to start recording (negative = after sunrise).
        pre_sunrise_min: i32,
        /// Minutes after sunset to stop recording (negative = before sunset).
        post_sunset_min: i32,
    },
}

/// A validated recording window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingWindow {
    /// The window kind.
    pub kind: WindowKind,
}

impl RecordingWindow {
    /// Create an all-day window (never inhibited).
    #[must_use]
    pub const fn all_day() -> Self {
        Self {
            kind: WindowKind::AllDay,
        }
    }

    /// Create a fixed window.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidWindow`] if start ≥ end or values are out of range.
    pub fn fixed(start_min: u32, end_min: u32) -> Result<Self, SchedulerError> {
        if start_min >= 1440 || end_min >= 1440 {
            return Err(SchedulerError::InvalidWindow(
                "time values must be < 1440 minutes".to_string(),
            ));
        }
        if start_min >= end_min {
            return Err(SchedulerError::InvalidWindow(format!(
                "start ({start_min}) must be < end ({end_min})"
            )));
        }
        Ok(Self {
            kind: WindowKind::Fixed { start_min, end_min },
        })
    }

    /// Create a solar-relative window.
    #[must_use]
    pub const fn solar(pre_sunrise_min: i32, post_sunset_min: i32) -> Self {
        Self {
            kind: WindowKind::Solar {
                pre_sunrise_min,
                post_sunset_min,
            },
        }
    }

    /// Resolve a solar window to a fixed window given sunrise/sunset minutes.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidWindow`] if the resolved times are out of range.
    pub fn resolve_solar(&self, sunrise_min: u32, sunset_min: u32) -> Result<Self, SchedulerError> {
        match self.kind {
            WindowKind::Solar {
                pre_sunrise_min,
                post_sunset_min,
            } => {
                // Compute in i64 so a negative offset can't wrap via `as u32`.
                // The prior cast `(... .min(1439)) as u32` on a negative value
                // produced a huge wrapped u32, which silently sailed past the
                // `start >= end` order check.
                let start_i = (i64::from(sunrise_min) - i64::from(pre_sunrise_min))
                    .clamp(0, 1439);
                let end_i = (i64::from(sunset_min) + i64::from(post_sunset_min))
                    .clamp(0, 1439);
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let start = start_i as u32;
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let end = end_i as u32;
                // Degrade gracefully on an inverted/empty window instead of
                // erroring out. A large `pre_sunrise_min` paired with a
                // negative `post_sunset_min` can compute `start >= end`; the
                // operator-friendly behaviour is to keep recording (24/7)
                // rather than silently disable it, so the station still
                // captures audio with the surprising config rather than going
                // dark with an obscure scheduler error. (`birdnet-scheduler`
                // is a pure-logic crate with no tracing dependency; the
                // observable signal is the resolved `WindowKind::AllDay`,
                // which callers can log if useful.)
                if start >= end {
                    return Ok(Self::all_day());
                }
                Self::fixed(start, end)
            }
            _ => Ok(self.clone()),
        }
    }
}

impl RecordingGate for RecordingWindow {
    fn is_allowed(&self, minutes_since_midnight: u32) -> bool {
        let m = minutes_since_midnight;
        match self.kind {
            // Unresolved solar window: allow everything (caller must resolve first).
            WindowKind::AllDay | WindowKind::Solar { .. } => true,
            WindowKind::Fixed { start_min, end_min } => m >= start_min && m < end_min,
        }
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
    fn time_of_day_from_hm() {
        assert_eq!(TimeOfDay::from_hm(6, 30).unwrap().as_minutes(), 390);
    }

    #[test]
    fn time_of_day_invalid() {
        assert!(TimeOfDay::from_hm(24, 0).is_err());
        assert!(TimeOfDay::from_hm(0, 60).is_err());
    }

    #[test]
    fn fixed_window_allows_within() {
        let w = RecordingWindow::fixed(360, 1320).unwrap(); // 06:00–22:00
        assert!(w.is_allowed(720)); // noon
        assert!(w.is_allowed(360)); // exactly at start
        assert!(!w.is_allowed(359)); // one minute before
        assert!(!w.is_allowed(1320)); // exactly at end (exclusive)
    }

    #[test]
    fn all_day_always_allowed() {
        let w = RecordingWindow::all_day();
        assert!(w.is_allowed(0));
        assert!(w.is_allowed(1439));
    }

    #[test]
    fn fixed_invalid_start_gte_end() {
        assert!(RecordingWindow::fixed(720, 360).is_err());
        assert!(RecordingWindow::fixed(600, 600).is_err());
    }

    #[test]
    fn solar_resolve() {
        let w = RecordingWindow::solar(30, 30);
        let resolved = w.resolve_solar(360, 1200).unwrap(); // sunrise 06:00, sunset 20:00
        // start = 360-30 = 330, end = 1200+30 = 1230
        assert!(resolved.is_allowed(330));
        assert!(resolved.is_allowed(900));
        assert!(!resolved.is_allowed(1230));
        assert!(!resolved.is_allowed(329));
    }

    #[test]
    fn solar_resolve_inverted_window_falls_back_to_all_day() {
        // start = (600-0).clamp(0,1439) = 600
        // end   = (700-200).clamp(0,1439) = 500
        // start >= end → all_day fallback.
        let w = RecordingWindow::solar(0, -200);
        let resolved = w.resolve_solar(600, 700).expect("must not error");
        assert!(resolved.is_allowed(0));
        assert!(resolved.is_allowed(720));
        assert!(resolved.is_allowed(1439));
    }

    #[test]
    fn solar_resolve_negative_end_does_not_wrap_around() {
        // Regression: the old `(sunset_min as i32 + post_sunset_min).min(1439)
        // as u32` would wrap a negative i32 into a huge u32, which then sailed
        // past the order check. Now the clamp runs in i64 first, so a hugely
        // negative offset clamps to 0 — start >= end → all_day fallback.
        let w = RecordingWindow::solar(0, -10_000);
        let resolved = w.resolve_solar(600, 700).expect("must not error");
        // With end clamped to 0 and start = 600, fallback to all_day kicks in.
        assert!(resolved.is_allowed(720));
    }
}
