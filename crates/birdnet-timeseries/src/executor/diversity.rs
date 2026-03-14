//! Diversity metrics query methods.

use crate::error::TimeSeriesError;
use crate::queries::diversity::{
    AccumulationCurve, DailyRichness, DailyShannon, TopSpeciesByCount,
};
use crate::types::{
    params::DiversityParams,
    results::{AccumulationRow, DiversityRow, PeakWindowRow},
};

impl<'conn> super::TimeSeriesDb<'conn> {
    /// Daily species richness, optionally including Shannon diversity.
    ///
    /// When `params.include_shannon` is `true`, also returns H and Pielou
    /// evenness alongside the raw richness count.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn daily_richness(
        &self,
        params: &DiversityParams,
    ) -> Result<Vec<DiversityRow>, TimeSeriesError> {
        if params.include_shannon {
            let q = DailyShannon {
                lookback_days: params.lookback_days,
            };
            let sql = q.sql();
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                Ok(DiversityRow {
                    date: row.get(0)?,
                    species_richness: row.get(1)?,
                    total_detections: row.get(2)?,
                    shannon_h: row.get(3)?,
                    pielou_evenness: row.get(4)?,
                })
            })?;
            rows.map(|r| r.map_err(Into::into)).collect()
        } else {
            let q = DailyRichness {
                lookback_days: params.lookback_days,
            };
            let sql = q.sql();
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                Ok(DiversityRow {
                    date: row.get(0)?,
                    species_richness: row.get(1)?,
                    total_detections: row.get(2)?,
                    shannon_h: None,
                    pielou_evenness: None,
                })
            })?;
            rows.map(|r| r.map_err(Into::into)).collect()
        }
    }

    /// Species accumulation curve: new species seen each day.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn accumulation_curve(
        &self,
        from_date: Option<String>,
        to_date: Option<String>,
    ) -> Result<Vec<AccumulationRow>, TimeSeriesError> {
        let q = AccumulationCurve { from_date, to_date };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(AccumulationRow {
                date: row.get(0)?,
                new_species_today: row.get(1)?,
                cumulative_species: row.get(2)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Top species by detection count over a date window.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn top_species(
        &self,
        lookback_days: u32,
        limit: u32,
    ) -> Result<Vec<PeakWindowRow>, TimeSeriesError> {
        let q = TopSpeciesByCount {
            lookback_days,
            limit,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(PeakWindowRow {
                window_start: row.get::<_, String>(3)?,
                window_end: row.get::<_, String>(4)?,
                detection_count: row.get(1)?,
                species_count: 1,
                peak_confidence: row.get(2)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }
}
