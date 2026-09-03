//! A station can be *changed* over its API, by something that is not a browser.
//!
//! # What was wrong (`O-1`)
//!
//! The `/api/v2` surface was 100 % read-only: a grep for `post(`, `put(`,
//! `delete(` and `patch(` across the fourteen routers nested under it returned
//! nothing, against upstream `birdnet-go`'s fifty-four mutating routes. Every
//! state change in the product was an HTMX form post returning an HTML
//! fragment, behind a same-origin check that any script satisfies by setting a
//! matching `Origin` header — so it was neither a security boundary nor a
//! contract anyone could build on.
//!
//! The consequences were concrete: Home Assistant and Node-RED could read a
//! station and never act on one; there was no supported automation of any kind;
//! and because our own front end was the only client, a change to fragment
//! markup would silently break whatever automation existed in the wild.
//!
//! # What this gate holds
//!
//! 1. A station with no `BNB_API_TOKEN` has **no** write API — every endpoint
//!    is 404. This is the default, and it must stay true.
//! 2. A configured station refuses a request with no credential, and one with
//!    the wrong credential, with 401.
//! 3. With the right credential, each endpoint actually changes the database —
//!    which is the half that fails against the shipped code, because the
//!    routes do not exist.
//! 4. `public_routes()` still exposes no way to change anything. The mutating
//!    endpoints are mounted in their own bearer-gated router precisely so this
//!    stays true; `public_router_is_read_only.rs` is the finding that made it
//!    a rule.
//! 5. The CSRF discrimination: a bearer call is *not* blocked by the
//!    same-origin rule (it could not be a cross-site form submission), while a
//!    cookie-shaped write from a foreign origin still is. Either half alone
//!    would be satisfied by removing the guard.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::api_token::ApiToken;
use birdnet_web::routes::api_write::WRITE_ROUTES;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A token long enough to be accepted, and a wrong one of the same length.
const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const WRONG: &str = "fedcba9876543210fedcba9876543210";

/// A station with one detection in it, optionally with the write API enabled.
fn station(with_token: bool) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs)
             VALUES ('2026-09-03', '09:00:00', 'Pica pica', 'Eurasian Magpie',
                     0.9, 0.7, 36, 1.25, 0.0, 'x.wav', 0)",
            [],
        )
        .expect("seed a detection");
    });
    let state = if with_token {
        state.with_api_token(ApiToken::new(TOKEN).expect("long enough"))
    } else {
        state
    };
    (dir, state)
}

/// One JSON request against the real router, with the real middleware stack.
async fn call(
    state: &AppState,
    path: &str,
    bearer: Option<&str>,
    origin: Option<&str>,
    body: &str,
) -> (StatusCode, String) {
    call_as(state, path, bearer, origin, body, "application/json").await
}

/// The same, with the content type spelled out.
///
/// The page handlers take `Form`, so a counterpart that exercises one has to
/// send `application/x-www-form-urlencoded` — otherwise the extractor refuses
/// it with 415 before the handler runs, and a test meant to prove the write
/// *did not happen* would be passing because the body was the wrong shape.
async fn call_as(
    state: &AppState,
    path: &str,
    bearer: Option<&str>,
    origin: Option<&str>,
    body: &str,
    content_type: &str,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, content_type);
    if let Some(t) = bearer {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    if let Some(o) = origin {
        req = req
            .header(header::ORIGIN, o)
            .header(header::HOST, "birdnet.local");
    }
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req.body(Body::from(body.to_owned())).expect("request"))
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The one detection's key, as a JSON body.
const KEY: &str = r#"{"date":"2026-09-03","time":"09:00:00","sci_name":"Pica pica"}"#;

/// Read back the row's review verdict and lock flag.
fn row(state: &AppState) -> (Option<String>, i64, i64) {
    state.with_db(|conn| {
        let verdict: Option<String> = conn
            .query_row("SELECT status FROM detection_reviews LIMIT 1", [], |r| {
                r.get(0)
            })
            .ok();
        let locked: i64 = conn
            .query_row("SELECT is_locked FROM detections LIMIT 1", [], |r| r.get(0))
            .unwrap_or(-1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap_or(-1);
        (verdict, locked, count)
    })
}

/// The default: no token, no write API.
#[tokio::test]
async fn a_station_with_no_token_has_no_write_api() {
    let (_dir, state) = station(false);
    for (_, path) in WRITE_ROUTES {
        let (status, body) = call(&state, path, Some(TOKEN), None, KEY).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "POST {path} answered {status} on a station with no BNB_API_TOKEN; the \
             mutating API must not exist until an operator enables it. Body: {body}"
        );
    }
    assert_eq!(row(&state).2, 1, "nothing was changed either");
}

/// A configured station still refuses a caller without the credential.
#[tokio::test]
async fn a_configured_station_refuses_a_missing_or_wrong_credential() {
    let (_dir, state) = station(true);
    for (_, path) in WRITE_ROUTES {
        let (status, _) = call(&state, path, None, None, KEY).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "POST {path} with no token"
        );

        let (status, _) = call(&state, path, Some(WRONG), None, KEY).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "POST {path} with the wrong token"
        );
    }
    let (verdict, locked, count) = row(&state);
    assert_eq!(
        (verdict, locked, count),
        (None, 0, 1),
        "a refused request must not have changed anything"
    );
}

/// The finding: with the right credential, the station can be changed.
#[tokio::test]
async fn the_api_can_review_lock_and_delete_a_detection() {
    let (_dir, state) = station(true);

    let (status, body) = call(
        &state,
        "/api/v2/detections/review",
        Some(TOKEN),
        None,
        r#"{"date":"2026-09-03","time":"09:00:00","sci_name":"Pica pica","status":"confirmed"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        row(&state).0.as_deref(),
        Some("confirmed"),
        "the verdict did not reach the database"
    );

    let (status, body) = call(&state, "/api/v2/detections/lock", Some(TOKEN), None, KEY).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(row(&state).1, 1, "the detection was not locked");

    let (status, body) = call(&state, "/api/v2/detections/unlock", Some(TOKEN), None, KEY).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(row(&state).1, 0, "the detection was not unlocked");

    let (status, body) = call(&state, "/api/v2/detections/delete", Some(TOKEN), None, KEY).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(row(&state).2, 0, "the detection was not deleted");
}

/// A malformed key is a 400; a well-formed key that matches nothing is a 404.
///
/// The counterpart to the gate above: without it, a handler that answered
/// `200 {"deleted": true}` to everything would pass.
#[tokio::test]
async fn a_bad_key_is_refused_and_a_missing_row_is_not_found() {
    let (_dir, state) = station(true);

    let (status, body) = call(
        &state,
        "/api/v2/detections/lock",
        Some(TOKEN),
        None,
        r#"{"date":"yesterday","time":"09:00:00","sci_name":"Pica pica"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("YYYY-MM-DD"), "{body}");

    let (status, body) = call(
        &state,
        "/api/v2/detections/lock",
        Some(TOKEN),
        None,
        r#"{"date":"2020-01-01","time":"09:00:00","sci_name":"Turdus merula"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = call(
        &state,
        "/api/v2/detections/review",
        Some(TOKEN),
        None,
        r#"{"date":"2026-09-03","time":"09:00:00","sci_name":"Pica pica","status":"maybe"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    assert_eq!(row(&state), (None, 0, 1), "nothing was changed");
}

/// `public_routes()` must still expose no way to change anything.
///
/// The mutating endpoints live in their own router for this reason. Merging
/// them into the public one would work, would pass every gate above, and would
/// hand an unauthenticated visitor the write API.
#[tokio::test]
async fn the_public_router_still_exposes_no_write_api() {
    let (_dir, state) = station(true);
    let public = birdnet_web::routes::public_routes().with_state(state);
    for (_, path) in WRITE_ROUTES {
        let req = Request::builder()
            .method("POST")
            .uri(*path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(KEY))
            .expect("request");
        let status = public
            .clone()
            .oneshot(req)
            .await
            .expect("router responds")
            .status();
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
            "POST {path} is reachable in the *public* router (status {status}) — anyone who \
             can load the dashboard could call it"
        );
    }
}

/// A bearer call from a foreign origin is allowed; a cookie-shaped write from
/// one is still blocked.
///
/// The discrimination for the CSRF change. A guard that simply stopped running
/// would satisfy the first half; one that kept running for everything would
/// make the API unusable from anything but a browser on the station's own
/// hostname, which is every automation it exists for.
#[tokio::test]
async fn the_csrf_skip_covers_bearer_calls_and_nothing_else() {
    let (_dir, state) = station(true);

    let (status, body) = call(
        &state,
        "/api/v2/detections/lock",
        Some(TOKEN),
        Some("https://homeassistant.example"),
        KEY,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a bearer call carrying a foreign Origin was refused; a cross-site *form* cannot \
         set an Authorization header, so the same-origin rule has nothing to protect \
         here. Body: {body}"
    );

    // The counterpart: the cookie-authenticated page write from the same
    // foreign origin is still refused by the guard.
    let (status, body) = call_as(
        &state,
        "/pages/today-delete",
        None,
        Some("https://homeassistant.example"),
        "date=2026-09-03&time=09:00:00&sci_name=Pica+pica",
        "application/x-www-form-urlencoded",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the CSRF skip widened past the mutating API and gave the cookie-authenticated \
         admin surface away. Body: {body}"
    );

    // And the same write *carrying a bearer header*. This is the assertion
    // that pins the skip to the path rather than to the header: a rule of
    // "any request with an Authorization header is exempt" satisfies both
    // assertions above and hands `/admin` and `/pages` their CSRF protection
    // away to anyone who can make a browser send one header.
    let (status, body) = call_as(
        &state,
        "/pages/today-delete",
        Some(TOKEN),
        Some("https://homeassistant.example"),
        "date=2026-09-03&time=09:00:00&sci_name=Pica+pica",
        "application/x-www-form-urlencoded",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a bearer header exempted a *page* write from the CSRF guard; the skip must be \
         scoped to the mutating API's own paths. Body: {body}"
    );
    assert_eq!(
        row(&state).2,
        1,
        "and the detection must still be there — the write must not have run"
    );
}
