//! Chart queries: the series behind birdnet-go's analytics charts.
//!
//! Each returns what the chart is computed from, read the way upstream reads
//! it, and leaves the calendar arithmetic to the caller, which has the civil
//! date functions this crate does not depend on. All read
//! `detections_analytic`, which leaves out rejected detections and, when the
//! operator asked, imported ones.
//!
//! A `species` filter is a scientific or common name in any case: upstream
//! resolves a common name to its scientific one first, which is the same set
//! of rows here, since both are stored on every detection.

use rusqlite::Connection;

use crate::sqlite::connection::DbError;

/// One species' confidence distribution over a date range.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfidenceHistogram {
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// The fraction of its detections in each of the equal-width bins over
    /// `[0, 1]`, lowest first; they sum to 1 up to rounding.
    pub bins: Vec<f64>,
    /// Its detections in the range.
    pub total: i64,
}

/// Detections a species needs in the range to have its distribution shown
/// when no species was asked for: below this a histogram is noise
/// (birdnet-go's `minConfidenceHistogramDetections`).
pub const CONFIDENCE_HISTOGRAM_MIN_DETECTIONS: i64 = 20;

/// Which bin of `bins` equal-width bins over `[0, 1]` a confidence falls in:
/// truncated, so a bin holds its lower edge and not its upper one, with 1.0
/// (and anything above) in the last and anything below 0 in the first.
fn confidence_bin(confidence: f64, bins: usize) -> usize {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    let index = (confidence * bins as f64).max(0.0) as usize;
    index.min(bins - 1)
}

/// Each species' confidence distribution over `start..=end`: birdnet-go's
/// confidence distribution.
///
/// With `species` (a scientific or common name, either case), that species
/// alone, whatever its count — at most `limit` of them, which only matters
/// for a name two species share. Without, the `limit` most detected species in
/// the range, the most detected first, less those with fewer than
/// [`CONFIDENCE_HISTOGRAM_MIN_DETECTIONS`] — chosen first and then thinned,
/// as upstream does, so fewer than `limit` can come back while a less
/// detected species would have qualified. Ties are broken by scientific name;
/// upstream breaks them by its internal label id. `bins` of zero returns
/// nothing.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn confidence_histograms(
    conn: &Connection,
    start: &str,
    end: &str,
    species: Option<&str>,
    bins: usize,
    limit: u32,
) -> Result<Vec<ConfidenceHistogram>, DbError> {
    if bins == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "WITH d AS (
             SELECT Sci_Name, Com_Name, Confidence FROM detections_analytic
             WHERE Date >= ?1 AND Date <= ?2
         ),
         chosen AS (
             SELECT Sci_Name, MAX(Com_Name) AS com, COUNT(*) AS n FROM d
             WHERE ?3 IS NULL OR lower(Sci_Name) = lower(?3) OR lower(Com_Name) = lower(?3)
             GROUP BY Sci_Name
             ORDER BY n DESC, Sci_Name
             LIMIT ?4
         )
         SELECT c.Sci_Name, c.com, c.n, d.Confidence
         FROM chosen c JOIN d ON d.Sci_Name = c.Sci_Name
         ORDER BY c.n DESC, c.Sci_Name",
    )?;
    let min_count = if species.is_some() {
        1
    } else {
        CONFIDENCE_HISTOGRAM_MIN_DETECTIONS
    };
    let mut out: Vec<(ConfidenceHistogram, Vec<i64>)> = Vec::new();
    let mut rows = stmt.query(rusqlite::params![start, end, species, limit])?;
    while let Some(row) = rows.next()? {
        let sci: String = row.get(0)?;
        let total: i64 = row.get(2)?;
        if total < min_count {
            continue;
        }
        if out.last().is_none_or(|(h, _)| h.sci_name != sci) {
            out.push((
                ConfidenceHistogram {
                    sci_name: sci,
                    com_name: row.get(1)?,
                    bins: Vec::new(),
                    total,
                },
                vec![0; bins],
            ));
        }
        let confidence: f64 = row.get(3)?;
        if let Some((_, counts)) = out.last_mut() {
            counts[confidence_bin(confidence, bins)] += 1;
        }
    }
    Ok(out
        .into_iter()
        .map(|(mut h, counts)| {
            #[allow(clippy::cast_precision_loss)]
            {
                h.bins = counts.iter().map(|&c| c as f64 / h.total as f64).collect();
            }
            h
        })
        .collect())
}

/// Detections on each day of `start..=end` that had any, oldest first.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn day_counts_between(
    conn: &Connection,
    start: &str,
    end: &str,
) -> Result<Vec<(String, i64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, COUNT(*) FROM detections_analytic
         WHERE Date >= ?1 AND Date <= ?2
         GROUP BY Date ORDER BY Date",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![start, end], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A station with `(date, time, sci_name, confidence)` detections.
    fn station(rows: &[(&str, &str, &str, f64)]) -> Connection {
        let conn = Connection::open_in_memory().expect("db");
        crate::migration::migrate(&conn).expect("migrate");
        for (date, time, sci, confidence) in rows {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (?1, ?2, ?3, ?3 || ' common', ?4)",
                rusqlite::params![date, time, sci, confidence],
            )
            .expect("insert");
        }
        conn
    }

    /// `n` detections of `sci` on `date` at confidence `c`, a second apart.
    fn many(
        n: u32,
        date: &'static str,
        sci: &'static str,
        c: f64,
    ) -> Vec<(String, &'static str, &'static str, f64)> {
        (0..n)
            .map(|i| (format!("06:{:02}:{:02}", i / 60, i % 60), date, sci, c))
            .collect()
    }

    fn station_of(groups: &[Vec<(String, &str, &str, f64)>]) -> Connection {
        let rows: Vec<(&str, &str, &str, f64)> = groups
            .iter()
            .flatten()
            .map(|(t, d, s, c)| (*d, t.as_str(), *s, *c))
            .collect();
        station(&rows)
    }

    /// Upstream's binning: `bins` equal-width bins over [0, 1], a bin holding
    /// its lower edge, 1.0 in the last.
    #[test]
    fn a_confidence_falls_in_the_bin_whose_lower_edge_it_reaches() {
        assert_eq!(confidence_bin(0.0, 20), 0);
        assert_eq!(confidence_bin(0.049, 20), 0);
        assert_eq!(confidence_bin(0.05, 20), 1);
        assert_eq!(confidence_bin(0.75, 4), 3);
        assert_eq!(confidence_bin(0.7499, 4), 2);
        assert_eq!(confidence_bin(1.0, 20), 19);
        assert_eq!(confidence_bin(1.5, 20), 19);
        assert_eq!(confidence_bin(-0.2, 20), 0);
    }

    /// Without a species: the `limit` most detected in the range, then those
    /// under 20 dropped; each species' bins are fractions of its total.
    #[test]
    fn the_most_detected_species_are_binned_and_sparse_ones_dropped() {
        let conn = station_of(&[
            // Blackbird: 30 detections, 10 at 0.95 and 20 at 0.55.
            many(10, "2026-10-01", "Turdus merula", 0.95),
            many(20, "2026-10-02", "Turdus merula", 0.55),
            // Robin: 25 at 0.85.
            many(25, "2026-10-03", "Erithacus rubecula", 0.85),
            // Wren: 19 — one short of the floor.
            many(19, "2026-10-04", "Troglodytes troglodytes", 0.9),
            // Outside the range.
            many(40, "2026-09-01", "Parus major", 0.9),
        ]);
        let got =
            confidence_histograms(&conn, "2026-10-01", "2026-10-31", None, 10, 5).expect("query");
        let names: Vec<&str> = got.iter().map(|h| h.sci_name.as_str()).collect();
        assert_eq!(names, ["Turdus merula", "Erithacus rubecula"]);
        assert_eq!(got[0].total, 30);
        assert_eq!(got[0].com_name, "Turdus merula common");
        assert_eq!(got[0].bins.len(), 10);
        assert!(
            (got[0].bins[9] - 10.0 / 30.0).abs() < 1e-12,
            "{:?}",
            got[0].bins
        );
        assert!(
            (got[0].bins[5] - 20.0 / 30.0).abs() < 1e-12,
            "{:?}",
            got[0].bins
        );
        assert!((got[0].bins.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!((got[1].bins[8] - 1.0).abs() < 1e-12, "{:?}", got[1].bins);

        // The limit is applied before the floor, as upstream does: the top
        // three include the wren, which is then dropped, leaving two.
        let top3 =
            confidence_histograms(&conn, "2026-10-01", "2026-10-31", None, 10, 3).expect("query");
        assert_eq!(top3.len(), 2);
        let top1 =
            confidence_histograms(&conn, "2026-10-01", "2026-10-31", None, 10, 1).expect("query");
        assert_eq!(top1.len(), 1);
        assert_eq!(top1[0].sci_name, "Turdus merula");
        // Both ends of the range are inclusive.
        let one_day =
            confidence_histograms(&conn, "2026-10-02", "2026-10-03", None, 10, 5).expect("query");
        assert_eq!(
            one_day.iter().map(|h| h.total).collect::<Vec<_>>(),
            [25, 20],
            "the robin's 25 on the 3rd and the blackbird's 20 on the 2nd"
        );
    }

    /// With a species: that one alone, by either name and in any case, with
    /// no floor; a rejected detection is in no count.
    #[test]
    fn a_named_species_is_shown_whatever_its_count() {
        let conn = station_of(&[
            many(3, "2026-10-01", "Troglodytes troglodytes", 0.42),
            many(30, "2026-10-01", "Turdus merula", 0.9),
        ]);
        conn.execute(
            "UPDATE detections SET review_verdict = 'rejected'
             WHERE Sci_Name = 'Troglodytes troglodytes' AND Time = '06:00:00'",
            [],
        )
        .expect("reject");
        for name in ["Troglodytes troglodytes", "troglodytes TROGLODYTES common"] {
            let got = confidence_histograms(&conn, "2026-10-01", "2026-10-01", Some(name), 5, 5)
                .expect("query");
            assert_eq!(got.len(), 1, "{name}");
            assert_eq!(got[0].sci_name, "Troglodytes troglodytes");
            assert_eq!(got[0].total, 2, "the rejected one is not counted");
            assert_eq!(got[0].bins, [0.0, 0.0, 1.0, 0.0, 0.0]);
        }
        assert!(
            confidence_histograms(&conn, "2026-10-01", "2026-10-01", Some("Nobody"), 5, 5)
                .expect("query")
                .is_empty()
        );
        assert!(
            confidence_histograms(&conn, "2026-10-01", "2026-10-01", None, 0, 5)
                .expect("query")
                .is_empty(),
            "no bins, no histogram"
        );
    }

    /// Each day's detections, both ends inclusive, rejected ones left out.
    #[test]
    fn days_are_counted_over_the_range() {
        let conn = station(&[
            ("2026-01-01", "06:00:00", "Turdus merula", 0.9),
            ("2026-01-01", "07:00:00", "Turdus merula", 0.9),
            ("2026-01-03", "06:00:00", "Turdus merula", 0.9),
            ("2026-01-05", "06:00:00", "Turdus merula", 0.9),
            ("2025-12-31", "06:00:00", "Turdus merula", 0.9),
        ]);
        conn.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Date = '2026-01-03'",
            [],
        )
        .expect("reject");
        assert_eq!(
            day_counts_between(&conn, "2026-01-01", "2026-01-05").expect("query"),
            [("2026-01-01".to_owned(), 2), ("2026-01-05".to_owned(), 1)]
        );
    }
}
