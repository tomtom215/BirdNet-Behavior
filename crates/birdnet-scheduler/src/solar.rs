//! Sunrise/sunset computation using the NOAA/Meeus algorithm.
//!
//! All arithmetic is in 64-bit floats.  The algorithm is accurate to
//! within a few minutes for latitudes up to ±66°.  Polar regions
//! return [`SchedulerError::PolarCondition`].
//!
//! Reference: Jean Meeus, *Astronomical Algorithms*, 2nd ed., Ch. 25.

use crate::error::SchedulerError;
use crate::traits::SolarCalculator;

/// Geographic location (latitude, longitude in decimal degrees).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Location {
    /// Latitude in decimal degrees (−90 to +90, N positive).
    pub lat: f64,
    /// Longitude in decimal degrees (−180 to +180, E positive).
    pub lon: f64,
}

impl Location {
    /// Create a new location.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidLatitude`] / [`SchedulerError::InvalidLongitude`]
    /// if the coordinates are out of range.
    pub fn new(lat: f64, lon: f64) -> Result<Self, SchedulerError> {
        if !(-90.0..=90.0).contains(&lat) {
            return Err(SchedulerError::InvalidLatitude(lat));
        }
        if !(-180.0..=180.0).contains(&lon) {
            return Err(SchedulerError::InvalidLongitude(lon));
        }
        Ok(Self { lat, lon })
    }

    /// Create without validation (for const contexts / trusted input).
    #[must_use]
    pub const fn new_unchecked(lat: f64, lon: f64) -> Self {
        Self { lat, lon }
    }
}

/// Solar events computed for one location on one calendar date.
///
/// All times are in **minutes since midnight, UTC**.
/// Callers are responsible for converting to local time.
#[derive(Debug, Clone, Copy)]
pub struct SolarDay {
    /// Location used for computation.
    pub location: Location,
    /// Day of year (1–366).
    pub day_of_year: u32,
    /// Sunrise in minutes since midnight UTC.
    pub sunrise_utc_min: Option<u32>,
    /// Sunset in minutes since midnight UTC.
    pub sunset_utc_min: Option<u32>,
    /// Civil dawn (−6° before sunrise).
    pub civil_dawn_utc_min: Option<u32>,
    /// Civil dusk (−6° after sunset).
    pub civil_dusk_utc_min: Option<u32>,
}

impl SolarDay {
    /// Compute solar events for the given calendar date at `location`.
    ///
    /// `year`, `month` (1-based), `day` must be valid Gregorian date.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidDate`] for invalid dates.
    pub fn for_date(
        location: Location,
        year: u32,
        month: u32,
        day: u32,
    ) -> Result<Self, SchedulerError> {
        if month < 1 || month > 12 || day < 1 || day > 31 {
            return Err(SchedulerError::InvalidDate { year, month, day });
        }

        let doy = day_of_year(year, month, day);
        let (rise, set) = compute_sunrise_sunset(location.lat, location.lon, doy, year, 0.833_f64);
        let (dawn, dusk) = compute_sunrise_sunset(location.lat, location.lon, doy, year, 6.0_f64);

        Ok(Self {
            location,
            day_of_year: doy,
            sunrise_utc_min: rise.map(|h| (h * 60.0) as u32),
            sunset_utc_min: set.map(|h| (h * 60.0) as u32),
            civil_dawn_utc_min: dawn.map(|h| (h * 60.0) as u32),
            civil_dusk_utc_min: dusk.map(|h| (h * 60.0) as u32),
        })
    }

    /// Return sunrise in minutes since midnight UTC (local-solar time convenience).
    ///
    /// Callers may add a UTC offset (in minutes) to convert to local wall time.
    pub fn sunrise_minutes(&self) -> Option<u32> {
        self.sunrise_utc_min
    }

    /// Return sunset in minutes since midnight UTC.
    pub fn sunset_minutes(&self) -> Option<u32> {
        self.sunset_utc_min
    }
}

impl SolarCalculator for SolarDay {
    fn sunrise_minutes(&self) -> Result<u32, SchedulerError> {
        self.sunrise_utc_min.ok_or(SchedulerError::PolarCondition)
    }

    fn sunset_minutes(&self) -> Result<u32, SchedulerError> {
        self.sunset_utc_min.ok_or(SchedulerError::PolarCondition)
    }

    fn civil_dawn_minutes(&self) -> Result<u32, SchedulerError> {
        self.civil_dawn_utc_min.ok_or(SchedulerError::PolarCondition)
    }

    fn civil_dusk_minutes(&self) -> Result<u32, SchedulerError> {
        self.civil_dusk_utc_min.ok_or(SchedulerError::PolarCondition)
    }
}

// ---------------------------------------------------------------------------
// Private computation helpers
// ---------------------------------------------------------------------------

/// Compute sunrise and sunset in decimal hours UTC.
///
/// `zenith_offset` = 0.833 for standard sunrise/set, 6.0 for civil twilight.
/// Returns `(None, None)` for polar conditions.
fn compute_sunrise_sunset(
    lat_deg: f64,
    lon_deg: f64,
    doy: u32,
    year: u32,
    zenith_offset: f64,
) -> (Option<f64>, Option<f64>) {
    let lat = lat_deg.to_radians();
    let lon = lon_deg;

    // Fractional year (radians)
    let days_in_year = if is_leap_year(year) { 366.0 } else { 365.0 };
    let gamma = 2.0 * std::f64::consts::PI / days_in_year * (f64::from(doy) - 1.0 + 0.5 / 24.0);

    // Equation of time (minutes)
    let eqtime = 229.18
        * (0.000_075
            + 0.001_868 * gamma.cos()
            - 0.032_077 * gamma.sin()
            - 0.014_615 * (2.0 * gamma).cos()
            - 0.040_849 * (2.0 * gamma).sin());

    // Solar declination (radians)
    let decl = 0.006_918
        - 0.399_912 * gamma.cos()
        + 0.070_257 * gamma.sin()
        - 0.006_758 * (2.0 * gamma).cos()
        + 0.000_907 * (2.0 * gamma).sin()
        - 0.002_697 * (3.0 * gamma).cos()
        + 0.001_480 * (3.0 * gamma).sin();

    // Hour angle (degrees): cos(ha) = (cos(90+zenith) - sin(lat)*sin(decl)) / (cos(lat)*cos(decl))
    let zenith_rad = (90.0 + zenith_offset).to_radians();
    let cos_ha = (zenith_rad.cos() - lat.sin() * decl.sin()) / (lat.cos() * decl.cos());

    if cos_ha < -1.0 {
        // Polar day — sun never sets
        return (None, None);
    }
    if cos_ha > 1.0 {
        // Polar night — sun never rises
        return (None, None);
    }

    let ha = cos_ha.acos().to_degrees();

    // Solar noon in minutes UTC
    let solar_noon = 720.0 - 4.0 * lon - eqtime;

    let sunrise_min = solar_noon - 4.0 * ha;
    let sunset_min = solar_noon + 4.0 * ha;

    (Some(sunrise_min / 60.0), Some(sunset_min / 60.0))
}

/// Compute day-of-year (1-based) for a Gregorian date.
fn day_of_year(year: u32, month: u32, day: u32) -> u32 {
    let leap = u32::from(is_leap_year(year));
    // Days in each month (Jan=1..Dec=12)
    const DAYS_BEFORE: [u32; 13] = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let extra_leap = if month > 2 { leap } else { 0 };
    DAYS_BEFORE[month as usize] + day + extra_leap
}

fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn london() -> Location {
        Location::new_unchecked(51.5074, -0.1278)
    }

    #[test]
    fn location_validation() {
        assert!(Location::new(51.5, -0.1).is_ok());
        assert!(Location::new(91.0, 0.0).is_err());
        assert!(Location::new(0.0, 181.0).is_err());
    }

    #[test]
    fn day_of_year_jan1() {
        assert_eq!(day_of_year(2026, 1, 1), 1);
    }

    #[test]
    fn day_of_year_dec31() {
        // 2026 is not a leap year
        assert_eq!(day_of_year(2026, 12, 31), 365);
    }

    #[test]
    fn day_of_year_mar1_leap() {
        // 2024 is a leap year; March 1 = day 61
        assert_eq!(day_of_year(2024, 3, 1), 61);
    }

    #[test]
    fn sunrise_is_before_sunset_london() {
        let solar = SolarDay::for_date(london(), 2026, 3, 14).unwrap();
        let rise = solar.sunrise_utc_min.expect("should have sunrise");
        let set = solar.sunset_utc_min.expect("should have sunset");
        assert!(rise < set, "sunrise={rise} sunset={set}");
    }

    #[test]
    fn sunrise_roughly_correct_london_summer() {
        // London, 21 June 2026 — sunrise around 04:43 UTC (≈ 283 min)
        let solar = SolarDay::for_date(london(), 2026, 6, 21).unwrap();
        let rise = solar.sunrise_utc_min.unwrap();
        assert!((200..400).contains(&rise), "unexpected sunrise: {rise}");
    }

    #[test]
    fn civil_dawn_before_sunrise() {
        let solar = SolarDay::for_date(london(), 2026, 3, 14).unwrap();
        let dawn = solar.civil_dawn_utc_min.expect("should have civil dawn");
        let rise = solar.sunrise_utc_min.expect("should have sunrise");
        assert!(dawn < rise, "dawn={dawn} rise={rise}");
    }
}
