//! Scheduler error type.

use std::fmt;

/// Errors produced by the scheduler crate.
#[derive(Debug, Clone, PartialEq)]
pub enum SchedulerError {
    /// Latitude is outside `[-90, 90]`.
    InvalidLatitude(f64),
    /// Longitude is outside `[-180, 180]`.
    InvalidLongitude(f64),
    /// Date components (year/month/day) are invalid.
    InvalidDate { year: u32, month: u32, day: u32 },
    /// The sun does not rise or set at this location on this date (polar day/night).
    PolarCondition,
    /// A time window is malformed (e.g. start >= end).
    InvalidWindow(String),
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLatitude(lat) => write!(f, "invalid latitude {lat}: must be -90..=90"),
            Self::InvalidLongitude(lon) => write!(f, "invalid longitude {lon}: must be -180..=180"),
            Self::InvalidDate { year, month, day } => {
                write!(f, "invalid date {year}-{month:02}-{day:02}")
            }
            Self::PolarCondition => write!(f, "sun does not rise or set (polar day/night)"),
            Self::InvalidWindow(msg) => write!(f, "invalid recording window: {msg}"),
        }
    }
}

impl std::error::Error for SchedulerError {}
