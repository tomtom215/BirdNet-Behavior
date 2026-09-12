//! Free-text, attributed, append-only comments on a detection (`G-23`).
//!
//! # Not the review notes field
//!
//! `detection_reviews.notes` looks like this and is not. Migration 13 puts that
//! table under `UNIQUE(date, time, sci_name)` and writes it with
//! `INSERT … ON CONFLICT`, so a second reviewer's note replaces the first, and
//! the table has no user column. Two people annotating the same detection lose
//! one of the two annotations, silently, and neither of them is named.
//!
//! A comment here is one row per thing said. Many per detection, each carrying
//! who said it and when, and none of them ever rewritten — the
//! `detection_comments_are_append_only` trigger in migration 49 aborts every
//! `UPDATE`, so that holds against a stray `conn.execute` and not only against
//! the absence of an update function in this module.
//!
//! Deleting stays possible. A comment with a mistake in it, or a person's name,
//! needs a way out; [`delete`] returns the row it removed so the caller can put
//! the author and the id in the audit log without putting the body there.
//!
//! # Identity
//!
//! A detection is the `(date, time, sci_name)` triple, the same key
//! `detection_reviews` and the detail page already use. `detections` has no
//! primary key of its own to point at.

use std::fmt;

use rusqlite::{Connection, OptionalExtension as _, params};

/// The longest comment accepted, in characters.
///
/// Long enough for the paragraph of reasoning the feature exists for; short
/// enough that a detection's comment list stays a page rather than a download.
/// A body over this is refused at the boundary rather than truncated, because
/// silently keeping the first 2 000 characters of somebody's reasoning is worse
/// than telling them it did not fit.
pub const MAX_BODY_CHARS: usize = 2_000;

/// Errors from comment operations.
#[derive(Debug)]
pub enum CommentError {
    /// `SQLite` error.
    Sqlite(rusqlite::Error),
    /// The comment as given could not be stored.
    Invalid(String),
}

impl fmt::Display for CommentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::Invalid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for CommentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Invalid(_) => None,
        }
    }
}

impl From<rusqlite::Error> for CommentError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// A stored comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionComment {
    /// Row id.
    pub id: i64,
    /// The detection's date (`YYYY-MM-DD`).
    pub date: String,
    /// The detection's time (`HH:MM:SS`).
    pub time: String,
    /// The detection's scientific name.
    pub sci_name: String,
    /// The account that wrote it, or `None` once that account is deleted.
    pub user_id: Option<i64>,
    /// The author's name **as it was when the comment was written**, which is
    /// what keeps the row readable after the account is gone.
    pub author: String,
    /// What they said.
    pub body: String,
    /// When, as `datetime('now')` (UTC, whole seconds).
    pub at: String,
}

/// A comment about to be written.
#[derive(Debug, Clone)]
pub struct NewComment<'a> {
    /// The detection's date.
    pub date: &'a str,
    /// The detection's time.
    pub time: &'a str,
    /// The detection's scientific name.
    pub sci_name: &'a str,
    /// The account writing it, if there is one.
    pub user_id: Option<i64>,
    /// The name to record against it.
    pub author: &'a str,
    /// What they said.
    pub body: &'a str,
}

/// Write one comment and return it as stored.
///
/// The body is trimmed of surrounding whitespace and must be non-empty and at
/// most [`MAX_BODY_CHARS`] characters *after* trimming — a box full of spaces
/// is not a comment. The detection key and the author must be non-empty too: a
/// comment attached to nothing, or signed by nobody, is a row that can never be
/// read back or answered for.
///
/// # Errors
///
/// [`CommentError::Invalid`] for an empty or oversized body or a missing key
/// field; [`CommentError::Sqlite`] for anything the database refuses.
pub fn insert(conn: &Connection, new: &NewComment<'_>) -> Result<DetectionComment, CommentError> {
    let body = new.body.trim();
    let author = new.author.trim();
    if new.date.trim().is_empty() || new.time.trim().is_empty() || new.sci_name.trim().is_empty() {
        return Err(CommentError::Invalid(
            "a comment needs the detection it is about (date, time and scientific name)".into(),
        ));
    }
    if author.is_empty() {
        return Err(CommentError::Invalid(
            "a comment needs an author: an unattributed note is what this table exists to replace"
                .into(),
        ));
    }
    if body.is_empty() {
        return Err(CommentError::Invalid("a comment cannot be empty".into()));
    }
    // Characters, not bytes: a reviewer writing in Japanese is not allowed a
    // third of the comment an English one gets.
    let chars = body.chars().count();
    if chars > MAX_BODY_CHARS {
        return Err(CommentError::Invalid(format!(
            "a comment is at most {MAX_BODY_CHARS} characters; this one is {chars}"
        )));
    }

    conn.execute(
        "INSERT INTO detection_comments (date, time, sci_name, user_id, author, body)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            new.date.trim(),
            new.time.trim(),
            new.sci_name.trim(),
            new.user_id,
            author,
            body
        ],
    )?;
    let id = conn.last_insert_rowid();
    get(conn, id)?.ok_or_else(|| {
        CommentError::Invalid("the comment was inserted but could not be read back".into())
    })
}

/// One comment by id.
///
/// # Errors
///
/// [`CommentError::Sqlite`] if the query fails.
pub fn get(conn: &Connection, id: i64) -> Result<Option<DetectionComment>, CommentError> {
    conn.query_row(
        "SELECT id, date, time, sci_name, user_id, author, body, at
         FROM detection_comments WHERE id = ?1",
        params![id],
        row_to_comment,
    )
    .optional()
    .map_err(Into::into)
}

/// Every comment on one detection, **oldest first**.
///
/// A thread reads forward: the reply has to come after the thing it answers, or
/// the disagreement the table exists to record is unreadable.
///
/// # Errors
///
/// [`CommentError::Sqlite`] if the query fails.
pub fn list(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<Vec<DetectionComment>, CommentError> {
    let mut stmt = conn.prepare(
        "SELECT id, date, time, sci_name, user_id, author, body, at
         FROM detection_comments
         WHERE date = ?1 AND time = ?2 AND sci_name = ?3
         ORDER BY id ASC",
    )?;
    // `at` is whole seconds, so two comments posted in the same second share
    // it; `id` is the only monotonic ordering the table has.
    let rows = stmt
        .query_map(params![date, time, sci_name], row_to_comment)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Remove one comment, returning the row that was removed.
///
/// The returned row is what lets the caller audit *who* wrote what was deleted
/// without copying the body into the audit log — a comment deleted because it
/// named somebody would otherwise survive in the log that recorded its removal.
///
/// # Errors
///
/// [`CommentError::Sqlite`] if the query fails.
pub fn delete(conn: &Connection, id: i64) -> Result<Option<DetectionComment>, CommentError> {
    let Some(existing) = get(conn, id)? else {
        return Ok(None);
    };
    conn.execute("DELETE FROM detection_comments WHERE id = ?1", params![id])?;
    Ok(Some(existing))
}

fn row_to_comment(row: &rusqlite::Row<'_>) -> rusqlite::Result<DetectionComment> {
    Ok(DetectionComment {
        id: row.get(0)?,
        date: row.get(1)?,
        time: row.get(2)?,
        sci_name: row.get(3)?,
        user_id: row.get(4)?,
        author: row.get(5)?,
        body: row.get(6)?,
        at: row.get(7)?,
    })
}

// ── comments are a record, not a scratch field (G-23) ───────────────────
//
// These gates were written against migration 48's schema — no
// `detection_comments` table, no trigger — and observed failing: every one
// stops at `no such table: detection_comments`. Against migration 49 with the
// trigger removed, `a_comment_can_never_be_rewritten` is the one that fails,
// which is the point of writing it separately from the rest.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        migration::migrate(&conn).expect("migrate");
        conn
    }

    fn write(conn: &Connection, author: &str, body: &str) -> DetectionComment {
        insert(
            conn,
            &NewComment {
                date: "2026-05-01",
                time: "06:00:00",
                sci_name: "Dryobates villosus",
                user_id: None,
                author,
                body,
            },
        )
        .expect("insert")
    }

    /// The defect in one test: a second comment must not replace the first.
    ///
    /// `detection_reviews.notes` is UNIQUE on this same triple and written with
    /// `INSERT … ON CONFLICT`, so the equivalent there leaves one row holding
    /// only the later note.
    #[test]
    fn a_second_comment_on_one_detection_does_not_replace_the_first() {
        let conn = db();
        write(
            &conn,
            "ada",
            "Call is too short for a Hairy — I think Downy.",
        );
        write(
            &conn,
            "bob",
            "Agreed on length, but the spectrogram is Hairy.",
        );

        let all = list(&conn, "2026-05-01", "06:00:00", "Dryobates villosus").expect("list");
        assert_eq!(all.len(), 2, "both must survive: {all:?}");
        assert_eq!(
            all[0].author, "ada",
            "oldest first, so a reply follows what it answers"
        );
        assert_eq!(all[1].author, "bob");
        assert!(all[0].body.contains("Downy"));
        assert!(all[1].body.contains("spectrogram"));
    }

    /// Append-only is enforced by the database, not by this module declining to
    /// write an `UPDATE`. Fails with migration 49's trigger removed.
    ///
    /// Every protected column is tried, not just the body: the trigger names
    /// its columns, so dropping one from that list would otherwise leave a
    /// comment whose *author* could be rewritten under a gate that only ever
    /// tested the text.
    #[test]
    fn no_part_of_a_comments_record_can_be_rewritten() {
        let conn = db();
        let c = write(&conn, "ada", "the original reasoning");

        for (column, value) in [
            ("body", "something else"),
            ("author", "bob"),
            ("date", "2020-01-01"),
            ("time", "23:59:59"),
            ("sci_name", "Strix varia"),
            ("at", "2020-01-01 00:00:00"),
            ("id", "999"),
        ] {
            let err = conn
                .execute(
                    &format!("UPDATE detection_comments SET {column} = ?1 WHERE id = ?2"),
                    params![value, c.id],
                )
                .unwrap_err();
            assert!(
                err.to_string().contains("append-only"),
                "rewriting {column} must be refused, and the refusal must say why: {err}"
            );
        }

        let after = get(&conn, c.id).expect("get").expect("still there");
        assert_eq!(after, c, "not one field moved: {after:?}");
    }

    /// The counterpart to the trigger: deleting is still possible, and returns
    /// the row so the caller can audit the author without logging the body.
    /// Without this, a trigger that aborted *every* write would pass the gate
    /// above.
    #[test]
    fn a_comment_can_still_be_deleted_and_names_its_author_on_the_way_out() {
        let conn = db();
        let c = write(&conn, "ada", "oops, that is a neighbour's name");

        let removed = delete(&conn, c.id).expect("delete").expect("was there");
        assert_eq!(removed.author, "ada");
        assert_eq!(removed.id, c.id);
        assert!(get(&conn, c.id).expect("get").is_none());
        assert!(
            list(&conn, "2026-05-01", "06:00:00", "Dryobates villosus")
                .expect("list")
                .is_empty()
        );
        assert!(
            delete(&conn, c.id).expect("second delete").is_none(),
            "deleting what is already gone is not an error"
        );
    }

    /// Attribution outlives the account. `ON DELETE SET NULL` rather than
    /// `CASCADE`, so removing a user does not silently delete their reasoning,
    /// and the denormalised `author` keeps the row readable.
    #[test]
    fn removing_the_account_keeps_the_comment_and_its_name() {
        let conn = db();
        conn.execute(
            "INSERT INTO users (username, pwd_argon2, role) VALUES ('ada', '', 'admin')",
            [],
        )
        .expect("user");
        let uid = conn.last_insert_rowid();
        conn.execute("PRAGMA foreign_keys = ON", [])
            .expect("fk pragma");

        let c = insert(
            &conn,
            &NewComment {
                date: "2026-05-01",
                time: "06:00:00",
                sci_name: "Dryobates villosus",
                user_id: Some(uid),
                author: "ada",
                body: "confirmed against the reference recording",
            },
        )
        .expect("insert");
        assert_eq!(c.user_id, Some(uid));

        conn.execute("DELETE FROM users WHERE id = ?1", params![uid])
            .expect("delete user");

        let after = get(&conn, c.id).expect("get").expect("comment survives");
        assert_eq!(after.user_id, None, "the join is gone");
        assert_eq!(after.author, "ada", "the name is not");
        assert_eq!(after.body, c.body);
    }

    /// What is refused at the boundary, and what is accepted just inside it.
    /// The upper pair is the discrimination: a gate that only checked the
    /// rejecting side would pass on an implementation that refused everything.
    #[test]
    fn an_empty_or_oversized_body_is_refused_and_the_limit_itself_is_not() {
        let conn = db();
        for body in ["", "   ", "\n\t "] {
            let err = insert(
                &conn,
                &NewComment {
                    date: "2026-05-01",
                    time: "06:00:00",
                    sci_name: "Dryobates villosus",
                    user_id: None,
                    author: "ada",
                    body,
                },
            )
            .expect_err("a box of whitespace is not a comment");
            assert!(matches!(err, CommentError::Invalid(_)), "{err:?}");
        }

        let at_limit = "x".repeat(MAX_BODY_CHARS);
        assert_eq!(
            write(&conn, "ada", &at_limit).body.chars().count(),
            MAX_BODY_CHARS,
            "exactly the limit is accepted"
        );

        let over = "x".repeat(MAX_BODY_CHARS + 1);
        let err = insert(
            &conn,
            &NewComment {
                date: "2026-05-01",
                time: "06:00:00",
                sci_name: "Dryobates villosus",
                user_id: None,
                author: "ada",
                body: &over,
            },
        )
        .expect_err("one over the limit is refused");
        assert!(
            err.to_string().contains(&(MAX_BODY_CHARS + 1).to_string()),
            "the refusal names the length so the writer knows by how much: {err}"
        );

        // Characters, not bytes: 2 000 three-byte characters is at the limit.
        let cjk = "鳥".repeat(MAX_BODY_CHARS);
        assert!(
            cjk.len() > MAX_BODY_CHARS,
            "the byte length of this is well over the limit, which is the point"
        );
        assert_eq!(
            write(&conn, "ada", &cjk).body.chars().count(),
            MAX_BODY_CHARS
        );
    }

    /// A comment attached to nothing, or signed by nobody, cannot be read back
    /// or answered for.
    #[test]
    fn a_comment_needs_a_detection_and_an_author() {
        let conn = db();
        let cases = [
            ("", "06:00:00", "Dryobates villosus", "ada"),
            ("2026-05-01", "", "Dryobates villosus", "ada"),
            ("2026-05-01", "06:00:00", "", "ada"),
            ("2026-05-01", "06:00:00", "Dryobates villosus", "   "),
        ];
        for (date, time, sci_name, author) in cases {
            let outcome = insert(
                &conn,
                &NewComment {
                    date,
                    time,
                    sci_name,
                    user_id: None,
                    author,
                    body: "a real comment",
                },
            );
            match outcome {
                Err(CommentError::Invalid(_)) => {}
                other => panic!(
                    "({date:?}, {time:?}, {sci_name:?}, {author:?}) must be refused as invalid, \
                     got {other:?}"
                ),
            }
        }
    }

    /// Comments belong to the detection they name and to no other.
    #[test]
    fn a_comment_is_listed_only_against_its_own_detection() {
        let conn = db();
        write(&conn, "ada", "on the woodpecker");
        insert(
            &conn,
            &NewComment {
                date: "2026-05-01",
                time: "06:00:00",
                sci_name: "Strix varia",
                user_id: None,
                author: "ada",
                body: "on the owl",
            },
        )
        .expect("insert");
        insert(
            &conn,
            &NewComment {
                date: "2026-05-02",
                time: "06:00:00",
                sci_name: "Dryobates villosus",
                user_id: None,
                author: "ada",
                body: "next day, same bird",
            },
        )
        .expect("insert");

        let here = list(&conn, "2026-05-01", "06:00:00", "Dryobates villosus").expect("list");
        assert_eq!(here.len(), 1, "{here:?}");
        assert_eq!(here[0].body, "on the woodpecker");

        assert!(
            list(&conn, "2026-05-01", "07:00:00", "Dryobates villosus")
                .expect("list")
                .is_empty(),
            "a different time is a different detection"
        );
    }

    /// The body is stored trimmed, so the rendered thread does not carry a
    /// textarea's trailing newline.
    #[test]
    fn the_body_is_stored_without_its_surrounding_whitespace() {
        let conn = db();
        assert_eq!(
            write(&conn, "  ada  ", "  spaced out \n").body,
            "spaced out"
        );
        let c = list(&conn, "2026-05-01", "06:00:00", "Dryobates villosus").expect("list");
        assert_eq!(c[0].author, "ada");
    }
}
