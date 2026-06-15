//! Behavioral analytics query methods on `AnalyticsDb`.
//!
//! Wraps the `duckdb-behavioral` extension functions (sessionize, retention,
//! `window_funnel`, `window_funnel_events`, `sequence_match`, `sequence_count`,
//! `sequence_next_node`) with typed Rust APIs. All methods require
//! `extension_loaded == true`.

use duckdb::types::Value;

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
        // DuckDB returns BIGINT for the session id, COUNT and DATEDIFF, and the
        // timestamps are CAST to VARCHAR in the query; convert the signed
        // counts (always non-negative here) to the struct's unsigned fields.
        let rows = stmt.query_map([], |row| {
            Ok(types::ActivitySession {
                species: row.get(0)?,
                session_id: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                detection_count: u32::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                start_time: row.get(3)?,
                end_time: row.get(4)?,
                duration_secs: u64::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
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
        // retention() accepts 2..=32 conditions; one anchors the cohort, so
        // 1..=31 day intervals are valid.
        let n = params.intervals.len();
        if !(1..=31).contains(&n) {
            return Err(AnalyticsError::InvalidData(format!(
                "retention requires 1..=31 day intervals, got {n}"
            )));
        }
        self.require_extension()?;
        let sql = queries::retention_sql(params);
        let mut stmt = self.conn.prepare(&sql)?;

        let rows = stmt.query_map([], |row| {
            let species: String = row.get(0)?;
            let rates_value: Value = row.get(1)?;
            let rates_raw: Vec<f64> = match rates_value {
                Value::List(list) => list
                    .into_iter()
                    .map(|v| match v {
                        Value::Double(f) => f,
                        _ => 0.0,
                    })
                    .collect(),
                _ => Vec::new(),
            };
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
        let n = params.species_sequence.len();
        if !(2..=32).contains(&n) {
            return Err(AnalyticsError::InvalidData(format!(
                "window_funnel requires 2..=32 species, got {n}"
            )));
        }
        self.require_extension()?;
        let sql = queries::funnel_sql(params);
        let total_steps = u32::try_from(n).unwrap_or(0);
        let sequence = params.species_sequence.clone();

        let mut stmt = self.conn.prepare(&sql)?;
        // window_funnel returns INTEGER (the furthest step reached).
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                u32::try_from(row.get::<_, i32>(1)?).unwrap_or(0),
            ))
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

    /// Execute a dawn-chorus funnel *step-timing* query
    /// (`window_funnel_events`, v0.8.0).
    ///
    /// Like [`Self::funnel`] but returns the timestamp each completed step
    /// fired (in funnel order), so callers can show the actual dawn
    /// progression rather than just how many steps were reached.
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::InvalidData` if the species sequence is not
    /// 2..=32 long, `AnalyticsError::ExtensionLoad` if the extension is not
    /// loaded, or `AnalyticsError::Database` on query failure.
    pub fn funnel_events(
        &self,
        params: &types::FunnelParams,
    ) -> Result<Vec<types::ChorusFunnelEvents>, AnalyticsError> {
        let n = params.species_sequence.len();
        if !(2..=32).contains(&n) {
            return Err(AnalyticsError::InvalidData(format!(
                "window_funnel_events requires 2..=32 species, got {n}"
            )));
        }
        self.require_extension()?;
        let sql = queries::funnel_events_sql(params);
        let sequence = params.species_sequence.clone();
        let mut stmt = self.conn.prepare(&sql)?;
        // window_funnel_events returns TIMESTAMP[]; the SQL casts each element
        // to VARCHAR, so the column reads back as a list of text values.
        let rows = stmt.query_map([], |row| {
            let date: String = row.get(0)?;
            let times_value: Value = row.get(1)?;
            let step_times: Vec<String> = match times_value {
                Value::List(list) => list
                    .into_iter()
                    .filter_map(|v| match v {
                        Value::Text(s) => Some(s),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            Ok(types::ChorusFunnelEvents {
                date,
                step_times,
                species_sequence: sequence.clone(),
            })
        })?;
        rows.map(|r| r.map_err(AnalyticsError::from)).collect()
    }

    /// Execute an ordered sequence-pattern match query.
    ///
    /// For each day, reports whether the configured species were detected in
    /// the given order (optionally within `params.max_gap_minutes` between
    /// consecutive steps), using `sequence_match` from duckdb-behavioral.
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::InvalidData` if the species sequence is not
    /// 2..=32 long, `AnalyticsError::ExtensionLoad` if the extension is not
    /// loaded, or `AnalyticsError::Database` on query failure.
    pub fn sequence_match(
        &self,
        params: &types::PatternParams,
    ) -> Result<Vec<types::PatternMatch>, AnalyticsError> {
        let n = params.species_sequence.len();
        if !(2..=32).contains(&n) {
            return Err(AnalyticsError::InvalidData(format!(
                "sequence_match requires 2..=32 species, got {n}"
            )));
        }
        self.require_extension()?;
        let sql = queries::sequence_match_sql(params);
        let sequence = params.species_sequence.clone();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(types::PatternMatch {
                date: row.get(0)?,
                matched: row.get(1)?,
                species_sequence: sequence.clone(),
            })
        })?;
        rows.map(|r| r.map_err(AnalyticsError::from)).collect()
    }

    /// Execute an ordered sequence *count* query (`sequence_count`, v0.8.0).
    ///
    /// Like [`Self::sequence_match`] but reports, per day, how many
    /// non-overlapping times the ordered species sequence occurred — turning
    /// "did A→B→C happen?" into "how often did A→B→C happen?".
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::InvalidData` if the species sequence is not
    /// 2..=32 long, `AnalyticsError::ExtensionLoad` if the extension is not
    /// loaded, or `AnalyticsError::Database` on query failure.
    pub fn sequence_count(
        &self,
        params: &types::PatternParams,
    ) -> Result<Vec<types::PatternCount>, AnalyticsError> {
        let n = params.species_sequence.len();
        if !(2..=32).contains(&n) {
            return Err(AnalyticsError::InvalidData(format!(
                "sequence_count requires 2..=32 species, got {n}"
            )));
        }
        self.require_extension()?;
        let sql = queries::sequence_count_sql(params);
        let sequence = params.species_sequence.clone();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(types::PatternCount {
                date: row.get(0)?,
                count: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                species_sequence: sequence.clone(),
            })
        })?;
        rows.map(|r| r.map_err(AnalyticsError::from)).collect()
    }

    /// Execute an ordered sequence *match-event* query
    /// (`sequence_match_events`, v0.8.0).
    ///
    /// Like [`Self::sequence_match`] but, per day, returns the timestamps of the
    /// events that satisfied the ordered pattern — the longest in-order prefix
    /// reached (the full set on a completing day, a partial otherwise) — so
    /// callers can show *when* a run happened. A day with a full set of step
    /// times is exactly one [`Self::sequence_count`] also counts.
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::InvalidData` if the species sequence is not
    /// 2..=32 long, `AnalyticsError::ExtensionLoad` if the extension is not
    /// loaded, or `AnalyticsError::Database` on query failure.
    pub fn sequence_match_events(
        &self,
        params: &types::PatternParams,
    ) -> Result<Vec<types::PatternMatchEvents>, AnalyticsError> {
        let n = params.species_sequence.len();
        if !(2..=32).contains(&n) {
            return Err(AnalyticsError::InvalidData(format!(
                "sequence_match_events requires 2..=32 species, got {n}"
            )));
        }
        self.require_extension()?;
        let sql = queries::sequence_match_events_sql(params);
        let sequence = params.species_sequence.clone();
        let mut stmt = self.conn.prepare(&sql)?;
        // sequence_match_events returns TIMESTAMP[]; the SQL casts each element
        // to VARCHAR, so the column reads back as a list of text values.
        let rows = stmt.query_map([], |row| {
            let date: String = row.get(0)?;
            let times_value: Value = row.get(1)?;
            let step_times: Vec<String> = match times_value {
                Value::List(list) => list
                    .into_iter()
                    .filter_map(|v| match v {
                        Value::Text(s) => Some(s),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            Ok(types::PatternMatchEvents {
                date,
                step_times,
                species_sequence: sequence.clone(),
            })
        })?;
        rows.map(|r| r.map_err(AnalyticsError::from)).collect()
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
        let err = db
            .sessionize(&types::SessionizeParams::default())
            .unwrap_err();
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
            ..types::FunnelParams::default()
        };
        let err = db.funnel(&params).unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    #[test]
    fn sequence_match_requires_extension() {
        let (db, _tmp) = make_db();
        let err = db
            .sequence_match(&types::PatternParams::default())
            .unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    // Input validation runs before the extension check, so these are testable
    // without a loaded extension.

    #[test]
    fn sequence_match_rejects_short_sequence() {
        let (db, _tmp) = make_db();
        let params = types::PatternParams {
            species_sequence: vec!["Robin".into()],
            ..types::PatternParams::default()
        };
        let err = db.sequence_match(&params).unwrap_err();
        assert!(err.to_string().contains("requires 2..=32"));
    }

    #[test]
    fn funnel_rejects_short_sequence() {
        let (db, _tmp) = make_db();
        let params = types::FunnelParams {
            species_sequence: vec!["Robin".into()],
            ..types::FunnelParams::default()
        };
        let err = db.funnel(&params).unwrap_err();
        assert!(err.to_string().contains("requires 2..=32"));
    }

    #[test]
    fn retention_rejects_empty_intervals() {
        let (db, _tmp) = make_db();
        let params = types::RetentionParams {
            intervals: vec![],
            min_detections: 5,
        };
        let err = db.retention(&params).unwrap_err();
        assert!(err.to_string().contains("requires 1..=31"));
    }

    #[test]
    fn sequence_count_requires_extension() {
        let (db, _tmp) = make_db();
        let err = db
            .sequence_count(&types::PatternParams::default())
            .unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    #[test]
    fn sequence_count_rejects_short_sequence() {
        let (db, _tmp) = make_db();
        let params = types::PatternParams {
            species_sequence: vec!["Robin".into()],
            ..types::PatternParams::default()
        };
        let err = db.sequence_count(&params).unwrap_err();
        assert!(err.to_string().contains("requires 2..=32"));
    }

    #[test]
    fn funnel_events_requires_extension() {
        let (db, _tmp) = make_db();
        let params = types::FunnelParams {
            species_sequence: vec!["Robin".into(), "Blackbird".into()],
            ..types::FunnelParams::default()
        };
        let err = db.funnel_events(&params).unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    #[test]
    fn funnel_events_rejects_short_sequence() {
        let (db, _tmp) = make_db();
        let params = types::FunnelParams {
            species_sequence: vec!["Robin".into()],
            ..types::FunnelParams::default()
        };
        let err = db.funnel_events(&params).unwrap_err();
        assert!(err.to_string().contains("requires 2..=32"));
    }

    #[test]
    fn sequence_match_events_requires_extension() {
        let (db, _tmp) = make_db();
        let err = db
            .sequence_match_events(&types::PatternParams::default())
            .unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    #[test]
    fn sequence_match_events_rejects_short_sequence() {
        let (db, _tmp) = make_db();
        let params = types::PatternParams {
            species_sequence: vec!["Robin".into()],
            ..types::PatternParams::default()
        };
        let err = db.sequence_match_events(&params).unwrap_err();
        assert!(err.to_string().contains("requires 2..=32"));
    }
}
