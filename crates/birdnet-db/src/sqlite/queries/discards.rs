//! What the detection filters discarded, per day, source and reason (ACC-02).
//!
//! `filter_discards` (migration 60) is credited once per analysed file by the
//! daemon. `windows` counts analysis windows masked whole (for a voice or a
//! watched noise); `detections` counts species detections a filter removed.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

/// One reason's discards on one day from one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardRow {
    /// Recording date, `YYYY-MM-DD`.
    pub date: String,
    /// The source label (`local`, or an RTSP stream id).
    pub source: String,
    /// The filter: `privacy`, `noise`, `corroboration`, `occurrence` or
    /// `species_list`.
    pub reason: String,
    /// Analysis windows masked whole.
    pub windows: u64,
    /// Species detections removed.
    pub detections: u64,
}

/// Add one file's discards for `reason` to its day and source. Zero counts
/// write nothing.
///
/// # Errors
///
/// Returns `DbError` on write failure.
pub fn add_discards(
    conn: &Connection,
    date: &str,
    source: &str,
    reason: &str,
    windows: u64,
    detections: u64,
) -> Result<(), DbError> {
    if windows == 0 && detections == 0 {
        return Ok(());
    }
    let w = i64::try_from(windows).unwrap_or(i64::MAX);
    let d = i64::try_from(detections).unwrap_or(i64::MAX);
    conn.execute(
        "INSERT INTO filter_discards (date, source, reason, windows, detections)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(date, source, reason) DO UPDATE SET
             windows = windows + excluded.windows,
             detections = detections + excluded.detections",
        params![date, source, reason, w, d],
    )?;
    Ok(())
}

/// Every row for `date`, ordered by source then reason.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn discards_on(conn: &Connection, date: &str) -> Result<Vec<DiscardRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT date, source, reason, windows, detections FROM filter_discards
          WHERE date = ?1 ORDER BY source, reason",
    )?;
    let rows = stmt
        .query_map([date], |r| {
            Ok(DiscardRow {
                date: r.get(0)?,
                source: r.get(1)?,
                reason: r.get(2)?,
                windows: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
                detections: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

    /// Two files' discards on one day add up; another source and another
    /// day are kept apart; a zero writes no row.
    #[test]
    fn discards_accumulate_per_day_source_and_reason() {
        let conn = conn();
        add_discards(&conn, "2026-05-19", "local", "privacy", 3, 2).unwrap();
        add_discards(&conn, "2026-05-19", "local", "privacy", 1, 0).unwrap();
        add_discards(&conn, "2026-05-19", "cam1", "privacy", 5, 0).unwrap();
        add_discards(&conn, "2026-05-20", "local", "privacy", 7, 7).unwrap();
        add_discards(&conn, "2026-05-19", "local", "noise", 0, 0).unwrap();
        let rows = discards_on(&conn, "2026-05-19").unwrap();
        let got: Vec<(&str, &str, u64, u64)> = rows
            .iter()
            .map(|r| {
                (
                    r.source.as_str(),
                    r.reason.as_str(),
                    r.windows,
                    r.detections,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![("cam1", "privacy", 5, 0), ("local", "privacy", 4, 2)]
        );
    }
}
