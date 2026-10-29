//! A picture is shown with who made it and on what terms.
//!
//! The station fetched photographs from Wikipedia, AviCommons and Flickr for years and printed
//! no credit for any of them: `NOTICE` and the manual said "attribution shown on each image",
//! and nothing in the code did. The credit is now captured with the picture, kept beside the
//! file, and shown under the photograph on the species page, on a Picture credits page the
//! footer of every page links to, and in the image API's answer.
//!
//! Each assertion has a counterpart: a picture cached before credits were kept is shown with
//! *no* credit rather than with a made-up one, and a credit with a hostile address or name is
//! printed as text.

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use birdnet_integrations::species_images::{
    ImageCache, ImageCredit, ImageError, ImageProvider, SpeciesImage,
};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A provider with no network: it says nothing is available, which is all a page test needs.
struct Offline;

impl ImageProvider for Offline {
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
        let name = scientific_name.to_owned();
        Box::pin(async move { Err(ImageError::Http(format!("offline: {name}"))) })
    }
}

/// A provider that answers every species with a credited picture. Its address is empty, so the
/// cache has nothing to download and the test stays off the network.
struct Credited;

impl ImageProvider for Credited {
    fn fetch<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _scientific_name: &'life1 str,
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
        Box::pin(async move {
            Ok(SpeciesImage {
                url: String::new(),
                cached_path: None,
                width: 300,
                description: None,
                wiki_url: None,
                credit: Some(credit()),
            })
        })
    }
}

fn credit() -> ImageCredit {
    ImageCredit {
        title: "Blackbird <b>in</b> the rain".into(),
        author: "Jo \"J.\" Smith & Co".into(),
        source: "Wikimedia Commons".into(),
        source_url: "https://commons.wikimedia.org/wiki/File:Blackbird.jpg".into(),
        licence: "CC BY-SA 4.0".into(),
        licence_url: "https://creativecommons.org/licenses/by-sa/4.0/".into(),
    }
}

/// A station that has heard a blackbird (its picture credited) and a magpie (its picture cached
/// before credits were kept).
fn station(dir: &tempfile::TempDir) -> AppState {
    std::fs::write(dir.path().join("turdus_merula.jpg"), b"jpeg").unwrap();
    std::fs::write(
        dir.path().join("turdus_merula.credit.json"),
        serde_json::to_vec(&credit()).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join("pica_pica.jpg"), b"jpeg").unwrap();

    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-04-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'a.wav'),
                ('2026-04-02', '06:00:00', 'Pica pica', 'Eurasian Magpie', 0.9, 'b.wav');",
    )
    .expect("seed");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_image_cache(ImageCache::new(dir.path(), Arc::new(Offline), 300).expect("cache"))
}

async fn get(state: &AppState, uri: &str) -> String {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn a_species_page_prints_the_credit_under_its_photo() {
    let dir = tempfile::tempdir().unwrap();
    let html = get(
        &station(&dir),
        "/pages/species-info?name=Eurasian%20Blackbird",
    )
    .await;
    assert!(html.contains("Photo: "), "{html}");
    assert!(
        html.contains("Blackbird &lt;b&gt;in&lt;/b&gt; the rain"),
        "escaped title: {html}"
    );
    assert!(
        html.contains("Jo &quot;J.&quot; Smith &amp; Co"),
        "escaped author: {html}"
    );
    assert!(
        html.contains(r#"href="https://commons.wikimedia.org/wiki/File:Blackbird.jpg""#),
        "{html}"
    );
    assert!(
        html.contains(r#"href="https://creativecommons.org/licenses/by-sa/4.0/""#),
        "{html}"
    );
    assert!(html.contains(r#"href="/system/credits""#), "{html}");
    assert!(
        !html.contains("<b>in</b>"),
        "a title is text, not markup: {html}"
    );
}

/// The counterpart: a picture with no credit is shown with none, not with an invented one.
#[tokio::test]
async fn a_picture_cached_before_credits_were_kept_is_shown_with_no_credit() {
    let dir = tempfile::tempdir().unwrap();
    let html = get(&station(&dir), "/pages/species-info?name=Eurasian%20Magpie").await;
    assert!(
        html.contains("spp-info-img"),
        "the picture is still shown: {html}"
    );
    assert!(!html.contains("Photo: "), "{html}");
}

#[tokio::test]
async fn the_credits_page_lists_each_credited_picture_and_counts_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let html = get(&station(&dir), "/system/credits").await;
    assert!(html.contains("Picture <em>credits</em>"), "{html}");
    assert!(html.contains("Eurasian Blackbird"), "{html}");
    assert!(html.contains("CC BY-SA 4.0"), "{html}");
    assert!(html.contains("Jo &quot;J.&quot; Smith &amp; Co"), "{html}");
    assert!(
        !html.contains("Eurasian Magpie"),
        "an uncredited picture is not listed as if it had a credit: {html}"
    );
    assert!(
        html.contains("1 picture was cached before credits were kept"),
        "but it is counted: {html}"
    );
    assert!(
        html.contains("engraves them, which is an adaptation"),
        "{html}"
    );
    // On a phone each row stacks and its cells print their column's name; without the labels a
    // stacked row is a list of unexplained names. (Seen: the table squeezed to words broken
    // mid-word, "Wikimedi/a Common/s", at 390 px.)
    for label in ["Species", "Picture", "Author", "Source", "Licence"] {
        assert!(
            html.contains(&format!(r#"data-label="{label}""#)),
            "{label}: {html}"
        );
    }
    assert!(
        !html.contains("data-fits-phone"),
        "a five-column credits table is not read whole on a phone: {html}"
    );
}

#[tokio::test]
async fn a_credit_with_a_script_address_is_printed_not_linked() {
    let dir = tempfile::tempdir().unwrap();
    let state = station(&dir);
    let mut hostile = credit();
    hostile.source_url = "javascript:alert(1)".into();
    hostile.licence_url = "data:text/html,<script>1</script>".into();
    std::fs::write(
        dir.path().join("turdus_merula.credit.json"),
        serde_json::to_vec(&hostile).unwrap(),
    )
    .unwrap();
    // A fresh cache reads the credit as the file now has it.
    let state =
        state.with_image_cache(ImageCache::new(dir.path(), Arc::new(Offline), 300).unwrap());
    for uri in [
        "/pages/species-info?name=Eurasian%20Blackbird",
        "/system/credits",
    ] {
        let html = get(&state, uri).await;
        assert!(!html.contains("href=\"javascript"), "{uri}: {html}");
        assert!(!html.contains("href=\"data:"), "{uri}: {html}");
        assert!(
            html.contains("Wikimedia Commons"),
            "{uri}: still named: {html}"
        );
    }
}

#[tokio::test]
async fn every_page_links_to_the_credits() {
    let dir = tempfile::tempdir().unwrap();
    let state = station(&dir);
    for uri in ["/", "/species", "/system/changelog"] {
        let html = get(&state, uri).await;
        assert!(
            html.contains(r#"href="/system/credits""#),
            "{uri} has no link to the credits"
        );
    }
}

#[tokio::test]
async fn the_image_api_carries_the_credit() {
    let dir = tempfile::tempdir().unwrap();
    let state = station(&dir);
    let body = get(&state, "/api/v2/species/image/Turdus%20merula").await;
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(json["credit"]["author"], "Jo \"J.\" Smith & Co", "{body}");
    assert_eq!(json["credit"]["licence"], "CC BY-SA 4.0");
    let magpie: serde_json::Value =
        serde_json::from_str(&get(&state, "/api/v2/species/image/Pica%20pica").await).unwrap();
    assert!(magpie["credit"].is_null(), "{magpie}");
}

/// A list that could not be read is a fault, not "no pictures": the page says so.
#[tokio::test]
async fn a_species_list_that_cannot_be_read_is_an_error_not_an_empty_page() {
    let dir = tempfile::tempdir().unwrap();
    let state = station(&dir);
    state.with_db(|c| {
        c.execute_batch("DROP TABLE IF EXISTS species_summary; DROP VIEW IF EXISTS detections_analytic; DROP TABLE detections")
            .expect("break the reads");
    });
    let html = get(&state, "/system/credits").await;
    assert!(html.contains("couldn't load"), "{html}");
    assert!(
        !html.contains("No credited pictures are held yet"),
        "{html}"
    );
}

/// The answer for a picture fetched just now carries the credit too, not only one read back
/// from the cache.
#[tokio::test]
async fn the_image_api_carries_the_credit_of_a_picture_fetched_just_now() {
    let dir = tempfile::tempdir().unwrap();
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-04-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'a.wav');",
    )
    .expect("seed");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_image_cache(ImageCache::new(dir.path(), Arc::new(Credited), 300).unwrap());
    let body = get(&state, "/api/v2/species/image/Turdus%20merula").await;
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(json["status"], "found", "{body}");
    assert_eq!(json["credit"]["licence"], "CC BY-SA 4.0", "{body}");
    assert!(
        dir.path().join("turdus_merula.credit.json").exists(),
        "and it is kept beside the file for next time"
    );
}
