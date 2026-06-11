//! Store-and-forward queue for outbound uploads that survived a network
//! failure (migration 19).
//!
//! A field station on flaky Wi-Fi/LTE loses every `BirdWeather` upload that
//! fails after its in-flight retries. Those payloads are parked here and
//! replayed by the binary's background drainer once the network returns —
//! bounded in size and age so a weeks-long outage can never grow the
//! database without limit.
//!
//! Scope is deliberate: only channels where late delivery is *correct* may
//! queue. `BirdWeather` qualifies (append-only community-science record;
//! payloads carry their own timestamp). MQTT and Apprise/email stay
//! fire-and-forget by design — they are live telemetry and look-now alerts,
//! and replaying them hours later is worse than dropping them.

use rusqlite::{Connection, OptionalExtension, params};
use std::fmt;

/// Maximum replay attempts before an entry is dropped as undeliverable.
/// With the capped backoff below this spans several days of outage.
pub const MAX_ATTEMPTS: u32 = 48;

/// Hard cap on rows kept per kind; the oldest beyond it are pruned at
/// enqueue time so the queue stays bounded however long the outage lasts.
pub const MAX_QUEUED_PER_KIND: u32 = 5_000;

/// Replay backoff for an entry that has failed `attempts` times: 1 min
/// doubling to a 1 h ceiling, so a returning uplink is rediscovered quickly
/// while a dead one is probed gently.
#[must_use]
pub fn replay_backoff_secs(attempts: u32) -> u64 {
    const BASE: u64 = 60;
    const CAP: u64 = 3_600;
    let shift = attempts.saturating_sub(1).min(6);
    (BASE << shift).min(CAP)
}

/// Errors from outbound-queue operations.
#[derive(Debug)]
pub enum QueueError {
    /// `SQLite` error.
    Sqlite(rusqlite::Error),
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "outbound queue error: {e}"),
        }
    }
}

impl std::error::Error for QueueError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for QueueError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// One queued payload awaiting replay.
#[derive(Debug, Clone)]
pub struct QueuedItem {
    /// Row id (stable handle for `delete` / `mark_failure`).
    pub id: i64,
    /// Channel tag (e.g. `birdweather`).
    pub kind: String,
    /// Channel-specific JSON payload, exactly as enqueued.
    pub payload: String,
    /// Replay attempts made so far (not the original in-flight retries).
    pub attempts: u32,
}

/// Park a failed payload for later replay.
///
/// Also prunes the oldest rows beyond [`MAX_QUEUED_PER_KIND`] for this kind,
/// so the queue stays bounded regardless of outage length — newest data is
/// the most valuable, and the upstream record can backfill the rest from the
/// local database if it ever matters.
///
/// # Errors
///
/// Returns [`QueueError::Sqlite`] when the insert or prune fails.
pub fn enqueue(
    conn: &Connection,
    kind: &str,
    payload: &str,
    now_unix: u64,
) -> Result<(), QueueError> {
    conn.execute(
        "INSERT INTO outbound_queue (kind, payload, next_attempt_at) VALUES (?1, ?2, ?3)",
        params![kind, payload, i64::try_from(now_unix).unwrap_or(i64::MAX)],
    )?;
    conn.execute(
        "DELETE FROM outbound_queue WHERE kind = ?1 AND id NOT IN (
             SELECT id FROM outbound_queue WHERE kind = ?1 ORDER BY id DESC LIMIT ?2
         )",
        params![kind, MAX_QUEUED_PER_KIND],
    )?;
    Ok(())
}

/// Fetch up to `limit` entries of `kind` whose backoff has elapsed, oldest
/// first (preserving upload order across an outage).
///
/// # Errors
///
/// Returns [`QueueError::Sqlite`] when the query fails.
pub fn due(
    conn: &Connection,
    kind: &str,
    now_unix: u64,
    limit: u32,
) -> Result<Vec<QueuedItem>, QueueError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, payload, attempts FROM outbound_queue
         WHERE kind = ?1 AND next_attempt_at <= ?2
         ORDER BY id ASC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(
            params![kind, i64::try_from(now_unix).unwrap_or(i64::MAX), limit],
            |row| {
                Ok(QueuedItem {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    payload: row.get(2)?,
                    attempts: row.get::<_, i64>(3)?.try_into().unwrap_or(u32::MAX),
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Remove a successfully replayed entry.
///
/// # Errors
///
/// Returns [`QueueError::Sqlite`] when the delete fails.
pub fn delete(conn: &Connection, id: i64) -> Result<(), QueueError> {
    conn.execute("DELETE FROM outbound_queue WHERE id = ?1", params![id])?;
    Ok(())
}

/// Record a failed replay: bump the attempt counter, arm the backoff,
/// remember the error.
///
/// Entries that exhaust [`MAX_ATTEMPTS`] are dropped as undeliverable
/// (returns `true` when that happened, so the caller can log the final
/// disposition loudly).
///
/// Read-then-write is safe here: the background drainer is the only writer
/// that touches existing rows, and `SQLite` serializes writers regardless.
///
/// # Errors
///
/// Returns [`QueueError::Sqlite`] when the update fails.
pub fn mark_failure(
    conn: &Connection,
    id: i64,
    error: &str,
    now_unix: u64,
) -> Result<bool, QueueError> {
    let attempts: u32 = conn
        .query_row(
            "SELECT attempts FROM outbound_queue WHERE id = ?1",
            params![id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .and_then(|a| u32::try_from(a).ok())
        .map_or(0, |a| a.saturating_add(1));

    if attempts >= MAX_ATTEMPTS {
        conn.execute("DELETE FROM outbound_queue WHERE id = ?1", params![id])?;
        return Ok(true);
    }

    let delay = i64::try_from(replay_backoff_secs(attempts)).unwrap_or(i64::MAX);
    conn.execute(
        "UPDATE outbound_queue
         SET attempts = ?2, last_error = ?3, next_attempt_at = ?4 + ?5
         WHERE id = ?1",
        params![
            id,
            attempts,
            error,
            i64::try_from(now_unix).unwrap_or(i64::MAX),
            delay
        ],
    )?;
    Ok(false)
}

/// Number of entries currently parked for `kind` (health/metrics surface).
///
/// # Errors
///
/// Returns [`QueueError::Sqlite`] when the count fails.
pub fn depth(conn: &Connection, kind: &str) -> Result<u64, QueueError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM outbound_queue WHERE kind = ?1",
        params![kind],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(n).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn enqueue_then_due_round_trips_payload() {
        let conn = test_conn();
        enqueue(&conn, "birdweather", r#"{"species":"Pica pica"}"#, 1_000).unwrap();
        let items = due(&conn, "birdweather", 1_000, 10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "birdweather");
        assert_eq!(items[0].payload, r#"{"species":"Pica pica"}"#);
        assert_eq!(items[0].attempts, 0);
        assert_eq!(depth(&conn, "birdweather").unwrap(), 1);
    }

    #[test]
    fn due_respects_kind_order_and_limit() {
        let conn = test_conn();
        for i in 0..5 {
            enqueue(&conn, "birdweather", &format!("p{i}"), 1_000).unwrap();
        }
        enqueue(&conn, "other", "x", 1_000).unwrap();
        let items = due(&conn, "birdweather", 1_000, 3).unwrap();
        assert_eq!(items.len(), 3, "limit respected");
        // Oldest first: upload order is preserved across the outage.
        assert_eq!(items[0].payload, "p0");
        assert_eq!(items[2].payload, "p2");
        assert!(items.iter().all(|i| i.kind == "birdweather"));
    }

    #[test]
    fn delete_removes_replayed_entry() {
        let conn = test_conn();
        enqueue(&conn, "birdweather", "p", 1_000).unwrap();
        let id = due(&conn, "birdweather", 1_000, 1).unwrap()[0].id;
        delete(&conn, id).unwrap();
        assert_eq!(depth(&conn, "birdweather").unwrap(), 0);
    }

    #[test]
    fn mark_failure_backs_off_and_preserves_entry() {
        let conn = test_conn();
        enqueue(&conn, "birdweather", "p", 1_000).unwrap();
        let id = due(&conn, "birdweather", 1_000, 1).unwrap()[0].id;

        let dropped = mark_failure(&conn, id, "connect timeout", 1_000).unwrap();
        assert!(!dropped);
        // Not due again until the backoff elapses…
        assert!(due(&conn, "birdweather", 1_000, 10).unwrap().is_empty());
        assert!(
            due(&conn, "birdweather", 1_000 + replay_backoff_secs(1), 10)
                .unwrap()
                .first()
                .is_some_and(|i| i.attempts == 1),
            "due again once the first backoff elapses, with the attempt recorded"
        );
    }

    #[test]
    fn entries_exhausting_max_attempts_are_dropped() {
        let conn = test_conn();
        enqueue(&conn, "birdweather", "p", 0).unwrap();
        let id = due(&conn, "birdweather", 0, 1).unwrap()[0].id;
        let mut dropped = false;
        for attempt in 0..MAX_ATTEMPTS {
            dropped = mark_failure(&conn, id, "still down", u64::from(attempt)).unwrap();
        }
        assert!(dropped, "the final attempt reports the drop");
        assert_eq!(depth(&conn, "birdweather").unwrap(), 0);
    }

    #[test]
    fn enqueue_prunes_oldest_beyond_cap() {
        let conn = test_conn();
        for i in 0..=MAX_QUEUED_PER_KIND {
            enqueue(&conn, "birdweather", &format!("p{i}"), 1_000).unwrap();
        }
        assert_eq!(
            depth(&conn, "birdweather").unwrap(),
            u64::from(MAX_QUEUED_PER_KIND),
            "bounded at the cap"
        );
        let oldest = due(&conn, "birdweather", 1_000, 1).unwrap();
        assert_eq!(oldest[0].payload, "p1", "the OLDEST entry was pruned");
    }

    #[test]
    fn replay_backoff_doubles_to_one_hour_cap() {
        assert_eq!(replay_backoff_secs(1), 60);
        assert_eq!(replay_backoff_secs(2), 120);
        assert_eq!(replay_backoff_secs(7), 3_600);
        assert_eq!(replay_backoff_secs(u32::MAX), 3_600, "no overflow");
    }
}
