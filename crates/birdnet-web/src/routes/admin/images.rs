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
    /// Scientific name of the species whose image should be blocked.
    pub sci_name: String,
    /// Full image URL to block.
    pub url: String,
    /// Optional free-text reason explaining why the image is blocked.
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
             <td class=\"img-td\">{sci}</td>\
             <td class=\"img-td-url\">{url}</td>\
             <td class=\"img-td\">{reason}</td>\
             <td class=\"img-td\">{at}</td>\
             <td class=\"img-td\">\
             <button hx-delete=\"/admin/images/blacklist/{id}\" \
             hx-target=\"#blacklist-table\" hx-swap=\"outerHTML\" \
             hx-confirm=\"Remove this blacklist entry?\" \
             data-confirm-action=\"hx-delete\" \
             data-confirm-url=\"/admin/images/blacklist/{id}\" \
             data-confirm-title=\"Remove blacklist entry\" \
             data-confirm-body=\"Remove this blacklist entry?\" \
             data-confirm-confirm-label=\"Remove\" \
             data-confirm-style=\"danger\" \
             class=\"img-del-btn\">\
             Remove</button>\
             </td></tr>"
        )
        .unwrap_or_default();
    }

    let count = entries.len();
    let body = format!(
        "<div class=\"img-wrap\">\
         <h1 class=\"img-h1\">Species Image Blacklist</h1>\
         <p class=\"img-lede\">\
         Block URLs from being displayed as species images. \
         {count} entr{pl} blacklisted.\
         </p>\
         <form hx-post=\"/admin/images/blacklist\" hx-target=\"#blacklist-table\" hx-swap=\"outerHTML\"\
         class=\"img-form\">\
         <h2 class=\"img-h2\">Add Blacklist Entry</h2>\
         <div class=\"img-field-row\">\
         <input name=\"sci_name\" placeholder=\"Scientific name\" required class=\"img-input\">\
         <input name=\"url\" placeholder=\"Image URL\" required class=\"img-input-wide\">\
         <input name=\"reason\" placeholder=\"Reason (optional)\" class=\"img-input\">\
         <button type=\"submit\" class=\"img-add-btn\">Add</button>\
         </div></form>\
         <table id=\"blacklist-table\" class=\"img-table\">\
         <thead><tr class=\"img-thead-row\">\
         <th class=\"img-th\">Species</th>\
         <th class=\"img-th\">URL</th>\
         <th class=\"img-th\">Reason</th>\
         <th class=\"img-th\">Added</th>\
         <th class=\"img-th\">Action</th>\
         </tr></thead>\
         <tbody>{rows}</tbody>\
         </table>\
         </div>",
        pl = if count == 1 { "y" } else { "ies" },
    );

    Html(crate::routes::admin::admin_subpage_shell(
        "Species images",
        "species",
        "Images",
        &body,
    ))
}

/// Add a URL to the image blacklist.
async fn add_blacklist(
    State(state): State<AppState>,
    Form(form): Form<BlacklistForm>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        let added = state.with_db(|conn| {
            birdnet_db::sqlite::add_image_blacklist(
                conn,
                &form.sci_name,
                &form.url,
                form.reason.as_deref(),
            )
        });
        // On success, evict any cached image for this species so the next
        // `/file` request re-fetches and is refused while the URL stays
        // blacklisted. This covers images cached before the blacklist entry,
        // whose source URL isn't retained in the on-disk cache across restarts.
        if added.is_ok()
            && let Some(cache) = state.image_cache()
        {
            cache.remove(&form.sci_name);
        }
        added
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
