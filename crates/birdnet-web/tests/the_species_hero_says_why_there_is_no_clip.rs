//! A species page with nothing to play says why.
//!
//! Retention reclaims clips and keeps their rows, so a bird recorded a hundred
//! times can have no playable clip left. The panel said "No recording
//! captured for this species yet", which for that bird is false.
//!
//! Observed failing with the old branch restored: the reclaimed species read
//! "No recording captured for this species yet."

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, Clip_Pruned_At)
         VALUES ('2026-04-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'old.wav', 1790000000),
                ('2026-04-02', '06:00:00', 'Pica pica', 'Eurasian Magpie', 0.9, '', NULL),
                ('2026-04-03', '06:00:00', 'Parus major', 'Great Tit', 0.9, 'tit.wav', NULL);",
    )
    .expect("seed");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn hero(name: &str) -> String {
    let res = birdnet_web::server::build_router(station())
        .oneshot(
            Request::get(format!("/pages/species-hero?name={name}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn a_reclaimed_species_is_not_one_never_recorded() {
    let html = hero("Eurasian%20Blackbird").await;
    assert!(html.contains("reclaimed to free space"), "{html}");
    assert!(!html.contains("No recording captured"), "{html}");
}

#[tokio::test]
async fn a_species_with_no_clip_ever_says_so() {
    let html = hero("Eurasian%20Magpie").await;
    assert!(
        html.contains("No recording captured for this species yet."),
        "{html}"
    );
}

/// The counterpart: a species with a playable clip gets its player.
#[tokio::test]
async fn a_species_with_a_clip_gets_it() {
    let html = hero("Great%20Tit").await;
    assert!(html.contains("tit.wav"), "{html}");
    assert!(!html.contains("reclaimed"), "{html}");
}
