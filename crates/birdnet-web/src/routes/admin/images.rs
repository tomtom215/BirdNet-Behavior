//! Image blacklist admin routes.
//!
//! Provides UI to block inappropriate or incorrect species images from
//! being displayed. Blacklisted URLs are never shown in the web UI.
//!
//! | Path | Method | Description |
//! |------|--------|-------------|
//! | `/admin/images` | GET | List all blacklisted URLs |
//! | `/admin/images/blacklist` | POST | Add URL to blacklist |
//! | `/admin/images/blacklist/{id}` | DELETE | Remove URL from blacklist |
//!
//! BirdNET-Pi equivalent: No direct equivalent, but BirdNET-Pi had a manual
//! process for hiding bad images. This provides a proper admin UI for it.

use std::fmt::Write as _;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::{Form, Router, routing::get};
use serde::Deserialize;

use crate::state::AppState;

/// Mount image blacklist routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/images", get(images_page))
        .route(
            "/admin/images/blacklist",
            axum::routing::post(add_blacklist),
        )
        .route(
            "/admin/images/blacklist/{id}",
            axum::routing::delete(remove_blacklist),
        )
}

/// Form data for adding a URL to the blacklist.
#[derive(Debug, Deserialize)]
pub struct BlacklistForm {
    pub sci_name: String,
    pub url: String,
    pub reason: Option<String>,
}

/// Render the image blacklist admin page.
async fn images_page(State(state): State<AppState>) -> Html<String> {
    let entries =
        state.with_db(|conn| birdnet_db::sqlite::list_image_blacklist(conn).unwrap_or_default());

    let mut rows = String::new();
    for entry in &entries {
        let id = entry.id;
        let sci = super::super::pages::escape_html(&entry.sci_name);
        let url = super::super::pages::escape_html(&entry.url);
        let reason = entry
            .reason
            .as_deref()
            .map(super::super::pages::escape_html)
            .unwrap_or_default();
        let at = super::super::pages::escape_html(&entry.blacklisted_at);
        write!(
            rows,
            "<tr>\
             <td style=\"padding:0.5rem;\">{sci}</td>\
             <td style=\"padding:0.5rem;word-break:break-all;\">{url}</td>\
             <td style=\"padding:0.5rem;\">{reason}</td>\
             <td style=\"padding:0.5rem;\">{at}</td>\
             <td style=\"padding:0.5rem;\">\
             <button hx-delete=\"/admin/images/blacklist/{id}\" \
             hx-target=\"#blacklist-table\" hx-swap=\"outerHTML\" \
             hx-confirm=\"Remove this blacklist entry?\" \
             style=\"background:none;border:1px solid var(--danger);color:var(--danger);\
             padding:0.2rem 0.5rem;border-radius:var(--radius);cursor:pointer;font-size:0.8rem;\">\
             Remove</button>\
             </td></tr>"
        )
        .unwrap_or_default();
    }

    let count = entries.len();
    let html = format!(
        "<!DOCTYPE html><html><head>\
         <title>Image Blacklist — Admin</title>\
         <script src=\"/static/htmx.min.js\"></script>\
         <link rel=\"stylesheet\" href=\"/static/style.css\">\
         </head><body>\
         <div style=\"max-width:960px;margin:2rem auto;padding:0 1rem;\">\
         <h1 style=\"font-size:1.5rem;margin-bottom:1rem;\">Species Image Blacklist</h1>\
         <p style=\"color:var(--text-muted);margin-bottom:1.5rem;\">\
         Block URLs from being displayed as species images. \
         {count} entr{pl} blacklisted.\
         </p>\
         <form hx-post=\"/admin/images/blacklist\" hx-target=\"#blacklist-table\" hx-swap=\"outerHTML\"\
         style=\"background:var(--card-bg);border:1px solid var(--border);border-radius:var(--radius);\
         padding:1rem;margin-bottom:1.5rem;\">\
         <h2 style=\"font-size:1rem;margin-bottom:0.75rem;\">Add Blacklist Entry</h2>\
         <div style=\"display:flex;gap:0.5rem;flex-wrap:wrap;\">\
         <input name=\"sci_name\" placeholder=\"Scientific name\" required \
         style=\"flex:1;min-width:150px;padding:0.4rem 0.6rem;border:1px solid var(--border);\
         border-radius:var(--radius);background:var(--bg);color:var(--text);\">\
         <input name=\"url\" placeholder=\"Image URL\" required \
         style=\"flex:2;min-width:250px;padding:0.4rem 0.6rem;border:1px solid var(--border);\
         border-radius:var(--radius);background:var(--bg);color:var(--text);\">\
         <input name=\"reason\" placeholder=\"Reason (optional)\" \
         style=\"flex:1;min-width:150px;padding:0.4rem 0.6rem;border:1px solid var(--border);\
         border-radius:var(--radius);background:var(--bg);color:var(--text);\">\
         <button type=\"submit\" \
         style=\"padding:0.4rem 1rem;background:var(--accent);color:#fff;\
         border:none;border-radius:var(--radius);cursor:pointer;\">Add</button>\
         </div></form>\
         <table id=\"blacklist-table\" style=\"width:100%;border-collapse:collapse;\
         background:var(--card-bg);border:1px solid var(--border);border-radius:var(--radius);\">\
         <thead><tr style=\"background:var(--bg-hover);\">\
         <th style=\"padding:0.5rem;text-align:left;\">Species</th>\
         <th style=\"padding:0.5rem;text-align:left;\">URL</th>\
         <th style=\"padding:0.5rem;text-align:left;\">Reason</th>\
         <th style=\"padding:0.5rem;text-align:left;\">Added</th>\
         <th style=\"padding:0.5rem;text-align:left;\">Action</th>\
         </tr></thead>\
         <tbody>{rows}</tbody>\
         </table>\
         </div></body></html>",
        pl = if count == 1 { "y" } else { "ies" },
    );

    Html(html)
}

/// Add a URL to the image blacklist.
async fn add_blacklist(
    State(state): State<AppState>,
    Form(form): Form<BlacklistForm>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::add_image_blacklist(
                conn,
                &form.sci_name,
                &form.url,
                form.reason.as_deref(),
            )
        })
    })
    .await;

    match result {
        Ok(Ok(_)) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/html")],
            blacklist_table_partial_redirect(),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(axum::http::header::CONTENT_TYPE, "text/html")],
            "<table id=\"blacklist-table\"><tbody><tr><td colspan=\"5\">Error adding entry</td></tr></tbody></table>".to_string(),
        ),
    }
}

/// Remove a URL from the image blacklist.
async fn remove_blacklist(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::remove_image_blacklist(conn, id))
    })
    .await;

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/html")],
        blacklist_table_partial_redirect(),
    )
}

/// Return HTMX trigger to reload the blacklist table.
fn blacklist_table_partial_redirect() -> String {
    "<table id=\"blacklist-table\" \
     hx-get=\"/admin/images\" hx-trigger=\"load\" hx-target=\"#blacklist-table\" \
     hx-swap=\"outerHTML\"></table>"
        .to_string()
}
