//! Recording time window types.
//!
//! A [`RecordingWindow`] defines when the system is allowed to record.
//! Windows can be:
//! - **Fixed** — absolute clock times (e.g. 06:00–22:00 every day)
//! - **Solar** — relative to sunrise/sunset (e.g. 30 min before sunrise
//!   to 30 min after sunset)
//! - **AllDay** — no restriction (always record)

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
    pub fn all_day() -> Self {
        Self { kind: WindowKind::AllDay }
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
    pub fn solar(pre_sunrise_min: i32, post_sunset_min: i32) -> Self {
        Self {
            kind: WindowKind::Solar { pre_sunrise_min, post_sunset_min },
        }
    }

    /// Resolve a solar window to a fixed window given sunrise/sunset minutes.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidWindow`] if the resolved times are out of range.
    pub fn resolve_solar(
        &self,
        sunrise_min: u32,
        sunset_min: u32,
    ) -> Result<Self, SchedulerError> {
        match self.kind {
            WindowKind::Solar { pre_sunrise_min, post_sunset_min } => {
                let start = (sunrise_min as i32 - pre_sunrise_min).max(0) as u32;
                let end = ((sunset_min as i32) + post_sunset_min).min(1439) as u32;
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
            WindowKind::AllDay => true,
            WindowKind::Fixed { start_min, end_min } => m >= start_min && m < end_min,
            WindowKind::Solar { .. } => {
                // Unresolved solar window: allow everything (caller must resolve first).
                true
            }
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
}
