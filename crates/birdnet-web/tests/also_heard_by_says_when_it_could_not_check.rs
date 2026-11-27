//! MSC-20. The detection page's "Also heard by" card is a claim about the
//! other microphones: listed sources heard this bird, and no card means none
//! did. A failed read of the other sources' rows must therefore say so, rather
//! than render as "only this microphone heard it", which a reviewer reads as
//! evidence.
//!
//! Which rows count as "the same time" is the event rule's: the instant
//! (`detected_at_utc`), within the event tolerance (`events.rs`), not a fixed
//! 30 s on the local wall clock.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const DATE: &str = "2026-05-01";
const TIME: &str = "06:00:00";
const SCI: &str = "Strix aluco";
const FAILED: &str = "We couldn't load the other sources’ detections of this bird";

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

/// A detection of the test species from `source`, `time`, stored with
/// `confidence` exactly as given (SQLite keeps a text value in a REAL column).
fn add(state: &AppState, source: &str, time: &str, confidence: &rusqlite::types::Value) {
    let clip = format!("{DATE}-birdnet-{source}-{time}.wav");
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs, Source)
             VALUES (?1, ?2, ?3, 'Tawny Owl', ?4, 0.7, 18, 1.25, 0.0, ?5, 0, ?6)",
            rusqlite::params![DATE, time, SCI, confidence, clip, source],
        )
        .expect("insert detection");
    });
}

async fn detail(state: &AppState) -> String {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/detections/detail?date={DATE}&time={TIME}&name=Tawny%20Owl"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router responds");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

#[tokio::test]
async fn a_failed_read_of_the_other_sources_is_reported_as_a_failure() {
    let (_dir, state) = station();
    add(&state, "src_a", TIME, &0.9.into());
    // The other source's row cannot be read as a detection: its confidence is
    // not a number, so the corroboration query fails while this page's own
    // detection reads cleanly.
    add(
        &state,
        "src_b",
        "06:00:03",
        &"unreadable".to_string().into(),
    );
    let page = detail(&state).await;
    assert!(
        page.contains("Tawny Owl"),
        "the detection itself must still render"
    );
    assert!(
        page.contains(FAILED),
        "a failed corroboration read rendered as if no other source heard the bird"
    );
}

/// Counterpart: a readable second source is listed, and the failure note is
/// absent.
#[tokio::test]
async fn a_readable_other_source_is_listed_without_a_failure_note() {
    let (_dir, state) = station();
    add(&state, "src_a", TIME, &0.9.into());
    add(&state, "src_b", "06:00:03", &0.8.into());
    let page = detail(&state).await;
    assert!(page.contains("Also heard by"), "card missing");
    assert!(page.contains("src_b"), "the other source is not listed");
    assert!(
        !page.contains(FAILED),
        "a successful read reported as failed"
    );
}

/// Counterpart: no other source heard it, so neither a card nor a failure.
#[tokio::test]
async fn a_lone_source_shows_neither_card_nor_failure() {
    let (_dir, state) = station();
    add(&state, "src_a", TIME, &0.9.into());
    let page = detail(&state).await;
    assert!(!page.contains("Also heard by"));
    assert!(!page.contains(FAILED));
}

/// Move `source`'s row to `instant` without touching its local `Date`/`Time`:
/// what the second pass of a repeated autumn hour looks like.
fn set_instant(state: &AppState, source: &str, instant: i64) {
    state.with_db(|conn| {
        conn.execute(
            "UPDATE detections SET detected_at_utc = ?2 WHERE Source = ?1",
            rusqlite::params![source, instant],
        )
        .expect("set instant");
    });
}

fn instant_of(state: &AppState, source: &str) -> i64 {
    state.with_db(|conn| {
        conn.query_row(
            "SELECT detected_at_utc FROM detections WHERE Source = ?1",
            [source],
            |r| r.get(0),
        )
        .expect("instant")
    })
}

/// MSC-20: the same local time an hour apart (the repeated autumn hour) is
/// not the same call, and the card must not say another source heard it.
#[tokio::test]
async fn the_same_wall_clock_an_hour_apart_is_not_also_heard() {
    let (_dir, state) = station();
    add(&state, "src_a", TIME, &0.9.into());
    add(&state, "src_b", TIME, &0.8.into());
    let a = instant_of(&state, "src_a");
    set_instant(&state, "src_b", a + 3600);
    let page = detail(&state).await;
    assert!(
        !page.contains("Also heard by"),
        "a row an hour later by the instant was listed because its local time matched"
    );
}

/// MSC-20: twenty seconds apart is outside the event tolerance (4 s for
/// V2.4), so it is not listed; the fixed 30 s window listed it.
#[tokio::test]
async fn twenty_seconds_apart_is_outside_the_event_tolerance() {
    let (_dir, state) = station();
    add(&state, "src_a", TIME, &0.9.into());
    add(&state, "src_b", "06:00:20", &0.8.into());
    let page = detail(&state).await;
    assert!(
        !page.contains("Also heard by"),
        "a row 20 s away was listed: the window is not the event tolerance"
    );
}
