//! Detection-review (confirm / reject) triage queries.
//!
//! A *review* is a reviewer's verdict on **one detection**: `confirmed` (the
//! species ID looks right) or `rejected` (likely a misidentification). The
//! detection is named by [`DetectionKey`]: date, time, species **and clip**.
//! The triple alone is not a key. Two microphones that hear one bird in the
//! same second write two detections that share it and differ only in the clip
//! they point at, and a verdict addressed by the triple reached both: a
//! rejection of the one the reviewer listened to also withdrew from every
//! count a detection nobody had listened to.
//!
//! Unlike quarantine, which gates uncertain rows *out* of `detections` before
//! they are ever admitted, a review is a non-destructive annotation layered on
//! detections that already exist, and no verdict is written about one that does
//! not.
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
//!
//! # Verdicts attached to no detection: `detached`
//!
//! A verdict can outlive the detection it was about, and is kept, not
//! deleted, because it records what a person decided:
//!
//! * `relabelled` — the detection was given another species, and a verdict on
//!   the old one says nothing about the new one (see migration 59);
//! * `ambiguous` — recorded before reviews named a clip, when more than one
//!   detection shared its date, time and species. Which one the reviewer meant
//!   cannot be recovered, so none carries it and each is unreviewed again;
//! * `orphaned` — recorded before reviews named a clip, and its detection no
//!   longer exists.
//!
//! A detached verdict is listed so it can be seen and dismissed
//! ([`dismiss_detached_review`]); it is in no count and excludes nothing.

use rusqlite::{Connection, params};

use super::DetectionKey;
use super::detections::KEY_WHERE;
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

/// Why a recorded verdict is attached to no detection. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detached {
    /// The detection it was about was given another species.
    Relabelled,
    /// More than one detection shared its date, time and species when it was
    /// recorded, and which one was meant cannot be known.
    Ambiguous,
    /// The detection it was about no longer exists.
    Orphaned,
}

impl Detached {
    /// The value stored in `detection_reviews.detached`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Relabelled => "relabelled",
            Self::Ambiguous => "ambiguous",
            Self::Orphaned => "orphaned",
        }
    }

    /// Read a stored value; `None` for anything the column's `CHECK` forbids.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "relabelled" => Some(Self::Relabelled),
            "ambiguous" => Some(Self::Ambiguous),
            "orphaned" => Some(Self::Orphaned),
            _ => None,
        }
    }
}

/// A detection-review row read from the database.
#[derive(Debug, Clone)]
pub struct DetectionReview {
    /// Row id, which names a detached verdict for [`dismiss_detached_review`].
    pub id: i64,
    /// Detection date (YYYY-MM-DD).
    pub date: String,
    /// Detection time (HH:MM:SS).
    pub time: String,
    /// Scientific name the detection had when it was reviewed.
    pub sci_name: String,
    /// The clip of the detection reviewed; `""` for a detection with no clip.
    /// Meaningless when [`detached`](Self::detached) is `Ambiguous` or
    /// `Orphaned`: those name no clip.
    pub file_name: String,
    /// Common name.
    pub com_name: String,
    /// Verdict (`confirmed` / `rejected`).
    pub status: String,
    /// Optional free-text reviewer note.
    pub notes: Option<String>,
    /// When the verdict was last set (`datetime('now')`).
    pub reviewed_at: String,
    /// `None` while the verdict is attached to its detection.
    pub detached: Option<Detached>,
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

/// The columns [`map_review`] reads, in its order.
const REVIEW_COLS: &str =
    "id, date, time, sci_name, file_name, com_name, status, notes, reviewed_at, detached";

// ---------------------------------------------------------------------------
// Write operations
// ---------------------------------------------------------------------------

/// Record (or update) a verdict for the one detection `key` names. Idempotent:
/// re-reviewing the same detection replaces the prior verdict via `ON CONFLICT`.
///
/// Returns `false`, writing nothing, when no detection has that key: a verdict
/// about nothing is not recorded, and a caller that holds a stale link is told
/// so instead of being thanked.
///
/// # Errors
///
/// Returns [`DbError`] on write failure.
pub fn set_detection_review(
    conn: &Connection,
    key: &DetectionKey<'_>,
    com_name: &str,
    status: ReviewStatus,
    notes: Option<&str>,
) -> Result<bool, DbError> {
    // One transaction: a verdict whose mirror failed would show as reviewed in
    // the queue while every analytic still counted the detection (DB8).
    let tx = conn.unchecked_transaction()?;
    // Mirror onto the detection itself (migration 26). `detection_reviews`
    // stays the record of who said what and when; this is the current verdict,
    // in the one place every analytic can filter on cheaply and identically in
    // both stores. Without it a verdict is recorded and applied to nothing.
    // First, because it is also the existence check.
    let mirrored = tx.execute(
        &format!("UPDATE detections SET review_verdict = ?5 WHERE {KEY_WHERE}"),
        params![
            key.date,
            key.time,
            key.sci_name,
            key.file_name,
            status.as_str()
        ],
    )?;
    if mirrored == 0 {
        // Dropping the transaction rolls it back; nothing was written.
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO detection_reviews
            (date, time, sci_name, file_name, com_name, status, notes, reviewed_at)
         VALUES (?1, ?2, ?3, COALESCE(?4, ''), ?5, ?6, ?7, datetime('now'))
         ON CONFLICT(date, time, sci_name, file_name) WHERE detached IS NULL DO UPDATE SET
            status = excluded.status,
            com_name = excluded.com_name,
            notes = excluded.notes,
            reviewed_at = datetime('now')",
        params![
            key.date,
            key.time,
            key.sci_name,
            key.file_name,
            com_name,
            status.as_str(),
            notes
        ],
    )?;
    tx.commit()?;
    Ok(true)
}

/// Remove the verdict on the one detection `key` names, returning it to the
/// unreviewed queue.
///
/// Returns `true` if there was a verdict, or a mirrored one, to remove.
///
/// # Errors
///
/// Returns [`DbError`] on write failure.
pub fn clear_detection_review(conn: &Connection, key: &DetectionKey<'_>) -> Result<bool, DbError> {
    let tx = conn.unchecked_transaction()?;
    let removed = tx.execute(
        "DELETE FROM detection_reviews
          WHERE date = ?1 AND time = ?2 AND sci_name = ?3
            AND file_name = COALESCE(?4, '') AND detached IS NULL",
        params![key.date, key.time, key.sci_name, key.file_name],
    )?;
    // Return the detection to "unreviewed" in the denormalised copy too, or the
    // exclusion would outlive the verdict that justified it.
    let cleared = tx.execute(
        &format!("UPDATE detections SET review_verdict = NULL WHERE {KEY_WHERE}"),
        params![key.date, key.time, key.sci_name, key.file_name],
    )?;
    tx.commit()?;
    Ok(removed > 0 || cleared > 0)
}

/// Remove a verdict that is attached to no detection (see the module docs),
/// by its row id. Never removes an attached one: that is
/// [`clear_detection_review`], which also clears the detection's mirror.
///
/// # Errors
///
/// Returns [`DbError`] on write failure.
pub fn dismiss_detached_review(conn: &Connection, id: i64) -> Result<bool, DbError> {
    let n = conn.execute(
        "DELETE FROM detection_reviews WHERE id = ?1 AND detached IS NOT NULL",
        params![id],
    )?;
    Ok(n > 0)
}

// ---------------------------------------------------------------------------
// Read operations
// ---------------------------------------------------------------------------

/// The clips of the detections that share a date, time and species: `None`
/// for a detection with no clip. One entry per distinct clip.
///
/// For a caller that names a detection by the triple alone (an API client
/// written before reviews carried a clip): one entry says which detection it
/// meant, none says there is none, and more than one says the triple does not
/// name a detection and the caller must not be guessed for.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn detection_clips(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<Vec<Option<String>>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT COALESCE(File_Name, '') FROM detections
          WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3
          ORDER BY 1",
    )?;
    let rows = stmt.query_map(params![date, time, sci_name], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        let clip = r?;
        out.push((!clip.is_empty()).then_some(clip));
    }
    Ok(out)
}

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

/// Fetch the verdict on the one detection `key` names, or `None` if
/// unreviewed. A detached verdict is not a detection's verdict and is not
/// returned.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn get_detection_review(
    conn: &Connection,
    key: &DetectionKey<'_>,
) -> Result<Option<DetectionReview>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REVIEW_COLS} FROM detection_reviews
          WHERE date = ?1 AND time = ?2 AND sci_name = ?3
            AND file_name = COALESCE(?4, '') AND detached IS NULL"
    ))?;
    let mut rows = stmt.query_map(
        params![key.date, key.time, key.sci_name, key.file_name],
        map_review,
    )?;
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
    let mut stmt = conn.prepare(&format!(
        "SELECT {REVIEW_COLS}
         FROM detection_reviews
         ORDER BY reviewed_at DESC, id DESC
         LIMIT ?1"
    ))?;
    let rows = stmt.query_map(params![limit], map_review)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// A page of recorded verdicts, newest first, optionally narrowed to one status.
/// Detached verdicts are in the list, marked as such, so they can be found and
/// dismissed.
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
        Some(_) => conn.prepare(&format!(
            "SELECT {REVIEW_COLS}
               FROM detection_reviews
              WHERE status = ?3
              ORDER BY reviewed_at DESC, id DESC
              LIMIT ?1 OFFSET ?2"
        ))?,
        None => conn.prepare(&format!(
            "SELECT {REVIEW_COLS}
               FROM detection_reviews
              ORDER BY reviewed_at DESC, id DESC
              LIMIT ?1 OFFSET ?2"
        ))?,
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

/// How many verdicts are recorded, optionally narrowed to one status. Counts
/// the detached ones too, because [`detection_reviews_page`] lists them.
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

/// Count verdicts that are attached to a detection, as `(confirmed, rejected)`.
/// A detached verdict (see [`detached_review_count`]) is not a verdict on any
/// detection and is in neither.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn detection_review_counts(conn: &Connection) -> Result<(i64, i64), DbError> {
    let confirmed = conn.query_row(
        "SELECT COUNT(*) FROM detection_reviews WHERE status = 'confirmed' AND detached IS NULL",
        [],
        |row| row.get(0),
    )?;
    let rejected = conn.query_row(
        "SELECT COUNT(*) FROM detection_reviews WHERE status = 'rejected' AND detached IS NULL",
        [],
        |row| row.get(0),
    )?;
    Ok((confirmed, rejected))
}

/// How many recorded verdicts are attached to no detection.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn detached_review_count(conn: &Connection) -> Result<i64, DbError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM detection_reviews WHERE detached IS NOT NULL",
        [],
        |row| row.get(0),
    )?)
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
           AND r.file_name = COALESCE(d.File_Name, '') AND r.detached IS NULL
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
    let detached: Option<String> = row.get(9)?;
    Ok(DetectionReview {
        id: row.get(0)?,
        date: row.get(1)?,
        time: row.get(2)?,
        sci_name: row.get(3)?,
        file_name: row.get(4)?,
        com_name: row.get(5)?,
        status: row.get(6)?,
        notes: row.get(7)?,
        reviewed_at: row.get(8)?,
        detached: detached.as_deref().and_then(Detached::parse),
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

    /// A detection that points at `clip`, as one microphone's would.
    fn seed_clip(conn: &Connection, date: &str, time: &str, sci: &str, com: &str, clip: &str) {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES (?1, ?2, ?3, ?4, 0.8, ?5)",
            params![date, time, sci, com, clip],
        )
        .unwrap();
    }

    /// The key of a detection with no clip.
    const fn key<'a>(date: &'a str, time: &'a str, sci: &'a str) -> DetectionKey<'a> {
        DetectionKey {
            date,
            time,
            sci_name: sci,
            file_name: None,
        }
    }

    /// The key of the detection that points at `clip`.
    const fn key_for<'a>(
        date: &'a str,
        time: &'a str,
        sci: &'a str,
        clip: &'a str,
    ) -> DetectionKey<'a> {
        DetectionKey {
            date,
            time,
            sci_name: sci,
            file_name: Some(clip),
        }
    }

    /// DB8: the verdict and its mirror on the detection are one change.
    ///
    /// They were two statements with no transaction: if the `detections`
    /// update failed (a busy database, a full disk), the review row stayed —
    /// the queue showed the detection as reviewed while every analytic that
    /// filters on `review_verdict` still counted it, and nothing reconciled
    /// the two. A trigger makes the second statement fail on demand.
    #[test]
    fn a_verdict_and_its_mirror_are_written_together_or_not_at_all() {
        let conn = db();
        seed_detection(
            &conn,
            "2026-05-01",
            "06:00:00",
            "Turdus merula",
            "Blackbird",
        );
        let k = key("2026-05-01", "06:00:00", "Turdus merula");
        conn.execute_batch(
            "CREATE TRIGGER fail_mirror BEFORE UPDATE OF review_verdict ON detections
             BEGIN SELECT RAISE(ABORT, 'mirror failed'); END;",
        )
        .unwrap();
        let reviews = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM detection_reviews", [], |r| r.get(0))
                .unwrap()
        };

        let set = set_detection_review(&conn, &k, "Blackbird", ReviewStatus::Rejected, None);
        assert!(
            set.is_err(),
            "precondition: the trigger made the mirror fail"
        );
        assert_eq!(reviews(&conn), 0, "a verdict was kept without its mirror");

        // Clearing: the delete must not stand when the mirror is refused.
        conn.execute_batch("DROP TRIGGER fail_mirror").unwrap();
        set_detection_review(&conn, &k, "Blackbird", ReviewStatus::Rejected, None)
            .expect("counterpart: without the trigger the verdict is written");
        assert_eq!(reviews(&conn), 1);
        conn.execute_batch(
            "CREATE TRIGGER fail_mirror BEFORE UPDATE OF review_verdict ON detections
             BEGIN SELECT RAISE(ABORT, 'mirror failed'); END;",
        )
        .unwrap();
        assert!(clear_detection_review(&conn, &k).is_err());
        assert_eq!(
            reviews(&conn),
            1,
            "a verdict was cleared without its mirror"
        );
        let verdict: Option<String> = conn
            .query_row("SELECT review_verdict FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(verdict.as_deref(), Some("rejected"));
    }

    /// A review is a verdict on one detection. Two microphones that heard one
    /// bird in the same second have two, which share date, time and species.
    #[test]
    fn a_verdict_names_one_detection_by_its_clip() {
        let conn = db();
        seed_clip(
            &conn,
            "2026-09-08",
            "06:00:00",
            "Turdus merula",
            "Blackbird",
            "a.wav",
        );
        seed_clip(
            &conn,
            "2026-09-08",
            "06:00:00",
            "Turdus merula",
            "Blackbird",
            "b.wav",
        );
        let a = key_for("2026-09-08", "06:00:00", "Turdus merula", "a.wav");
        let b = key_for("2026-09-08", "06:00:00", "Turdus merula", "b.wav");

        assert!(
            set_detection_review(&conn, &a, "Blackbird", ReviewStatus::Rejected, None).unwrap()
        );

        assert_eq!(
            get_detection_review(&conn, &a).unwrap().map(|r| r.status),
            Some("rejected".to_owned())
        );
        assert!(
            get_detection_review(&conn, &b).unwrap().is_none(),
            "the other microphone's detection was never reviewed"
        );
        let mirrored: Vec<(String, Option<String>)> = conn
            .prepare("SELECT File_Name, review_verdict FROM detections ORDER BY File_Name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            mirrored,
            [
                ("a.wav".to_owned(), Some("rejected".to_owned())),
                ("b.wav".to_owned(), None)
            ]
        );
        let pending = unreviewed_recent_detections(&conn, 10).unwrap();
        assert_eq!(
            pending
                .iter()
                .map(|d| d.file_name.as_deref())
                .collect::<Vec<_>>(),
            [Some("b.wav")],
            "only the detection nobody reviewed is in the queue"
        );

        // Reviewing the second does not disturb the first; clearing one leaves the other.
        set_detection_review(&conn, &b, "Blackbird", ReviewStatus::Confirmed, None).unwrap();
        assert!(clear_detection_review(&conn, &a).unwrap());
        assert!(get_detection_review(&conn, &a).unwrap().is_none());
        assert_eq!(
            get_detection_review(&conn, &b).unwrap().map(|r| r.status),
            Some("confirmed".to_owned())
        );
        assert_eq!(detection_review_counts(&conn).unwrap(), (1, 0));
    }

    /// A verdict about nothing is not recorded: a stale link must be told so.
    #[test]
    fn a_verdict_about_a_detection_that_is_not_there_is_not_recorded() {
        let conn = db();
        seed_clip(
            &conn,
            "2026-09-08",
            "06:00:00",
            "Turdus merula",
            "Blackbird",
            "a.wav",
        );
        let missing = key_for("2026-09-08", "06:00:00", "Turdus merula", "other.wav");

        assert!(
            !set_detection_review(&conn, &missing, "Blackbird", ReviewStatus::Rejected, None)
                .unwrap()
        );

        assert_eq!(detection_review_total(&conn, None).unwrap(), 0);
        let verdict: Option<String> = conn
            .query_row("SELECT review_verdict FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            verdict, None,
            "and the detection that is there is untouched"
        );
    }

    /// A detection with no clip is a detection: `None` and `Some("")` name it
    /// the way the unique index does.
    #[test]
    fn a_detection_with_no_clip_can_be_reviewed() {
        let conn = db();
        seed_detection(&conn, "2026-09-08", "06:00:00", "Pica pica", "Magpie");

        assert!(
            set_detection_review(
                &conn,
                &key("2026-09-08", "06:00:00", "Pica pica"),
                "Magpie",
                ReviewStatus::Confirmed,
                None
            )
            .unwrap()
        );
        let said_empty = key_for("2026-09-08", "06:00:00", "Pica pica", "");
        assert!(get_detection_review(&conn, &said_empty).unwrap().is_some());
    }

    /// `detection_clips` is what lets a caller that names only the triple be
    /// refused instead of guessed for.
    #[test]
    fn the_triple_says_how_many_detections_it_names() {
        let conn = db();
        assert!(
            detection_clips(&conn, "2026-09-08", "06:00:00", "Pica pica")
                .unwrap()
                .is_empty()
        );
        seed_clip(
            &conn,
            "2026-09-08",
            "06:00:00",
            "Pica pica",
            "Magpie",
            "a.wav",
        );
        assert_eq!(
            detection_clips(&conn, "2026-09-08", "06:00:00", "Pica pica").unwrap(),
            [Some("a.wav".to_owned())]
        );
        seed_detection(&conn, "2026-09-08", "06:00:00", "Pica pica", "Magpie");
        assert_eq!(
            detection_clips(&conn, "2026-09-08", "06:00:00", "Pica pica").unwrap(),
            [None, Some("a.wav".to_owned())],
            "a detection with no clip is one of the two, reported as None"
        );
    }

    /// A detached verdict is history: listed, in no count, and the only kind
    /// [`dismiss_detached_review`] will remove.
    #[test]
    fn a_detached_verdict_is_listed_counted_apart_and_dismissable() {
        let conn = db();
        seed_clip(
            &conn,
            "2026-09-08",
            "06:00:00",
            "Pica pica",
            "Magpie",
            "a.wav",
        );
        let k = key_for("2026-09-08", "06:00:00", "Pica pica", "a.wav");
        set_detection_review(&conn, &k, "Magpie", ReviewStatus::Rejected, None).unwrap();
        let attached_id: i64 = conn
            .query_row("SELECT id FROM detection_reviews", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO detection_reviews (date, time, sci_name, com_name, status, detached)
             VALUES ('2026-09-09', '07:00:00', 'Pica pica', 'Magpie', 'rejected', 'ambiguous')",
            [],
        )
        .unwrap();

        assert_eq!(
            detection_review_counts(&conn).unwrap(),
            (0, 1),
            "attached only"
        );
        assert_eq!(detached_review_count(&conn).unwrap(), 1);
        assert_eq!(
            detection_review_total(&conn, None).unwrap(),
            2,
            "the list shows both"
        );
        let page = detection_reviews_page(&conn, None, 10, 0).unwrap();
        assert!(page.iter().any(|r| r.detached == Some(Detached::Ambiguous)));
        assert!(
            get_detection_review(&conn, &key("2026-09-09", "07:00:00", "Pica pica"))
                .unwrap()
                .is_none(),
            "a detached verdict is no detection's verdict"
        );

        assert!(
            !dismiss_detached_review(&conn, attached_id).unwrap(),
            "never an attached one"
        );
        let detached_id: i64 = conn
            .query_row(
                "SELECT id FROM detection_reviews WHERE detached IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(dismiss_detached_review(&conn, detached_id).unwrap());
        assert_eq!(detached_review_count(&conn).unwrap(), 0);
        assert_eq!(detection_review_counts(&conn).unwrap(), (0, 1));
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
            seed_detection(&conn, "2026-01-01", &time, "Turdus merula", "Blackbird");
            set_detection_review(
                &conn,
                &key("2026-01-01", &time, "Turdus merula"),
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
            seed_detection(&conn, "2026-01-01", &time, "Turdus merula", "Blackbird");
            set_detection_review(
                &conn,
                &key("2026-01-01", &time, "Turdus merula"),
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
        seed_detection(
            &conn,
            "2026-05-20",
            "06:14:08",
            "Pica pica",
            "Eurasian Magpie",
        );
        let k = key("2026-05-20", "06:14:08", "Pica pica");
        set_detection_review(
            &conn,
            &k,
            "Eurasian Magpie",
            ReviewStatus::Confirmed,
            Some("clear call"),
        )
        .unwrap();
        let r = get_detection_review(&conn, &k)
            .unwrap()
            .expect("verdict present");
        assert_eq!(r.status, "confirmed");
        assert_eq!(r.notes.as_deref(), Some("clear call"));
        assert_eq!(r.detached, None);

        // Re-review updates in place rather than inserting a second row.
        set_detection_review(&conn, &k, "Eurasian Magpie", ReviewStatus::Rejected, None).unwrap();
        let (confirmed, rejected) = detection_review_counts(&conn).unwrap();
        assert_eq!((confirmed, rejected), (0, 1));
        let r = get_detection_review(&conn, &k).unwrap().unwrap();
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

        let magpie = key("2026-05-20", "06:00:00", "Pica pica");
        set_detection_review(
            &conn,
            &magpie,
            "Eurasian Magpie",
            ReviewStatus::Confirmed,
            None,
        )
        .unwrap();
        let pending = unreviewed_recent_detections(&conn, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sci_name, "Turdus merula");

        clear_detection_review(&conn, &magpie).unwrap();
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

    #[test]
    fn a_detached_reason_round_trips_and_an_unknown_one_is_refused() {
        for d in [
            Detached::Relabelled,
            Detached::Ambiguous,
            Detached::Orphaned,
        ] {
            assert_eq!(Detached::parse(d.as_str()), Some(d));
        }
        assert_eq!(Detached::parse("garbage"), None);
    }
}
