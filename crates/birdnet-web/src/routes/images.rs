//! Species image API endpoints.
//!
//! Serves cached species images and provides metadata about species
//! images from Wikipedia. Images are fetched on-demand and cached
//! to disk for offline/air-gapped operation.

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};

use crate::state::AppState;

/// Image routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/species/image/{scientific_name}", get(species_image_info))
        .route(
            "/species/image/{scientific_name}/file",
            get(species_image_file),
        )
        .route(
            "/species/plate/{scientific_name}/file",
            get(species_plate_file),
        )
}

/// Get species image metadata (URL, cache status, description).
///
/// Does NOT fetch or download the image -- returns metadata only.
/// If the species image is cached, returns the cached info.
/// Otherwise, queries Wikipedia for the image URL and description.
async fn species_image_info(
    State(state): State<AppState>,
    Path(scientific_name): Path<String>,
) -> (StatusCode, Json<Value>) {
    let Some(cache) = state.image_cache() else {
        return (
            StatusCode::OK,
            Json(json!({
                "status": "disabled",
                "message": "Species image caching is not configured. Start with --image-cache-dir to enable."
            })),
        );
    };

    // Check cache first (synchronous, no network)
    if let Some(image) = cache.get_cached(&scientific_name) {
        return (
            StatusCode::OK,
            Json(json!({
                "status": "cached",
                "scientific_name": scientific_name,
                "url": image.url,
                "cached": image.cached_path.is_some(),
                "description": image.description,
                "wiki_url": image.wiki_url,
                "width": image.width,
                "credit": image.credit,
            })),
        );
    }

    let name = scientific_name.clone();
    if !state.run_blocking(move |s| has_been_heard(s, &name)).await {
        return (
            StatusCode::OK,
            Json(json!({
                "status": "not_found",
                "scientific_name": scientific_name,
                "error": "not a species this station has detected",
            })),
        );
    }

    // Try to fetch from Wikipedia (get_image fetches and caches in one step).
    match cache.get_image(&scientific_name).await {
        Ok(image) => (
            StatusCode::OK,
            Json(json!({
                "status": "found",
                "scientific_name": scientific_name,
                "url": image.url,
                "cached": image.cached_path.is_some(),
                "description": image.description,
                "wiki_url": image.wiki_url,
                "width": image.width,
                "credit": image.credit,
            })),
        ),
        Err(e) => (
            StatusCode::OK,
            Json(json!({
                "status": "not_found",
                "scientific_name": scientific_name,
                "error": e.to_string(),
            })),
        ),
    }
}

/// Whether the station has detected `scientific_name` — the only names a
/// cache miss is looked up for.
///
/// Both image endpoints are public, and a miss used to be looked up and
/// downloaded for any name at all: a stranger could have the station fetch
/// and store a picture of every Wikipedia title in turn. Every picture the UI
/// asks for is of a bird in a detection row, so nothing it shows is lost. A
/// database that cannot answer answers "no": the picture is decoration.
fn has_been_heard(state: &AppState, scientific_name: &str) -> bool {
    state
        .with_read_db(|conn| {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM detections WHERE Sci_Name = ?1)",
                [scientific_name],
                |r| r.get::<_, bool>(0),
            )
        })
        .unwrap_or(false)
}

/// How long a browser may reuse an answer from the image file endpoint.
///
/// Both the picture and the 404 carry this. Neither used to carry any cache
/// header at all, and with no `ETag` or `Last-Modified` either there was
/// nothing for a browser to revalidate against, so every render fetched again.
/// That cost nothing while the only `<img>` tags were on the species gallery
/// and the detail page. It is not nothing now every detection row's avatar has
/// one: the live feed re-renders on a timer, and each re-render would have
/// pulled a fresh copy of the same thumbnail down the same wire.
///
/// Five minutes is short enough that an admin who blacklists a photo sees it
/// go within the time it takes to walk to the window, and long enough that a
/// feed polling every few seconds asks once.
///
/// `private`, not `public`: a station may sit behind a shared proxy, and its
/// pages are not ours to let a cache serve to somebody else.
const IMAGE_CACHE_CONTROL: &str = "private, max-age=300";

/// Which kind of picture a file response is: `custom` for a file the operator put in the custom
/// image directory, absent for everything the provider chain fetched. `static/plate.js` reads it.
const IMAGE_SOURCE_HEADER: axum::http::HeaderName =
    axum::http::HeaderName::from_static("x-bnb-image-source");

/// Build a JSON `{"error": msg}` response for the image file endpoint.
///
/// Carries [`IMAGE_CACHE_CONTROL`] as well. A species the provider has no
/// picture for answers 404 for as long as that stays true, and a browser that
/// re-asks on every render of every row is the same waste as re-downloading
/// the picture — worse, in fact, because a miss is the case that reaches
/// through to the provider.
fn image_error(status: StatusCode, msg: &str) -> axum::response::Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, IMAGE_CACHE_CONTROL),
        ],
        json!({ "error": msg }).to_string().into_bytes(),
    )
        .into_response()
}

/// "Nothing to show": `204`, kept as long as an error answer is, so a kiosk that redraws does not
/// ask again each time.
fn no_picture() -> axum::response::Response {
    (
        StatusCode::NO_CONTENT,
        [(header::CACHE_CONTROL, IMAGE_CACHE_CONTROL)],
    )
        .into_response()
}

/// The operator's own picture of a species, if they put one in the custom image directory: one
/// file per species. (Not BirdNET-Pi's `CUSTOM_IMAGE`, which is one picture of the site; that is
/// `routes/dashboard_image.rs`.) It answers with the `custom` source header, which is how the
/// kiosk knows it is art somebody made and draws it as it is.
async fn custom_picture(
    state: &AppState,
    scientific_name: &str,
) -> Option<axum::response::Response> {
    let custom_dir = state.custom_image_dir()?;
    let key = scientific_name.to_lowercase().replace(' ', "_");
    // Reject path separators / traversal in the URL-decoded species segment so a request like
    // `/api/v2/species/image/..%2f..%2fetc%2fpasswd/file` can't escape the custom image
    // directory. An unsafe key simply falls through to the cache path (which strips `/` in its
    // own key derivation). `replace(' ', "_")` only handles spaces, not `/`.
    if key.contains('/') || key.contains('\\') || key.contains("..") || key.contains('\0') {
        return None;
    }
    for ext in ["jpg", "jpeg", "png", "webp"] {
        let candidate = custom_dir.join(format!("{key}.{ext}"));
        if let Ok(bytes) = tokio::fs::read(&candidate).await {
            let content_type = match ext {
                "png" => "image/png",
                "webp" => "image/webp",
                _ => "image/jpeg",
            };
            return Some(
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, content_type),
                        (header::CACHE_CONTROL, IMAGE_CACHE_CONTROL),
                        // The kiosk plate engraves a photograph but draws art as it is made, and
                        // this is the only way it can tell which one it was handed.
                        (IMAGE_SOURCE_HEADER, "custom"),
                    ],
                    bytes,
                )
                    .into_response(),
            );
        }
    }
    None
}

/// The content type a cached picture is served as, from its extension.
fn content_type_of(path: &std::path::Path) -> &'static str {
    path.extension()
        .and_then(|e| e.to_str())
        .map_or("image/jpeg", |ext| match ext.to_lowercase().as_str() {
            "png" => "image/png",
            "webp" => "image/webp",
            _ => "image/jpeg",
        })
}

/// Serve the species image file bytes.
///
/// Checks the custom image directory first, then the Wikipedia cache, fetching
/// on a cache miss (like `species_image_info`) so `<img>` previews populate on
/// first view. Returns the image bytes with the appropriate content type, or
/// 404 when no cache is configured or the species genuinely has no image.
async fn species_image_file(
    State(state): State<AppState>,
    Path(scientific_name): Path<String>,
) -> impl IntoResponse {
    if let Some(custom) = custom_picture(&state, &scientific_name).await {
        return custom;
    }

    let Some(cache) = state.image_cache() else {
        return image_error(StatusCode::NOT_FOUND, "image cache not configured");
    };

    // Resolve the image, fetching on miss so every `<img>` tag self-heals on
    // first view. Mirrors `species_image_info` (which already fetches on miss);
    // without this, file requests 404 until the gallery's background warmer
    // happens to populate that species, leaving species- and detection-detail
    // previews permanently broken if the gallery is never opened. `get_image`
    // is a no-op network-wise once the file is already cached.
    let image = if let Some(image) = cache.get_cached(&scientific_name) {
        image
    } else {
        let name = scientific_name.clone();
        if !state.run_blocking(move |s| has_been_heard(s, &name)).await {
            return image_error(StatusCode::NOT_FOUND, "image not available");
        }
        match cache.get_image(&scientific_name).await {
            Ok(image) => image,
            // Lookup/download failed (offline, rate-limited, not found, …).
            Err(_) => return image_error(StatusCode::NOT_FOUND, "image not available"),
        }
    };

    // Honour the admin image blacklist: never serve a blacklisted URL. Only
    // query when the URL is known (image fetched/warmed this session) — disk-
    // only hits carry no URL and are covered by the cache purge in
    // `add_blacklist`. A blacklisted hit also evicts the cached file so the
    // re-fetch is refused rather than re-served from disk next time.
    //
    // A blacklist that cannot be read fails *closed*. An entry is the operator
    // withdrawing a picture — a wrong bird, an unlicensed photo, something that
    // should not be on the station's pages — and serving it because the database
    // happened not to answer would undo that decision at exactly the moment
    // nobody is looking. The cost of refusing is a placeholder avatar for as long
    // as the fault lasts, and the same database already gates the request above:
    // `has_been_heard` answers "no" when it cannot answer.
    let blacklisted = if image.url.is_empty() {
        Ok(false)
    } else {
        let (name, url) = (scientific_name.clone(), image.url.clone());
        state
            .run_blocking(move |s| {
                s.with_read_db(|conn| birdnet_db::sqlite::is_image_blacklisted(conn, &name, &url))
            })
            .await
    };
    match blacklisted {
        Ok(false) => {}
        Ok(true) => {
            cache.remove(&scientific_name);
            return image_error(StatusCode::NOT_FOUND, "image blacklisted");
        }
        Err(e) => {
            tracing::warn!(error = %e, species = %scientific_name, "image blacklist could not be read; not serving the picture");
            // Not `image_error`: that answer is cacheable for five minutes, and a
            // fault that clears in one should not leave a browser holding a refusal.
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                json!({ "error": "image blacklist unavailable" })
                    .to_string()
                    .into_bytes(),
            )
                .into_response();
        }
    }

    // Provider resolved but the species genuinely has no cached image bytes.
    let Some(path) = image.cached_path else {
        return image_error(StatusCode::NOT_FOUND, "no image available");
    };

    let Ok(bytes) = tokio::fs::read(&path).await else {
        return image_error(StatusCode::NOT_FOUND, "cached file not readable");
    };

    let content_type = content_type_of(&path);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, IMAGE_CACHE_CONTROL),
        ],
        bytes,
    )
        .into_response()
}

/// Serve a species' picture for the kiosk plate: the operator's own art if they supplied some,
/// else the public-domain plate the index names for the species, fetched from Wikimedia Commons on
/// the first ask and kept. `204` for a species with no plate, which is the normal case (about
/// 7% of BirdNET's species have one): the kiosk then asks for the photograph instead. It is not a
/// `404` because nothing is missing, and a browser logs a failed request for every `404`, once per
/// species on every redraw.
///
/// The answer carries `X-Bnb-Image-Source: plate`, so the kiosk draws it whole, as a plate, and
/// not engraved as a photograph. Gated like the photograph endpoint: only species this station
/// has heard are fetched. The admin photo blacklist is not consulted: it names photograph URLs
/// an operator withdrew, and a plate is a curated public-domain work; to replace one, put a
/// picture in the custom image directory, which is looked at first.
async fn species_plate_file(
    State(state): State<AppState>,
    Path(scientific_name): Path<String>,
) -> impl IntoResponse {
    if let Some(custom) = custom_picture(&state, &scientific_name).await {
        return custom;
    }
    let Some(cache) = state.plate_cache() else {
        return no_picture();
    };
    let image = if let Some(image) = cache.get_cached(&scientific_name) {
        image
    } else {
        let name = scientific_name.clone();
        if !state.run_blocking(move |s| has_been_heard(s, &name)).await {
            return image_error(StatusCode::NOT_FOUND, "image not available");
        }
        match cache.get_image(&scientific_name).await {
            Ok(image) => image,
            Err(_) => return no_picture(),
        }
    };
    let Some(path) = image.cached_path else {
        return no_picture();
    };
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return image_error(StatusCode::NOT_FOUND, "cached file not readable");
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type_of(&path)),
            (header::CACHE_CONTROL, IMAGE_CACHE_CONTROL),
            (IMAGE_SOURCE_HEADER, "plate"),
        ],
        bytes,
    )
        .into_response()
}
