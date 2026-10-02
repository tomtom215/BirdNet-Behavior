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

/// A station with one quarantined bird and one detection, whose `table`
/// refuses every write when `refuse` is set.
fn review_station(refuse: Option<&str>) -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-05-01', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'b.wav');
         INSERT INTO quarantine (date, time, sci_name, com_name, confidence, reason)
         VALUES ('2026-05-01', '04:10:00', 'Bubo scandiacus', 'Snowy Owl', 0.9, 'below_sf_thresh');
         -- An existing verdict, so a clear has a row to delete: a row trigger
         -- does not fire on a DELETE that matches nothing.
         INSERT INTO detection_reviews (date, time, sci_name, com_name, status)
         VALUES ('2026-05-01', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 'confirmed');",
    )
    .expect("seed");
    if let Some(table) = refuse {
        for event in ["INSERT", "UPDATE", "DELETE"] {
            conn.execute_batch(&format!(
                "CREATE TRIGGER refuse_{event} BEFORE {event} ON {table}
                 BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;"
            ))
            .expect("trigger");
        }
    }
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

async fn post_form(app: &axum::Router, uri: &str, form: &str) -> (Option<String>, String) {
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
                .body(Body::from(form.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{uri}");
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
    (reswap, body)
}

/// Rejecting or deleting a quarantined bird answered "Rejected." / "deleted"
/// whatever the database said. A rejection is what withdraws a shared link.
#[tokio::test]
async fn a_refused_quarantine_verdict_is_not_announced() {
    for (uri, done) in [
        ("/pages/quarantine-reject", "Rejected."),
        ("/pages/quarantine-delete", "Quarantine entry deleted."),
    ] {
        let (_, body) = post_form(&review_station(Some("quarantine")), uri, "id=1").await;
        assert!(
            !body.contains(done),
            "{uri} announced a write the store refused: {body}"
        );
        assert!(
            body.contains("could not save"),
            "{uri} said nothing: {body}"
        );
        // Counterpart: the same request on a working store says it happened.
        let (_, body) = post_form(&review_station(None), uri, "id=1").await;
        assert!(body.contains(done), "{uri}: {body}");
    }
}

/// The review queue and the detection page's inline widget discarded the
/// write's result, so a refused verdict showed as recorded.
#[tokio::test]
async fn a_refused_review_is_not_shown_as_recorded() {
    let row = "date=2026-05-01&time=06%3A10%3A00&sci_name=Turdus+merula\
               &com_name=Eurasian+Blackbird&status=rejected";
    for uri in [
        "/pages/detection-review",
        "/pages/detection-review-inline",
        "/pages/detection-review-clear",
    ] {
        let (reswap, body) = post_form(&review_station(Some("detection_reviews")), uri, row).await;
        assert_eq!(
            reswap.as_deref(),
            Some("none"),
            "{uri} changed the page: {body}"
        );
        assert!(body.contains("could not"), "{uri} said nothing: {body}");
        let (reswap, _) = post_form(&review_station(None), uri, row).await;
        assert_eq!(reswap, None, "{uri} refused on a working store");
    }
}

/// The quarantine list reload carries the filter back into an `hx-get`
/// attribute. It was the raw form field.
#[tokio::test]
async fn the_quarantine_filter_is_not_echoed_into_markup() {
    let (_, body) = post_form(
        &review_station(None),
        "/pages/quarantine-reject",
        "id=1&filter=%22%3E%3Cx+y%3D%22&offset=0",
    )
    .await;
    assert!(!body.contains("\"><x"), "{body}");
    assert!(body.contains("filter=pending"), "{body}");
}
