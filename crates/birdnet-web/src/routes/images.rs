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
        Ok(image) => {
            (
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
            )
        }
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

/// Serve the cached species image file.
///
/// Returns the image bytes with the appropriate content type.
/// If the image is not cached, returns 404.
async fn species_image_file(
    State(state): State<AppState>,
    Path(scientific_name): Path<String>,
) -> impl IntoResponse {
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

    // Only serve from cache (no network fetch for file serving)
    let image = cache.get_cached(&scientific_name);
    let cached_path = image.and_then(|img| img.cached_path);

    let Some(path) = cached_path else {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": "image not cached"})
                .to_string()
                .into_bytes(),
        )
            .into_response();
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
