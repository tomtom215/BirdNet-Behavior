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
//! `detection_comments_are_append_only` trigger (migration 49; migration 62
//! added the clip to what it locks) aborts every `UPDATE` of a comment's
//! content, so that holds against a stray `conn.execute` and not only against
//! the absence of an update function in this module.
//!
//! Deleting stays possible. A comment with a mistake in it, or a person's name,
//! needs a way out; [`delete`] returns the row it removed so the caller can put
//! the author and the id in the audit log without putting the body there.
//!
//! # Identity
//!
//! A detection is `(date, time, sci_name, file_name)`, the key `DetectionKey`
//! and `detection_reviews` use (migrations 59 and 62): two sources that hear
//! one bird in one second write two detections that differ only in their clip,
//! and a comment on one is not a comment on the other. `file_name` is the
//! detection's `COALESCE(File_Name, '')`, so `""` is a detection with no clip.
//! `detections` has no primary key of its own to point at.
//!
//! A comment with `file_name` NULL was written before comments named a clip,
//! on a second that more than one detection of the species shares (migration
//! 62 gave every other legacy comment its clip). Which detection it was about
//! cannot be told, so [`list`] returns it on each of them, and
//! [`DetectionComment::file_name`] being `None` is what a reader shows as
//! "may be about another source's detection".

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
    /// It named no clip, and no detection has that date, time and name.
    NoSuchDetection,
    /// It named no clip, and that date, time and name are shared by this many
    /// detections (one per source that heard the bird in that second). Nothing
    /// was written: a comment is never attached by guessing between them.
    Ambiguous {
        /// How many detections share the date, time and name.
        detections: usize,
    },
}

impl fmt::Display for CommentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::Invalid(m) => write!(f, "{m}"),
            Self::NoSuchDetection => {
                write!(
                    f,
                    "no detection matches that date, time and scientific name"
                )
            }
            Self::Ambiguous { detections } => write!(
                f,
                "{detections} detections share that date, time and scientific name; \
                 name the one you mean by its file_name"
            ),
        }
    }
}

impl std::error::Error for CommentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Invalid(_) | Self::NoSuchDetection | Self::Ambiguous { .. } => None,
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
    /// The detection's clip (`""` for one with none), or `None` for a legacy
    /// comment whose second more than one detection shares, which is listed on
    /// each of them (see the module docs).
    pub file_name: Option<String>,
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
    /// The detection's clip: `Some("")` for a detection with none. `None`
    /// when the writer did not say, which is accepted only when the date, time
    /// and name leave exactly one detection; that detection's clip is stored.
    pub file_name: Option<&'a str>,
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
/// A comment is stored against one detection's clip. When `new.file_name` is
/// `None` the clip is the one the date, time and name leave, if they leave
/// exactly one.
///
/// # Errors
///
/// [`CommentError::Invalid`] for an empty or oversized body or a missing key
/// field; [`CommentError::NoSuchDetection`] or [`CommentError::Ambiguous`]
/// when no clip was named and the date, time and name do not identify one
/// detection; [`CommentError::Sqlite`] for anything the database refuses.
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

    let (date, time, sci_name) = (new.date.trim(), new.time.trim(), new.sci_name.trim());
    let clip = if let Some(clip) = new.file_name {
        clip.to_owned()
    } else {
        let clips = crate::sqlite::detection_clips(conn, date, time, sci_name).map_err(|e| {
            if let crate::sqlite::DbError::Sqlite(e) = e {
                CommentError::Sqlite(e)
            } else {
                CommentError::Invalid(e.to_string())
            }
        })?;
        match clips.as_slice() {
            [] => return Err(CommentError::NoSuchDetection),
            [only] => only.clone().unwrap_or_default(),
            more => {
                return Err(CommentError::Ambiguous {
                    detections: more.len(),
                });
            }
        }
    };

    conn.execute(
        "INSERT INTO detection_comments (date, time, sci_name, file_name, user_id, author, body)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![date, time, sci_name, clip, new.user_id, author, body],
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
        "SELECT id, date, time, sci_name, file_name, user_id, author, body, at
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
/// `file_name` names the detection's clip (`Some("")` for one with none); its
/// thread is the comments written on that clip and the legacy comments whose
/// clip could not be told (`file_name` NULL; see the module docs), which the
/// caller can tell apart by [`DetectionComment::file_name`]. `None` lists every
/// comment on the date, time and name, whichever detection it is about, for a
/// caller that does not know the clip.
///
/// # Errors
///
/// [`CommentError::Sqlite`] if the query fails.
pub fn list(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
    file_name: Option<&str>,
) -> Result<Vec<DetectionComment>, CommentError> {
    let mut stmt = conn.prepare(
        "SELECT id, date, time, sci_name, file_name, user_id, author, body, at
         FROM detection_comments
         WHERE date = ?1 AND time = ?2 AND sci_name = ?3
           AND (?4 IS NULL OR file_name IS NULL OR file_name = ?4)
         ORDER BY id ASC",
    )?;
    // `at` is whole seconds, so two comments posted in the same second share
    // it; `id` is the only monotonic ordering the table has.
    let rows = stmt
        .query_map(params![date, time, sci_name, file_name], row_to_comment)?
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
        file_name: row.get(4)?,
        user_id: row.get(5)?,
        author: row.get(6)?,
        body: row.get(7)?,
        at: row.get(8)?,
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

    /// The clip every fixture comment here is written on.
    const CLIP: &str = "2026-05-01-birdnet-06:00:00.wav";

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
                file_name: Some(CLIP),
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

        let all = list(
            &conn,
            "2026-05-01",
            "06:00:00",
            "Dryobates villosus",
            Some(CLIP),
        )
        .expect("list");
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
            ("file_name", "another-clip.wav"),
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
            list(
                &conn,
                "2026-05-01",
                "06:00:00",
                "Dryobates villosus",
                Some(CLIP)
            )
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
                file_name: Some(CLIP),
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
                    file_name: Some(CLIP),
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
                file_name: Some(CLIP),
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
                    file_name: Some(CLIP),
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
                file_name: Some(CLIP),
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
                file_name: Some(CLIP),
                user_id: None,
                author: "ada",
                body: "next day, same bird",
            },
        )
        .expect("insert");

        let here = list(
            &conn,
            "2026-05-01",
            "06:00:00",
            "Dryobates villosus",
            Some(CLIP),
        )
        .expect("list");
        assert_eq!(here.len(), 1, "{here:?}");
        assert_eq!(here[0].body, "on the woodpecker");

        assert!(
            list(
                &conn,
                "2026-05-01",
                "07:00:00",
                "Dryobates villosus",
                Some(CLIP)
            )
            .expect("list")
            .is_empty(),
            "a different time is a different detection"
        );
    }

    /// Two detections of one species in one second, one per source.
    fn two_sources(conn: &Connection) {
        for file in ["garden.wav", "camera.wav"] {
            conn.execute(
                "INSERT INTO detections
                    (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
                 VALUES ('2026-05-01', '06:00:00', 'Strix aluco', 'Tawny Owl', 0.9, ?1, 0)",
                [file],
            )
            .expect("detection");
        }
    }

    fn on(
        conn: &Connection,
        file_name: Option<&str>,
        body: &str,
    ) -> Result<DetectionComment, CommentError> {
        insert(
            conn,
            &NewComment {
                date: "2026-05-01",
                time: "06:00:00",
                sci_name: "Strix aluco",
                file_name,
                user_id: None,
                author: "ada",
                body,
            },
        )
    }

    /// MSC-05: a comment on one source's detection is not on the other's,
    /// though both share a date, a time and a species.
    #[test]
    fn a_comment_is_listed_only_against_its_own_clip() {
        let conn = db();
        two_sources(&conn);
        on(&conn, Some("garden.wav"), "the neighbour's dog").expect("insert");

        let garden = list(
            &conn,
            "2026-05-01",
            "06:00:00",
            "Strix aluco",
            Some("garden.wav"),
        )
        .expect("list");
        assert_eq!(garden.len(), 1, "{garden:?}");
        assert_eq!(garden[0].file_name.as_deref(), Some("garden.wav"));
        let camera = list(
            &conn,
            "2026-05-01",
            "06:00:00",
            "Strix aluco",
            Some("camera.wav"),
        )
        .expect("list");
        assert!(camera.is_empty(), "{camera:?}");
        assert_eq!(
            list(&conn, "2026-05-01", "06:00:00", "Strix aluco", None)
                .expect("list")
                .len(),
            1,
            "a caller that names no clip still sees every comment on the second"
        );
    }

    /// A comment that names no clip is stored against the one detection its
    /// date, time and name leave, and refused, not guessed, when they leave two
    /// or none.
    #[test]
    fn an_unnamed_clip_is_resolved_only_when_one_detection_is_meant() {
        let conn = db();
        assert!(matches!(
            on(&conn, None, "about nothing"),
            Err(CommentError::NoSuchDetection)
        ));
        conn.execute(
            "INSERT INTO detections
                (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
             VALUES ('2026-05-01', '06:00:00', 'Strix aluco', 'Tawny Owl', 0.9, NULL, 0)",
            [],
        )
        .expect("detection");
        let c = on(&conn, None, "the only one").expect("resolved");
        assert_eq!(c.file_name.as_deref(), Some(""), "a detection with no clip");

        let conn = db();
        two_sources(&conn);
        assert!(matches!(
            on(&conn, None, "which one?"),
            Err(CommentError::Ambiguous { detections: 2 })
        ));
        assert!(
            list(&conn, "2026-05-01", "06:00:00", "Strix aluco", None)
                .expect("list")
                .is_empty(),
            "and nothing was written"
        );
    }

    /// A legacy comment whose clip could not be told is shown on every
    /// detection of its second, and says so by its `None` clip.
    #[test]
    fn a_legacy_comment_is_listed_on_each_detection_of_its_second() {
        let conn = db();
        two_sources(&conn);
        conn.execute(
            "INSERT INTO detection_comments (date, time, sci_name, author, body)
             VALUES ('2026-05-01', '06:00:00', 'Strix aluco', 'ada', 'from before')",
            [],
        )
        .expect("legacy");
        for clip in ["garden.wav", "camera.wav"] {
            let thread =
                list(&conn, "2026-05-01", "06:00:00", "Strix aluco", Some(clip)).expect("list");
            assert_eq!(thread.len(), 1, "{clip}: {thread:?}");
            assert_eq!(thread[0].file_name, None);
        }
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
        let c = list(
            &conn,
            "2026-05-01",
            "06:00:00",
            "Dryobates villosus",
            Some(CLIP),
        )
        .expect("list");
        assert_eq!(c[0].author, "ada");
    }
}
