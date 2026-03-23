//! Tumbling (non-overlapping fixed-size) window specification.
//!
//! Tumbling windows divide the time axis into equal, non-overlapping
//! intervals. Each detection falls in exactly one bucket.
//!
//! Uses `DuckDB`'s `time_bucket` function for sub-day granularities and
//! `date_trunc` for day/week/month. Gaps in the series are preserved
//! (empty buckets are not synthesised).

use super::{Granularity, WindowSpec};

/// Specification for a tumbling window query on `detections_ts`.
///
/// # Example SQL (hourly, last 7 days, all species)
/// ```sql
/// SELECT
///     time_bucket(INTERVAL 1 HOUR, detection_timestamp) AS window_start,
///     window_start + INTERVAL 1 HOUR                    AS window_end,
///     COUNT(*)                                           AS detection_count,
///     COUNT(DISTINCT Com_Name)                           AS species_count,
///     AVG(Confidence)                                    AS avg_confidence
/// FROM detections_ts
/// WHERE detection_date >= CURRENT_DATE - INTERVAL 7 DAYS
/// GROUP BY ALL
/// ORDER BY window_start
/// LIMIT 500
/// ```
#[derive(Debug, Clone)]
pub struct TumblingSpec {
    /// Bucket granularity.
    pub granularity: Granularity,
    /// ISO-8601 date string for the start of the range (inclusive).
    pub from_date: Option<String>,
    /// ISO-8601 date string for the end of the range (inclusive).
    pub to_date: Option<String>,
    /// Filter to a single species common name, or `None` for all.
    pub species: Option<String>,
    /// Maximum rows returned.
    pub limit: u32,
}

impl Default for TumblingSpec {
    fn default() -> Self {
        Self {
            granularity: Granularity::Hour,
            from_date: None,
            to_date: None,
            species: None,
            limit: 500,
        }
    }
}

impl TumblingSpec {
    /// Convenience constructor for the last N days at a given granularity.
    pub fn last_n_days(n: u32, granularity: Granularity) -> Self {
        Self {
            granularity,
            from_date: Some(format!("CURRENT_DATE - INTERVAL {n} DAYS")),
            to_date: None,
            species: None,
            limit: 500,
        }
    }
}

impl WindowSpec for TumblingSpec {
    fn build_sql(&self) -> String {
        let interval = self.granularity.interval_sql();

        // Bucket expression varies by granularity
        let bucket_expr = match self.granularity {
            Granularity::QuarterHour | Granularity::Hour => {
                format!("time_bucket({interval}, detection_timestamp)")
            }
            Granularity::Day => "detection_date".to_string(),
            Granularity::Week => "date_trunc('week', detection_date)".to_string(),
            Granularity::Month => "date_trunc('month', detection_date)".to_string(),
        };

        let mut where_clauses = Vec::new();
        if let Some(from) = &self.from_date {
            // Raw SQL expression (e.g. "CURRENT_DATE - INTERVAL 7 DAYS") or literal date
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
            "SELECT
    {bucket_expr} AS window_start,
    {bucket_expr} + {interval} AS window_end,
    COUNT(*) AS detection_count,
    COUNT(DISTINCT Com_Name) AS species_count,
    AVG(Confidence) AS avg_confidence,
    MAX(Confidence) AS max_confidence
FROM detections_ts
{where_sql}
GROUP BY ALL
ORDER BY window_start
LIMIT {limit}"
        )
    }

    fn description(&self) -> &'static str {
        "Tumbling window: fixed non-overlapping intervals"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hourly_sql_contains_time_bucket() {
        let spec = TumblingSpec {
            granularity: Granularity::Hour,
            ..Default::default()
        };
        let sql = spec.build_sql();
        assert!(sql.contains("time_bucket"));
        assert!(sql.contains("INTERVAL 1 HOUR"));
    }

    #[test]
    fn daily_sql_uses_detection_date() {
        let spec = TumblingSpec {
            granularity: Granularity::Day,
            ..Default::default()
        };
        let sql = spec.build_sql();
        assert!(sql.contains("detection_date"));
    }

    #[test]
    fn species_filter_escapes_quotes() {
        let spec = TumblingSpec {
            species: Some("O'Brien's Warbler".into()),
            ..Default::default()
        };
        let sql = spec.build_sql();
        assert!(sql.contains("O''Brien''s Warbler"));
    }

    #[test]
    fn date_range_filter() {
        let spec = TumblingSpec {
            from_date: Some("2026-01-01".into()),
            to_date: Some("2026-03-01".into()),
            ..Default::default()
        };
        let sql = spec.build_sql();
        assert!(sql.contains("2026-01-01"));
        assert!(sql.contains("2026-03-01"));
    }
}
