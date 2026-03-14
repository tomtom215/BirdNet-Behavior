//! Weekly detection summary report.
//!
//! Generates a human-readable report of bird detections for the past week,
//! suitable for sending via notification (Apprise, email, etc.).

use std::fmt;

/// A single species entry in the weekly report.
#[derive(Debug, Clone)]
pub struct SpeciesCount {
    /// Common name of the species.
    pub common_name: String,
    /// Scientific name of the species.
    pub scientific_name: String,
    /// Number of detections this week.
    pub count: u64,
}

/// Weekly detection summary report.
#[derive(Debug, Clone)]
pub struct WeeklyReport {
    /// ISO week number.
    pub week_number: u32,
    /// Year.
    pub year: u32,
    /// Top species by detection count (up to 10).
    pub top_species: Vec<SpeciesCount>,
    /// Total detections this week.
    pub total_detections: u64,
    /// Total detections last week (for trend comparison).
    pub total_detections_last_week: u64,
    /// Unique species this week.
    pub unique_species: u32,
    /// Unique species last week (for trend comparison).
    pub unique_species_last_week: u32,
    /// Species detected this week for the first time ever.
    pub first_time_species: Vec<String>,
}

impl WeeklyReport {
    /// Percentage change in detections vs last week.
    ///
    /// Returns `None` if last week had zero detections.
    #[must_use]
    pub fn detection_trend_pct(&self) -> Option<f64> {
        if self.total_detections_last_week == 0 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        let current = self.total_detections as f64;
        #[allow(clippy::cast_precision_loss)]
        let last = self.total_detections_last_week as f64;
        Some(((current - last) / last) * 100.0)
    }

    /// Format the report as a plain-text notification body.
    #[must_use]
    pub fn format_text(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str(&format!(
            "Weekly Bird Report - Week {} ({})\n",
            self.week_number, self.year
        ));
        out.push_str(&"=".repeat(40));
        out.push('\n');

        // Summary.
        out.push_str(&format!("\nTotal detections: {}\n", self.total_detections));
        if let Some(trend) = self.detection_trend_pct() {
            let arrow = if trend > 0.0 { "+" } else { "" };
            out.push_str(&format!("  vs last week: {arrow}{trend:.1}%\n"));
        }

        out.push_str(&format!("Unique species: {}\n", self.unique_species));
        if self.unique_species_last_week > 0 {
            let diff = i64::from(self.unique_species) - i64::from(self.unique_species_last_week);
            let arrow = if diff > 0 { "+" } else { "" };
            out.push_str(&format!("  vs last week: {arrow}{diff}\n"));
        }

        // First-time species.
        if !self.first_time_species.is_empty() {
            out.push_str("\nNew species this week:\n");
            for sp in &self.first_time_species {
                out.push_str(&format!("  * {sp}\n"));
            }
        }

        // Top species.
        if !self.top_species.is_empty() {
            out.push_str("\nTop species:\n");
            for (i, sc) in self.top_species.iter().enumerate() {
                out.push_str(&format!(
                    "  {}. {} ({}) - {} detections\n",
                    i + 1,
                    sc.common_name,
                    sc.scientific_name,
                    sc.count
                ));
            }
        }

        out
    }
}

impl fmt::Display for WeeklyReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_text())
    }
}

/// Trait for generating weekly reports from a data source.
///
/// Typically implemented by a wrapper around a database connection.
pub trait WeeklyReportSource: Send + Sync {
    /// Generate a weekly report for the current week.
    ///
    /// # Errors
    ///
    /// Returns a string error if the data source is unavailable.
    fn generate_weekly_report(&self) -> Result<WeeklyReport, String>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> WeeklyReport {
        WeeklyReport {
            week_number: 11,
            year: 2026,
            top_species: vec![
                SpeciesCount {
                    common_name: "European Robin".to_string(),
                    scientific_name: "Erithacus rubecula".to_string(),
                    count: 142,
                },
                SpeciesCount {
                    common_name: "Great Tit".to_string(),
                    scientific_name: "Parus major".to_string(),
                    count: 98,
                },
                SpeciesCount {
                    common_name: "Eurasian Blackbird".to_string(),
                    scientific_name: "Turdus merula".to_string(),
                    count: 76,
                },
            ],
            total_detections: 1234,
            total_detections_last_week: 1100,
            unique_species: 42,
            unique_species_last_week: 38,
            first_time_species: vec!["Bohemian Waxwing".to_string()],
        }
    }

    #[test]
    fn detection_trend_positive() {
        let report = sample_report();
        let trend = report.detection_trend_pct().unwrap();
        assert!(trend > 0.0, "expected positive trend, got {trend}");
        // (1234-1100)/1100 * 100 = 12.18%
        assert!((trend - 12.18).abs() < 0.1);
    }

    #[test]
    fn detection_trend_none_when_no_last_week() {
        let mut report = sample_report();
        report.total_detections_last_week = 0;
        assert!(report.detection_trend_pct().is_none());
    }

    #[test]
    fn format_text_contains_key_info() {
        let report = sample_report();
        let text = report.format_text();
        assert!(text.contains("Week 11"));
        assert!(text.contains("Total detections: 1234"));
        assert!(text.contains("Unique species: 42"));
        assert!(text.contains("European Robin"));
        assert!(text.contains("142 detections"));
        assert!(text.contains("Bohemian Waxwing"));
        assert!(text.contains("New species this week"));
    }

    #[test]
    fn format_text_no_first_time() {
        let mut report = sample_report();
        report.first_time_species.clear();
        let text = report.format_text();
        assert!(!text.contains("New species this week"));
    }

    #[test]
    fn display_matches_format_text() {
        let report = sample_report();
        assert_eq!(format!("{report}"), report.format_text());
    }
}
