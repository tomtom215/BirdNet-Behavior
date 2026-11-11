//! A review on the web is a verdict on one detection, named by its clip.
//!
//! Two microphones that hear one bird in the same second have two detections
//! that share date, time and species and differ only in their clip. The review
//! forms posted the triple, so a verdict on the one the reviewer listened to was
//! written to both, and a rejection withdrew from every count a detection nobody
//! had judged. These go through the real router, as the browser does.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const DATE: &str = "2026-09-08";
const TIME: &str = "06:00:00";

/// A station whose two microphones heard one Blackbird in one second.
fn two_sources() -> (axum::Router, AppState) {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for clip in ["a.wav", "b.wav"] {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9, ?3)",
            rusqlite::params![DATE, TIME, clip],
        )
        .expect("seed");
    }
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    (build_router(state.clone()), state)
}

/// The same station with only one of them: a triple that names one detection.
fn one_source() -> (axum::Router, AppState) {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9, 'a.wav')",
        rusqlite::params![DATE, TIME],
    )
    .expect("seed");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    (build_router(state.clone()), state)
}

/// `(clip, verdict)` of every detection, by clip.
fn verdicts(state: &AppState) -> Vec<(String, Option<String>)> {
    state.with_db(|conn| {
        conn.prepare("SELECT File_Name, review_verdict FROM detections ORDER BY File_Name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    })
}

fn review_rows(state: &AppState) -> i64 {
    state.with_db(|conn| {
        conn.query_row("SELECT COUNT(*) FROM detection_reviews", [], |r| r.get(0))
            .unwrap()
    })
}

async fn post(app: &axum::Router, uri: &str, form: &str) -> (Option<String>, String) {
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

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = String::from_utf8_lossy(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned();
    (status, body)
}

fn form(clip: Option<&str>, status: &str) -> String {
    let mut f = format!(
        "date={DATE}&time=06%3A00%3A00&sci_name=Turdus+merula&com_name=Eurasian+Blackbird&status={status}"
    );
    if let Some(clip) = clip {
        f.push_str("&file_name=");
        f.push_str(clip);
    }
    f
}

/// The inline widget on a detection's page and the queue's own form both post
/// the clip. A verdict on microphone A's detection does not reach B's.
#[tokio::test]
async fn a_verdict_posted_with_a_clip_reaches_only_that_detection() {
    for uri in ["/pages/detection-review-inline", "/pages/detection-review"] {
        let (app, state) = two_sources();

        let (reswap, body) = post(&app, uri, &form(Some("a.wav"), "rejected")).await;

        assert_eq!(reswap, None, "{uri}: the write happened: {body}");
        assert_eq!(
            verdicts(&state),
            [
                ("a.wav".to_owned(), Some("rejected".to_owned())),
                ("b.wav".to_owned(), None)
            ],
            "{uri} reached the other microphone's detection"
        );
        assert_eq!(review_rows(&state), 1, "{uri}");
    }
}

/// An older page left open posts no clip. When the date, time and species name
/// two detections the station does not guess: nothing is written, and the page
/// says why in a toast, a `200` because htmx drops the body of an error status.
/// The counterpart: when they name one, the old form still works.
#[tokio::test]
async fn a_verdict_posted_without_a_clip_is_refused_rather_than_guessed() {
    let (app, state) = two_sources();
    for uri in [
        "/pages/detection-review",
        "/pages/detection-review-inline",
        "/pages/detection-review-clear",
    ] {
        let (reswap, body) = post(&app, uri, &form(None, "rejected")).await;
        assert_eq!(
            reswap.as_deref(),
            Some("none"),
            "{uri} changed the page: {body}"
        );
        assert!(
            body.contains("does not say which"),
            "{uri} said nothing useful: {body}"
        );
    }
    assert_eq!(
        verdicts(&state),
        [("a.wav".to_owned(), None), ("b.wav".to_owned(), None)],
        "nothing was written"
    );
    assert_eq!(review_rows(&state), 0);

    let (app, state) = one_source();
    let (reswap, body) = post(
        &app,
        "/pages/detection-review-inline",
        &form(None, "rejected"),
    )
    .await;
    assert_eq!(reswap, None, "{body}");
    assert_eq!(
        verdicts(&state),
        [("a.wav".to_owned(), Some("rejected".to_owned()))]
    );
}

/// A stale link: the detection was deleted since the page was drawn. A verdict
/// about nothing is not recorded, and the person is told.
#[tokio::test]
async fn a_verdict_on_a_detection_that_is_gone_is_refused() {
    let (app, state) = two_sources();

    let (reswap, body) = post(
        &app,
        "/pages/detection-review-inline",
        &form(Some("gone.wav"), "rejected"),
    )
    .await;

    assert_eq!(reswap.as_deref(), Some("none"), "{body}");
    assert!(body.contains("no longer in the log"), "{body}");
    assert_eq!(review_rows(&state), 0, "no verdict about nothing");
    assert_eq!(
        verdicts(&state),
        [("a.wav".to_owned(), None), ("b.wav".to_owned(), None)]
    );
}

/// A page opened from a list row names its clip, so it opens that detection and
/// its widget posts that clip. Without one it opens the first, as it always did.
#[tokio::test]
async fn the_detail_page_opens_the_detection_its_clip_names() {
    let (app, _state) = two_sources();
    let base = format!("/detections/detail?date={DATE}&time=06%3A00%3A00&name=Eurasian+Blackbird");

    let (status, page) = get(&app, &format!("{base}&file=b.wav")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains(r#"name="file_name" value="b.wav""#),
        "the review widget on b's page must post b's clip"
    );
    assert!(!page.contains(r#"name="file_name" value="a.wav""#));

    let (_, first) = get(&app, &base).await;
    assert!(
        first.contains(r#"name="file_name" value="a.wav""#),
        "a link that does not say which still opens the first, as before"
    );
}

/// The queue lists each unreviewed detection with its clip, and a verdict on
/// one takes only that one out of it.
#[tokio::test]
async fn the_queue_takes_only_the_reviewed_detection_out() {
    let (app, _state) = two_sources();
    let (_, before) = get(&app, "/pages/detection-reviews-queue").await;
    assert!(before.contains(r#"name="file_name" value="a.wav""#));
    assert!(before.contains(r#"name="file_name" value="b.wav""#));
    assert!(
        before.contains("file=a.wav") && before.contains("file=b.wav"),
        "each opens its own page"
    );

    post(
        &app,
        "/pages/detection-review",
        &form(Some("a.wav"), "confirmed"),
    )
    .await;

    let (_, after) = get(&app, "/pages/detection-reviews-queue").await;
    let pending = after
        .split("Awaiting review")
        .nth(1)
        .unwrap()
        .split("All verdicts")
        .next()
        .unwrap();
    assert!(
        pending.contains(r#"value="b.wav""#),
        "the one nobody judged is still waiting"
    );
    assert!(
        !pending.contains(r#"name="file_name" value="a.wav""#),
        "the one that was judged is not"
    );
}

/// A verdict attached to no detection is listed with what it is, counted apart,
/// and can be dismissed. Dismissing never removes an attached one.
#[tokio::test]
async fn a_legacy_verdict_is_listed_marked_and_dismissable() {
    let (app, state) = two_sources();
    post(
        &app,
        "/pages/detection-review",
        &form(Some("a.wav"), "confirmed"),
    )
    .await;
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detection_reviews (date, time, sci_name, com_name, status, detached)
             VALUES ('2026-09-01', '05:00:00', 'Turdus merula', 'Eurasian Blackbird', 'rejected', 'ambiguous')",
            [],
        )
        .unwrap();
    });

    let (_, queue) = get(&app, "/pages/detection-reviews-queue").await;
    assert!(queue.contains("Not attached"), "{queue}");
    assert!(
        queue.contains("1 not attached to a detection"),
        "counted apart"
    );
    assert!(
        queue.contains("1 confirmed") && queue.contains("0 rejected"),
        "attached verdicts only"
    );
    assert!(queue.contains("/pages/detection-review-dismiss"));

    let (legacy_id, attached_id): (i64, i64) = state.with_db(|conn| {
        (
            conn.query_row(
                "SELECT id FROM detection_reviews WHERE detached IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap(),
            conn.query_row(
                "SELECT id FROM detection_reviews WHERE detached IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap(),
        )
    });
    post(
        &app,
        "/pages/detection-review-dismiss",
        &format!("id={attached_id}"),
    )
    .await;
    assert_eq!(
        review_rows(&state),
        2,
        "an attached verdict is not dismissable"
    );
    post(
        &app,
        "/pages/detection-review-dismiss",
        &format!("id={legacy_id}"),
    )
    .await;
    assert_eq!(review_rows(&state), 1);
    let (_, queue) = get(&app, "/pages/detection-reviews-queue").await;
    assert!(!queue.contains("Not attached"));
}

/// Undo names the detection too: lifting the verdict on microphone A's
/// detection leaves B's in force. (And the verdict row it was undone from
/// carries the clip, so the Undo button can.)
#[tokio::test]
async fn undoing_a_verdict_clears_only_the_detection_it_names() {
    let (app, state) = two_sources();
    post(
        &app,
        "/pages/detection-review",
        &form(Some("a.wav"), "rejected"),
    )
    .await;
    post(
        &app,
        "/pages/detection-review",
        &form(Some("b.wav"), "confirmed"),
    )
    .await;
    assert_eq!(
        review_rows(&state),
        2,
        "precondition: each has its own verdict"
    );

    let (_, queue) = get(&app, "/pages/detection-reviews-queue").await;
    assert!(
        queue.contains(r#"name="file_name" value="a.wav""#),
        "the Undo form for a verdict carries its detection's clip"
    );
    let (reswap, body) = post(
        &app,
        "/pages/detection-review-clear",
        &format!("date={DATE}&time=06%3A00%3A00&sci_name=Turdus+merula&file_name=a.wav"),
    )
    .await;

    assert_eq!(reswap, None, "{body}");
    assert_eq!(
        verdicts(&state),
        [
            ("a.wav".to_owned(), None),
            ("b.wav".to_owned(), Some("confirmed".to_owned()))
        ],
        "B's verdict survived A's undo"
    );
    assert_eq!(review_rows(&state), 1);
}
