//! Embedded static file serving.
//!
//! Serves JavaScript, CSS, and fonts compiled into the binary via
//! `include_bytes!`. This keeps deployments fully air-gapped with no external
//! CDN dependencies — important for offline Raspberry Pi field installs.

use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::{Router, routing::get};

use crate::state::AppState;

/// HTMX library (minified, embedded at compile time).
const HTMX_JS: &[u8] = include_bytes!("../../static/htmx.min.js");

/// HTMX SSE extension (embedded at compile time).
const HTMX_SSE_JS: &[u8] = include_bytes!("../../static/htmx-sse.js");

/// Design-system stylesheet (tokens, atoms, shell, screen layouts).
const APP_CSS: &[u8] = include_bytes!("../../static/css/app.css");

/// Self-hosted webfonts (latin + latin-ext subsets). Non-Latin scripts fall
/// back to Noto Sans, which the page `<head>` loads as a progressive enhancement.
const FONTS: &[(&str, &[u8])] = &[
    (
        "inter-tight-latin-400-normal.woff2",
        include_bytes!("../../static/fonts/inter-tight-latin-400-normal.woff2"),
    ),
    (
        "inter-tight-latin-ext-400-normal.woff2",
        include_bytes!("../../static/fonts/inter-tight-latin-ext-400-normal.woff2"),
    ),
    (
        "inter-tight-latin-500-normal.woff2",
        include_bytes!("../../static/fonts/inter-tight-latin-500-normal.woff2"),
    ),
    (
        "inter-tight-latin-ext-500-normal.woff2",
        include_bytes!("../../static/fonts/inter-tight-latin-ext-500-normal.woff2"),
    ),
    (
        "inter-tight-latin-600-normal.woff2",
        include_bytes!("../../static/fonts/inter-tight-latin-600-normal.woff2"),
    ),
    (
        "inter-tight-latin-ext-600-normal.woff2",
        include_bytes!("../../static/fonts/inter-tight-latin-ext-600-normal.woff2"),
    ),
    (
        "instrument-serif-latin-400-normal.woff2",
        include_bytes!("../../static/fonts/instrument-serif-latin-400-normal.woff2"),
    ),
    (
        "instrument-serif-latin-ext-400-normal.woff2",
        include_bytes!("../../static/fonts/instrument-serif-latin-ext-400-normal.woff2"),
    ),
    (
        "instrument-serif-latin-400-italic.woff2",
        include_bytes!("../../static/fonts/instrument-serif-latin-400-italic.woff2"),
    ),
    (
        "instrument-serif-latin-ext-400-italic.woff2",
        include_bytes!("../../static/fonts/instrument-serif-latin-ext-400-italic.woff2"),
    ),
    (
        "jetbrains-mono-latin-400-normal.woff2",
        include_bytes!("../../static/fonts/jetbrains-mono-latin-400-normal.woff2"),
    ),
    (
        "jetbrains-mono-latin-ext-400-normal.woff2",
        include_bytes!("../../static/fonts/jetbrains-mono-latin-ext-400-normal.woff2"),
    ),
    (
        "jetbrains-mono-latin-500-normal.woff2",
        include_bytes!("../../static/fonts/jetbrains-mono-latin-500-normal.woff2"),
    ),
    (
        "jetbrains-mono-latin-ext-500-normal.woff2",
        include_bytes!("../../static/fonts/jetbrains-mono-latin-ext-500-normal.woff2"),
    ),
];

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// Static file routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/static/htmx.min.js", get(htmx_js))
        .route("/static/htmx-sse.js", get(htmx_sse_js))
        .route("/static/css/app.css", get(app_css))
        .route("/static/fonts/{file}", get(font_file))
}

async fn htmx_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        HTMX_JS,
    )
}

async fn htmx_sse_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        HTMX_SSE_JS,
    )
}

async fn app_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        APP_CSS,
    )
}

async fn font_file(axum::extract::Path(file): axum::extract::Path<String>) -> impl IntoResponse {
    match FONTS.iter().find(|(name, _)| *name == file) {
        Some((_, bytes)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, IMMUTABLE),
            ],
            *bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fonts_embedded_and_valid_woff2() {
        // Every advertised face must be present and carry the woff2 magic ("wOF2").
        assert_eq!(FONTS.len(), 14);
        for (name, bytes) in FONTS {
            assert!(bytes.len() > 1000, "{name} suspiciously small");
            assert_eq!(&bytes[0..4], b"wOF2", "{name} is not a woff2 file");
        }
    }

    #[test]
    fn app_css_has_tokens_and_fontface() {
        let css = std::str::from_utf8(APP_CSS).unwrap();
        assert!(css.contains("--moss"));
        assert!(css.contains("@font-face"));
        assert!(css.contains("Inter Tight"));
    }
}
