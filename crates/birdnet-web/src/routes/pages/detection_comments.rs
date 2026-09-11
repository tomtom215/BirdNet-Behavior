//! The comment thread on a detection's detail page (`G-23`).
//!
//! Free text, attributed to whoever is signed in, and never rewritten —
//! `birdnet_db::detection_comments` and migration 49's trigger own that
//! guarantee; this module is the page.
//!
//! # Why not the review widget's notes box
//!
//! The widget immediately above this one records a *verdict*, and its `notes`
//! field is `UNIQUE(date, time, sci_name)` — a second reviewer's note replaces
//! the first, unattributed. That is the right shape for "what is the station's
//! current verdict on this row" and the wrong shape for "why". Two observers
//! disagreeing is the thing worth keeping.

use std::fmt::Write as _;

use axum::extract::{Form, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::{Router, routing::get, routing::post};
use serde::Deserialize;

use birdnet_db::detection_comments::{self, MAX_BODY_CHARS, NewComment};

use super::{escape_html, simple_url_encode};
use crate::auth_middleware::RequestUser;
use crate::state::AppState;

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
}

/// A comment being written.
#[derive(Debug, Deserialize)]
pub(super) struct AddForm {
    date: String,
    time: String,
    sci_name: String,
    body: String,
}

/// A comment being removed.
#[derive(Debug, Deserialize)]
pub(super) struct DeleteForm {
    date: String,
    time: String,
    sci_name: String,
    id: i64,
}

async fn thread_partial(
    State(state): State<AppState>,
    Query(q): Query<ThreadQuery>,
) -> impl IntoResponse {
    let (Some(date), Some(time), Some(sci_name)) = (q.date, q.time, q.sci_name) else {
        return html(render_error("No detection specified."));
    };
    let st = state.clone();
    let rendered =
        tokio::task::spawn_blocking(move || render_thread(&st, &date, &time, &sci_name, None))
            .await
            .unwrap_or_else(|_| render_error("The comments could not be loaded."));
    html(rendered)
}

async fn add_comment(
    State(state): State<AppState>,
    user: RequestUser,
    Form(form): Form<AddForm>,
) -> impl IntoResponse {
    // The author is the signed-in identity, never anything the form carried. A
    // name a browser could choose is not attribution.
    let (user_id, author) = (Some(user.user.id), user.user.username.clone());

    let st = state.clone();
    let (date, time, sci) = (form.date.clone(), form.time.clone(), form.sci_name.clone());
    let body = form.body.clone();
    let written = tokio::task::spawn_blocking(move || {
        st.with_db(|conn| {
            detection_comments::insert(
                conn,
                &NewComment {
                    date: &date,
                    time: &time,
                    sci_name: &sci,
                    user_id,
                    author: &author,
                    body: &body,
                },
            )
        })
    })
    .await;

    let problem = match written {
        Ok(Ok(comment)) => {
            crate::audit::audit(
                &state,
                Some(&user),
                "detection.comment.add",
                Some(&target_of(&form.date, &form.time, &form.sci_name)),
                Some(&format!("id={}", comment.id)),
            );
            None
        }
        // The database's own words: it is the one that knows the body was
        // empty, or 40 characters too long, and an operator who has just typed
        // a paragraph deserves better than "something went wrong".
        Ok(Err(e)) => Some(e.to_string()),
        Err(_) => Some("the comment could not be saved".to_owned()),
    };

    html(render_thread(
        &state,
        &form.date,
        &form.time,
        &form.sci_name,
        problem.as_deref(),
    ))
}

async fn delete_comment(
    State(state): State<AppState>,
    user: RequestUser,
    Form(form): Form<DeleteForm>,
) -> impl IntoResponse {
    let st = state.clone();
    let id = form.id;
    let removed = tokio::task::spawn_blocking(move || {
        st.with_db(|conn| detection_comments::delete(conn, id))
    })
    .await;

    let problem = match removed {
        Ok(Ok(Some(comment))) => {
            crate::audit::audit(
                &state,
                Some(&user),
                "detection.comment.delete",
                Some(&target_of(&form.date, &form.time, &form.sci_name)),
                // The id and the author, never the body: a comment deleted
                // because it named somebody must not survive in the log that
                // recorded its removal.
                Some(&format!("id={id} author={}", comment.author)),
            );
            None
        }
        // Already gone. Two people with the page open both pressing Remove is
        // not an error; the second one gets the list without it.
        Ok(Ok(None)) => None,
        _ => Some("the comment could not be removed".to_owned()),
    };

    html(render_thread(
        &state,
        &form.date,
        &form.time,
        &form.sci_name,
        problem.as_deref(),
    ))
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
pub(super) fn render_thread(
    state: &AppState,
    date: &str,
    time: &str,
    sci_name: &str,
    problem: Option<&str>,
) -> String {
    let comments = state
        .with_db(|conn| detection_comments::list(conn, date, time, sci_name))
        .unwrap_or_default();

    let date_e = escape_html(date);
    let time_e = escape_html(time);
    let sci_e = escape_html(sci_name);

    let mut items = String::new();
    for c in &comments {
        // Rendered for everyone, like the Confirm/Reject buttons in the review
        // widget directly above: the admin gate on the POST is what decides,
        // and a control that is hidden rather than refused tells a signed-out
        // visitor less, not more.
        let remove = {
            format!(
                r##"<form hx-post="/pages/detection-comments/delete" hx-target="#{ANCHOR}" hx-swap="outerHTML" class="dc-del"><input type="hidden" name="date" value="{date_e}"><input type="hidden" name="time" value="{time_e}"><input type="hidden" name="sci_name" value="{sci_e}"><input type="hidden" name="id" value="{id}"><button type="submit" class="bnb-btn ghost dc-del-btn" title="Remove this comment">Remove</button></form>"##,
                id = c.id,
            )
        };
        let _ = write!(
            items,
            r#"<li class="dc-item"><div class="dc-head"><span class="dc-author">{author}</span><span class="bnb-meta dc-at">{at}</span>{remove}</div><p class="dc-body">{body}</p></li>"#,
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
            r##"<form hx-post="/pages/detection-comments/add" hx-target="#{ANCHOR}" hx-swap="outerHTML" class="dc-form"><input type="hidden" name="date" value="{date_e}"><input type="hidden" name="time" value="{time_e}"><input type="hidden" name="sci_name" value="{sci_e}"><label class="bnb-meta" for="dc-body">Add a comment</label><textarea id="dc-body" name="body" rows="3" maxlength="{MAX_BODY_CHARS}" placeholder="Why this identification is right, or wrong…"></textarea><button type="submit" class="bnb-btn">Post</button></form>"##
        )
    };

    format!(
        r#"<div id="{ANCHOR}" class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Notes</div><h3>Comments</h3></div><span class="bnb-pill">{n}</span></div>{note}{list}{form}</div>"#,
        n = comments.len(),
    )
}

/// The lazily-loaded placeholder the detail page renders.
///
/// The thread is a database read the page does not need to block on, and the
/// detail page already loads its other panels this way.
#[must_use]
pub(super) fn thread_placeholder(date: &str, time: &str, sci_name: &str) -> String {
    format!(
        r#"<div id="{ANCHOR}" class="bnb-card pad" hx-get="/pages/detection-comments?date={d}&amp;time={t}&amp;sci_name={s}" hx-trigger="load" hx-swap="outerHTML"><p class="bnb-meta">Loading comments…</p></div>"#,
        d = simple_url_encode(date),
        t = simple_url_encode(time),
        s = simple_url_encode(sci_name),
    )
}
