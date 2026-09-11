//! Comments on a detection survive each other, and the page and API agree.
//!
//! The pieces are tested where they live — the table and its append-only
//! trigger in `birdnet-db`, the rendering in `birdnet-web`'s unit tests. What
//! none of those covers is that they are *connected*: a version where the
//! detail page never renders the thread, or where the API's POST writes to a
//! different key than its GET reads, passes every one of them.
//!
//! Observed failing against migration 48 and the pre-feature router: every test
//! here stops either at `no such table: detection_comments` or at a 404 for
//! `/api/v2/detections/comments`.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::api_token::ApiToken;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const DATE: &str = "2026-05-01";
const TIME: &str = "06:00:00";
const SCI: &str = "Dryobates villosus";
const TOKEN: &str = "test-token-for-detection-comments-0123456789";

fn state() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs)
             VALUES (?1, ?2, ?3, 'Hairy Woodpecker', 0.9, 0.7, 18, 1.25, 0.0, 'x.wav', 0)",
            rusqlite::params![DATE, TIME, SCI],
        )
        .expect("insert detection");
    });
    (dir, state)
}

async fn get_text(state: &AppState, uri: &str) -> String {
    let app = birdnet_web::server::build_router(state.clone());
    let res = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    assert_eq!(res.status(), StatusCode::OK, "GET {uri}");
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

async fn api(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let app = birdnet_web::server::build_router(state.clone());
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    let req = match body {
        Some(v) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = app.oneshot(req).await.expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// The defect this feature exists for, end to end: two people commenting on one
/// detection both keep their comment. `detection_reviews.notes` is UNIQUE on
/// this same key, so the equivalent there leaves only the second.
#[tokio::test]
async fn two_comments_on_one_detection_both_survive_and_the_page_shows_both() {
    let (_dir, state) = state();
    state.with_db(|conn| {
        for (author, body) in [
            ("ada", "Call is too short for a Hairy."),
            ("bob", "Spectrogram says otherwise."),
        ] {
            birdnet_db::detection_comments::insert(
                conn,
                &birdnet_db::detection_comments::NewComment {
                    date: DATE,
                    time: TIME,
                    sci_name: SCI,
                    user_id: None,
                    author,
                    body,
                },
            )
            .expect("insert");
        }
    });

    let thread = get_text(
        &state,
        &format!("/pages/detection-comments?date={DATE}&time={TIME}&sci_name=Dryobates%20villosus"),
    )
    .await;

    assert!(thread.contains("ada") && thread.contains("bob"), "{thread}");
    assert!(thread.contains("Call is too short"), "{thread}");
    assert!(thread.contains("Spectrogram says otherwise"), "{thread}");
    let ada = thread.find("Call is too short").expect("ada's comment");
    let bob = thread.find("Spectrogram says").expect("bob's comment");
    assert!(
        ada < bob,
        "oldest first, so a reply follows what it answers"
    );
}

/// The detail page asks for the thread. Without this, every gate above could
/// pass against a page that never renders the panel.
#[tokio::test]
async fn the_detail_page_loads_the_thread() {
    let (_dir, state) = state();
    let page = get_text(
        &state,
        &format!("/detections/detail?date={DATE}&time={TIME}&name=Hairy%20Woodpecker"),
    )
    .await;
    assert!(
        page.contains("/pages/detection-comments?date="),
        "the detail page must load the comment thread: {page}"
    );
    assert!(
        page.contains("Dryobates%20villosus"),
        "keyed on the scientific name: {page}"
    );
}

/// The API writes and reads the same thread the page does.
#[tokio::test]
async fn the_api_writes_what_the_page_reads() {
    let (_dir, state) = state();
    let state = state.with_api_token(ApiToken::new(TOKEN).expect("long enough"));

    let (status, created) = api(
        &state,
        "POST",
        "/api/v2/detections/comments",
        Some(serde_json::json!({
            "date": DATE, "time": TIME, "sci_name": SCI,
            "body": "Posted through the API."
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        created["author"], "api",
        "a token is not a person: {created}"
    );
    let id = created["id"].as_i64().expect("an id");

    let (status, listed) = api(
        &state,
        "GET",
        &format!(
            "/api/v2/detections/comments?date={DATE}&time={TIME}&sci_name=Dryobates%20villosus"
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        listed["comments"].as_array().expect("array").len(),
        1,
        "{listed}"
    );
    assert_eq!(listed["comments"][0]["body"], "Posted through the API.");

    let page = get_text(
        &state,
        &format!("/pages/detection-comments?date={DATE}&time={TIME}&sci_name=Dryobates%20villosus"),
    )
    .await;
    assert!(page.contains("Posted through the API."), "{page}");

    let (status, deleted) = api(
        &state,
        "POST",
        "/api/v2/detections/comments/delete",
        Some(serde_json::json!({ "id": id })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted["author"], "api");

    let (_, after) = api(
        &state,
        "GET",
        &format!(
            "/api/v2/detections/comments?date={DATE}&time={TIME}&sci_name=Dryobates%20villosus"
        ),
        None,
    )
    .await;
    assert!(
        after["comments"].as_array().expect("array").is_empty(),
        "{after}"
    );
}

/// A body the database refuses comes back as the caller's mistake with the
/// reason, not as a 500. The counterpart to the write gate above: without it, a
/// handler that accepted everything would pass.
#[tokio::test]
async fn an_empty_body_is_the_callers_mistake_and_says_why() {
    let (_dir, state) = state();
    let state = state.with_api_token(ApiToken::new(TOKEN).expect("long enough"));
    let (status, body) = api(
        &state,
        "POST",
        "/api/v2/detections/comments",
        Some(serde_json::json!({
            "date": DATE, "time": TIME, "sci_name": SCI, "body": "   "
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().is_some_and(|e| e.contains("empty")),
        "the refusal must name the problem: {body}"
    );
}

/// The page's write endpoints exist **and** are behind the sign-in.
///
/// Both halves matter and neither is enough alone: a router that never mounts
/// them is as broken as one that mounts them publicly, and the second is what
/// the read/write split in `pages::router` / `pages::mutating_router` exists to
/// prevent. The seed admin is given a real password so the middleware makes an
/// authorisation decision rather than taking the fresh-station bypass.
#[tokio::test]
async fn the_pages_write_endpoints_exist_and_need_a_sign_in() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let hash = birdnet_db::accounts::hash_password("a-real-password").expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&hash],
    )
    .expect("give the seed admin a password");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));

    for path in [
        "/pages/detection-comments/add",
        "/pages/detection-comments/delete",
    ] {
        let app = birdnet_web::server::build_router(state.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!(
                        "date={DATE}&time={TIME}&sci_name=Dryobates+villosus&body=x&id=1"
                    )))
                    .unwrap(),
            )
            .await
            .expect("router responds");
        assert_ne!(
            res.status(),
            StatusCode::NOT_FOUND,
            "{path} must be mounted — a write endpoint that does not exist is not \
             a secure one, it is a missing feature"
        );
        assert_ne!(
            res.status(),
            StatusCode::OK,
            "{path} answered a signed-out request; it belongs in mutating_router"
        );
    }

    assert!(
        state
            .with_db(|conn| birdnet_db::detection_comments::list(conn, DATE, TIME, SCI))
            .expect("list")
            .is_empty(),
        "and nothing was written"
    );
}

/// The API's write path is behind the bearer token.
#[tokio::test]
async fn the_api_write_path_is_not_open() {
    let (_dir, state) = state();

    let app = birdnet_web::server::build_router(state.clone());
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v2/detections/comments")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"date": DATE, "time": TIME, "sci_name": SCI, "body": "x"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("router responds");
    assert_ne!(
        res.status(),
        StatusCode::CREATED,
        "the API must not accept an unauthenticated comment"
    );

    assert!(
        state
            .with_db(|conn| birdnet_db::detection_comments::list(conn, DATE, TIME, SCI))
            .expect("list")
            .is_empty(),
        "and must not have written one"
    );
}
