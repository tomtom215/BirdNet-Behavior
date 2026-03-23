//! Hopping (overlapping fixed-size) window specification.
//!
//! A hopping window is defined by:
//! - **window size**: how much time each window covers
//! - **hop size**: how often a new window starts (hop ≤ window → overlap)
//!
//! Implemented by generating all window boundaries with `DuckDB`'s `range()`
//! table function and joining the detection data into each interval.
//! Empty windows (zero detections) are included via `LEFT JOIN`.
//!
//! Primary use case: finding the busiest N-minute period across any day.

use super::WindowSpec;

/// Size unit for hopping windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopUnit {
    /// Minutes.
    Minutes(u32),
    /// Hours.
    Hours(u32),
}

impl HopUnit {
    fn interval_sql(self) -> String {
        match self {
            Self::Minutes(m) => format!("INTERVAL {m} MINUTE"),
            Self::Hours(h) => format!("INTERVAL {h} HOUR"),
        }
    }
}

/// Specification for a hopping window query on `detections_ts`.
///
/// # Example SQL (15-min window, 5-min hop, last 24 h, top 10 results)
/// ```sql
/// WITH time_range AS (
///     SELECT range AS window_start,
///            window_start + INTERVAL 15 MINUTE AS window_end
///     FROM range(CURRENT_TIMESTAMP - INTERVAL 1 DAY,
///                CURRENT_TIMESTAMP,
///                INTERVAL 5 MINUTE)
/// )
/// SELECT tr.window_start, tr.window_end,
///        COUNT(d.Com_Name) AS detection_count,
///        COUNT(DISTINCT d.Com_Name) AS species_count
/// FROM time_range tr
/// LEFT JOIN detections_ts d
///        ON d.detection_timestamp >= tr.window_start
///       AND d.detection_timestamp <  tr.window_end
/// GROUP BY tr.window_start, tr.window_end
/// ORDER BY detection_count DESC
/// LIMIT 10
/// ```
#[derive(Debug, Clone)]
pub struct HoppingSpec {
    /// How large each window is.
    pub window_size: HopUnit,
    /// How much the window advances each step.
    pub hop_size: HopUnit,
    /// Start of the candidate range (`DuckDB` expression string).
    pub range_start: String,
    /// End of the candidate range (`DuckDB` expression string).
    pub range_end: String,
    /// Optional species filter.
    pub species: Option<String>,
    /// Sort order: `true` = most detections first.
    pub order_by_count_desc: bool,
    /// Maximum number of windows returned.
    pub limit: u32,
}

impl Default for HoppingSpec {
    fn default() -> Self {
        Self {
            window_size: HopUnit::Minutes(15),
            hop_size: HopUnit::Minutes(5),
            range_start: "CURRENT_TIMESTAMP - INTERVAL 1 DAY".into(),
            range_end: "CURRENT_TIMESTAMP".into(),
            species: None,
            order_by_count_desc: true,
            limit: 10,
        }
    }
}

impl HoppingSpec {
    /// Build a spec targeting the last `days` calendar days.
    pub fn last_n_days(days: u32, window_minutes: u32, hop_minutes: u32) -> Self {
        Self {
            window_size: HopUnit::Minutes(window_minutes),
            hop_size: HopUnit::Minutes(hop_minutes),
            range_start: format!("CURRENT_TIMESTAMP - INTERVAL {days} DAYS"),
            range_end: "CURRENT_TIMESTAMP".into(),
            ..Default::default()
        }
    }
}

impl WindowSpec for HoppingSpec {
    fn build_sql(&self) -> String {
        let window_interval = self.window_size.interval_sql();
        let hop_interval = self.hop_size.interval_sql();
        let range_start = &self.range_start;
        let range_end = &self.range_end;
        let limit = self.limit;
        let order_dir = if self.order_by_count_desc {
            "DESC"
        } else {
            "ASC"
        };

        let species_filter = self
            .species
            .as_deref()
            .map(|s| {
                let escaped = s.replace('\'', "''");
                format!("AND d.Com_Name = '{escaped}'")
            })
            .unwrap_or_default();

        format!(
            "WITH time_range AS (
    SELECT
        range                             AS window_start,
        range + {window_interval}         AS window_end
    FROM range(
        ({range_start})::TIMESTAMP,
        ({range_end})::TIMESTAMP,
        {hop_interval}
    )
)
SELECT
    tr.window_start,
    tr.window_end,
    COUNT(d.Com_Name)          AS detection_count,
    COUNT(DISTINCT d.Com_Name) AS species_count,
    AVG(d.Confidence)          AS avg_confidence
FROM time_range tr
LEFT JOIN detections_ts d
    ON d.detection_timestamp >= tr.window_start
   AND d.detection_timestamp <  tr.window_end
   {species_filter}
GROUP BY tr.window_start, tr.window_end
ORDER BY detection_count {order_dir}
LIMIT {limit}"
        )
    }

    fn description(&self) -> &'static str {
        "Hopping window: overlapping fixed-size intervals"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sql_has_range_function() {
        let spec = HoppingSpec::default();
        let sql = spec.build_sql();
        assert!(sql.contains("FROM range("));
        assert!(sql.contains("LEFT JOIN detections_ts"));
    }

    #[test]
    fn species_filter_in_join() {
        let spec = HoppingSpec {
            species: Some("European Robin".into()),
            ..Default::default()
        };
        let sql = spec.build_sql();
        assert!(sql.contains("European Robin"));
    }
}
