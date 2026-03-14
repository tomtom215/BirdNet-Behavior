//! High-level daily recording schedule.
//!
//! [`DailySchedule`] combines a [`NightInhibit`] (or a fixed window) to
//! answer "is recording allowed right now?" using a live clock.

use serde::{Deserialize, Serialize};

use crate::inhibit::NightInhibit;
use crate::solar::{Location, SolarDay};
use crate::traits::RecordingGate;
use crate::window::RecordingWindow;

/// Configuration for the recording schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    /// Observer location (for solar calculations).
    pub location: Option<Location>,
    /// Extra minutes before sunrise to start recording (default 0).
    #[serde(default)]
    pub pre_sunrise_offset_min: u32,
    /// Extra minutes after sunset to keep recording (default 0).
    #[serde(default)]
    pub post_sunset_offset_min: u32,
    /// If `true`, night inhibit is active (requires `location`).
    #[serde(default)]
    pub night_inhibit: bool,
    /// Override fixed window (supersedes solar calculation when set).
    pub fixed_window: Option<RecordingWindow>,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            location: None,
            pre_sunrise_offset_min: 0,
            post_sunset_offset_min: 0,
            night_inhibit: false,
            fixed_window: None,
        }
    }
}

/// A resolved daily recording schedule for a specific date.
///
/// Created by [`DailySchedule::for_today`] or [`DailySchedule::for_date`].
#[derive(Debug, Clone)]
pub struct DailySchedule {
    gate: ScheduleGate,
    /// Solar events for this day, if available.
    pub solar: Option<SolarDay>,
}

/// Internal gate — either a NightInhibit or a fixed window.
#[derive(Debug, Clone)]
enum ScheduleGate {
    Inhibit(NightInhibit),
    Window(RecordingWindow),
}

impl DailySchedule {
    /// Build a schedule for the given date.
    ///
    /// Order of precedence:
    /// 1. `fixed_window` in config (ignores solar entirely)
    /// 2. `night_inhibit` with solar calculation (requires `location`)
    /// 3. All-day (no restriction)
    #[must_use]
    pub fn for_date(config: &ScheduleConfig, year: u32, month: u32, day: u32) -> Self {
        // 1. Fixed window override.
        if let Some(ref fw) = config.fixed_window {
            return Self {
                gate: ScheduleGate::Window(fw.clone()),
                solar: None,
            };
        }

        // 2. Solar-based night inhibit.
        if config.night_inhibit {
            if let Some(loc) = config.location {
                if let Ok(solar) = SolarDay::for_date(loc, year, month, day) {
                    let inhibit = if let (Some(rise), Some(set)) =
                        (solar.sunrise_utc_min, solar.sunset_utc_min)
                    {
                        NightInhibit::new(
                            rise,
                            set,
                            config.pre_sunrise_offset_min,
                            config.post_sunset_offset_min,
                        )
                    } else {
                        // Polar condition — allow all day.
                        NightInhibit::disabled()
                    };
                    return Self {
                        gate: ScheduleGate::Inhibit(inhibit),
                        solar: Some(solar),
                    };
                }
            }
        }

        // 3. All-day.
        Self {
            gate: ScheduleGate::Window(RecordingWindow::all_day()),
            solar: None,
        }
    }

    /// Is recording allowed at `minutes_since_midnight`?
    #[must_use]
    pub fn is_allowed(&self, minutes_since_midnight: u32) -> bool {
        match &self.gate {
            ScheduleGate::Inhibit(inh) => inh.is_allowed(minutes_since_midnight),
            ScheduleGate::Window(w) => w.is_allowed(minutes_since_midnight),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solar::Location;

    fn london() -> Location {
        Location::new_unchecked(51.5074, -0.1278)
    }

    #[test]
    fn all_day_no_config() {
        let config = ScheduleConfig::default();
        let schedule = DailySchedule::for_date(&config, 2026, 3, 14);
        assert!(schedule.is_allowed(0));
        assert!(schedule.is_allowed(1439));
    }

    #[test]
    fn night_inhibit_active() {
        let config = ScheduleConfig {
            location: Some(london()),
            night_inhibit: true,
            ..Default::default()
        };
        let schedule = DailySchedule::for_date(&config, 2026, 3, 14);
        // London March 14: sunrise ~06:10 UTC, sunset ~18:00 UTC
        assert!(schedule.is_allowed(720)); // noon — allowed
        assert!(!schedule.is_allowed(180)); // 03:00 UTC — inhibited
    }

    #[test]
    fn fixed_window_overrides_solar() {
        let config = ScheduleConfig {
            location: Some(london()),
            night_inhibit: true,
            fixed_window: Some(RecordingWindow::fixed(480, 1080).unwrap()), // 08:00–18:00
            ..Default::default()
        };
        let schedule = DailySchedule::for_date(&config, 2026, 3, 14);
        assert!(schedule.is_allowed(600)); // 10:00 — within window
        assert!(!schedule.is_allowed(479)); // just before window
        assert!(!schedule.is_allowed(1080)); // at end (exclusive)
    }
}
