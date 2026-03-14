//! Peak activity window query methods.

use crate::error::TimeSeriesError;
use crate::queries::peak::{PeakWindows, SpeciesPeak};
use crate::types::{params::PeakParams, results::PeakWindowRow};

impl<'conn> super::TimeSeriesDb<'conn> {
    /// Busiest N-minute detection windows over a lookback period.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn peak_windows(&self, params: &PeakParams) -> Result<Vec<PeakWindowRow>, TimeSeriesError> {
        let days = params.lookback_days;
        let q = PeakWindows {
            window_minutes: params.window_minutes,
            hop_minutes: params.hop_minutes,
            range_start: format!("CURRENT_TIMESTAMP - INTERVAL {days} DAYS"),
            range_end: "CURRENT_TIMESTAMP".into(),
            limit: params.limit,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(PeakWindowRow {
                window_start: row.get(0)?,
                window_end: row.get(1)?,
                detection_count: row.get(2)?,
                species_count: row.get(3)?,
                peak_confidence: row.get(4)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Peak hours of day for a specific species.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn species_peak_hours(
        &self,
        species: &str,
        _lookback_days: u32,
    ) -> Result<Vec<PeakWindowRow>, TimeSeriesError> {
        let q = SpeciesPeak::hourly(species.to_string());
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let hour: i64 = row.get(0)?;
            Ok(PeakWindowRow {
                window_start: format!("{hour:02}:00"),
                window_end: format!("{}:00", (hour + 1) % 24),
                detection_count: row.get(1)?,
                species_count: 1,
                peak_confidence: row.get(3)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }
}
