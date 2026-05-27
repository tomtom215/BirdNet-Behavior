//! Species photo partial.
//!
//! Mounts:  GET /pages/species-photo?name=<common name>
//!
//! Used by:
//!   * Detection detail page (`templates/detection_detail.html`)
//!   * Species detail page (`templates/species_detail.html`, future use)
//!   * Public share page (`templates/share_rare.html`, future use)
//!
//! Returns an `<img>` element pointing at the existing
//! `/api/v2/species/image/<sci>/file` endpoint when the project's
//! `ImageCache` has a cached photo for the species, or an empty body
//! (HTTP 200) when no photo is available — in which case the surrounding
//! `.bnb-photo` element keeps its design-system placeholder pattern.
//!
//! ### Why this is the production path
//!
//! The workspace already ships `birdnet_integrations::species_images::ImageCache`
//! (held by `AppState::image_cache()`). It fetches and caches
//! Wikimedia / Macaulay Library imagery per scientific name, governed by the
//! `--image-cache-dir` flag. This partial just emits an `<img>` referencing
//! the existing serving route — no new download, no new asset pipeline.
//!
//! ### Mounting
//!
//! In `crates/birdnet-web/src/routes/pages/mod.rs`:
//!
//! ```rust,ignore
//! pub mod species_photo;                 // top of file, near `species_pages`
//! // ...inside the `router()` chain:
//! .merge(species_photo::router())
//! ```
//!
//! ### Behavior
//!
//! 1. Look up `Sci_Name` for the supplied common name in the `detections`
//!    table (same single-row lookup `species_info_partial` already uses).
//! 2. If `state.image_cache()` has the species cached **and** the image has
//!    been downloaded to disk (`cached_path.is_some()`), return an `<img>`
//!    tag whose `src` is the existing `/api/v2/species/image/<sci>/file`
//!    serving endpoint, plus a small attribution caption.
//! 3. Otherwise return `204 No Content` — htmx's `hx-swap="innerHTML"` will
//!    not replace the placeholder, so the `.bnb-photo` hatched pattern
//!    remains visible.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{escape_html, simple_url_encode};
use crate::state::AppState;

/// Mount the partial route.
pub fn router() -> Router<AppState> {
    Router::new().route("/pages/species-photo", get(species_photo_partial))
}

#[derive(Debug, Deserialize)]
pub struct NameQuery {
    pub name: String,
    /// Optional caller-supplied caption to append over the photo.
    /// When omitted, no caption is rendered (the surrounding template usually
    /// supplies its own via `data-caption` on the `.bnb-photo` element).
    pub caption: Option<String>,
    /// Optional attribution override. Defaults to the cached image's source
    /// (Wikimedia / Macaulay) if present, falling back to "" when unknown.
    pub attribution: Option<String>,
}

/// HTMX partial: an `<img>` for the species's reference photo, or 204 if
/// no photo is cached.
async fn species_photo_partial(
    State(state): State<AppState>,
    Query(query): Query<NameQuery>,
) -> Response {
    let name = query.name.trim().to_string();
    if name.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }

    // Resolve the scientific name from the detections table (same lookup
    // pattern as `species_info_partial`).
    let com_name = name.clone();
    let sci_name: String = {
        let state = state.clone();
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| {
                conn.query_row(
                    "SELECT Sci_Name FROM detections WHERE Com_Name = ?1 LIMIT 1",
                    [&com_name],
                    |row| row.get::<_, String>(0),
                )
                .unwrap_or_default()
            })
        })
        .await
        .unwrap_or_default()
    };

    if sci_name.is_empty() {
        // Unknown species — let the placeholder remain.
        return StatusCode::NO_CONTENT.into_response();
    }

    // Ask the existing image cache whether we have a photo. The cache
    // implementation (`birdnet_integrations::species_images::ImageCache`)
    // returns `Some` once a fetch attempt has been made (whether or not the
    // download succeeded); `cached_path.is_some()` is the "we have bytes on
    // disk" signal.
    let Some(cache) = state.image_cache() else {
        return StatusCode::NO_CONTENT.into_response();
    };

    let Some(image) = cache.get_cached(&sci_name) else {
        return StatusCode::NO_CONTENT.into_response();
    };

    if image.cached_path.is_none() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let enc_sci = simple_url_encode(&sci_name);
    let alt = escape_html(&name);

    // Attribution string — pull from the cached image's source URL if available.
    // Caller can override via `?attribution=...` (e.g. share page may want
    // a shorter attribution for the public-facing footer).
    let attribution = query
        .attribution
        .as_deref()
        .map(str::to_owned)
        .or_else(|| {
            image
                .wiki_url
                .as_ref()
                .map(|u| format!("Source · {}", host_only(u)))
        })
        .unwrap_or_default();
    let attribution_safe = escape_html(&attribution);

    // Optional caller-supplied caption.
    let caption_html = query
        .caption
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .map(|c| {
            format!(
                r#"<div style="position:absolute;left:16px;bottom:16px;background:color-mix(in oklch, var(--bg) 92%, transparent);backdrop-filter:blur(10px);-webkit-backdrop-filter:blur(10px);border:0.5px solid var(--border-2);border-radius:10px;padding:10px 14px;display:flex;flex-direction:column;gap:2px;z-index:2;">{}</div>"#,
                escape_html(c),
            )
        })
        .unwrap_or_default();

    let body = format!(
        r#"<img src="/api/v2/species/image/{enc_sci}/file" alt="{alt}" loading="lazy" decoding="async" style="display:block;width:100%;height:100%;object-fit:cover;"/>{caption_html}{attribution_block}"#,
        attribution_block = if attribution_safe.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="mono" style="position:absolute;right:14px;bottom:14px;font-size:9.5px;letter-spacing:0.06em;color:var(--fg-3);background:color-mix(in oklch, var(--bg) 86%, transparent);backdrop-filter:blur(8px);padding:3px 8px;border-radius:4px;z-index:2;">{attribution_safe}</div>"#
            )
        },
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
        // Cache one hour — the image-cache populates on its own schedule,
        // and the underlying photo rarely changes per species.
        [(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"))],
        body,
    )
        .into_response()
}

/// Best-effort host extraction for the attribution chip. Avoids pulling in
/// a URL-parsing dep; treats the input as `scheme://host/path` and lifts
/// the host segment.
fn host_only(url: &str) -> String {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    after_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_only_strips_scheme_and_path() {
        assert_eq!(host_only("https://commons.wikimedia.org/wiki/File:Foo.jpg"), "commons.wikimedia.org");
        assert_eq!(host_only("https://www.macaulaylibrary.org/asset/123"), "macaulaylibrary.org");
        assert_eq!(host_only("http://example.com"), "example.com");
        assert_eq!(host_only("plainstring"), "plainstring");
    }
}
