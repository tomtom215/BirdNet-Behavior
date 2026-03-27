//! Quarantine CRUD queries.
//!
//! The quarantine table holds detections that failed a per-species confidence
//! threshold (or were flagged manually) for manual review before admission
//! into the main detections table.
//!
//! # Workflow
//!
//! 1. **Insert** — daemon calls [`insert_quarantine`] when a detection passes the
//!    global threshold but fails a stricter per-species threshold.
//! 2. **Review** — the web UI lists pending items via [`list_quarantine`].
//! 3. **Approve** — [`approve_quarantine`] copies the row into `detections` and
//!    marks it `reviewed = 1, approved = 1` atomically within a transaction.
//! 4. **Reject** — [`reject_quarantine`] marks `reviewed = 1, approved = 0`.
//! 5. **Delete** — [`delete_quarantine`] removes the row entirely.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The reason a detection was placed in quarantine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuarantineReason {
    /// Detection passed the global threshold but the species-frequency metadata
    /// model assigned a probability below the configured `SF_THRESH`.
    BelowSfThresh,
    /// Detection passed the global threshold but failed a stricter per-species
    /// confidence threshold set by the user.
    LowConfidence,
    /// Manually quarantined by a user from the Today page or API.
    Manual,
}

impl QuarantineReason {
    /// Canonical string stored in the database.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::BelowSfThresh => "below_sf_thresh",
            Self::LowConfidence => "low_confidence",
            Self::Manual => "manual",
        }
    }

    /// Human-readable label for UI display.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::BelowSfThresh => "Below SF threshold",
            Self::LowConfidence => "Below species threshold",
            Self::Manual => "Manually flagged",
        }
    }

    /// Parse from a database value; unknown strings map to [`QuarantineReason::Manual`].
    #[must_use]
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "below_sf_thresh" => Self::BelowSfThresh,
            "low_confidence" => Self::LowConfidence,
            _ => Self::Manual,
        }
    }
}

/// Data used to insert a new quarantine entry.
#[derive(Debug, Clone)]
pub struct QuarantineRecord<'a> {
    /// Detection date (YYYY-MM-DD).
    pub date: &'a str,
    /// Detection time (HH:MM:SS).
    pub time: &'a str,
    /// Scientific name.
    pub sci_name: &'a str,
    /// Common name.
    pub com_name: &'a str,
    /// Model confidence (0.0 – 1.0).
    pub confidence: f64,
    /// Species-frequency model probability, if available.
    pub sf_probability: Option<f64>,
    /// Why this detection was quarantined.
    pub reason: QuarantineReason,
    /// Source audio file path (may be empty).
    pub file_name: Option<&'a str>,
    /// Latitude (may be absent).
    pub lat: Option<f64>,
    /// Longitude (may be absent).
    pub lon: Option<f64>,
    /// ISO week number (may be absent).
    pub week: Option<i32>,
}

/// A quarantine row read from the database.
#[derive(Debug, Clone)]
pub struct QuarantineRow {
    /// Primary key.
    pub id: i64,
    /// Detection date (YYYY-MM-DD).
    pub date: String,
    /// Detection time (HH:MM:SS).
    pub time: String,
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Model confidence (0.0 – 1.0).
    pub confidence: f64,
    /// Species-frequency model probability, if available.
    pub sf_probability: Option<f64>,
    /// Why this detection was quarantined.
    pub reason: String,
    /// Whether the entry has been reviewed.
    pub reviewed: bool,
    /// Whether the entry was approved (only meaningful when `reviewed = true`).
    pub approved: bool,
    /// Source audio file path, if any.
    pub file_name: Option<String>,
    /// Latitude, if available.
    pub lat: Option<f64>,
    /// Longitude, if available.
    pub lon: Option<f64>,
    /// ISO week number, if available.
    pub week: Option<i32>,
    /// When the entry was created (UTC, RFC3339-ish).
    pub created_at: String,
}

/// Aggregate counts for the quarantine queue.
#[derive(Debug, Clone, Default)]
pub struct QuarantineStats {
    /// Entries awaiting review (`reviewed = 0`).
    pub pending: i64,
    /// Entries approved and copied to detections.
    pub approved: i64,
    /// Entries rejected (`reviewed = 1, approved = 0`).
    pub rejected: i64,
    /// Total entries ever quarantined.
    pub total: i64,
}

/// Filter for [`list_quarantine`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum QuarantineFilter {
    /// Only unreviewed entries (default).
    #[default]
    Pending,
    /// Only approved entries.
    Approved,
    /// Only rejected entries.
    Rejected,
    /// All entries regardless of status.
    All,
}

// ---------------------------------------------------------------------------
// Write operations
// ---------------------------------------------------------------------------

/// Insert a new quarantine entry.
///
/// Duplicate entries (same `date`, `time`, `sci_name`) are silently ignored so that
/// re-processing a recording does not create duplicate quarantine rows.
///
/// # Errors
///
/// Returns [`DbError`] on insert failure.
pub fn insert_quarantine(conn: &Connection, record: &QuarantineRecord<'_>) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR IGNORE INTO quarantine
            (date, time, sci_name, com_name, confidence, sf_probability,
             reason, file_name, lat, lon, week)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            record.date,
            record.time,
            record.sci_name,
            record.com_name,
            record.confidence,
            record.sf_probability,
            record.reason.as_str(),
            record.file_name,
            record.lat,
            record.lon,
            record.week,
        ],
    )?;
    Ok(())
}

/// Approve a quarantine entry.
///
/// Copies the detection into the `detections` table (using `INSERT OR IGNORE`
/// to handle the case where the detection was already admitted), then marks the
/// quarantine row as `reviewed = 1, approved = 1`.  Both writes are wrapped in
/// a transaction so the database stays consistent if either fails.
///
/// Returns `true` if the detection was newly inserted into `detections`
/// (i.e., it was not already there), `false` if it was already present.
///
/// # Errors
///
/// Returns [`DbError`] if the transaction cannot be committed.
pub fn approve_quarantine(conn: &Connection, id: i64) -> Result<bool, DbError> {
    let tx = conn.unchecked_transaction()?;

    // Copy into detections (INSERT OR IGNORE handles duplicates).
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO detections
            (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff,
             Week, Sens, Overlap, File_Name, is_locked)
         SELECT date, time, sci_name, com_name, confidence,
                lat, lon, NULL, week, NULL, NULL, file_name, 0
         FROM quarantine WHERE id = ?1",
        params![id],
    )?;

    // Mark as reviewed + approved.
    tx.execute(
        "UPDATE quarantine SET reviewed = 1, approved = 1 WHERE id = ?1",
        params![id],
    )?;

    tx.commit()?;
    Ok(inserted > 0)
}

/// Reject a quarantine entry (mark reviewed without admitting to detections).
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn reject_quarantine(conn: &Connection, id: i64) -> Result<(), DbError> {
    conn.execute(
        "UPDATE quarantine SET reviewed = 1, approved = 0 WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Delete a quarantine entry permanently.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn delete_quarantine(conn: &Connection, id: i64) -> Result<(), DbError> {
    conn.execute("DELETE FROM quarantine WHERE id = ?1", params![id])?;
    Ok(())
}

/// Prune reviewed quarantine entries older than `days` days.
///
/// This prevents the table from growing unbounded on long-running stations.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn prune_quarantine(conn: &Connection, days: u32) -> Result<u64, DbError> {
    let deleted = conn.execute(
        "DELETE FROM quarantine
         WHERE reviewed = 1
           AND created_at < datetime('now', ?1)",
        params![format!("-{days} days")],
    )?;
    Ok(deleted as u64)
}

// ---------------------------------------------------------------------------
// Read operations
// ---------------------------------------------------------------------------

/// Get a single quarantine entry by ID, or `None` if not found.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn get_quarantine(conn: &Connection, id: i64) -> Result<Option<QuarantineRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, date, time, sci_name, com_name, confidence, sf_probability,
                reason, reviewed, approved, file_name, lat, lon, week, created_at
         FROM quarantine WHERE id = ?1",
    )?;

    let mut rows = stmt.query_map(params![id], map_row)?;
    match rows.next() {
        Some(r) => Ok(Some(r?)),
        None => Ok(None),
    }
}

/// List quarantine entries with optional status filter and pagination.
///
/// Results are ordered by `created_at DESC` (newest first).
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn list_quarantine(
    conn: &Connection,
    filter: QuarantineFilter,
    limit: u32,
    offset: u32,
) -> Result<Vec<QuarantineRow>, DbError> {
    let where_clause = match filter {
        QuarantineFilter::Pending => "WHERE reviewed = 0",
        QuarantineFilter::Approved => "WHERE reviewed = 1 AND approved = 1",
        QuarantineFilter::Rejected => "WHERE reviewed = 1 AND approved = 0",
        QuarantineFilter::All => "",
    };

    let sql = format!(
        "SELECT id, date, time, sci_name, com_name, confidence, sf_probability,
                reason, reviewed, approved, file_name, lat, lon, week, created_at
         FROM quarantine
         {where_clause}
         ORDER BY created_at DESC
         LIMIT ?1 OFFSET ?2"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![limit, offset], map_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Count pending (unreviewed) quarantine entries.
///
/// Used to show the badge count in the navigation.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn quarantine_pending_count(conn: &Connection) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT COUNT(*) FROM quarantine WHERE reviewed = 0",
        [],
        |row| row.get(0),
    )
    .map_err(DbError::Sqlite)
}

/// Aggregate quarantine statistics (pending / approved / rejected / total).
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn quarantine_stats(conn: &Connection) -> Result<QuarantineStats, DbError> {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM quarantine", [], |r| r.get(0))?;
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM quarantine WHERE reviewed = 0",
        [],
        |r| r.get(0),
    )?;
    let approved: i64 = conn.query_row(
        "SELECT COUNT(*) FROM quarantine WHERE reviewed = 1 AND approved = 1",
        [],
        |r| r.get(0),
    )?;
    let rejected: i64 = conn.query_row(
        "SELECT COUNT(*) FROM quarantine WHERE reviewed = 1 AND approved = 0",
        [],
        |r| r.get(0),
    )?;
    Ok(QuarantineStats {
        pending,
        approved,
        rejected,
        total,
    })
}

/// Count quarantine entries matching a given filter (for pagination totals).
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn count_quarantine(conn: &Connection, filter: QuarantineFilter) -> Result<i64, DbError> {
    let sql = match filter {
        QuarantineFilter::Pending => {
            "SELECT COUNT(*) FROM quarantine WHERE reviewed = 0".to_string()
        }
        QuarantineFilter::Approved => {
            "SELECT COUNT(*) FROM quarantine WHERE reviewed = 1 AND approved = 1".to_string()
        }
        QuarantineFilter::Rejected => {
            "SELECT COUNT(*) FROM quarantine WHERE reviewed = 1 AND approved = 0".to_string()
        }
        QuarantineFilter::All => "SELECT COUNT(*) FROM quarantine".to_string(),
    };
    conn.query_row(&sql, [], |r| r.get(0))
        .map_err(DbError::Sqlite)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuarantineRow> {
    Ok(QuarantineRow {
        id: row.get(0)?,
        date: row.get(1)?,
        time: row.get(2)?,
        sci_name: row.get(3)?,
        com_name: row.get(4)?,
        confidence: row.get(5)?,
        sf_probability: row.get(6)?,
        reason: row.get(7)?,
        reviewed: row.get::<_, i64>(8)? != 0,
        approved: row.get::<_, i64>(9)? != 0,
        file_name: row.get(10)?,
        lat: row.get(11)?,
        lon: row.get(12)?,
        week: row.get(13)?,
        created_at: row.get(14)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::migrate;

    fn open() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn sample<'a>(reason: QuarantineReason) -> QuarantineRecord<'a> {
        QuarantineRecord {
            date: "2026-03-27",
            time: "07:15:30",
            sci_name: "Upupa epops",
            com_name: "Eurasian Hoopoe",
            confidence: 0.42,
            sf_probability: Some(0.61),
            reason,
            file_name: Some("BirdSongs/2026-03-27_07-15-30.wav"),
            lat: Some(51.5),
            lon: Some(-0.12),
            week: Some(13),
        }
    }

    #[test]
    fn insert_and_retrieve() {
        let conn = open();
        let rec = sample(QuarantineReason::LowConfidence);
        insert_quarantine(&conn, &rec).unwrap();

        let rows = list_quarantine(&conn, QuarantineFilter::Pending, 10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.sci_name, "Upupa epops");
        assert_eq!(row.reason, "low_confidence");
        assert!(!row.reviewed);
        assert!(!row.approved);
    }

    #[test]
    fn insert_ignores_duplicate() {
        let conn = open();
        let rec = sample(QuarantineReason::LowConfidence);
        insert_quarantine(&conn, &rec).unwrap();
        insert_quarantine(&conn, &rec).unwrap(); // duplicate
        let rows = list_quarantine(&conn, QuarantineFilter::All, 10, 0).unwrap();
        assert_eq!(rows.len(), 1, "duplicate should be ignored");
    }

    #[test]
    fn approve_moves_to_detections() {
        let conn = open();
        insert_quarantine(&conn, &sample(QuarantineReason::LowConfidence)).unwrap();
        let id = list_quarantine(&conn, QuarantineFilter::Pending, 1, 0).unwrap()[0].id;

        let newly_inserted = approve_quarantine(&conn, id).unwrap();
        assert!(newly_inserted);

        // quarantine row is now approved
        let row = get_quarantine(&conn, id).unwrap().unwrap();
        assert!(row.reviewed);
        assert!(row.approved);

        // detection is now in detections table
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Sci_Name = 'Upupa epops'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn approve_second_time_returns_false() {
        let conn = open();
        insert_quarantine(&conn, &sample(QuarantineReason::LowConfidence)).unwrap();
        let id = list_quarantine(&conn, QuarantineFilter::Pending, 1, 0).unwrap()[0].id;
        approve_quarantine(&conn, id).unwrap();

        // Approving again should not fail but inserted = false (duplicate ignored)
        let again = approve_quarantine(&conn, id).unwrap();
        assert!(!again);
    }

    #[test]
    fn reject_sets_flags() {
        let conn = open();
        insert_quarantine(&conn, &sample(QuarantineReason::Manual)).unwrap();
        let id = list_quarantine(&conn, QuarantineFilter::Pending, 1, 0).unwrap()[0].id;
        reject_quarantine(&conn, id).unwrap();

        let row = get_quarantine(&conn, id).unwrap().unwrap();
        assert!(row.reviewed);
        assert!(!row.approved);
    }

    #[test]
    fn delete_removes_row() {
        let conn = open();
        insert_quarantine(&conn, &sample(QuarantineReason::BelowSfThresh)).unwrap();
        let id = list_quarantine(&conn, QuarantineFilter::All, 1, 0).unwrap()[0].id;
        delete_quarantine(&conn, id).unwrap();

        let row = get_quarantine(&conn, id).unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn stats_counts_correctly() {
        let conn = open();

        // Insert 3 different entries (different times to avoid dedup)
        let mut rec = sample(QuarantineReason::LowConfidence);
        insert_quarantine(&conn, &rec).unwrap();

        rec.time = "07:16:00";
        rec.sci_name = "Picus viridis";
        rec.com_name = "European Green Woodpecker";
        insert_quarantine(&conn, &rec).unwrap();

        rec.time = "07:17:00";
        rec.sci_name = "Jynx torquilla";
        rec.com_name = "Eurasian Wryneck";
        insert_quarantine(&conn, &rec).unwrap();

        // Approve first, reject second, leave third pending.
        let rows = list_quarantine(&conn, QuarantineFilter::Pending, 10, 0).unwrap();
        approve_quarantine(&conn, rows[0].id).unwrap();
        reject_quarantine(&conn, rows[1].id).unwrap();

        let stats = quarantine_stats(&conn).unwrap();
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.approved, 1);
        assert_eq!(stats.rejected, 1);
        assert_eq!(stats.total, 3);
    }

    #[test]
    fn filter_pending_excludes_reviewed() {
        let conn = open();
        insert_quarantine(&conn, &sample(QuarantineReason::LowConfidence)).unwrap();
        let id = list_quarantine(&conn, QuarantineFilter::Pending, 1, 0).unwrap()[0].id;
        reject_quarantine(&conn, id).unwrap();

        let pending = list_quarantine(&conn, QuarantineFilter::Pending, 10, 0).unwrap();
        assert!(pending.is_empty(), "rejected should not appear in Pending");
    }

    #[test]
    fn pending_count_matches() {
        let conn = open();
        let n = quarantine_pending_count(&conn).unwrap();
        assert_eq!(n, 0);

        insert_quarantine(&conn, &sample(QuarantineReason::LowConfidence)).unwrap();
        let n = quarantine_pending_count(&conn).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn prune_removes_old_reviewed() {
        let conn = open();

        // Insert and immediately review
        insert_quarantine(&conn, &sample(QuarantineReason::LowConfidence)).unwrap();
        let id = list_quarantine(&conn, QuarantineFilter::All, 1, 0).unwrap()[0].id;
        reject_quarantine(&conn, id).unwrap();

        // Force the created_at to be ancient
        conn.execute(
            "UPDATE quarantine SET created_at = '2020-01-01 00:00:00' WHERE id = ?1",
            params![id],
        )
        .unwrap();

        let deleted = prune_quarantine(&conn, 30).unwrap();
        assert_eq!(deleted, 1);

        let stats = quarantine_stats(&conn).unwrap();
        assert_eq!(stats.total, 0);
    }

    #[test]
    fn reason_round_trips() {
        for reason in [
            QuarantineReason::BelowSfThresh,
            QuarantineReason::LowConfidence,
            QuarantineReason::Manual,
        ] {
            let s = reason.as_str();
            let back = QuarantineReason::from_db_str(s);
            assert_eq!(reason, back);
        }
    }
}
