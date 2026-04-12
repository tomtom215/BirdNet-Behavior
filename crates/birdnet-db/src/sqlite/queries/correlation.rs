//! Species co-occurrence and correlation queries.
//!
//! Analyses which species tend to appear together (same date or within a short
//! time window), enabling the UI to show "birds you might also see" suggestions
//! and seasonal co-occurrence patterns.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

/// A species pair with their co-occurrence count.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeciesPair {
    /// First species common name (alphabetically first).
    pub species_a: String,
    /// Second species common name.
    pub species_b: String,
    /// Number of dates on which both species were detected.
    pub co_occurrence_days: i64,
    /// Total detections of `species_a` across all dates.
    pub count_a: i64,
    /// Total detections of `species_b` across all dates.
    pub count_b: i64,
}

/// Species frequently seen *after* a given species on the same day.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FollowOn {
    /// The trigger species.
    pub trigger: String,
    /// Species commonly seen on the same day.
    pub companion: String,
    /// Days on which both appeared.
    pub shared_days: i64,
    /// Average confidence of companion detections.
    pub avg_confidence: f64,
}

/// Query the top N species co-occurrence pairs for the last `days` days.
///
/// Uses a self-join on `Date` to find pairs detected on the same calendar day.
/// Only pairs with at least `min_co_days` shared days are returned.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn top_cooccurrence_pairs(
    conn: &Connection,
    days: u32,
    limit: usize,
    min_co_days: u32,
) -> Result<Vec<SpeciesPair>, DbError> {
    let mut stmt = conn.prepare(
        "WITH daily AS (
            SELECT DISTINCT Date, Com_Name FROM detections
            WHERE Date >= DATE('now', '-' || ?1 || ' days')
         ),
         counts AS (
            SELECT Com_Name, COUNT(DISTINCT Date) AS total_days
            FROM daily GROUP BY Com_Name
         ),
         pairs AS (
            SELECT
                CASE WHEN a.Com_Name < b.Com_Name THEN a.Com_Name ELSE b.Com_Name END AS species_a,
                CASE WHEN a.Com_Name < b.Com_Name THEN b.Com_Name ELSE a.Com_Name END AS species_b,
                COUNT(DISTINCT a.Date) AS co_days
            FROM daily a
            JOIN daily b ON a.Date = b.Date AND a.Com_Name != b.Com_Name
            GROUP BY species_a, species_b
            HAVING co_days >= ?3
         )
         SELECT p.species_a, p.species_b, p.co_days,
                ca.total_days, cb.total_days
         FROM pairs p
         JOIN counts ca ON ca.Com_Name = p.species_a
         JOIN counts cb ON cb.Com_Name = p.species_b
         ORDER BY p.co_days DESC
         LIMIT ?2",
    )?;

    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
    let min_i64 = i64::from(min_co_days);

    let rows = stmt
        .query_map(params![days, limit_i64, min_i64], |row| {
            Ok(SpeciesPair {
                species_a: row.get(0)?,
                species_b: row.get(1)?,
                co_occurrence_days: row.get(2)?,
                count_a: row.get(3)?,
                count_b: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Query species that commonly appear on the same day as `trigger_species`.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn companion_species(
    conn: &Connection,
    trigger_species: &str,
    days: u32,
    limit: usize,
) -> Result<Vec<FollowOn>, DbError> {
    let mut stmt = conn.prepare(
        "WITH trigger_dates AS (
            SELECT DISTINCT Date FROM detections
            WHERE Com_Name = ?1
              AND Date >= DATE('now', '-' || ?2 || ' days')
         )
         SELECT
            ?1 AS trigger,
            d.Com_Name AS companion,
            COUNT(DISTINCT d.Date) AS shared_days,
            AVG(d.Confidence) AS avg_confidence
         FROM detections d
         JOIN trigger_dates td ON d.Date = td.Date
         WHERE d.Com_Name != ?1
         GROUP BY d.Com_Name
         ORDER BY shared_days DESC, avg_confidence DESC
         LIMIT ?3",
    )?;

    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);

    let rows = stmt
        .query_map(params![trigger_species, days, limit_i64], |row| {
            Ok(FollowOn {
                trigger: row.get(0)?,
                companion: row.get(1)?,
                shared_days: row.get(2)?,
                avg_confidence: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Get species detected together within a narrow time window (minutes) on the same day.
///
/// Finds pairs where both species appeared within `window_minutes` of each other
/// on the same recording date — tighter than daily co-occurrence.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn temporal_cooccurrence(
    conn: &Connection,
    window_minutes: u32,
    days: u32,
    limit: usize,
) -> Result<Vec<SpeciesPair>, DbError> {
    let window_secs = i64::from(window_minutes) * 60;
    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);

    let mut stmt = conn.prepare(
        "WITH timed AS (
            SELECT Date,
                   SUBSTR(Time, 1, 2) * 3600 +
                   SUBSTR(Time, 4, 2) * 60  +
                   SUBSTR(Time, 7, 2)        AS secs,
                   Com_Name
            FROM detections
            WHERE Date >= DATE('now', '-' || ?3 || ' days')
         )
         SELECT
            CASE WHEN a.Com_Name < b.Com_Name THEN a.Com_Name ELSE b.Com_Name END,
            CASE WHEN a.Com_Name < b.Com_Name THEN b.Com_Name ELSE a.Com_Name END,
            COUNT(*) AS co_count,
            COUNT(*) AS co_count_a,
            COUNT(*) AS co_count_b
         FROM timed a
         JOIN timed b
           ON  a.Date = b.Date
           AND a.Com_Name != b.Com_Name
           AND ABS(a.secs - b.secs) <= ?1
         GROUP BY 1, 2
         ORDER BY co_count DESC
         LIMIT ?2",
    )?;

    let rows = stmt
        .query_map(params![window_secs, limit_i64, days], |row| {
            Ok(SpeciesPair {
                species_a: row.get(0)?,
                species_b: row.get(1)?,
                co_occurrence_days: row.get(2)?,
                count_a: row.get(3)?,
                count_b: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Dates are computed at insert time via SQLite's DATE('now', '-N days')
        // so the fixture stays within the 30-day window used by the queries
        // under test, regardless of when the suite runs.
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT
            );
            INSERT INTO detections VALUES
              (DATE('now', '-7 days'),'07:00:00','A sp','Robin',  0.9,0,0,0,0,0,0,''),
              (DATE('now', '-7 days'),'07:05:00','B sp','Wren',   0.8,0,0,0,0,0,0,''),
              (DATE('now', '-7 days'),'08:00:00','C sp','Finch',  0.7,0,0,0,0,0,0,''),
              (DATE('now', '-6 days'),'07:00:00','A sp','Robin',  0.9,0,0,0,0,0,0,''),
              (DATE('now', '-6 days'),'07:10:00','B sp','Wren',   0.8,0,0,0,0,0,0,''),
              (DATE('now', '-5 days'),'07:00:00','A sp','Robin',  0.9,0,0,0,0,0,0,'');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn top_pairs_basic() {
        let conn = setup();
        let pairs = top_cooccurrence_pairs(&conn, 30, 10, 1).unwrap();
        // Robin+Wren share 2 days, Robin+Finch share 1 day
        assert!(!pairs.is_empty());
        let top = &pairs[0];
        // should be Robin / Wren (2 shared days)
        assert!(
            (top.species_a == "Robin" || top.species_b == "Robin"),
            "top pair should include Robin"
        );
        assert!(
            (top.species_a == "Wren" || top.species_b == "Wren"),
            "top pair should include Wren"
        );
        assert_eq!(top.co_occurrence_days, 2);
    }

    #[test]
    fn companion_species_robin() {
        let conn = setup();
        let companions = companion_species(&conn, "Robin", 30, 10).unwrap();
        // Wren appears on 2 of 3 robin days, Finch on 1
        assert!(!companions.is_empty());
        assert_eq!(companions[0].companion, "Wren");
        assert_eq!(companions[0].shared_days, 2);
    }

    #[test]
    fn temporal_cooccurrence_5min_window() {
        let conn = setup();
        // Robin at 07:00, Wren at 07:05 → within 5 min
        let pairs = temporal_cooccurrence(&conn, 5, 30, 10).unwrap();
        assert!(!pairs.is_empty());
        let names: Vec<_> = pairs
            .iter()
            .flat_map(|p| [p.species_a.as_str(), p.species_b.as_str()])
            .collect();
        assert!(names.contains(&"Robin"));
        assert!(names.contains(&"Wren"));
    }

    #[test]
    fn temporal_cooccurrence_1min_excludes_5min_gap() {
        let conn = setup();
        // Robin at 07:00, Wren at 07:05 → NOT within 1 min
        let pairs = temporal_cooccurrence(&conn, 1, 30, 10).unwrap();
        // Should not include Robin+Wren pair from 5-min gap
        let has_robin_wren = pairs.iter().any(|p| {
            (p.species_a == "Robin" && p.species_b == "Wren")
                || (p.species_a == "Wren" && p.species_b == "Robin")
        });
        assert!(!has_robin_wren);
    }
}
