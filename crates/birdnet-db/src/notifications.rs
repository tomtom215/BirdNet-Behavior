//! Notification log database queries.
//!
//! Provides a structured log of all notification attempts — BirdWeather uploads,
//! Apprise pushes, and any future channels — stored in the local SQLite database.
//! The log is append-only and kept for 90 days by default.

use rusqlite::{Connection, params};
use std::fmt;

/// Errors from notification log operations.
#[derive(Debug)]
pub enum NotifError {
    /// `SQLite` error.
    Sqlite(rusqlite::Error),
}

impl fmt::Display for NotifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "notification log error: {e}"),
        }
    }
}

impl std::error::Error for NotifError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for NotifError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// Notification outcome status.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifStatus {
    /// Successfully sent.
    Sent,
    /// Delivery attempt failed.
    Failed,
    /// Skipped (e.g. confidence below threshold, duplicate suppression).
    Skipped,
}

impl NotifStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

impl fmt::Display for NotifStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A notification log entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NotifEntry {
    /// Row ID.
    pub id: i64,
    /// UTC timestamp of the notification attempt.
    pub sent_at: String,
    /// Channel name (e.g. `"birdweather"`, `"apprise"`, `"email"`).
    pub channel: String,
    /// Common name of the detected species (if applicable).
    pub species_com_name: Option<String>,
    /// Scientific name of the detected species (if applicable).
    pub species_sci_name: Option<String>,
    /// Detection confidence (0–1, if applicable).
    pub confidence: Option<f64>,
    /// Detection date (YYYY-MM-DD).
    pub detection_date: Option<String>,
    /// Detection time (HH:MM:SS).
    pub detection_time: Option<String>,
    /// Delivery status.
    pub status: String,
    /// Human-readable notification message or subject line.
    pub message: Option<String>,
    /// Error message (only set for `status = "failed"`).
    pub error: Option<String>,
}

/// Parameters for inserting a notification log entry.
#[derive(Debug, Clone)]
pub struct NotifRecord<'a> {
    /// Channel name.
    pub channel: &'a str,
    /// Common species name.
    pub species_com_name: Option<&'a str>,
    /// Scientific species name.
    pub species_sci_name: Option<&'a str>,
    /// Detection confidence.
    pub confidence: Option<f64>,
    /// Detection date.
    pub detection_date: Option<&'a str>,
    /// Detection time.
    pub detection_time: Option<&'a str>,
    /// Outcome status.
    pub status: NotifStatus,
    /// Human-readable message.
    pub message: Option<&'a str>,
    /// Error detail (for failures).
    pub error: Option<&'a str>,
}

/// Insert a notification log entry.
///
/// # Errors
///
/// Returns `NotifError` on insert failure.
pub fn log_notification(conn: &Connection, record: &NotifRecord<'_>) -> Result<i64, NotifError> {
    conn.execute(
        "INSERT INTO notification_log
             (channel, species_com_name, species_sci_name, confidence,
              detection_date, detection_time, status, message, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            record.channel,
            record.species_com_name,
            record.species_sci_name,
            record.confidence,
            record.detection_date,
            record.detection_time,
            record.status.as_str(),
            record.message,
            record.error,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Query recent notification log entries, newest first.
///
/// # Errors
///
/// Returns `NotifError` on query failure.
pub fn recent_notifications(
    conn: &Connection,
    limit: u32,
    offset: u32,
) -> Result<Vec<NotifEntry>, NotifError> {
    let mut stmt = conn.prepare(
        "SELECT id, sent_at, channel, species_com_name, species_sci_name, confidence,
                detection_date, detection_time, status, message, error
         FROM notification_log
         ORDER BY sent_at DESC
         LIMIT ?1 OFFSET ?2",
    )?;
    let rows = stmt
        .query_map(params![limit, offset], |row| {
            Ok(NotifEntry {
                id: row.get(0)?,
                sent_at: row.get(1)?,
                channel: row.get(2)?,
                species_com_name: row.get(3)?,
                species_sci_name: row.get(4)?,
                confidence: row.get(5)?,
                detection_date: row.get(6)?,
                detection_time: row.get(7)?,
                status: row.get(8)?,
                message: row.get(9)?,
                error: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Query notification log entries for a specific channel.
///
/// # Errors
///
/// Returns `NotifError` on query failure.
pub fn notifications_by_channel(
    conn: &Connection,
    channel: &str,
    limit: u32,
) -> Result<Vec<NotifEntry>, NotifError> {
    let mut stmt = conn.prepare(
        "SELECT id, sent_at, channel, species_com_name, species_sci_name, confidence,
                detection_date, detection_time, status, message, error
         FROM notification_log
         WHERE channel = ?1
         ORDER BY sent_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![channel, limit], |row| {
            Ok(NotifEntry {
                id: row.get(0)?,
                sent_at: row.get(1)?,
                channel: row.get(2)?,
                species_com_name: row.get(3)?,
                species_sci_name: row.get(4)?,
                confidence: row.get(5)?,
                detection_date: row.get(6)?,
                detection_time: row.get(7)?,
                status: row.get(8)?,
                message: row.get(9)?,
                error: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Count notifications grouped by status over the last `days` days.
///
/// Returns `(sent, failed, skipped)`.
///
/// # Errors
///
/// Returns `NotifError` on query failure.
pub fn notification_stats(
    conn: &Connection,
    days: u32,
) -> Result<(i64, i64, i64), NotifError> {
    let mut sent = 0i64;
    let mut failed = 0i64;
    let mut skipped = 0i64;

    let mut stmt = conn.prepare(
        "SELECT status, COUNT(*) FROM notification_log
         WHERE sent_at >= datetime('now', '-' || ?1 || ' days')
         GROUP BY status",
    )?;
    let rows = stmt.query_map(params![days], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (status, count) = row?;
        match status.as_str() {
            "sent" => sent = count,
            "failed" => failed = count,
            "skipped" => skipped = count,
            _ => {}
        }
    }
    Ok((sent, failed, skipped))
}

/// Prune notification log entries older than `days` days.
///
/// # Errors
///
/// Returns `NotifError` on delete failure.
pub fn prune_old_notifications(conn: &Connection, days: u32) -> Result<u64, NotifError> {
    let deleted = conn.execute(
        "DELETE FROM notification_log WHERE sent_at < datetime('now', '-' || ?1 || ' days')",
        params![days],
    )?;
    Ok(u64::try_from(deleted).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;
    use crate::migration::migrate;

    fn test_db() -> Connection {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        migrate(&conn).unwrap();
        // Keep tmp alive by leaking (test-only pattern acceptable here).
        std::mem::forget(tmp);
        conn
    }

    fn make_record(channel: &str, status: NotifStatus) -> NotifRecord<'_> {
        NotifRecord {
            channel,
            species_com_name: Some("European Robin"),
            species_sci_name: Some("Erithacus rubecula"),
            confidence: Some(0.92),
            detection_date: Some("2026-03-13"),
            detection_time: Some("06:15:00"),
            status,
            message: Some("Detected: European Robin (0.92)"),
            error: None,
        }
    }

    #[test]
    fn log_and_retrieve() {
        let conn = test_db();
        let id = log_notification(&conn, &make_record("birdweather", NotifStatus::Sent)).unwrap();
        assert!(id > 0);
        let entries = recent_notifications(&conn, 10, 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].channel, "birdweather");
        assert_eq!(entries[0].status, "sent");
    }

    #[test]
    fn filter_by_channel() {
        let conn = test_db();
        log_notification(&conn, &make_record("birdweather", NotifStatus::Sent)).unwrap();
        log_notification(&conn, &make_record("apprise", NotifStatus::Sent)).unwrap();
        let bw = notifications_by_channel(&conn, "birdweather", 10).unwrap();
        assert_eq!(bw.len(), 1);
    }

    #[test]
    fn stats_counts_by_status() {
        let conn = test_db();
        log_notification(&conn, &make_record("birdweather", NotifStatus::Sent)).unwrap();
        log_notification(&conn, &make_record("apprise", NotifStatus::Failed)).unwrap();
        log_notification(&conn, &make_record("birdweather", NotifStatus::Skipped)).unwrap();
        let (sent, failed, skipped) = notification_stats(&conn, 30).unwrap();
        assert_eq!(sent, 1);
        assert_eq!(failed, 1);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn prune_removes_old_entries() {
        let conn = test_db();
        // Insert an old entry by overriding sent_at.
        conn.execute(
            "INSERT INTO notification_log (channel, status, sent_at)
             VALUES ('test', 'sent', '2020-01-01 00:00:00')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO notification_log (channel, status) VALUES ('test', 'sent')",
            [],
        )
        .unwrap();
        let deleted = prune_old_notifications(&conn, 30).unwrap();
        assert_eq!(deleted, 1);
    }
}
