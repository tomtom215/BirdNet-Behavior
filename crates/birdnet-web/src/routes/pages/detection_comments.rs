//! The comment thread on a detection's detail page (`G-23`).
//!
//! Free text, attributed to whoever is signed in, and never rewritten —
//! `birdnet_db::detection_comments` and migration 49's trigger own that
//! guarantee; this module is the page.
//!
//! # Why not the review widget's notes box
//!
//! The widget immediately above this one records a *verdict*, and its `notes`
//! field is one per detection — a second reviewer's note replaces the first,
//! unattributed. That is the right shape for "what is the station's current
//! verdict on this row" and the wrong shape for "why". Two observers
//! disagreeing is the thing worth keeping.
//!
//! # Which detection
//!
//! A thread is one detection's, clip included (MSC-05): two sources that heard
//! one bird in one second share the date, time and species. The detail page
//! names the clip; a request that does not is accepted when the date, time and
//! species leave one detection, and refused when they leave several, as the
//! review API refuses it.

use std::fmt::Write as _;

use crate::routes::form::HtmxForm;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::{Router, routing::get, routing::post};
use serde::Deserialize;

use birdnet_db::detection_comments::{self, MAX_BODY_CHARS, NewComment};

use super::{escape_html, simple_url_encode};
use crate::auth_middleware::RequestUser;
use crate::state::{AppState, ReviewOutcome, RowRef};

/// The element the thread swaps itself into.
const ANCHOR: &str = "dc-thread";

/// The read-only half: anyone who can see the detection can read its thread.
pub fn router() -> Router<AppState> {
    Router::new().route("/pages/detection-comments", get(thread_partial))
}

/// The writing half, mounted behind the admin gate by
/// [`super::mutating_router`].
///
/// That gate is the authorization, and it is why nothing here checks a role:
/// `auth_middleware` refuses every non-safe method from a viewer, so a request
/// that reaches these handlers is an admin's. Writing a second check on top
/// would be a branch that can never decide anything — and a gate for it would
/// be one that can never fail.
pub fn mutating_router() -> Router<AppState> {
    Router::new()
        .route("/pages/detection-comments/add", post(add_comment))
        .route("/pages/detection-comments/delete", post(delete_comment))
}

/// Which detection's thread.
#[derive(Debug, Deserialize)]
pub(super) struct ThreadQuery {
    date: Option<String>,
    time: Option<String>,
    sci_name: Option<String>,
    /// The detection's clip, `""` for one with none; absent when not said.
    file: Option<String>,
}

/// A comment being written.
#[derive(Debug, Deserialize)]
pub(super) struct AddForm {
    date: String,
    time: String,
    sci_name: String,
    /// The detection's clip, as the thread was drawn with it.
    #[serde(default)]
    file_name: Option<String>,
    body: String,
}

/// A comment being removed.
#[derive(Debug, Deserialize)]
pub(super) struct DeleteForm {
    date: String,
    time: String,
    sci_name: String,
    /// The detection's clip, so the thread is redrawn for the same detection.
    #[serde(default)]
    file_name: Option<String>,
    id: i64,
}

impl AddForm {
    fn row(&self) -> RowRef {
        RowRef {
            date: self.date.clone(),
            time: self.time.clone(),
            sci_name: self.sci_name.clone(),
            file_name: self.file_name.clone(),
        }
    }
}

impl DeleteForm {
    fn row(&self) -> RowRef {
        RowRef {
            date: self.date.clone(),
            time: self.time.clone(),
            sci_name: self.sci_name.clone(),
            file_name: self.file_name.clone(),
        }
    }
}

/// The clip of the detection a thread is about, or what to say instead.
///
/// `Ok(Err(message))` is a refusal a person reads: no detection has that date,
/// time and species, or several do and the request did not say which.
fn thread_clip(
    conn: &rusqlite::Connection,
    row: &RowRef,
) -> Result<Result<Option<String>, &'static str>, birdnet_db::sqlite::DbError> {
    Ok(match crate::state::review_clip(conn, row)? {
        Ok(clip) => Ok(clip),
        Err(ReviewOutcome::Ambiguous { .. }) => Err(
            "More than one detection has this date, time and species. Open the thread from the detection's own page.",
        ),
        Err(_) => Err("No detection has this date, time and species."),
    })
}

async fn thread_partial(
    State(state): State<AppState>,
    Query(q): Query<ThreadQuery>,
) -> impl IntoResponse {
    let (Some(date), Some(time), Some(sci_name)) = (q.date, q.time, q.sci_name) else {
        return html(render_error("No detection specified."));
    };
    let row = RowRef {
        date,
        time,
        sci_name,
        file_name: q.file,
    };
    let st = state.clone();
    let rendered = tokio::task::spawn_blocking(move || render_thread(&st, &row, None))
        .await
        .unwrap_or_else(|_| render_error("The comments could not be loaded."));
    html(rendered)
}

async fn add_comment(
    State(state): State<AppState>,
    user: RequestUser,
    HtmxForm(form): HtmxForm<AddForm>,
) -> impl IntoResponse {
    // The write, its audit row and the re-read run as one piece on the
    // blocking pool: none of them may hold a runtime worker, and a client that
    // hangs up part way must not leave a comment with no audit row.
    let rendered = state
        .run_blocking(move |state| {
            // The author is the signed-in identity, never anything the form
            // carried. A name a browser could choose is not attribution.
            let (user_id, author) = (Some(user.user.id), user.user.username.clone());
            let mut row = form.row();
            // The detection is resolved and written to under one connection, so
            // the clip the comment is filed under is the one that was checked.
            let written = state.with_db(|conn| {
                let clip = match thread_clip(conn, &row) {
                    Ok(Ok(clip)) => clip,
                    Ok(Err(refused)) => return Err(refused.to_owned()),
                    Err(e) => {
                        tracing::warn!(error = %e, "comment: the detection could not be read");
                        return Err("the detection could not be read".to_owned());
                    }
                };
                let written = detection_comments::insert(
                    conn,
                    &NewComment {
                        date: &form.date,
                        time: &form.time,
                        sci_name: &form.sci_name,
                        file_name: clip.as_deref(),
                        user_id,
                        author: &author,
                        body: &form.body,
                    },
                )
                .map_err(|e| e.to_string());
                // Redraw the thread for the detection the comment went to.
                row.file_name = Some(clip.unwrap_or_default());
                written
            });

            let problem = match written {
                Ok(comment) => {
                    crate::audit::audit(
                        state,
                        Some(&user),
                        "detection.comment.add",
                        Some(&target_of(&form.date, &form.time, &form.sci_name)),
                        Some(&format!("id={}", comment.id)),
                    );
                    None
                }
                // The database's own words: it is the one that knows the body
                // was empty, or 40 characters too long, and an operator who has
                // just typed a paragraph deserves better than "something went
                // wrong".
                Err(e) => Some(e),
            };

            render_thread(state, &row, problem.as_deref())
        })
        .await;
    html(rendered)
}

async fn delete_comment(
    State(state): State<AppState>,
    user: RequestUser,
    HtmxForm(form): HtmxForm<DeleteForm>,
) -> impl IntoResponse {
    // As in `add_comment`: delete, audit and re-read together, off the runtime.
    let rendered = state
        .run_blocking(move |state| {
            let id = form.id;
            let problem = match state.with_db(|conn| detection_comments::delete(conn, id)) {
                Ok(Some(comment)) => {
                    crate::audit::audit(
                        state,
                        Some(&user),
                        "detection.comment.delete",
                        Some(&target_of(&form.date, &form.time, &form.sci_name)),
                        // The id and the author, never the body: a comment
                        // deleted because it named somebody must not survive in
                        // the log that recorded its removal.
                        Some(&format!("id={id} author={}", comment.author)),
                    );
                    None
                }
                // Already gone. Two people with the page open both pressing
                // Remove is not an error; the second one gets the list without
                // it.
                Ok(None) => None,
                Err(_) => Some("the comment could not be removed".to_owned()),
            };

            render_thread(state, &form.row(), problem.as_deref())
        })
        .await;
    html(rendered)
}

/// The audit target for a detection: the tuple, in the schema's order.
fn target_of(date: &str, time: &str, sci_name: &str) -> String {
    format!("{date} {time} {sci_name}")
}

fn html(body: String) -> impl IntoResponse {
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], body)
}

fn render_error(message: &str) -> String {
    format!(
        r#"<div id="{ANCHOR}" class="bnb-card pad bnb-meta">{}</div>"#,
        escape_html(message)
    )
}

/// The whole thread: the comments, then the box to add one.
///
/// Rendered as one self-replacing element so every write swaps the list and the
/// form together — a form that posted and left a stale list beside it is how
/// somebody writes the same comment twice.
pub(super) fn render_thread(state: &AppState, row: &RowRef, problem: Option<&str>) -> String {
    // A failed read is said, not shown as "No comments yet" — which invites
    // someone to write again the note that is already there.
    let read = state.with_read_db(|conn| -> Result<_, String> {
        let clip = match thread_clip(conn, row).map_err(|e| e.to_string())? {
            Ok(clip) => clip,
            Err(refused) => return Ok(Err(refused)),
        };
        let comments = detection_comments::list(conn, &row.key_with(clip.as_deref()))
            .map_err(|e| e.to_string())?;
        Ok(Ok((clip, comments)))
    });
    let (clip, comments) = match read {
        Ok(Ok(read)) => read,
        Ok(Err(refused)) => return render_error(refused),
        Err(e) => {
            tracing::error!(error = %e, "detection comments could not be read");
            return format!(
                r#"<div id="{ANCHOR}" class="bnb-card pad">{}</div>"#,
                super::error_states::inline("the comments on this detection")
            );
        }
    };

    let date_e = escape_html(&row.date);
    let time_e = escape_html(&row.time);
    let sci_e = escape_html(&row.sci_name);
    // Every form in the thread names the clip, so a post or a removal is about
    // this detection and redraws this detection's thread.
    let file_e = escape_html(clip.as_deref().unwrap_or_default());

    let mut items = String::new();
    for c in &comments {
        // Rendered for everyone, like the Confirm/Reject buttons in the review
        // widget directly above: the admin gate on the POST is what decides,
        // and a control that is hidden rather than refused tells a signed-out
        // visitor less, not more.
        let remove = {
            format!(
                r##"<form hx-post="/pages/detection-comments/delete" hx-target="#{ANCHOR}" hx-swap="outerHTML" class="dc-del"><input type="hidden" name="date" value="{date_e}"><input type="hidden" name="time" value="{time_e}"><input type="hidden" name="sci_name" value="{sci_e}"><input type="hidden" name="file_name" value="{file_e}"><input type="hidden" name="id" value="{id}"><button type="submit" class="bnb-btn ghost dc-del-btn" title="Remove this comment">Remove</button></form>"##,
                id = c.id,
            )
        };
        // A comment from before comments named a clip, on a second more than
        // one detection shares, is on each of them; the reader is told so
        // rather than left to take it as this detection's.
        let shared = if c.file_name.is_none() {
            r#"<span class="bnb-meta dc-shared">written before comments named one detection: shown on every detection of this species in this second</span>"#
        } else {
            ""
        };
        let _ = write!(
            items,
            r#"<li class="dc-item"><div class="dc-head"><span class="dc-author">{author}</span><span class="bnb-meta dc-at">{at}</span>{remove}</div>{shared}<p class="dc-body">{body}</p></li>"#,
            author = escape_html(&c.author),
            at = escape_html(&c.at),
            body = escape_html(&c.body),
        );
    }

    let list = if comments.is_empty() {
        r#"<p class="bnb-meta dc-empty">No comments yet. Why a detection is right or wrong is worth writing down while you still remember.</p>"#.to_owned()
    } else {
        format!(r#"<ul class="dc-list">{items}</ul>"#)
    };

    let note = problem.map_or_else(String::new, |m| {
        format!(
            r#"<p class="dc-problem" role="alert">{}</p>"#,
            escape_html(m)
        )
    });

    let form = {
        format!(
            r##"<form hx-post="/pages/detection-comments/add" hx-target="#{ANCHOR}" hx-swap="outerHTML" class="dc-form"><input type="hidden" name="date" value="{date_e}"><input type="hidden" name="time" value="{time_e}"><input type="hidden" name="sci_name" value="{sci_e}"><input type="hidden" name="file_name" value="{file_e}"><label class="bnb-meta" for="dc-body">Add a comment</label><textarea id="dc-body" name="body" rows="3" maxlength="{MAX_BODY_CHARS}" placeholder="Why this identification is right, or wrong…"></textarea><button type="submit" class="bnb-btn">Post</button></form>"##
        )
    };

    format!(
        r#"<div id="{ANCHOR}" class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Notes</div><h2 class="sh-h">Comments</h2></div><span class="bnb-pill">{n}</span></div>{note}{list}{form}</div>"#,
        n = comments.len(),
    )
}

/// The lazily-loaded placeholder the detail page renders.
///
/// The thread is a database read the page does not need to block on, and the
/// detail page already loads its other panels this way.
///
/// `file` is the detection's clip (`""` for none), so the thread is this
/// detection's and not every source's that heard the bird in the same second.
#[must_use]
pub(super) fn thread_placeholder(date: &str, time: &str, sci_name: &str, file: &str) -> String {
    format!(
        r#"<div id="{ANCHOR}" class="bnb-card pad" hx-get="/pages/detection-comments?date={d}&amp;time={t}&amp;sci_name={s}&amp;file={f}" hx-trigger="load" hx-swap="outerHTML"><p class="bnb-meta">Loading comments…</p></div>"#,
        d = simple_url_encode(date),
        t = simple_url_encode(time),
        s = simple_url_encode(sci_name),
        f = simple_url_encode(file),
    )
}
