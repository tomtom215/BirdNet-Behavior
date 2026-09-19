//! A model score shown to a reader is a percentage, never `0.87`.
//!
//! # What this guards
//!
//! 0.16.0 replaced the bare decimal with `87%` "beside every detection", and
//! the CHANGELOG says so. Two surfaces were missed, and both of them are ones
//! a reader meets without looking for them:
//!
//! * The Today page's **Best recordings** card — `dashboard/partials.rs`
//!   wrote `{time_short} · {conf:.2}`, so the front page printed
//!   `06:10 · 0.99` under a heading reading "Today · Highest confidence".
//! * The **clip player's now-playing strip** — `recordings.rs` built
//!   `data-clip-meta="{time} · {date} · {:.2}"`, which `recordings.html`
//!   assigns straight to `.rc-hp-meta` when a clip starts playing.
//!
//! Neither was reachable by reading `conf_bar`, which is where the percentage
//! lives and which both of these deliberately do not use: the card is too
//! small for a track, and the player strip is a plain-text attribute.
//!
//! # Why it looks for the decimal rather than for the percent sign
//!
//! A page is full of percent signs, so asserting one is present says almost
//! nothing. What the defect looks like is a confidence written as `0.` and
//! then two digits, next to the separator these surfaces use. That is what is
//! asserted against — with a precondition that the fixture actually produced
//! the row, because both of these sections render nothing at all when there is
//! no clip to show and "no rows" would otherwise pass.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Three of today's detections with clips, at confidences whose bare-decimal
/// form is unmistakable: `0.99`, `0.87`, `0.72`.
fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    for (i, conf) in [0.99_f64, 0.87, 0.72].into_iter().enumerate() {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens,
                  Overlap, File_Name, chunk_offset_secs)
             VALUES (date('now','localtime'), ?1, 'Turdus merula',
                     'Eurasian Blackbird', ?2, 0.7, 38, 1.25, 0.0, ?3, 0)",
            rusqlite::params![format!("0{}:10:00", 6 + i), conf, format!("clip{i}.wav")],
        )
        .expect("seed");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The three seeded scores, written the way the defect wrote them.
const BARE: [&str; 3] = ["0.99", "0.87", "0.72"];

#[tokio::test]
async fn the_best_recordings_card_gives_a_percentage() {
    let state = station();
    let (status, body) = get(&state, "/pages/best-detections").await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        body.contains("x-best"),
        "precondition: the card must have rendered a row, or this test asserts \
         nothing. Body: {}",
        body.chars().take(300).collect::<String>()
    );
    assert!(
        body.contains("99%"),
        "the highest-confidence card must give a percentage: {body}"
    );
    for bare in BARE {
        assert!(
            !body.contains(bare),
            "the card printed the raw model score {bare}: {body}"
        );
    }
}

#[tokio::test]
async fn the_clip_players_now_playing_strip_gives_a_percentage() {
    let state = station();
    let (status, body) = get(&state, "/recordings").await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        body.contains("data-clip-meta="),
        "precondition: a clip row must have rendered, or this test asserts \
         nothing"
    );
    assert!(
        body.contains("99%"),
        "the player strip must give a percentage: {}",
        body.chars().take(600).collect::<String>()
    );
    for bare in BARE {
        assert!(
            !body.contains(bare),
            "the clip player's meta carried the raw model score {bare}"
        );
    }
}

/// The counterpart. `conf_bar` — the atom both of the above deliberately do
/// not use — must still be giving percentages, so this file fails if the
/// percentage disappears from the surfaces that always had it, not only from
/// the two that did not.
#[tokio::test]
async fn the_surfaces_that_always_had_a_percentage_still_do() {
    let state = station();
    for uri in ["/pages/detections", "/pages/today-list"] {
        let (status, body) = get(&state, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri} did not answer");
        assert!(
            body.contains("bnb-conf"),
            "precondition: {uri} must render a confidence bar"
        );
        assert!(
            body.contains("99%"),
            "{uri} stopped giving a percentage: {}",
            body.chars().take(400).collect::<String>()
        );
        for bare in BARE {
            assert!(!body.contains(bare), "{uri} printed the raw score {bare}");
        }
    }
}
