//! Detection-review (confirm / reject) triage queries.
//!
//! A *review* is a reviewer's verdict on an individual detection — `confirmed`
//! (the species ID looks right) or `rejected` (likely a misidentification),
//! identified by the same `(date, time, sci_name)` triple the rest of the UI
//! keys on. Unlike quarantine — which gates uncertain rows *out* of
//! `detections` before they are ever admitted — a review is a non-destructive
//! annotation layered on detections that already exist.
//!
//! # Workflow
//!
//! 1. **Review** — the operator opens a detection (or the `/detection-reviews`
//!    triage queue) and records a verdict via [`set_detection_review`].
//! 2. **Inspect** — verdicts surface on the detection-detail page
//!    ([`get_detection_review`]) and in the queue ([`recent_detection_reviews`],
//!    [`detection_review_counts`]).
//! 3. **Queue** — [`unreviewed_recent_detections`] lists detections still
//!    awaiting a verdict.
//! 4. **Undo** — [`clear_detection_review`] removes a verdict.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A reviewer's verdict on a detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStatus {
    /// The species identification looks correct.
    Confirmed,
    /// The detection is a likely misidentification.
    Rejected,
}

impl ReviewStatus {
    /// Canonical string stored in the database.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
        }
    }

    /// Human-readable label for UI display.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Confirmed => "Confirmed",
            Self::Rejected => "Rejected",
        }
    }

    /// Parse from an untrusted string (e.g. a form field). Unknown values map
    /// to `None` so the caller can reject bad input rather than guess.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "confirmed" => Some(Self::Confirmed),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// A detection-review row read from the database.
#[derive(Debug, Clone)]
pub struct DetectionReview {
    /// Detection date (YYYY-MM-DD).
    pub date: String,
    /// Detection time (HH:MM:SS).
    pub time: String,
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Verdict (`confirmed` / `rejected`).
    pub status: String,
    /// Optional free-text reviewer note.
    pub notes: Option<String>,
    /// When the verdict was last set (`datetime('now')`).
    pub reviewed_at: String,
}

/// A recent detection still awaiting a review verdict.
#[derive(Debug, Clone)]
pub struct UnreviewedDetection {
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
    /// Source audio file path, if any.
    pub file_name: Option<String>,
}

// ---------------------------------------------------------------------------
// Write operations
// ---------------------------------------------------------------------------

/// Record (or update) a verdict for the detection identified by
/// `(date, time, sci_name)`. Idempotent: re-reviewing the same detection
/// replaces the prior verdict via `ON CONFLICT`.
///
/// # Errors
///
/// Returns [`DbError`] on write failure.
pub fn set_detection_review(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
    com_name: &str,
    status: ReviewStatus,
    notes: Option<&str>,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO detection_reviews
            (date, time, sci_name, com_name, status, notes, reviewed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
         ON CONFLICT(date, time, sci_name) DO UPDATE SET
            status = excluded.status,
            com_name = excluded.com_name,
            notes = excluded.notes,
            reviewed_at = datetime('now')",
        params![date, time, sci_name, com_name, status.as_str(), notes],
    )?;
    // Mirror onto the detection itself (migration 26). `detection_reviews` stays
    // the record of who said what and when; this is the current verdict, in the
    // one place every analytic can filter on cheaply and identically in both
    // stores. Without it a verdict is recorded and applied to nothing.
    conn.execute(
        "UPDATE detections SET review_verdict = ?4
          WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3",
        params![date, time, sci_name, status.as_str()],
    )?;
    Ok(())
}

/// Remove a verdict, returning the detection to the unreviewed queue.
///
/// # Errors
///
/// Returns [`DbError`] on write failure.
pub fn clear_detection_review(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM detection_reviews WHERE date = ?1 AND time = ?2 AND sci_name = ?3",
        params![date, time, sci_name],
    )?;
    // Return the detection to "unreviewed" in the denormalised copy too, or the
    // exclusion would outlive the verdict that justified it.
    conn.execute(
        "UPDATE detections SET review_verdict = NULL
          WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3",
        params![date, time, sci_name],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Read operations
// ---------------------------------------------------------------------------

/// Count detections carrying a `rejected` verdict.
///
/// Read from `detections.review_verdict` — the denormalised copy the analytics
/// filter on — rather than from `detection_reviews`, because the drift check
/// this backs is asking whether the *two stores' filters* agree, not whether the
/// review log does.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn rejected_detection_count(conn: &Connection) -> Result<u64, DbError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE review_verdict = 'rejected'",
        [],
        |r| r.get(0),
    )?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// Fetch the verdict for a single detection, or `None` if unreviewed.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn get_detection_review(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<Option<DetectionReview>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT date, time, sci_name, com_name, status, notes, reviewed_at
         FROM detection_reviews WHERE date = ?1 AND time = ?2 AND sci_name = ?3",
    )?;
    let mut rows = stmt.query_map(params![date, time, sci_name], map_review)?;
    match rows.next() {
        Some(r) => Ok(Some(r?)),
        None => Ok(None),
    }
}

/// List the most recent verdicts, newest first.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn recent_detection_reviews(
    conn: &Connection,
    limit: u32,
) -> Result<Vec<DetectionReview>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT date, time, sci_name, com_name, status, notes, reviewed_at
         FROM detection_reviews
         ORDER BY reviewed_at DESC, id DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], map_review)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// A page of recorded verdicts, newest first, optionally narrowed to one status.
///
/// # Why this exists beside [`recent_detection_reviews`]
///
/// That function takes only a limit, and the review page passed 25. Since the
/// aggregates started excluding rejections, the "Recent verdicts" list became
/// the *only* surface in the app that lists a rejected detection — so the 26th
/// rejection was reachable through no page at all, only by a URL the operator
/// had happened to keep. A verdict you cannot find is a verdict you cannot
/// undo, and the whole design rests on rejection being reversible.
///
/// `status` narrows to one verdict because "show me what I rejected" is the
/// question that actually gets asked; `None` returns both.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn detection_reviews_page(
    conn: &Connection,
    status: Option<ReviewStatus>,
    limit: u32,
    offset: u32,
) -> Result<Vec<DetectionReview>, DbError> {
    // Two prepared statements rather than one with an `IS NULL OR` predicate:
    // the latter is a filter SQLite cannot use the status index for, and this
    // list is read on a page an operator pages through.
    let mut stmt = match status {
        Some(_) => conn.prepare(
            "SELECT date, time, sci_name, com_name, status, notes, reviewed_at
               FROM detection_reviews
              WHERE status = ?3
              ORDER BY reviewed_at DESC, id DESC
              LIMIT ?1 OFFSET ?2",
        )?,
        None => conn.prepare(
            "SELECT date, time, sci_name, com_name, status, notes, reviewed_at
               FROM detection_reviews
              ORDER BY reviewed_at DESC, id DESC
              LIMIT ?1 OFFSET ?2",
        )?,
    };
    let mut out = Vec::new();
    match status {
        Some(st) => {
            for r in stmt.query_map(params![limit, offset, st.as_str()], map_review)? {
                out.push(r?);
            }
        }
        None => {
            for r in stmt.query_map(params![limit, offset], map_review)? {
                out.push(r?);
            }
        }
    }
    Ok(out)
}

/// How many verdicts are recorded, optionally narrowed to one status.
///
/// Paired with [`detection_reviews_page`] so the page can say whether there is
/// more to show. A list that silently ends is the failure this replaces.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn detection_review_total(
    conn: &Connection,
    status: Option<ReviewStatus>,
) -> Result<i64, DbError> {
    let n = status.map_or_else(
        || {
            conn.query_row("SELECT COUNT(*) FROM detection_reviews", [], |row| {
                row.get(0)
            })
        },
        |st| {
            conn.query_row(
                "SELECT COUNT(*) FROM detection_reviews WHERE status = ?1",
                params![st.as_str()],
                |row| row.get(0),
            )
        },
    )?;
    Ok(n)
}

/// Count verdicts as `(confirmed, rejected)`.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn detection_review_counts(conn: &Connection) -> Result<(i64, i64), DbError> {
    let confirmed = conn.query_row(
        "SELECT COUNT(*) FROM detection_reviews WHERE status = 'confirmed'",
        [],
        |row| row.get(0),
    )?;
    let rejected = conn.query_row(
        "SELECT COUNT(*) FROM detection_reviews WHERE status = 'rejected'",
        [],
        |row| row.get(0),
    )?;
    Ok((confirmed, rejected))
}

/// List recent detections that have no verdict yet — the triage queue.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn unreviewed_recent_detections(
    conn: &Connection,
    limit: u32,
) -> Result<Vec<UnreviewedDetection>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT d.Date, d.Time, d.Sci_Name, d.Com_Name, d.Confidence, d.File_Name
         FROM detections d
         LEFT JOIN detection_reviews r
            ON r.date = d.Date AND r.time = d.Time AND r.sci_name = d.Sci_Name
         WHERE r.id IS NULL
         ORDER BY d.Date DESC, d.Time DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(UnreviewedDetection {
            date: row.get(0)?,
            time: row.get(1)?,
            sci_name: row.get(2)?,
            com_name: row.get(3)?,
            confidence: row.get(4)?,
            file_name: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn map_review(row: &rusqlite::Row<'_>) -> rusqlite::Result<DetectionReview> {
    Ok(DetectionReview {
        date: row.get(0)?,
        time: row.get(1)?,
        sci_name: row.get(2)?,
        com_name: row.get(3)?,
        status: row.get(4)?,
        notes: row.get(5)?,
        reviewed_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::migrate;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn seed_detection(conn: &Connection, date: &str, time: &str, sci: &str, com: &str) {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES (?1, ?2, ?3, ?4, 0.8)",
            params![date, time, sci, com],
        )
        .unwrap();
    }

    /// Every recorded verdict must be reachable, not just the newest page.
    ///
    /// The review page asked for 25 and showed them. Once the aggregates began
    /// excluding rejections, that list became the only surface in the app that
    /// lists a rejected detection — so the 26th rejection was reachable through
    /// no page at all. A verdict you cannot find is one you cannot undo.
    #[test]
    fn every_verdict_is_reachable_by_paging() {
        let conn = db();
        for i in 0..60 {
            let time = format!("{:02}:{:02}:00", i / 60, i % 60);
            let status = if i % 3 == 0 {
                ReviewStatus::Rejected
            } else {
                ReviewStatus::Confirmed
            };
            set_detection_review(
                &conn,
                "2026-01-01",
                &time,
                "Turdus merula",
                "Blackbird",
                status,
                None,
            )
            .unwrap();
        }
        assert_eq!(detection_review_total(&conn, None).unwrap(), 60);

        // Page through in 25s and collect everything.
        let mut seen = Vec::new();
        for page in 0..3 {
            let rows = detection_reviews_page(&conn, None, 25, page * 25).unwrap();
            seen.extend(rows.into_iter().map(|r| r.time));
        }
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            60,
            "paging must reach every verdict, with no gaps or repeats"
        );
        // …and the page past the end is empty rather than an error.
        assert!(
            detection_reviews_page(&conn, None, 25, 75)
                .unwrap()
                .is_empty()
        );
    }

    /// "Show me what I rejected" is the question that actually gets asked, and
    /// on a station with a long confirm streak the rejections are exactly the
    /// rows an unfiltered newest-first list buries.
    #[test]
    fn verdicts_can_be_narrowed_to_one_status() {
        let conn = db();
        for i in 0..60 {
            let time = format!("{:02}:{:02}:00", i / 60, i % 60);
            let status = if i % 3 == 0 {
                ReviewStatus::Rejected
            } else {
                ReviewStatus::Confirmed
            };
            set_detection_review(
                &conn,
                "2026-01-01",
                &time,
                "Turdus merula",
                "Blackbird",
                status,
                None,
            )
            .unwrap();
        }
        assert_eq!(
            detection_review_total(&conn, Some(ReviewStatus::Rejected)).unwrap(),
            20
        );
        let rejected = detection_reviews_page(&conn, Some(ReviewStatus::Rejected), 25, 0).unwrap();
        assert_eq!(rejected.len(), 20);
        assert!(
            rejected.iter().all(|r| r.status == "rejected"),
            "the filter must not leak confirmations"
        );
        // The counterpart, so the filter cannot degrade into "return nothing".
        assert_eq!(
            detection_reviews_page(&conn, Some(ReviewStatus::Confirmed), 100, 0)
                .unwrap()
                .len(),
            40
        );
    }

    #[test]
    fn an_empty_review_table_pages_cleanly() {
        let conn = db();
        assert_eq!(detection_review_total(&conn, None).unwrap(), 0);
        assert!(
            detection_reviews_page(&conn, None, 25, 0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn set_get_and_upsert_verdict() {
        let conn = db();
        set_detection_review(
            &conn,
            "2026-05-20",
            "06:14:08",
            "Pica pica",
            "Eurasian Magpie",
            ReviewStatus::Confirmed,
            Some("clear call"),
        )
        .unwrap();
        let r = get_detection_review(&conn, "2026-05-20", "06:14:08", "Pica pica")
            .unwrap()
            .expect("verdict present");
        assert_eq!(r.status, "confirmed");
        assert_eq!(r.notes.as_deref(), Some("clear call"));

        // Re-review updates in place rather than inserting a second row.
        set_detection_review(
            &conn,
            "2026-05-20",
            "06:14:08",
            "Pica pica",
            "Eurasian Magpie",
            ReviewStatus::Rejected,
            None,
        )
        .unwrap();
        let (confirmed, rejected) = detection_review_counts(&conn).unwrap();
        assert_eq!((confirmed, rejected), (0, 1));
        let r = get_detection_review(&conn, "2026-05-20", "06:14:08", "Pica pica")
            .unwrap()
            .unwrap();
        assert_eq!(r.status, "rejected");
    }

    #[test]
    fn unreviewed_excludes_reviewed_and_clear_restores() {
        let conn = db();
        seed_detection(
            &conn,
            "2026-05-20",
            "06:00:00",
            "Pica pica",
            "Eurasian Magpie",
        );
        seed_detection(
            &conn,
            "2026-05-20",
            "06:05:00",
            "Turdus merula",
            "Eurasian Blackbird",
        );

        assert_eq!(unreviewed_recent_detections(&conn, 10).unwrap().len(), 2);

        set_detection_review(
            &conn,
            "2026-05-20",
            "06:00:00",
            "Pica pica",
            "Eurasian Magpie",
            ReviewStatus::Confirmed,
            None,
        )
        .unwrap();
        let pending = unreviewed_recent_detections(&conn, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sci_name, "Turdus merula");

        clear_detection_review(&conn, "2026-05-20", "06:00:00", "Pica pica").unwrap();
        assert_eq!(unreviewed_recent_detections(&conn, 10).unwrap().len(), 2);
    }

    #[test]
    fn parse_rejects_unknown_status() {
        assert_eq!(
            ReviewStatus::parse("confirmed"),
            Some(ReviewStatus::Confirmed)
        );
        assert_eq!(
            ReviewStatus::parse("rejected"),
            Some(ReviewStatus::Rejected)
        );
        assert_eq!(ReviewStatus::parse("garbage"), None);
    }
}
