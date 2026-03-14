//! BirdNet-Behavior recording scheduler.
//!
//! Provides:
//! - Solar position and sunrise/sunset calculation (pure Rust, no dependencies)
//! - Recording time windows (fixed or solar-relative)
//! - Night-inhibit: suppress recording during configurable dark hours
//!
//! # Design
//!
//! All types implement traits from [`traits`] so callers depend only on
//! behaviour, not concrete implementations.  The crate is `no_std`-friendly
//! except for [`std::time`] usage in the executors.
//!
//! # Example
//!
//! ```rust
//! use birdnet_scheduler::{Location, SolarDay, RecordingWindow, NightInhibit};
//!
//! let loc = Location::new(51.5, -0.1);
//! let day = SolarDay::for_date(loc, 2026, 3, 14).unwrap();
//! println!("Sunrise: {:?}", day.sunrise_minutes());
//!
//! let inhibit = NightInhibit::new(day.sunrise_minutes(), day.sunset_minutes(), 0, 0);
//! // At noon (720 minutes) recording should be allowed.
//! assert!(inhibit.is_recording_allowed(720));
//! ```

pub mod error;
pub mod solar;
pub mod traits;
pub mod window;
pub mod inhibit;
pub mod schedule;

pub use error::SchedulerError;
pub use solar::{Location, SolarDay};
pub use window::{RecordingWindow, TimeOfDay, WindowKind};
pub use inhibit::NightInhibit;
pub use schedule::{DailySchedule, ScheduleConfig};
