//! Night-inhibit logic.
//!
//! [`NightInhibit`] wraps sunrise/sunset times and optional offset margins
//! to produce a simple "is recording allowed right now?" answer.
//!
//! Unlike [`RecordingWindow`] this type is constructed directly from solar
//! event minutes (as returned by [`crate::solar::SolarDay`]) with offset
//! minutes supplied by the user configuration.

use serde::{Deserialize, Serialize};

use crate::traits::RecordingGate;

/// Inhibit recording during darkness.
///
/// Recording is allowed between:
/// `(sunrise − pre_offset_min)` and `(sunset + post_offset_min)`.
///
/// Both times are clamped to `[0, 1439]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NightInhibit {
    /// Resolved recording start (minutes since midnight).
    pub allow_from_min: u32,
    /// Resolved recording end (minutes since midnight).
    pub allow_until_min: u32,
}

impl NightInhibit {
    /// Create a new inhibit from pre-computed sunrise/sunset minutes.
    ///
    /// `pre_offset_min`  — extra minutes before sunrise to start recording (≥ 0).
    /// `post_offset_min` — extra minutes after sunset to keep recording (≥ 0).
    #[must_use]
    pub fn new(
        sunrise_min: u32,
        sunset_min: u32,
        pre_offset_min: u32,
        post_offset_min: u32,
    ) -> Self {
        let from = sunrise_min.saturating_sub(pre_offset_min);
        let until = (sunset_min + post_offset_min).min(1439);
        Self {
            allow_from_min: from,
            allow_until_min: until,
        }
    }

    /// Convenience: always allow recording (disables night inhibit).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            allow_from_min: 0,
            // 1440 so that `m < 1440` holds for all valid minutes (0..=1439).
            allow_until_min: 1440,
        }
    }

    /// Is recording currently permitted at `minutes_since_midnight`?
    #[must_use]
    pub fn is_recording_allowed(&self, minutes_since_midnight: u32) -> bool {
        let m = minutes_since_midnight;
        m >= self.allow_from_min && m < self.allow_until_min
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

    #[test]
    fn pre_offset_clamps_to_zero() {
        // sunrise at 5 min, offset 30 → should clamp to 0
        let inhibit = NightInhibit::new(5, 1200, 30, 0);
        assert_eq!(inhibit.allow_from_min, 0);
    }

    #[test]
    fn post_offset_clamps_to_1439() {
        // sunset 1430, offset 30 → should clamp to 1439
        let inhibit = NightInhibit::new(360, 1430, 0, 30);
        assert_eq!(inhibit.allow_until_min, 1439);
    }
}
