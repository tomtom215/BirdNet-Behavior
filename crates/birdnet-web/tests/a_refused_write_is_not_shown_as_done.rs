//! A lock, unlock or delete the database refused is not shown as done.
//!
//! The handlers discarded the write's result (`let _ =`) and answered as if it
//! had worked: Recordings' 🔒 flipped to "locked" and a deleted row vanished
//! even when the store had refused. A lock is what keeps a clip from the
//! disk-full purge, so "locked" on screen and unlocked on disk is the worst
//! way for this to go wrong. A refused write now leaves the page as it was
//! (`HX-Reswap: none`) and says why in a toast — a `200`, because htmx drops
//! the body of an error status and would show nothing at all.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const ROW: &str = "date=2026-05-01&time=06%3A10%3A00&sci_name=Turdus+merula";

fn station(refuse: Option<&str>) -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-05-01', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'b.wav')",
        [],
    )
    .expect("seed");
    if let Some(event) = refuse {
        conn.execute_batch(&format!(
            "CREATE TRIGGER refuse BEFORE {event} ON detections
             BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;"
        ))
        .expect("trigger");
    }
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

async fn post(app: &axum::Router, uri: &str) -> (StatusCode, Option<String>, String) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .body(Body::from(ROW))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let reswap = res
        .headers()
        .get("hx-reswap")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = String::from_utf8_lossy(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned();
    (status, reswap, body)
}

#[tokio::test]
async fn a_refused_lock_leaves_the_button_unlocked_and_says_so() {
    for uri in ["/pages/recordings-lock", "/pages/today-lock"] {
        let (status, reswap, body) = post(&station(Some("UPDATE")), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(
            reswap.as_deref(),
            Some("none"),
            "{uri} changed the page after a refused lock"
        );
        assert!(
            body.contains("could not lock"),
            "{uri} said nothing: {body}"
        );
    }
    // Counterpart: a lock that happens flips the button, with no reswap.
    let (_, reswap, body) = post(&station(None), "/pages/recordings-lock").await;
    assert_eq!(reswap, None);
    assert!(
        body.contains("recordings-unlock"),
        "a successful lock must offer unlock: {body}"
    );
}

#[tokio::test]
async fn a_refused_delete_leaves_the_row_and_says_so() {
    for uri in ["/pages/recordings-delete", "/pages/today-delete"] {
        let (status, reswap, body) = post(&station(Some("DELETE")), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(
            reswap.as_deref(),
            Some("none"),
            "{uri} removed a row the store kept"
        );
        assert!(
            body.contains("could not delete"),
            "{uri} said nothing: {body}"
        );
    }
    // Counterpart: a delete that happens removes the row (an empty body swaps
    // the row out).
    let (_, reswap, body) = post(&station(None), "/pages/recordings-delete").await;
    assert_eq!(reswap, None);
    assert!(body.trim().is_empty());
}
