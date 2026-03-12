//! Species diversity queries: richness, Shannon entropy, and evenness over time.
//!
//! "Species richness" counts how many distinct species were observed.
//! "Shannon diversity" (H′) measures both richness and evenness:
//!   H′ = -Σ(pᵢ · ln pᵢ)  where pᵢ = detections of species i / total detections
//!
//! Higher H′ → more balanced community. Useful for comparing seasonal windows.

use super::QueryPlan;

/// Daily species richness (count of distinct species per day).
#[derive(Debug, Clone)]
pub struct DailyRichness {
    /// Look back this many days (default: 90).
    pub lookback_days: u32,
}

impl Default for DailyRichness {
    fn default() -> Self {
        Self { lookback_days: 90 }
    }
}

impl QueryPlan for DailyRichness {
    fn sql(&self) -> String {
        let days = self.lookback_days;
        format!(
            "SELECT
    detection_date              AS date,
    COUNT(DISTINCT Com_Name)    AS species_richness,
    COUNT(*)                    AS total_detections
FROM detections_ts
WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
GROUP BY detection_date
ORDER BY detection_date"
        )
    }
}

/// Shannon diversity index per day.
///
/// Uses DuckDB's `entropy` aggregate (available in DuckDB 0.8+) which
/// computes the Shannon entropy of a distribution given a column of values.
/// Falls back to a manual formula if needed.
#[derive(Debug, Clone)]
pub struct DailyShannon {
    /// Look back this many days (default: 90).
    pub lookback_days: u32,
}

impl Default for DailyShannon {
    fn default() -> Self {
        Self { lookback_days: 90 }
    }
}

impl QueryPlan for DailyShannon {
    fn sql(&self) -> String {
        let days = self.lookback_days;
        // Manual Shannon: -SUM(p * ln(p)) where p = count/total per species-day
        format!(
            "WITH species_daily AS (
    SELECT
        detection_date,
        Com_Name,
        COUNT(*) AS n
    FROM detections_ts
    WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
    GROUP BY detection_date, Com_Name
),
totals AS (
    SELECT
        detection_date,
        SUM(n) AS total
    FROM species_daily
    GROUP BY detection_date
)
SELECT
    sd.detection_date                     AS date,
    COUNT(DISTINCT sd.Com_Name)           AS species_richness,
    t.total                               AS total_detections,
    -SUM((sd.n * 1.0 / t.total) * ln(sd.n * 1.0 / t.total)) AS shannon_h,
    -SUM((sd.n * 1.0 / t.total) * ln(sd.n * 1.0 / t.total))
        / NULLIF(ln(COUNT(DISTINCT sd.Com_Name)), 0) AS pielou_evenness
FROM species_daily sd
JOIN totals t ON sd.detection_date = t.detection_date
GROUP BY sd.detection_date, t.total
ORDER BY sd.detection_date"
        )
    }
}

/// Species accumulation curve: cumulative new species seen over time.
///
/// Shows how species discoveries accumulate — useful for assessing how long
/// it takes to characterise a site's avifauna.
#[derive(Debug, Clone)]
pub struct AccumulationCurve {
    /// Start date (ISO-8601), or earliest available if `None`.
    pub from_date: Option<String>,
    /// End date (ISO-8601), or today if `None`.
    pub to_date: Option<String>,
}

impl Default for AccumulationCurve {
    fn default() -> Self {
        Self {
            from_date: None,
            to_date: None,
        }
    }
}

impl QueryPlan for AccumulationCurve {
    fn sql(&self) -> String {
        let mut where_parts = Vec::new();
        if let Some(from) = &self.from_date {
            let esc = from.replace('\'', "''");
            where_parts.push(format!("detection_date >= '{esc}'"));
        }
        if let Some(to) = &self.to_date {
            let esc = to.replace('\'', "''");
            where_parts.push(format!("detection_date <= '{esc}'"));
        }
        let where_sql = if where_parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_parts.join(" AND "))
        };
        format!(
            "WITH first_seen AS (
    SELECT
        Com_Name,
        MIN(detection_date) AS first_date
    FROM detections_ts
    {where_sql}
    GROUP BY Com_Name
)
SELECT
    first_date            AS date,
    COUNT(*)              AS new_species_today,
    SUM(COUNT(*)) OVER (
        ORDER BY first_date
        ROWS UNBOUNDED PRECEDING
    )                     AS cumulative_species
FROM first_seen
GROUP BY first_date
ORDER BY first_date"
        )
    }
}

/// Top N species by detection count over a date window.
#[derive(Debug, Clone)]
pub struct TopSpeciesByCount {
    /// Look back this many days (default: 30).
    pub lookback_days: u32,
    /// Maximum species to return (default: 20).
    pub limit: u32,
}

impl Default for TopSpeciesByCount {
    fn default() -> Self {
        Self {
            lookback_days: 30,
            limit: 20,
        }
    }
}

impl QueryPlan for TopSpeciesByCount {
    fn sql(&self) -> String {
        let days = self.lookback_days;
        let limit = self.limit;
        format!(
            "SELECT
    Com_Name                 AS species,
    COUNT(*)                 AS detection_count,
    AVG(Confidence)          AS avg_confidence,
    MIN(detection_date)      AS first_seen,
    MAX(detection_date)      AS last_seen,
    COUNT(DISTINCT detection_date) AS active_days
FROM detections_ts
WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
GROUP BY Com_Name
ORDER BY detection_count DESC
LIMIT {limit}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_richness_sql() {
        let q = DailyRichness::default();
        let sql = q.sql();
        assert!(sql.contains("COUNT(DISTINCT Com_Name)"));
        assert!(sql.contains("GROUP BY detection_date"));
    }

    #[test]
    fn shannon_sql_has_ln() {
        let q = DailyShannon::default();
        let sql = q.sql();
        assert!(sql.contains("ln("));
        assert!(sql.contains("pielou_evenness"));
    }

    #[test]
    fn accumulation_sql_has_cumulative_sum() {
        let q = AccumulationCurve::default();
        let sql = q.sql();
        assert!(sql.contains("SUM(COUNT(*)) OVER"));
        assert!(sql.contains("cumulative_species"));
    }
}
