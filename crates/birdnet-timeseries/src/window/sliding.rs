//! Sliding window specification using DuckDB RANGE framing.
//!
//! Sliding windows are dynamically generated from the data itself using
//! the SQL `OVER (ORDER BY … RANGE BETWEEN … PRECEDING AND … FOLLOWING)`
//! clause. Unlike hopping windows the set of windows changes whenever
//! new rows are added.
//!
//! Primary use cases:
//! - Smoothing noisy daily detection counts with a 7-day centred average
//! - Running totals over a trailing window (e.g. species seen in last N days)

use super::WindowSpec;

/// Direction of the sliding window relative to the current row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlidingDirection {
    /// Centre the window around the current row (preceding + following).
    Centred,
    /// Trailing: only look at rows up to and including current.
    Trailing,
}

/// Specification for a sliding window query on daily detection aggregates.
///
/// The query first aggregates detections to daily totals, then applies
/// a RANGE-based window function to produce moving averages / running stats.
#[derive(Debug, Clone)]
pub struct SlidingSpec {
    /// Half-width of the window in days (total width = 2 × half + 1 for centred).
    pub half_window_days: u32,
    /// Whether the window is centred or trailing.
    pub direction: SlidingDirection,
    /// Start of the date range to query (inclusive).
    pub from_date: Option<String>,
    /// End of the date range to query (inclusive).
    pub to_date: Option<String>,
    /// Optional species filter.
    pub species: Option<String>,
    /// Maximum rows returned.
    pub limit: u32,
}

impl Default for SlidingSpec {
    fn default() -> Self {
        Self {
            half_window_days: 3,
            direction: SlidingDirection::Centred,
            from_date: None,
            to_date: None,
            species: None,
            limit: 365,
        }
    }
}

impl SlidingSpec {
    /// 7-day centred moving average over the last 90 days.
    pub fn seven_day_avg() -> Self {
        Self {
            half_window_days: 3,
            direction: SlidingDirection::Centred,
            from_date: Some("CURRENT_DATE - INTERVAL 90 DAYS".into()),
            ..Default::default()
        }
    }

    /// 30-day trailing window to show cumulative species richness trend.
    pub fn trailing_30_days() -> Self {
        Self {
            half_window_days: 30,
            direction: SlidingDirection::Trailing,
            from_date: Some("CURRENT_DATE - INTERVAL 365 DAYS".into()),
            ..Default::default()
        }
    }
}

impl WindowSpec for SlidingSpec {
    fn build_sql(&self) -> String {
        let half = self.half_window_days;
        let frame_sql = match self.direction {
            SlidingDirection::Centred => format!(
                "RANGE BETWEEN INTERVAL {half} DAYS PRECEDING AND INTERVAL {half} DAYS FOLLOWING"
            ),
            SlidingDirection::Trailing => format!(
                "RANGE BETWEEN INTERVAL {half} DAYS PRECEDING AND CURRENT ROW"
            ),
        };

        let mut where_clauses = Vec::new();
        if let Some(from) = &self.from_date {
            where_clauses.push(format!("detection_date >= {from}"));
        }
        if let Some(to) = &self.to_date {
            where_clauses.push(format!("detection_date <= {to}"));
        }
        if let Some(sp) = &self.species {
            let escaped = sp.replace('\'', "''");
            where_clauses.push(format!("Com_Name = '{escaped}'"));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let limit = self.limit;
        format!(
            "WITH daily AS (
    SELECT
        detection_date,
        COUNT(*)                 AS daily_detections,
        COUNT(DISTINCT Com_Name) AS species_richness,
        AVG(Confidence)          AS avg_confidence
    FROM detections_ts
    {where_sql}
    GROUP BY detection_date
)
SELECT
    detection_date,
    daily_detections,
    species_richness,
    avg_confidence,
    AVG(daily_detections) OVER (
        ORDER BY detection_date
        {frame_sql}
    ) AS moving_avg_detections,
    AVG(species_richness) OVER (
        ORDER BY detection_date
        {frame_sql}
    ) AS moving_avg_species
FROM daily
ORDER BY detection_date
LIMIT {limit}"
        )
    }

    fn description(&self) -> &'static str {
        "Sliding window: RANGE-framed moving statistics"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centred_sql_has_following() {
        let spec = SlidingSpec::seven_day_avg();
        let sql = spec.build_sql();
        assert!(sql.contains("FOLLOWING"));
        assert!(sql.contains("PRECEDING"));
    }

    #[test]
    fn trailing_sql_has_current_row() {
        let spec = SlidingSpec::trailing_30_days();
        let sql = spec.build_sql();
        assert!(sql.contains("CURRENT ROW"));
    }

    #[test]
    fn species_filter_applied_to_daily_cte() {
        let spec = SlidingSpec {
            species: Some("Great Tit".into()),
            ..Default::default()
        };
        let sql = spec.build_sql();
        assert!(sql.contains("Great Tit"));
        // Filter must appear inside the CTE's WHERE, before the GROUP BY
        let cte_end = sql.find("GROUP BY").unwrap_or(0);
        let filter_pos = sql.find("Great Tit").unwrap_or(usize::MAX);
        assert!(filter_pos < cte_end);
    }
}
