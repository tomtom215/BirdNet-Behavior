//! Peak activity window query methods.

use crate::error::TimeSeriesError;
use crate::queries::QueryPlan;
use crate::queries::peak::{PeakWindows, SpeciesPeak};
use crate::types::{params::PeakParams, results::PeakWindowRow};

impl super::TimeSeriesDb<'_> {
    /// Busiest N-minute detection windows over a lookback period, no two of
    /// which overlap.
    ///
    /// The candidate windows hop by less than their width, so a single burst
    /// of song lies inside several of them, and ranked by count those
    /// neighbours took every one of the top places — the card showed one burst
    /// five times. Candidates are therefore taken greedily from the busiest
    /// down, skipping any that overlaps one already kept (non-maximum
    /// suppression), so each row is a distinct stretch of time.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn peak_windows(&self, params: &PeakParams) -> Result<Vec<PeakWindowRow>, TimeSeriesError> {
        let days = params.lookback_days;
        let q = PeakWindows {
            window_minutes: params.window_minutes,
            hop_minutes: params.hop_minutes,
            // Anchor the window grid to the newest detection rather than
            // wall-clock CURRENT_TIMESTAMP. Anchoring to now() makes every
            // window boundary drift by the seconds elapsed between requests, so
            // the table visibly reshuffles on each refresh; anchoring to the
            // data is both deterministic and more meaningful — "the busiest
            // windows in the last N days of recorded activity".
            range_start: format!(
                "(SELECT max(detection_timestamp) FROM detections_ts) - INTERVAL {days} DAYS"
            ),
            range_end: "(SELECT max(detection_timestamp) FROM detections_ts)".into(),
            // Every candidate: suppression needs the ones below the cut too.
            limit: u32::MAX,
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
        let candidates = rows
            .map(|r| r.map_err(Into::into))
            .collect::<Result<Vec<_>, TimeSeriesError>>()?;
        Ok(suppress_overlaps(candidates, params.limit))
    }

    /// Peak hours of day for a specific species over the last `lookback_days`
    /// dates, today included.
    ///
    /// Each row's `peak_confidence` is the most confident detection in that
    /// hour, as the field's name says, and both ends of the hour are `HH:00`.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn species_peak_hours(
        &self,
        species: &str,
        lookback_days: u32,
    ) -> Result<Vec<PeakWindowRow>, TimeSeriesError> {
        let q = SpeciesPeak {
            lookback_days,
            ..SpeciesPeak::hourly(species.to_string())
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let hour: i64 = row.get(0)?;
            Ok(PeakWindowRow {
                window_start: format!("{hour:02}:00"),
                window_end: format!("{:02}:00", (hour + 1) % 24),
                detection_count: row.get(1)?,
                species_count: 1,
                peak_confidence: row.get(5)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }
}

/// Keep candidates in rank order, dropping any that overlaps one already kept,
/// until `limit` are kept. Empty windows are not peaks and are dropped.
///
/// Windows are compared by their formatted `YYYY-MM-DD HH:MM:SS` bounds, which
/// order the same way as the timestamps they print.
fn suppress_overlaps(candidates: Vec<PeakWindowRow>, limit: u32) -> Vec<PeakWindowRow> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut kept: Vec<PeakWindowRow> = Vec::new();
    for c in candidates {
        if kept.len() >= limit {
            break;
        }
        if c.detection_count == 0 {
            continue;
        }
        let overlaps = kept
            .iter()
            .any(|k| c.window_start < k.window_end && k.window_start < c.window_end);
        if !overlaps {
            kept.push(c);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(start: &str, end: &str, n: i64) -> PeakWindowRow {
        PeakWindowRow {
            window_start: start.into(),
            window_end: end.into(),
            detection_count: n,
            species_count: 1,
            peak_confidence: None,
        }
    }

    #[test]
    fn touching_windows_do_not_overlap_and_empty_ones_are_not_peaks() {
        let kept = suppress_overlaps(
            vec![
                w("2026-05-01 05:00:00", "2026-05-01 05:15:00", 9),
                w("2026-05-01 05:05:00", "2026-05-01 05:20:00", 8),
                w("2026-05-01 05:15:00", "2026-05-01 05:30:00", 3),
                w("2026-05-01 07:00:00", "2026-05-01 07:15:00", 0),
            ],
            5,
        );
        let starts: Vec<_> = kept.iter().map(|k| k.window_start.as_str()).collect();
        assert_eq!(starts, ["2026-05-01 05:00:00", "2026-05-01 05:15:00"]);
    }
}
