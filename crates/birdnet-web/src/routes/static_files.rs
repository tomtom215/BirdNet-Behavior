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

/// Print-only stylesheet (loaded with `media="print"` by the layout).
const PRINT_CSS: &[u8] = include_bytes!("../../static/css/print.css");

/// PWA manifest (O-24). Browsers fetch this to discover `display:standalone`
/// + icons + shortcuts for the home-screen install flow.
const MANIFEST_WEBMANIFEST: &[u8] = include_bytes!("../../static/manifest.webmanifest");

/// Service worker (O-24). Cache-first for `/static/*`,
/// stale-while-revalidate for public dashboard surfaces, and a hard
/// network-only bypass for everything under `/admin/`, `/login`,
/// `/logout`, `/r/`, `/feeds/`. The bnb-build query param invalidates
/// the cache on a new release.
const SERVICE_WORKER_JS: &[u8] = include_bytes!("../../static/sw.js");

/// PWA icons (O-24). Three sizes per the manifest.
const ICON_192: &[u8] = include_bytes!("../../static/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../../static/icon-512.png");
const ICON_MASKABLE_512: &[u8] = include_bytes!("../../static/icon-maskable-512.png");

/// Pre-paint theme/density guard for standalone pages (admin, kiosk, player).
const THEME_GUARD_JS: &[u8] = include_bytes!("../../static/theme-guard.js");

/// Reconnecting live-detection WebSocket client (embedded at compile time).
const LIVE_DETECTIONS_JS: &[u8] = include_bytes!("../../static/live-detections.js");
/// Keeps script-built URLs inside a reverse-proxy prefix (M8).
const BASE_PATH_JS: &[u8] = include_bytes!("../../static/base-path.js");
/// Holds `every` polls back while the tab is hidden (M13).
const POLL_VISIBILITY_JS: &[u8] = include_bytes!("../../static/poll-visibility.js");
const CLIP_PLAYER_JS: &[u8] = include_bytes!("../../static/clip-player.js");
/// Gain, high-pass and compressor for every player, in the browser (P-8).
const LISTEN_FX_JS: &[u8] = include_bytes!("../../static/listen-fx.js");
/// The styles of those controls, which the script loads when it builds them.
const LISTEN_FX_CSS: &[u8] = include_bytes!("../../static/css/listen-fx.css");
/// Keeps SVG chart labels legible at any scale (see the script's header).
const CHART_TEXT_JS: &[u8] = include_bytes!("../../static/chart-text.js");

/// Type-to-filter for the admin settings page (embedded at compile time).
/// Hover, focus, keyboard and entrance for every chart (see the script's header).
const CHART_KIT_JS: &[u8] = include_bytes!("../../static/chart-kit.js");
const SETTINGS_FILTER_JS: &[u8] = include_bytes!("../../static/settings-filter.js");

/// Select-all for the search page's bulk action bar (embedded at compile time).
const SEARCH_SELECT_JS: &[u8] = include_bytes!("../../static/search-select.js");

/// Self-hosted webfonts (latin + latin-ext subsets), embedded at compile time
/// so the UI renders fully offline with no CDN dependency. `Inter Tight` (UI),
/// `Instrument Serif` (display) and `JetBrains Mono` (numeric) cover Latin
/// scripts; Noto Sans is the self-hosted fallback for Latin-script `BirdNET`
/// packs. Non-Latin scripts (CJK, Devanagari, …) fall back to the client's
/// installed system fonts named in the `--font-ui` stack.
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
    (
        "noto-sans-latin-400-normal.woff2",
        include_bytes!("../../static/fonts/noto-sans-latin-400-normal.woff2"),
    ),
    (
        "noto-sans-latin-ext-400-normal.woff2",
        include_bytes!("../../static/fonts/noto-sans-latin-ext-400-normal.woff2"),
    ),
    (
        "noto-sans-latin-500-normal.woff2",
        include_bytes!("../../static/fonts/noto-sans-latin-500-normal.woff2"),
    ),
    (
        "noto-sans-latin-ext-500-normal.woff2",
        include_bytes!("../../static/fonts/noto-sans-latin-ext-500-normal.woff2"),
    ),
    (
        "noto-sans-latin-600-normal.woff2",
        include_bytes!("../../static/fonts/noto-sans-latin-600-normal.woff2"),
    ),
    (
        "noto-sans-latin-ext-600-normal.woff2",
        include_bytes!("../../static/fonts/noto-sans-latin-ext-600-normal.woff2"),
    ),
];

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// Static file routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/static/htmx.min.js", get(htmx_js))
        .route("/static/htmx-sse.js", get(htmx_sse_js))
        .route("/static/theme-guard.js", get(theme_guard_js))
        .route("/static/live-detections.js", get(live_detections_js))
        .route("/static/poll-visibility.js", get(poll_visibility_js))
        .route("/static/base-path.js", get(base_path_js))
        .route("/static/clip-player.js", get(clip_player_js))
        .route("/static/listen-fx.js", get(listen_fx_js))
        .route("/static/chart-text.js", get(chart_text_js))
        .route("/static/settings-filter.js", get(settings_filter_js))
        .route("/static/search-select.js", get(search_select_js))
        .route("/static/css/app.css", get(app_css))
        .route("/static/css/print.css", get(print_css))
        .route("/static/css/listen-fx.css", get(listen_fx_css))
        .route("/static/fonts/{file}", get(font_file))
        // O-24 PWA assets.
        .route("/static/manifest.webmanifest", get(manifest_webmanifest))
        .route("/static/chart-kit.js", get(chart_kit_js))
        .route("/static/sw.js", get(service_worker_js))
        // `/favicon.ico` is the browser's *fallback* request: a document with
        // no `rel="icon"` asks for it unprompted, and this server used to
        // answer 404. `templates/layout.html` carries explicit icon links and
        // so escaped it — but seven full documents are rendered outside that
        // layout (login, share and its 404, onboarding, kiosk, the standalone
        // audio player, the admin shell, the log viewer) and none of them did.
        // Every one showed a blank tab and logged a 404 to the console.
        //
        // Routing the fallback fixes all seven, and the eighth someone writes
        // next year, which adding a link tag to each would not. It serves the
        // 192 px PNG for the reason layout.html already gives for using PNGs
        // here: every browser this dashboard supports accepts one, and an
        // actual `.ico` would be a second copy of the same artwork.
        //
        // Found by the visual-QA sweep the moment `/login` entered the route
        // table for the first time (see `qa_routes_cover_the_navigation`):
        //   login__light__desktop: console=["Failed to load resource: … 404 …"]
        .route("/favicon.ico", get(favicon))
        .route("/static/icon-192.png", get(icon_192))
        .route("/static/icon-512.png", get(icon_512))
        .route("/static/icon-maskable-512.png", get(icon_maskable_512))
}

async fn manifest_webmanifest() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/manifest+json"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        MANIFEST_WEBMANIFEST,
    )
}

async fn service_worker_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            // Workers self-version through `?v=` from the layout. We hint
            // no-store so the registered file always reflects the latest
            // shipped binary.
            (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
            // Lets the worker control top-level navigations.
            ("Service-Worker-Allowed".parse().unwrap(), "/"),
        ],
        SERVICE_WORKER_JS,
    )
}

/// The browser-tab icon for any document that does not name one itself.
async fn favicon() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        ICON_192,
    )
}

async fn icon_192() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        ICON_192,
    )
}

async fn icon_512() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        ICON_512,
    )
}

async fn icon_maskable_512() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        ICON_MASKABLE_512,
    )
}

async fn base_path_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        BASE_PATH_JS,
    )
}

async fn poll_visibility_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        POLL_VISIBILITY_JS,
    )
}

async fn chart_text_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            // Same policy as the other app scripts: it ships with the binary and
            // is requested with `?v=<version>`, so a day is safe.
            (header::CACHE_CONTROL, "public, max-age=86400"),
async fn chart_kit_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        CHART_KIT_JS,
    )
}

        ],
        CHART_TEXT_JS,
    )
}

async fn live_detections_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            // Short cache (not immutable): this script ships with the binary
            // and can change between versions under the same URL.
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        LIVE_DETECTIONS_JS,
    )
}

async fn settings_filter_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        SETTINGS_FILTER_JS,
    )
}

async fn search_select_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        SEARCH_SELECT_JS,
    )
}

async fn listen_fx_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        LISTEN_FX_CSS,
    )
}

async fn listen_fx_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            // Requested with `?v=<version>` like the other app scripts.
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        LISTEN_FX_JS,
    )
}

async fn clip_player_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        CLIP_PLAYER_JS,
    )
}

async fn theme_guard_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        THEME_GUARD_JS,
    )
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

async fn print_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        PRINT_CSS,
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
        assert_eq!(FONTS.len(), 20);
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

    #[test]
    fn chart_text_fitter_embedded_and_safe() {
        let js = std::str::from_utf8(CHART_TEXT_JS).unwrap();
        // The floor the legibility gate (tools/visual-qa/legibility.mjs) holds
        // the page to is below this, with room for rounding.
        assert!(js.contains("var FLOOR = 9.5;"));
        // Opt-out is documented by the script and honoured.
        assert!(js.contains("data-no-text-fit"));
        // Chart text is data (species names); it is only ever moved with
        // textContent, never parsed as markup.
        assert!(!js.contains(".innerHTML"));
        assert!(!js.contains("insertAdjacentHTML"));
        // It reads sizes through the CSSOM, which the CSP allows; it must not
        // write a `style` attribute (setAttribute / cssText), which it forbids.
        assert!(!js.contains("setAttribute(\"style\""));
        assert!(!js.contains("cssText"));
    }

    #[test]
    fn live_detections_client_embedded() {
        let js = std::str::from_utf8(LIVE_DETECTIONS_JS).unwrap();
        assert!(js.contains("/api/v2/ws/detections"));
        assert!(js.contains("birdnet:detection"));
        // Must never assign untrusted species names via innerHTML.
        assert!(!js.contains(".innerHTML"));
    }
}
    /// The chart kit renders species names into tooltips and a live region: only
    /// ever as text. And it must not write a `style` attribute, which the CSP
    /// forbids; `style.setProperty` goes through the CSSOM, which it allows.
    #[test]
    fn chart_kit_embedded_and_safe() {
        let js = std::str::from_utf8(CHART_KIT_JS).unwrap();
        assert!(js.contains("svg[data-ck]"));
        assert!(!js.contains(".innerHTML"));
        assert!(!js.contains("insertAdjacentHTML"));
        assert!(!js.contains("setAttribute(\"style\""));
        assert!(!js.contains("cssText"));
        // Plain script: no module syntax, no network.
        assert!(!js.contains("import "));
        assert!(!js.contains("fetch("));
    }

