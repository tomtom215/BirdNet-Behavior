//! Integration tests for the species image serving endpoints.
//!
//! Regression coverage for BUG-1: `GET /api/v2/species/image/{sci}/file` must
//! fetch-on-miss (mirroring the JSON `species_image_info` endpoint) rather than
//! 404 on a cold cache. Before the fix the `<img>` tags rendered across the
//! gallery, species-detail and detection-detail pages 404'd until the gallery's
//! background warmer happened to fill that species, so previews looked broken.
//!
//! The test is hermetic: a stubbed `ImageProvider` returns a URL pointing at a
//! throwaway localhost HTTP server, so the real fetch → download → store → serve
//! path is exercised without reaching Wikipedia/Wikimedia.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use birdnet_integrations::species_images::{ImageCache, ImageError, ImageProvider, SpeciesImage};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// Bytes the throwaway server returns. The content need not be a valid JPEG —
/// `ImageCache::get_image` only checks the `Content-Type` header to decide the
/// download is a genuine image, and the handler echoes whatever was stored.
const FAKE_IMAGE: &[u8] = b"\xFF\xD8\xFF\xE0fake-jpeg-bytes\xFF\xD9";

/// Spawn a minimal localhost HTTP server that answers every request with
/// `FAKE_IMAGE` and `Content-Type: image/jpeg`. Returns the bound address; the
/// accept loop runs until the test process exits.
async fn spawn_image_server() -> SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                // Drain the request first so the client finishes writing before
                // we reply (avoids a connection reset on the client side). One
                // read captures a GET request line + headers.
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    FAKE_IMAGE.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(FAKE_IMAGE).await;
                let _ = sock.flush().await;
            });
        }
    });
    addr
}

/// Stub provider that resolves any species to a localhost URL — no real network.
struct LocalProvider {
    base: String,
}

impl ImageProvider for LocalProvider {
    fn fetch<'life0, 'life1, 'async_trait>(
        &'life0 self,
        scientific_name: &'life1 str,
    ) -> Pin<Box<dyn Future<Output = Result<SpeciesImage, ImageError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        let url = format!("{}/{}.jpg", self.base, scientific_name.replace(' ', "_"));
        Box::pin(async move {
            Ok(SpeciesImage {
                url,
                cached_path: None,
                width: 300,
                description: None,
                wiki_url: None,
            })
        })
    }
}

/// Build an `AppState` whose image cache is backed by the stub provider and a
/// fresh (empty) on-disk cache directory.
fn state_with_stub_cache(base: String, cache_dir: &std::path::Path) -> AppState {
    let provider = Arc::new(LocalProvider { base });
    let cache = ImageCache::new(cache_dir, provider, 300).expect("build image cache");
    assert_eq!(cache.cached_count(), 0, "precondition: cache starts cold");

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:")).with_image_cache(cache)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn species_image_file_fetches_on_cold_cache() {
    let addr = spawn_image_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let app = build_router(state_with_stub_cache(format!("http://{addr}"), tmp.path()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/image/Turdus%20merula/file")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Before the fix this was 404 ("image not cached"); fetch-on-miss makes it 200.
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "cold-cache /file must fetch-on-miss, not 404"
    );

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        content_type.starts_with("image/"),
        "expected an image/* content-type, got {content_type:?}"
    );

    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(
        body.as_ref(),
        FAKE_IMAGE,
        "served bytes must be the freshly fetched image"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn species_image_file_caches_after_first_fetch() {
    let addr = spawn_image_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let app = build_router(state_with_stub_cache(format!("http://{addr}"), tmp.path()));

    // First request fetches-on-miss and writes the file to the on-disk cache.
    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/image/Parus%20major/file")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    // The cache file now exists on disk (keyed by the normalised scientific name).
    assert!(
        tmp.path().join("parus_major.jpg").exists(),
        "fetch-on-miss should persist the image to the disk cache"
    );

    // Second request is served straight from the warm cache.
    let second = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/image/Parus%20major/file")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let body = axum::body::to_bytes(second.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), FAKE_IMAGE);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn species_image_file_404_when_cache_not_configured() {
    // No image cache installed: the endpoint still reports a clean 404 rather
    // than panicking or hanging.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/image/Turdus%20merula/file")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
