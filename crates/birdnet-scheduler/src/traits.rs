//! Core scheduler traits.
//!
//! Each subsystem (solar, window, inhibit) depends on these abstractions so
//! that implementations can be swapped — e.g. for testing with a fake clock.

use crate::error::SchedulerError;

/// A source of the current time (minutes since midnight, local solar time).
///
/// The default implementation reads from [`std::time::SystemTime`].
pub trait TimeSource: Send + Sync {
    /// Returns minutes since midnight (0–1439) in local time.
    fn minutes_since_midnight(&self) -> u32;
    /// Returns the current day-of-year (1–366).
    fn day_of_year(&self) -> u32;
}

/// Computes solar events (sunrise, sunset, civil twilight) for a given location and date.
pub trait SolarCalculator: Send + Sync {
    /// Sunrise in minutes since midnight (local solar time), or `Err(PolarCondition)`.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::PolarCondition`] when the sun does not rise.
    fn sunrise_minutes(&self) -> Result<u32, SchedulerError>;

    /// Sunset in minutes since midnight (local solar time), or `Err(PolarCondition)`.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::PolarCondition`] when the sun does not set.
    fn sunset_minutes(&self) -> Result<u32, SchedulerError>;

    /// Civil twilight start (minutes before sunrise).
    fn civil_dawn_minutes(&self) -> Result<u32, SchedulerError>;

    /// Civil twilight end (minutes after sunset).
    fn civil_dusk_minutes(&self) -> Result<u32, SchedulerError>;
}

/// Decides whether recording is currently allowed.
pub trait RecordingGate: Send + Sync {
    /// Returns `true` if recording should proceed at the given minute of the day.
    fn is_allowed(&self, minutes_since_midnight: u32) -> bool;
}
