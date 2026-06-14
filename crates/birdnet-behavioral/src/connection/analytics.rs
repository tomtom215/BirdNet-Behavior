//! Behavioral analytics query methods on `AnalyticsDb`.
//!
//! Wraps the `duckdb-behavioral` extension functions (sessionize, retention,
//! `window_funnel` / `window_funnel_events`, `sequence_match` /
//! `sequence_count` / `sequence_match_events`, `sequence_next_node`) with typed
//! Rust APIs. All methods require `extension_loaded == true`.

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

    /// Execute a dawn-chorus funnel query returning per-step completion times.
    ///
    /// Like [`Self::funnel`] but backed by `window_funnel_events`
    /// (duckdb-behavioral v0.8.0): each day carries the timestamp at which every
    /// completed step finished, not just the count.
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::InvalidData` if the species sequence is not
    /// 2..=32 long, `AnalyticsError::ExtensionLoad` if the extension is not
    /// loaded (or predates v0.8.0), or `AnalyticsError::Database` on failure.
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
        let total_steps = u32::try_from(n).unwrap_or(0);
        let sql = queries::funnel_events_sql(params);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let date: String = row.get(0)?;
            let step_times = value_to_strings(row.get(1)?);
            Ok((date, step_times))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (date, step_times) = row?;
            results.push(types::ChorusFunnelEvents {
                date,
                steps_completed: u32::try_from(step_times.len()).unwrap_or(u32::MAX),
                total_steps,
                step_times,
            });
        }
        Ok(results)
    }

    /// Execute a combined ordered-sequence analysis query.
    ///
    /// For each day, reports whether the configured species occurred in order
    /// (`sequence_match`), how many non-overlapping times (`sequence_count`),
    /// and the timestamps of the matched events (`sequence_match_events`) — the
    /// duckdb-behavioral v0.8.0 parity functions, in a single pass.
    ///
    /// # Errors
    ///
    /// Returns `AnalyticsError::InvalidData` if the species sequence is not
    /// 2..=32 long, `AnalyticsError::ExtensionLoad` if the extension is not
    /// loaded (or predates v0.8.0), or `AnalyticsError::Database` on failure.
    pub fn sequence_analysis(
        &self,
        params: &types::PatternParams,
    ) -> Result<Vec<types::SequenceAnalysis>, AnalyticsError> {
        let n = params.species_sequence.len();
        if !(2..=32).contains(&n) {
            return Err(AnalyticsError::InvalidData(format!(
                "sequence_analysis requires 2..=32 species, got {n}"
            )));
        }
        self.require_extension()?;
        let sql = queries::sequence_analysis_sql(params);
        let sequence = params.species_sequence.clone();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(types::SequenceAnalysis {
                date: row.get(0)?,
                matched: row.get(1)?,
                occurrences: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                event_times: value_to_strings(row.get(3)?),
                species_sequence: sequence.clone(),
            })
        })?;
        rows.map(|r| r.map_err(AnalyticsError::from)).collect()
    }

    /// The loaded extension's version via its native `behavioral_version()`
    /// scalar (added in v0.8.0).
    ///
    /// Best-effort: returns `None` when the extension is not loaded or predates
    /// the function, so it doubles as a "v0.8.0-or-newer" probe. For a
    /// version-agnostic check use [`Self::extension_version`].
    pub fn behavioral_version(&self) -> Option<String> {
        self.conn
            .query_row(queries::BEHAVIORAL_VERSION_FN, [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
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

/// Collect the text elements of a `DuckDB` `VARCHAR[]` value into a `Vec<String>`.
///
/// The `TIMESTAMP[]` results of `window_funnel_events` / `sequence_match_events`
/// are cast to `VARCHAR[]` in SQL, so each list element arrives as a
/// `Value::Text`; non-text or NULL elements are skipped. A non-list value
/// (e.g. SQL NULL for a day with no events) yields an empty vector.
fn value_to_strings(value: Value) -> Vec<String> {
    match value {
        Value::List(items) => items
            .into_iter()
            .filter_map(|item| match item {
                Value::Text(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
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
    fn sequence_analysis_requires_extension() {
        let (db, _tmp) = make_db();
        // A valid 2-species sequence so the guard, not the length check, fires.
        let err = db
            .sequence_analysis(&types::PatternParams::default())
            .unwrap_err();
        assert!(err.to_string().contains("extension not loaded"));
    }

    #[test]
    fn sequence_analysis_rejects_short_sequence() {
        let (db, _tmp) = make_db();
        let params = types::PatternParams {
            species_sequence: vec!["Robin".into()],
            ..types::PatternParams::default()
        };
        let err = db.sequence_analysis(&params).unwrap_err();
        assert!(err.to_string().contains("requires 2..=32"));
    }

    #[test]
    fn value_to_strings_extracts_text_and_skips_others() {
        let list = Value::List(vec![
            Value::Text("2024-05-01 05:00:00".into()),
            Value::Null,
            Value::Text("2024-05-01 05:10:00".into()),
        ]);
        assert_eq!(
            super::value_to_strings(list),
            vec![
                "2024-05-01 05:00:00".to_string(),
                "2024-05-01 05:10:00".to_string()
            ]
        );
        // A non-list (SQL NULL) yields an empty vector, not a panic.
        assert!(super::value_to_strings(Value::Null).is_empty());
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
}
