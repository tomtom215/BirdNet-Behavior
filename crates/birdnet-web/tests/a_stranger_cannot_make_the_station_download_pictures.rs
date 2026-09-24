//! A stranger cannot make the station download pictures of anything.
//!
//! `GET /api/v2/species/image/{name}/file` needs no sign-in, and on a cache
//! miss it asked the image provider about whatever `{name}` was — any
//! Wikipedia title — then downloaded and stored the answer, up to 8 MiB a
//! time. The disk cache kept writing files after it was full ("Still write the
//! file"), so a loop over titles filled the card the recordings live on.
//!
//! Every picture the UI asks for is of a bird the station has detected: each
//! avatar is drawn from a detection row. So a miss is looked up only for a
//! name in `detections`. The counterpart holds that a heard bird still is.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::Request;
use birdnet_integrations::species_images::types::{ImageError, SpeciesImage};
use birdnet_integrations::species_images::{ImageCache, ImageProvider};
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// A provider that counts how often it is asked, and never has a picture.
struct Counting(Arc<AtomicUsize>);

impl ImageProvider for Counting {
    fn fetch<'life0, 'life1, 'async_trait>(
        &'life0 self,
        scientific_name: &'life1 str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<SpeciesImage, ImageError>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        self.0.fetch_add(1, Ordering::SeqCst);
        let name = scientific_name.to_string();
        Box::pin(async move { Err(ImageError::NotFound(name)) })
    }
}

fn state(asked: &Arc<AtomicUsize>, dir: &std::path::Path) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES ('2026-05-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
        [],
    )
    .expect("seed");
    let cache = ImageCache::new(dir, Arc::new(Counting(Arc::clone(asked))), 300).expect("cache");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:")).with_image_cache(cache)
}

async fn get(state: &AppState, uri: &str, status: u16) {
    let resp = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    // The provider has nothing either way; what matters is whether it was
    // asked.
    assert_eq!(resp.status().as_u16(), status, "{uri}");
}

#[tokio::test]
async fn a_name_the_station_has_never_heard_is_not_looked_up() {
    let dir = tempfile::tempdir().unwrap();
    let asked = Arc::new(AtomicUsize::new(0));
    let st = state(&asked, dir.path());
    get(&st, "/api/v2/species/image/Eiffel%20Tower/file", 404).await;
    // The metadata endpoint looked up on a miss too.
    get(&st, "/api/v2/species/image/Anything%20at%20all", 200).await;
    assert_eq!(
        asked.load(Ordering::SeqCst),
        0,
        "the provider was asked about a stranger's titles"
    );
}

#[tokio::test]
async fn a_bird_the_station_has_heard_is_still_looked_up() {
    let dir = tempfile::tempdir().unwrap();
    let asked = Arc::new(AtomicUsize::new(0));
    let st = state(&asked, dir.path());
    get(&st, "/api/v2/species/image/Turdus%20merula/file", 404).await;
    assert_eq!(asked.load(Ordering::SeqCst), 1, "the file endpoint");
    get(&st, "/api/v2/species/image/Turdus%20merula", 200).await;
    // The miss is remembered (MISS_TTL), so the second ask may be served
    // from that; at least the first was a real lookup.
    assert!(asked.load(Ordering::SeqCst) >= 1);
}
