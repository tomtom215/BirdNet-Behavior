//! The "About this species" card says why it has nothing, and says it to the
//! right reader.
//!
//! The card told a station whose photo cache was already on to "Enable
//! `--image-cache-dir`" — a sentence about a flag, to a person with a garden,
//! about something already done. A station that is simply offline, or has not
//! fetched this species yet, looks exactly like that from the outside, and is the
//! common case. The honest line is "it appears once the station has fetched it".
//!
//! Observed failing with the old branch restored: the cache-on station was told
//! to enable the cache.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

use birdnet_integrations::species_images::{ImageCache, ImageError, ImageProvider, SpeciesImage};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// A provider that can never find a picture — an offline station, from the
/// card's side.
struct Offline;

impl ImageProvider for Offline {
    fn fetch<'life0, 'life1, 'async_trait>(
        &'life0 self,
        scientific_name: &'life1 str,
    ) -> Pin<Box<dyn Future<Output = Result<SpeciesImage, ImageError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        let name = scientific_name.to_owned();
        Box::pin(async move { Err(ImageError::NotFound(name)) })
    }
}

fn station(with_cache: bool, dir: &std::path::Path) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
         VALUES ('2026-09-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
        [],
    )
    .unwrap();
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    if with_cache {
        let cache = ImageCache::new(dir, Arc::new(Offline), 300).expect("build image cache");
        state.with_image_cache(cache)
    } else {
        state
    }
}

async fn card(state: AppState) -> String {
    let res = build_router(state)
        .oneshot(
            Request::get("/pages/species-info?name=Eurasian%20Blackbird")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_station_with_the_cache_on_is_not_told_to_enable_it() {
    let dir = tempfile::tempdir().unwrap();
    let html = card(station(true, dir.path())).await;
    assert!(html.contains("No additional info for"), "{html}");
    assert!(
        html.contains("once the station has fetched"),
        "it should say the photo is still to come: {html}"
    );
    assert!(
        !html.contains("--image-cache-dir"),
        "the cache is on, so the flag that turns it on is not the answer: {html}"
    );
}

/// The counterpart, and the discrimination: a station with the cache off is
/// still told what is switched off — in words first, the flag second.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_station_with_the_cache_off_is_told_it_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let html = card(station(false, dir.path())).await;
    assert!(html.contains("switched off on this station"), "{html}");
    assert!(
        html.contains("For whoever set it up") && html.contains("--image-cache-dir"),
        "the operator still gets the flag: {html}"
    );
}
