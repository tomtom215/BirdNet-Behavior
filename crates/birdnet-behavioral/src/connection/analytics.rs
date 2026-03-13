//! Behavioral analytics query methods on `AnalyticsDb`.
//!
//! Wraps the `duckdb-behavioral` extension functions (sessionize, retention,
//! window_funnel, sequence_next_node) with typed Rust APIs.
//! All methods require `extension_loaded == true`.

use super::{AnalyticsDb, AnalyticsError};
use crate::{queries, types};

impl AnalyticsDb {
    /// Execute a sessionize query.
    ///
    /// Groups continuous activity for each species into discrete sessions
    /// separated by inactivity gaps larger than `params.gap_minutes`.
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::ExtensionLoad` if the behavioral extension
    /// is not loaded, or `AnalyticsError::Database` on query failure.
    pub fn sessionize(
        &self,
        params: &types::SessionizeParams,
    ) -> Result<Vec<types::ActivitySession>, AnalyticsError> {
        self.require_extension()?;
        let sql = queries::sessionize_sql(params);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(types::ActivitySession {
                species: row.get(0)?,
                session_id: row.get(1)?,
                detection_count: row.get(2)?,
                start_time: row.get(3)?,
                end_time: row.get(4)?,
                duration_secs: row.get(5)?,
            })
        })?;
        rows.map(|r| r.map_err(AnalyticsError::from)).collect()
    }

    /// Execute a retention query to track species return patterns.
    ///
    /// Computes daily/weekly return rates for each species and classifies
    /// each as a resident, migrant, or rare visitor.
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::ExtensionLoad` if the extension is not loaded.
    pub fn retention(
        &self,
        params: &types::RetentionParams,
    ) -> Result<Vec<types::SpeciesRetention>, AnalyticsError> {
        self.require_extension()?;
        let sql = queries::retention_sql(params);
        let mut stmt = self.conn.prepare(&sql)?;

        let rows = stmt.query_map([], |row| {
            let species: String = row.get(0)?;
            let rates_raw: Vec<f64> = row.get(1)?;
            Ok((species, rates_raw))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (species, rates_raw) = row?;
            let retention_rates: Vec<types::RetentionRate> = params
                .intervals
                .iter()
                .zip(rates_raw.iter())
                .map(|(&days, &rate)| types::RetentionRate { days, rate })
                .collect();
            let long_term = retention_rates.last().map_or(0.0, |r| r.rate);
            results.push(types::SpeciesRetention {
                species,
                retention_rates,
                classification: types::ResidencyType::from_retention_rate(long_term),
            });
        }
        Ok(results)
    }

    /// Execute a dawn chorus funnel analysis query.
    ///
    /// Finds days where a specified sequence of species was detected,
    /// measuring how many steps of the funnel were completed.
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::ExtensionLoad` if the extension is not loaded.
    pub fn funnel(
        &self,
        params: &types::FunnelParams,
    ) -> Result<Vec<types::ChorusFunnel>, AnalyticsError> {
        self.require_extension()?;
        let sql = queries::funnel_sql(params);
        let total_steps = u32::try_from(params.species_sequence.len()).unwrap_or(0);
        let sequence = params.species_sequence.clone();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (date, steps_completed) = row?;
            let matched_species = sequence
                .iter()
                .take(steps_completed as usize)
                .cloned()
                .collect();
            results.push(types::ChorusFunnel {
                date,
                steps_completed,
                total_steps,
                matched_species,
            });
        }
        Ok(results)
    }

    /// Execute a next-species prediction query.
    ///
    /// Finds which species are most likely to be detected after `trigger`
    /// within `window_minutes` minutes, based on historical co-occurrence.
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::ExtensionLoad` if the extension is not loaded.
    pub fn next_species(
        &self,
        trigger: &str,
        window_minutes: u32,
        limit: u32,
    ) -> Result<Vec<types::NextSpeciesPrediction>, AnalyticsError> {
        self.require_extension()?;
        let sql = queries::next_species_sql(trigger, window_minutes, limit);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let frequency: i64 = row.get(1)?;
            Ok(types::NextSpeciesPrediction {
                after_species: trigger.to_string(),
                predicted_species: row.get(0)?,
                frequency: u64::try_from(frequency).unwrap_or(0),
                probability: 0.0,
            })
        })?;

        let mut results: Vec<types::NextSpeciesPrediction> = rows
            .map(|r| r.map_err(AnalyticsError::from))
            .collect::<Result<_, _>>()?;

        let total: u64 = results.iter().map(|r| r.frequency).sum();
        if total > 0 {
            for result in &mut results {
                #[allow(clippy::cast_precision_loss)]
                {
                    result.probability = result.frequency as f64 / total as f64;
                }
            }
        }
        Ok(results)
    }

    /// Guard: return an error if the extension is not loaded.
    fn require_extension(&self) -> Result<(), AnalyticsError> {
        if self.extension_loaded {
            Ok(())
        } else {
            Err(AnalyticsError::ExtensionLoad(
                "behavioral extension not loaded".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_db() -> (AnalyticsDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).unwrap();
        (db, dir)
    }

    #[test]
    fn sessionize_requires_extension() {
        let (db, _tmp) = make_db();
        let err = db.sessionize(&types::SessionizeParams::default()).unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    #[test]
    fn next_species_requires_extension() {
        let (db, _tmp) = make_db();
        let err = db.next_species("European Robin", 60, 10).unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    #[test]
    fn funnel_requires_extension() {
        let (db, _tmp) = make_db();
        let params = types::FunnelParams {
            species_sequence: vec!["Robin".into(), "Blackbird".into()],
            lookback_days: 30,
        };
        let err = db.funnel(&params).unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }
}
