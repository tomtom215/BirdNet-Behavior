//! Phenology analytics types.
//!
//! Types representing the output of phenological queries applied to
//! multi-year bird detection data.  Phenology is the study of cyclic
//! and seasonal natural phenomena — in ornithology, this covers
//! migration timing, breeding windows, and inter-annual abundance trends.

use serde::Serialize;

/// Phenological timing record for a single species in a single year.
///
/// Derived from first-detection and last-detection dates, this record
/// allows tracking of arrival and departure across multiple seasons.
#[derive(Debug, Clone, Serialize)]
pub struct PhenologyRecord {
    /// Species common name.
    pub species: String,
    /// Calendar year (e.g., 2026).
    pub year: u32,
    /// Date of first detection in this year (ISO 8601, `YYYY-MM-DD`).
    pub first_detection: String,
    /// Date of last detection in this year (ISO 8601, `YYYY-MM-DD`).
    pub last_detection: String,
    /// Total number of detections in this year.
    pub detection_count: u32,
    /// Day-of-year of first detection (1–366).
    pub first_doy: u32,
    /// Day-of-year of last detection (1–366).
    pub last_doy: u32,
    /// Approximate presence duration in days.
    pub presence_days: u32,
}

/// Estimated migration / seasonal window for a species.
///
/// Derived from the 10th and 90th percentiles of first-detection
/// day-of-year across multiple years, providing a robust arrival and
/// departure window that is insensitive to outlier years.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationWindow {
    /// Species common name.
    pub species: String,
    /// Number of years with detections.
    pub years_observed: u32,
    /// Earliest typical arrival (10th percentile of `first_doy`).
    pub arrival_early_doy: f64,
    /// Median arrival day-of-year (50th percentile of `first_doy`).
    pub arrival_median_doy: f64,
    /// Latest typical arrival (90th percentile of `first_doy`).
    pub arrival_late_doy: f64,
    /// Earliest typical departure (10th percentile of `last_doy`).
    pub departure_early_doy: f64,
    /// Median departure day-of-year (50th percentile of `last_doy`).
    pub departure_median_doy: f64,
    /// Latest typical departure (90th percentile of `last_doy`).
    pub departure_late_doy: f64,
}

/// Weekly relative abundance index.
///
/// Expresses how common a species is in a given ISO week relative to
/// its peak week (which has an index of 1.0).  Values approaching 0
/// indicate near-absence; 1.0 is peak abundance.
#[derive(Debug, Clone, Serialize)]
pub struct WeeklyAbundance {
    /// Species common name.
    pub species: String,
    /// ISO calendar year.
    pub year: u32,
    /// ISO week number (1–53).
    pub iso_week: u32,
    /// Raw detection count for this week.
    pub detection_count: u32,
    /// Relative abundance index \[0.0, 1.0\].
    ///
    /// `detection_count / peak_week_count` across all weeks for this
    /// species in this year.
    pub relative_abundance: f64,
}

/// Parameters for phenology queries.
#[derive(Debug, Clone)]
pub struct PhenologyParams {
    /// Filter to a single species (None = all species).
    pub species: Option<String>,
    /// Include only years >= this value.
    pub year_start: Option<u32>,
    /// Include only years <= this value.
    pub year_end: Option<u32>,
    /// Minimum detections per species per year to include in results.
    pub min_detections: u32,
    /// Maximum number of records to return.
    pub limit: u32,
}

impl Default for PhenologyParams {
    fn default() -> Self {
        Self {
            species: None,
            year_start: None,
            year_end: None,
            min_detections: 3,
            limit: 500,
        }
    }
}

/// Parameters for weekly abundance queries.
#[derive(Debug, Clone)]
pub struct AbundanceParams {
    /// Filter to a single species (None = all species).
    pub species: Option<String>,
    /// Calendar year to analyse.
    pub year: u32,
    /// Minimum weekly count to include in results.
    pub min_weekly_count: u32,
}

impl AbundanceParams {
    /// Construct parameters for a specific year.
    #[must_use]
    pub const fn for_year(year: u32) -> Self {
        Self {
            year,
            species: None,
            min_weekly_count: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_phenology_params() {
        let p = PhenologyParams::default();
        assert!(p.species.is_none());
        assert_eq!(p.min_detections, 3);
        assert_eq!(p.limit, 500);
    }

    #[test]
    fn abundance_params_for_year() {
        let p = AbundanceParams::for_year(2026);
        assert_eq!(p.year, 2026);
        assert!(p.species.is_none());
    }

    #[test]
    fn phenology_record_serialises() {
        let rec = PhenologyRecord {
            species: "Eurasian Blackbird".into(),
            year: 2026,
            first_detection: "2026-03-01".into(),
            last_detection: "2026-11-15".into(),
            detection_count: 142,
            first_doy: 60,
            last_doy: 319,
            presence_days: 259,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("Eurasian Blackbird"));
        assert!(json.contains("2026"));
    }

    #[test]
    fn migration_window_serialises() {
        let w = MigrationWindow {
            species: "Common Swift".into(),
            years_observed: 5,
            arrival_early_doy: 120.0,
            arrival_median_doy: 130.0,
            arrival_late_doy: 145.0,
            departure_early_doy: 210.0,
            departure_median_doy: 220.0,
            departure_late_doy: 235.0,
        };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("Common Swift"));
        assert!(json.contains("130.0"));
    }

    #[test]
    fn weekly_abundance_relative_bounds() {
        let ab = WeeklyAbundance {
            species: "House Sparrow".into(),
            year: 2026,
            iso_week: 12,
            detection_count: 50,
            relative_abundance: 0.75,
        };
        assert!((0.0..=1.0).contains(&ab.relative_abundance));
    }
}
