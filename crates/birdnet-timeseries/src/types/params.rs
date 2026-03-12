//! Parameter types for time-series analytics endpoint requests.
//!
//! Each `*Params` struct maps directly to the query parameters accepted
//! by the corresponding REST API endpoint.

/// Parameters for hourly activity queries.
#[derive(Debug, Clone)]
pub struct HourlyParams {
    /// Number of days to look back (default: 7).
    pub lookback_days: u32,
    /// Optional species filter (common name).
    pub species: Option<String>,
}

impl Default for HourlyParams {
    fn default() -> Self {
        Self {
            lookback_days: 7,
            species: None,
        }
    }
}

/// Parameters for daily activity / trend queries.
#[derive(Debug, Clone)]
pub struct DailyParams {
    /// Number of days to look back (default: 30).
    pub lookback_days: u32,
    /// Optional species filter.
    pub species: Option<String>,
}

impl Default for DailyParams {
    fn default() -> Self {
        Self {
            lookback_days: 30,
            species: None,
        }
    }
}

/// Parameters for weekly aggregation queries.
#[derive(Debug, Clone)]
pub struct WeeklyParams {
    /// Number of weeks to look back (default: 52).
    pub lookback_weeks: u32,
}

impl Default for WeeklyParams {
    fn default() -> Self {
        Self { lookback_weeks: 52 }
    }
}

/// Parameters for moving-average trend queries.
#[derive(Debug, Clone)]
pub struct TrendParams {
    /// Moving-average window width in days (default: 7).
    pub window_days: u32,
    /// Start of the date range (ISO-8601), or look-back expression.
    pub from_date: Option<String>,
    /// End of the date range (ISO-8601).
    pub to_date: Option<String>,
    /// Optional species filter.
    pub species: Option<String>,
}

impl Default for TrendParams {
    fn default() -> Self {
        Self {
            window_days: 7,
            from_date: Some("CURRENT_DATE - INTERVAL 90 DAYS".into()),
            to_date: None,
            species: None,
        }
    }
}

/// Parameters for peak window queries.
#[derive(Debug, Clone)]
pub struct PeakParams {
    /// Window width in minutes (default: 15).
    pub window_minutes: u32,
    /// Hop size in minutes (default: 5).
    pub hop_minutes: u32,
    /// Number of days to look back (default: 1).
    pub lookback_days: u32,
    /// Maximum windows to return (default: 10).
    pub limit: u32,
}

impl Default for PeakParams {
    fn default() -> Self {
        Self {
            window_minutes: 15,
            hop_minutes: 5,
            lookback_days: 1,
            limit: 10,
        }
    }
}

/// Parameters for session (gap-based grouping) queries.
#[derive(Debug, Clone)]
pub struct SessionParams {
    /// Gap threshold in minutes (default: 30).
    pub gap_minutes: u32,
    /// Restrict to a single date (ISO-8601), or `None` for all dates.
    pub date_filter: Option<String>,
    /// Number of days to look back when no specific date is given (default: 7).
    pub lookback_days: u32,
    /// Maximum sessions to return (default: 100).
    pub limit: u32,
}

impl Default for SessionParams {
    fn default() -> Self {
        Self {
            gap_minutes: 30,
            date_filter: None,
            lookback_days: 7,
            limit: 100,
        }
    }
}

/// Parameters for species diversity queries.
#[derive(Debug, Clone)]
pub struct DiversityParams {
    /// Number of days to look back (default: 90).
    pub lookback_days: u32,
    /// Whether to compute Shannon diversity (default: true).
    pub include_shannon: bool,
}

impl Default for DiversityParams {
    fn default() -> Self {
        Self {
            lookback_days: 90,
            include_shannon: true,
        }
    }
}

/// Parameters for anomaly detection queries.
#[derive(Debug, Clone)]
pub struct AnomalyParams {
    /// Z-score threshold (default: 2.0).
    pub z_threshold: f64,
    /// Rolling window in days for statistics (default: 30).
    pub window_days: u32,
    /// Number of days to analyse (default: 180).
    pub lookback_days: u32,
}

impl Default for AnomalyParams {
    fn default() -> Self {
        Self {
            z_threshold: 2.0,
            window_days: 30,
            lookback_days: 180,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hourly_params() {
        let p = HourlyParams::default();
        assert_eq!(p.lookback_days, 7);
        assert!(p.species.is_none());
    }

    #[test]
    fn default_anomaly_params() {
        let p = AnomalyParams::default();
        assert!((p.z_threshold - 2.0).abs() < f64::EPSILON);
        assert_eq!(p.window_days, 30);
    }
}
