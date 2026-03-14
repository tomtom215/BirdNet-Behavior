//! Weekly activity heatmap queries.
//!
//! Returns detection counts aggregated by (hour-of-day × day-of-week) so the
//! web dashboard can render a calendar-heat-map showing when birds are most
//! active throughout the week.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

/// One cell in the hour × day-of-week heatmap.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HeatmapCell {
    /// Day of week: 0 = Sunday … 6 = Saturday (SQLite `strftime('%w')`).
    pub dow: u8,
    /// Hour of day: 0-23.
    pub hour: u8,
    /// Detection count in this cell.
    pub count: i64,
}

/// Hourly detection totals across all species for a rolling window.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HourTotal {
    /// Hour of day: 0-23.
    pub hour: u8,
    /// Total detections across all days in the window.
    pub count: i64,
}

/// Query the hour × day-of-week heatmap for the last `days` days.
///
/// Returns up to 168 cells (7 × 24).  Cells with zero detections are omitted.
///
/// # Errors
///
/// Returns [`DbError`] on SQLite failure.
pub fn weekly_heatmap(conn: &Connection, days: u32) -> Result<Vec<HeatmapCell>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT
            CAST(strftime('%w', Date) AS INTEGER) AS dow,
            CAST(SUBSTR(Time, 1, 2) AS INTEGER)   AS hour,
            COUNT(*)                               AS count
         FROM detections
         WHERE Date >= DATE('now', '-' || ?1 || ' days')
         GROUP BY dow, hour
         ORDER BY dow, hour",
    )?;

    let rows = stmt
        .query_map(params![days], |row| {
            Ok(HeatmapCell {
                dow: row.get::<_, u8>(0)?,
                hour: row.get::<_, u8>(1)?,
                count: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Query hourly totals (summed across all days) for the last `days` days.
///
/// Useful for a simple bar chart showing peak detection hours.
///
/// # Errors
///
/// Returns [`DbError`] on SQLite failure.
pub fn hourly_totals(conn: &Connection, days: u32) -> Result<Vec<HourTotal>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT
            CAST(SUBSTR(Time, 1, 2) AS INTEGER) AS hour,
            COUNT(*)                             AS count
         FROM detections
         WHERE Date >= DATE('now', '-' || ?1 || ' days')
         GROUP BY hour
         ORDER BY hour",
    )?;

    let rows = stmt
        .query_map(params![days], |row| {
            Ok(HourTotal {
                hour: row.get::<_, u8>(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Species-level daily detection totals for a heatmap over `days` days.
///
/// Returns `(date, com_name, count)` triples, useful for building a
/// species-presence calendar.
///
/// # Errors
///
/// Returns [`DbError`] on SQLite failure.
pub fn species_daily_heatmap(
    conn: &Connection,
    days: u32,
) -> Result<Vec<(String, String, i64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, Com_Name, COUNT(*) AS count
         FROM detections
         WHERE Date >= DATE('now', '-' || ?1 || ' days')
         GROUP BY Date, Com_Name
         ORDER BY Date, count DESC",
    )?;

    let rows = stmt
        .query_map(params![days], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
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
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT
            );
            INSERT INTO detections VALUES
              ('2026-03-10','07:00:00','A','Robin',0.9,0,0,0,0,0,0,''),
              ('2026-03-10','07:30:00','A','Robin',0.8,0,0,0,0,0,0,''),
              ('2026-03-10','08:00:00','B','Wren', 0.7,0,0,0,0,0,0,''),
              ('2026-03-11','07:00:00','A','Robin',0.9,0,0,0,0,0,0,'');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn weekly_heatmap_counts() {
        let conn = setup();
        let cells = weekly_heatmap(&conn, 30).unwrap();
        // total detections across all cells should equal 4
        let total: i64 = cells.iter().map(|c| c.count).sum();
        assert_eq!(total, 4);
    }

    #[test]
    fn weekly_heatmap_empty_window() {
        let conn = setup();
        let cells = weekly_heatmap(&conn, 0).unwrap();
        // 0-day window: SQLite DATE('now', '0 days') is today; all inserts are
        // historical so this may return 0 or a small set depending on runtime date.
        let _ = cells; // just assert no panic / error
    }

    #[test]
    fn hourly_totals_sum() {
        let conn = setup();
        let totals = hourly_totals(&conn, 30).unwrap();
        let total: i64 = totals.iter().map(|h| h.count).sum();
        assert_eq!(total, 4);
        // All hours should be ≤ 23
        assert!(totals.iter().all(|h| h.hour <= 23));
    }

    #[test]
    fn species_daily_heatmap_rows() {
        let conn = setup();
        let rows = species_daily_heatmap(&conn, 30).unwrap();
        // 3 unique (date, species) combos
        assert_eq!(rows.len(), 3);
    }
}
