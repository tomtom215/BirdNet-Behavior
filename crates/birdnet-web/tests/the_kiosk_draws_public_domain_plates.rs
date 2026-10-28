//! The kiosk draws a public-domain plate where there is one.
//!
//! `/api/v2/species/plate/{name}/file` answers with, in order: the operator's own art for the
//! species, the nineteenth-century plate the built-in index names for it, or `404` (most species:
//! the books are of European and Asian birds), on which the kiosk asks for the photograph.
//! Each answer says which it is in `X-Bnb-Image-Source`, and the plate is only ever fetched for a
//! bird this station has heard, like a photograph.
//!
//! No network: the cache is pre-filled, or its provider is a counter that has nothing, so a test
//! can say how many times the station went looking.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_integrations::species_images::{
    ImageCache, ImageCredit, ImageError, ImageProvider, PlateIndex, SpeciesImage,
};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A provider with nothing to offer that counts how often it was asked.
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
        let name = scientific_name.to_owned();
        Box::pin(async move { Err(ImageError::NotFound(name)) })
    }
}

fn plate_credit() -> ImageCredit {
    ImageCredit {
        title: "Turdus merula, plate from The Birds of Europe (1832–1837)".into(),
        author: "John Gould".into(),
        source: "Wikimedia Commons".into(),
        source_url: "https://commons.wikimedia.org/wiki/File:Turdus_merula_plate.jpg".into(),
        licence: "Public domain".into(),
        licence_url: String::new(),
    }
}

/// A station that has heard a blackbird (which has a plate, cached) and a cardinal (which has
/// none), and never a Blue Jay. `calls` counts the times its plate provider was asked.
fn station(plates: &tempfile::TempDir, calls: &Arc<AtomicUsize>, with_cache: bool) -> AppState {
    std::fs::write(plates.path().join("turdus_merula.jpg"), b"plate-bytes").unwrap();
    std::fs::write(
        plates.path().join("turdus_merula.credit.json"),
        serde_json::to_vec(&plate_credit()).unwrap(),
    )
    .unwrap();
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES (date('now','localtime'), '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'a.wav'),
                (date('now','localtime'), '06:30:00', 'Cardinalis cardinalis', 'Northern Cardinal', 0.9, 'b.wav');",
    )
    .expect("seed");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    if with_cache {
        state.with_plate_cache(
            ImageCache::new(plates.path(), Arc::new(Counting(calls.clone())), 900).expect("cache"),
        )
    } else {
        state
    }
}

async fn request(state: &AppState, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    (status, headers, bytes.to_vec())
}

fn source(headers: &axum::http::HeaderMap) -> &str {
    headers
        .get("x-bnb-image-source")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

#[tokio::test]
async fn a_bird_with_a_plate_is_served_the_plate_and_told_so() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (status, headers, body) = request(
        &station(&dir, &calls, true),
        "/api/v2/species/plate/Turdus%20merula/file",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(source(&headers), "plate");
    assert_eq!(body, b"plate-bytes");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a cached plate asks no one"
    );
}

/// Art the operator put in the custom image directory comes first, and says so: the kiosk draws it
/// as it was made and not as a plate. The counterpart is the test above, which has no such file.
#[tokio::test]
async fn the_operators_own_art_is_served_before_the_plate() {
    let (plates, art) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(art.path().join("turdus_merula.png"), b"their-art").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let state = station(&plates, &calls, true).with_custom_image_dir(art.path().to_path_buf());
    let (status, headers, body) =
        request(&state, "/api/v2/species/plate/Turdus%20merula/file").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(source(&headers), "custom");
    assert_eq!(body, b"their-art");
}

/// Most species have no plate. The endpoint says `404` and the kiosk goes on to the photograph; it
/// does so after asking its provider once, which is what a heard bird is entitled to.
#[tokio::test]
async fn a_heard_bird_with_no_plate_is_a_404_after_one_look() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (status, headers, _) = request(
        &station(&dir, &calls, true),
        "/api/v2/species/plate/Cardinalis%20cardinalis/file",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(source(&headers), "", "a refusal is not a picture");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// The endpoint is public, so a stranger cannot make the station look up any name they like.
#[tokio::test]
async fn a_bird_the_station_has_not_heard_is_not_looked_up() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (status, _, _) = request(
        &station(&dir, &calls, true),
        "/api/v2/species/plate/Cyanocitta%20cristata/file",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_station_without_a_plate_cache_answers_404_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (status, _, body) = request(
        &station(&dir, &calls, false),
        "/api/v2/species/plate/Turdus%20merula/file",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(String::from_utf8_lossy(&body).contains("plates not configured"));
}

/// The plate's credit is on the credits page like a photograph's, with the books the plates are
/// from; a station with no plate cache has no such section.
#[tokio::test]
async fn the_credits_page_names_the_books_and_the_plates_held() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (_, _, body) = request(&station(&dir, &calls, true), "/system/credits").await;
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("The plates the kiosk draws"), "{html}");
    assert!(html.contains("The Birds of Europe"), "{html}");
    assert!(
        html.contains("https://commons.wikimedia.org/wiki/Category:The_Birds_of_Europe_(Gould)"),
        "{html}"
    );
    assert!(
        html.contains("plate from The Birds of Europe (1832–1837)"),
        "{html}"
    );
    assert!(html.contains("Public domain"), "{html}");

    let calls = Arc::new(AtomicUsize::new(0));
    let (_, _, body) = request(&station(&dir, &calls, false), "/system/credits").await;
    assert!(
        !String::from_utf8_lossy(&body).contains("The plates the kiosk draws"),
        "no plate cache, no plates section"
    );
}

/// The kiosk marks each figure whose bird has a plate with its book, which is how the script
/// names the books it drew in the foot, and marks no other.
#[tokio::test]
async fn the_kiosk_marks_the_figures_that_have_a_plate_with_their_book() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (status, _, body) =
        request(&station(&dir, &calls, true), "/pages/kiosk-content?ar=1.6").await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8_lossy(&body);
    let (_, work) = PlateIndex::embedded()
        .plate_for("Turdus merula")
        .expect("the index has a blackbird plate");
    let figure = |sci: &str| {
        html.split("<g class=\"plate-fig\"")
            .find(|f| f.contains(&format!("data-sci=\"{sci}\"")))
            .and_then(|f| f.split('>').next())
            .unwrap_or_default()
            .to_owned()
    };
    assert!(
        figure("Turdus merula").contains(&format!("data-work=\"{}\"", work.citation())),
        "{}",
        figure("Turdus merula")
    );
    let cardinal = figure("Cardinalis cardinalis");
    assert!(!cardinal.is_empty(), "the cardinal is drawn: {html}");
    assert!(!cardinal.contains("data-work"), "{cardinal}");
}
