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
    // Check custom image directory first (BirdNET-Pi: CUSTOM_IMAGE).
    if let Some(custom_dir) = state.custom_image_dir() {
        let key = scientific_name.to_lowercase().replace(' ', "_");
        for ext in &["jpg", "jpeg", "png", "webp"] {
            let candidate = custom_dir.join(format!("{key}.{ext}"));
            if let Ok(bytes) = std::fs::read(&candidate) {
                let content_type = match *ext {
                    "png" => "image/png",
                    "webp" => "image/webp",
                    _ => "image/jpeg",
                };
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, content_type)],
                    bytes,
                )
                    .into_response();
            }
        }
    }

    let Some(cache) = state.image_cache() else {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": "image cache not configured"})
                .to_string()
                .into_bytes(),
        )
            .into_response();
    };

    // Serve from the on-disk cache, fetching on miss so every `<img>` tag
    // self-heals on first view. Mirrors `species_image_info` (which already
    // fetches on miss); without this, file requests 404 until the gallery's
    // background warmer happens to populate that species, leaving species- and
    // detection-detail previews permanently broken if the gallery is never
    // opened. `get_image` is a no-op network-wise once the file is cached.
    let path = match cache
        .get_cached(&scientific_name)
        .and_then(|img| img.cached_path)
    {
        Some(path) => path,
        None => match cache.get_image(&scientific_name).await {
            // Fetched and cached: serve the freshly downloaded bytes.
            Ok(image) => {
                let Some(path) = image.cached_path else {
                    // Provider resolved but the species genuinely has no image.
                    return (
                        StatusCode::NOT_FOUND,
                        [(header::CONTENT_TYPE, "application/json")],
                        json!({"error": "no image available"})
                            .to_string()
                            .into_bytes(),
                    )
                        .into_response();
                };
                path
            }
            // Lookup/download failed (offline, rate-limited, not found, …).
            Err(_) => {
                return (
                    StatusCode::NOT_FOUND,
                    [(header::CONTENT_TYPE, "application/json")],
                    json!({"error": "image not available"})
                        .to_string()
                        .into_bytes(),
                )
                    .into_response();
            }
        },
    };

    let Ok(bytes) = std::fs::read(&path) else {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": "cached file not readable"})
                .to_string()
                .into_bytes(),
        )
            .into_response();
    };

    // Determine content type from extension
    let content_type = path
        .extension()
        .and_then(|e| e.to_str())
        .map_or("image/jpeg", |ext| match ext.to_lowercase().as_str() {
            "png" => "image/png",
            "webp" => "image/webp",
            _ => "image/jpeg",
        });

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        bytes,
    )
        .into_response()
}
