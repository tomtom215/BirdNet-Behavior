//! Serves the OpenAPI description of the public JSON API.
//!
//! The committed [`openapi.json`](../../../openapi.json) (an OpenAPI 3.1
//! document) is embedded at build time and served verbatim at
//! `GET /api/v2/openapi.json`, so any tool — Swagger UI, Redoc, Postman,
//! `openapi-generator` — can point at a running station and get a complete,
//! machine-readable map of the API. The document is hand-maintained; the
//! `every_documented_path_is_routed` test below keeps it from drifting away
//! from the actual router.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};

use crate::state::AppState;

/// The OpenAPI 3.1 document for the public API, embedded at build time.
const OPENAPI_JSON: &str = include_str!("../../openapi.json");

/// Mount the OpenAPI spec route.
pub fn router() -> Router<AppState> {
    Router::new().route("/openapi.json", get(serve_openapi))
}

/// `GET /api/v2/openapi.json` — the embedded OpenAPI document.
async fn serve_openapi() -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            ),
        ],
        OPENAPI_JSON,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate schema");
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    #[test]
    fn embedded_spec_is_valid_and_complete() {
        let spec: serde_json::Value =
            serde_json::from_str(OPENAPI_JSON).expect("openapi.json must be valid JSON");
        assert_eq!(spec["openapi"], "3.1.0", "OpenAPI version");
        assert_eq!(
            spec["info"]["version"],
            env!("CARGO_PKG_VERSION"),
            "spec info.version must track the crate version"
        );
        let paths = spec["paths"].as_object().expect("paths object");
        // A guard against accidentally truncating the document.
        assert!(
            paths.len() >= 40,
            "expected the full API surface, found {} paths",
            paths.len()
        );
    }

    /// Every path the spec documents must actually be routed by the app — so a
    /// documented endpoint can never silently not exist (the classic spec drift).
    #[tokio::test]
    async fn every_documented_path_is_routed() {
        let spec: serde_json::Value = serde_json::from_str(OPENAPI_JSON).unwrap();
        let paths = spec["paths"].as_object().unwrap();
        // The mutating endpoints (`O-1`) are documented here but are *not* in
        // `public_routes()` — that router is asserted read-only by
        // `tests/public_router_is_read_only.rs`, and these authenticate with a
        // bearer token rather than a cookie. Merged in without their auth
        // layer, because this gate asks whether a documented path is routed at
        // all, not who may call it: a documented `POST` answers 405 to the
        // `GET` below, which is not the "404 echoing its own URL" that means
        // unrouted.
        let app = crate::routes::public_routes()
            .merge(crate::routes::api_write::router())
            .with_state(test_state());

        for path in paths.keys() {
            // Fill path templates with throwaway-but-valid (URL-safe) values.
            let concrete = path
                .replace("{filename}", "nope.wav")
                .replace("{scientific_name}", "Cardinalis_cardinalis");
            let url = format!("/api/v2{concrete}");

            let resp = app
                .clone()
                .oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap())
                .await
                .expect("router responds");
            let status = resp.status();
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();

            // An *unmatched* path under /api hits the JSON 404 fallback, whose
            // body uniquely echoes the request path (`{"error":"not found",
            // "path":"<url>"}`). A *matched* route returns anything else — a 200,
            // or a handler 404 whose body never echoes the request path (e.g.
            // "recording not found", or `{"error":"image cache not configured"}`).
            // So a documented path that isn't routed is exactly: 404 whose body
            // contains its own URL.
            let is_unrouted = status == StatusCode::NOT_FOUND && body.contains(&url);
            assert!(
                !is_unrouted,
                "documented path is not routed: {url} (status {status})"
            );
        }
    }
}
