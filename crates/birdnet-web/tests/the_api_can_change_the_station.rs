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
use birdnet_web::routes::api_write::{BATCH_MAX, READ_ROUTES, WRITE_ROUTES};
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

/// One JSON `POST` against the real router, with the real middleware stack.
async fn call(
    state: &AppState,
    path: &str,
    bearer: Option<&str>,
    origin: Option<&str>,
    body: &str,
) -> (StatusCode, String) {
    call_as(state, path, bearer, origin, body, "application/json").await
}

/// One JSON request with the method spelled out.
///
/// `WRITE_ROUTES` is no longer all-`POST` — `PUT /api/v2/settings` is in it —
/// so a loop over the table that hard-coded `POST` would be asserting 405
/// handling rather than the authentication it means to assert.
async fn call_method(
    state: &AppState,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: &str,
) -> (StatusCode, String) {
    request(state, method, path, bearer, None, body, "application/json").await
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
    request(state, "POST", path, bearer, origin, body, content_type).await
}

#[allow(clippy::too_many_arguments)]
async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    origin: Option<&str>,
    body: &str,
    content_type: &str,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
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
    for (method, path) in WRITE_ROUTES.iter().chain(READ_ROUTES) {
        let (status, body) = call_method(&state, method, path, Some(TOKEN), KEY).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} answered {status} on a station with no BNB_API_TOKEN; the \
             mutating API must not exist until an operator enables it. Body: {body}"
        );
    }
    assert_eq!(row(&state).2, 1, "nothing was changed either");
}

/// A configured station still refuses a caller without the credential.
#[tokio::test]
async fn a_configured_station_refuses_a_missing_or_wrong_credential() {
    let (_dir, state) = station(true);
    for (method, path) in WRITE_ROUTES.iter().chain(READ_ROUTES) {
        let (status, _) = call_method(&state, method, path, None, KEY).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} with no token"
        );

        let (status, _) = call_method(&state, method, path, Some(WRONG), KEY).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} with the wrong token"
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
    for (method, path) in WRITE_ROUTES.iter().chain(READ_ROUTES) {
        let req = Request::builder()
            .method(*method)
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
            "{method} {path} is reachable in the *public* router (status {status}) — anyone \
             who can load the dashboard could call it"
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

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Seed the settings table with one secret, one secret-shaped URL, and one
/// ordinary value.
fn seed_settings(state: &AppState) {
    state.with_db(|conn| {
        birdnet_db::settings::ensure_settings_table(conn).expect("settings table");
        for (k, v, c) in [
            (
                "email_smtp_pass",
                "hunter2",
                birdnet_db::settings::SettingsCategory::Notifications,
            ),
            (
                "apprise_url",
                "ntfy://alice:hunter2@ntfy.example/topic",
                birdnet_db::settings::SettingsCategory::Notifications,
            ),
            (
                "latitude",
                "51.0",
                birdnet_db::settings::SettingsCategory::Location,
            ),
        ] {
            birdnet_db::settings::set(conn, k, v, c).expect("seed a setting");
        }
    });
}

/// Read one setting straight out of the database.
fn setting(state: &AppState, key: &str) -> Option<String> {
    state.with_db(|conn| birdnet_db::settings::get(conn, key).ok())
}

/// `GET /api/v2/settings` hands a client the station's configuration with the
/// credentials taken out.
///
/// Automation needs to *read* what it is about to change; handing it
/// `email_smtp_pass` in the clear over an endpoint that exists to be scripted
/// is not an acceptable price for that.
#[tokio::test]
async fn the_api_reads_settings_with_the_credentials_removed() {
    let (_dir, state) = station(true);
    seed_settings(&state);

    let (status, body) = call_method(&state, "GET", "/api/v2/settings", Some(TOKEN), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Redacted by key name.
    assert!(
        !body.contains("hunter2"),
        "a credential was served in the clear: {body}"
    );
    assert!(
        body.contains("email_smtp_pass"),
        "the key itself must survive — \"not set\" and \"not shown\" are different \
         answers, and a caller needs to tell them apart: {body}"
    );
    // And named, so a client knows which values it must not write back.
    assert!(
        body.contains("\"redacted\""),
        "the response does not say which keys it withheld: {body}"
    );

    // The counterpart: blanket redaction would satisfy every assertion above
    // and make the endpoint useless.
    assert!(
        body.contains("51.0"),
        "an ordinary value was redacted too, which leaves nothing to read: {body}"
    );
    // Redacted by value shape, not by key name: nothing about `apprise_url`
    // says "secret", and it routinely carries one. The host survives; the
    // scheme and user do not, because the two shape rules compose — see
    // `settings_are_redacted_by_key_and_by_shape` in `api_write.rs`, which
    // pins the exact output and says which rule produces it.
    assert!(
        body.contains("ntfy.example"),
        "the URL's host should survive so the value stays recognisable: {body}"
    );
}

/// `PUT /api/v2/settings` changes the station, through the settings page's own
/// normalisation.
#[tokio::test]
async fn the_api_can_change_a_setting() {
    let (_dir, state) = station(true);
    seed_settings(&state);

    // `51,5` is what a browser on a comma-decimal locale sends, and what the
    // settings page normalises. Asserting the *stored* form is `51.5` is how
    // this gate proves the API reuses `build_settings_items` rather than
    // having grown a second, subtly different writer: a handler that stored
    // the string it was given would answer 200 and store `51,5`.
    let (status, body) = call_method(
        &state,
        "PUT",
        "/api/v2/settings",
        Some(TOKEN),
        r#"{"latitude":"51,5","station_name":"Back Garden"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        setting(&state, "latitude").as_deref(),
        Some("51.5"),
        "{body}"
    );
    assert_eq!(
        setting(&state, "station_name").as_deref(),
        Some("Back Garden"),
        "{body}"
    );

    // A JSON client will send `{"latitude": 51.5}`, not `{"latitude": "51.5"}`.
    let (status, body) = call_method(
        &state,
        "PUT",
        "/api/v2/settings",
        Some(TOKEN),
        r#"{"latitude":52.25}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        setting(&state, "latitude").as_deref(),
        Some("52.25"),
        "{body}"
    );

    // Only what changed is written, and the response says which keys those
    // were — so a caller can tell a no-op from a write.
    let (status, body) = call_method(
        &state,
        "PUT",
        "/api/v2/settings",
        Some(TOKEN),
        r#"{"latitude":52.25}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"updated\":0"), "{body}");
}

/// The two ways a settings write is refused, and the reason each exists.
#[tokio::test]
async fn a_settings_write_refuses_unknown_keys_and_the_redaction_placeholder() {
    let (_dir, state) = station(true);
    seed_settings(&state);

    // A misspelled key that got a 200 would have told the caller their change
    // landed.
    let (status, body) = call_method(
        &state,
        "PUT",
        "/api/v2/settings",
        Some(TOKEN),
        r#"{"confidence_treshold":"0.8"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("confidence_treshold"), "{body}");
    assert!(
        body.contains("writable_keys"),
        "the refusal should say what *is* writable: {body}"
    );

    // The round-trip trap: read the whole object, change one field, write it
    // back — and every secret arrives as `***REDACTED***`. Storing that would
    // silently destroy the station's SMTP password.
    let (status, body) = call_method(
        &state,
        "PUT",
        "/api/v2/settings",
        Some(TOKEN),
        r#"{"email_smtp_pass":"***REDACTED***"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        setting(&state, "email_smtp_pass").as_deref(),
        Some("hunter2"),
        "the redaction placeholder overwrote a real credential"
    );

    // A value with no string form is refused rather than guessed at.
    let (status, body) = call_method(
        &state,
        "PUT",
        "/api/v2/settings",
        Some(TOKEN),
        r#"{"latitude":[51,5]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        setting(&state, "latitude").as_deref(),
        Some("51.0"),
        "the seeded value should be untouched"
    );

    // The counterpart: a handler that refused everything would pass all three
    // assertions above.
    let (status, body) = call_method(
        &state,
        "PUT",
        "/api/v2/settings",
        Some(TOKEN),
        r#"{"station_name":"Back Garden"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A settings write is recorded in the audit log, by key name and never by
/// value.
///
/// `/admin/audit` renders this table. An entry reading
/// `birdweather_token=abc123` would have put a credential on a page.
#[tokio::test]
async fn a_settings_write_is_audited_by_key_and_not_by_value() {
    let (_dir, state) = station(true);

    let (status, body) = call_method(
        &state,
        "PUT",
        "/api/v2/settings",
        Some(TOKEN),
        r#"{"birdweather_token":"bw-live-abcdef"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let entries: Vec<(String, String)> = state.with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT action, COALESCE(metadata, '') FROM audit_log")
            .expect("audit_log exists");
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .filter_map(Result::ok)
            .collect()
    });

    let update = entries
        .iter()
        .find(|(action, _)| action == "settings.update")
        .unwrap_or_else(|| panic!("no settings.update entry; the log holds {entries:?}"));
    assert!(
        update.1.contains("birdweather_token"),
        "the entry does not say which key changed: {update:?}"
    );
    assert!(
        !update.1.contains("bw-live-abcdef"),
        "the audit entry carries the credential itself: {update:?}"
    );
    assert!(
        update.1.contains("via=api"),
        "the entry does not distinguish an API write from an admin one: {update:?}"
    );
}

// ---------------------------------------------------------------------------
// Control
// ---------------------------------------------------------------------------

/// A restart request answers honestly when nothing would restart the process.
///
/// A station built without `with_supervised_by_systemd` is not supervised, so
/// this reaches the refusing branch deterministically. It did not always: the
/// handler used to read `INVOCATION_ID`/`JOURNAL_STREAM` per request, a GitHub
/// Actions runner sets `INVOCATION_ID`, and in CI this test therefore took the
/// *signalling* branch and had the test process `kill -TERM` itself 400 ms
/// later. The decision now arrives on the state.
///
/// What still cannot be exercised end to end is the signalling branch itself,
/// because reaching it through the router means a real `SIGTERM` to this
/// process. Its decision and its rendering are asserted in `service.rs`'s unit
/// tests, against the pure `restart_outcome` and `restart_fragment`.
///
/// The audit entry is asserted here because it is written before the decision,
/// so it must exist on the refusing branch too.
#[tokio::test]
async fn a_restart_says_so_when_nothing_would_bring_the_station_back() {
    let (_dir, state) = station(true);
    assert!(
        !state.supervised_by_systemd(),
        "a test station must never be marked supervised: the restart endpoint would \
         then SIGTERM this test process"
    );

    let (status, body) = call(&state, "/api/v2/control/restart", Some(TOKEN), None, "{}").await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a caller that got a 200 here would have been told the station was coming back \
         when nothing was going to bring it back. Body: {body}"
    );
    assert!(body.contains("\"restarting\":false"), "{body}");

    let actions: Vec<String> = state.with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT action FROM audit_log")
            .expect("audit_log exists");
        stmt.query_map([], |r| r.get(0))
            .expect("query")
            .filter_map(Result::ok)
            .collect()
    });
    assert!(
        actions.iter().any(|a| a == "system.restart"),
        "a restart request left no trace; the log holds {actions:?}"
    );
}

/// Every `(method, path)` in the two route tables is actually mounted.
///
/// The tables are read by the CSRF guard, by the OpenAPI gate and by every
/// loop in this file, so an entry the router does not serve would quietly
/// weaken all three. The unit test in `api_write.rs` cannot check this —
/// `axum::Router` exposes no route list — and an earlier version of it was
/// named as though it could while passing with `.put(write_settings)` deleted
/// from `router()`. This is the half that noticed.
#[tokio::test]
async fn every_documented_route_is_mounted() {
    let (_dir, state) = station(true);
    for (method, path) in WRITE_ROUTES.iter().chain(READ_ROUTES) {
        let (status, body) = call_method(&state, method, path, Some(TOKEN), KEY).await;
        assert!(
            status != StatusCode::NOT_FOUND && status != StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path} is in the route table but the router answers {status}; the \
             table is what the CSRF guard and the OpenAPI gate read. Body: {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// Batch
// ---------------------------------------------------------------------------

/// Seed `n` extra magpie detections at 10:00:00, 10:00:01, …
fn seed_many(state: &AppState, n: usize) -> Vec<String> {
    let mut times = Vec::with_capacity(n);
    state.with_db(|conn| {
        for i in 0..n {
            let t = format!("10:{:02}:{:02}", i / 60, i % 60);
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                      File_Name, chunk_offset_secs)
                 VALUES ('2026-09-03', ?1, 'Pica pica', 'Eurasian Magpie',
                         0.9, 0.7, 36, 1.25, 0.0, 'x.wav', 0)",
                rusqlite::params![t],
            )
            .expect("seed");
            times.push(t);
        }
    });
    times
}

/// A batch body naming `times`, all on the seeded date and species.
fn batch_body(op: &str, extra: &str, times: &[String]) -> String {
    let keys: Vec<String> = times
        .iter()
        .map(|t| format!(r#"{{"date":"2026-09-03","time":"{t}","sci_name":"Pica pica"}}"#))
        .collect();
    format!(
        r#"{{"op":"{op}"{extra},"detections":[{}]}}"#,
        keys.join(",")
    )
}

/// How many of the seeded detections are locked, and how many exist.
fn locked_and_total(state: &AppState) -> (i64, i64) {
    state.with_db(|conn| {
        let locked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE is_locked = 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap_or(-1);
        (locked, total)
    })
}

/// The finding: one request changes many detections.
///
/// Without it a triage client rejecting forty overnight false positives makes
/// forty authenticated round trips, which is the shape the audit's remedy
/// named as the eighth endpoint.
#[tokio::test]
async fn a_batch_applies_one_operation_to_many_detections() {
    let (_dir, state) = station(true);
    let times = seed_many(&state, 5);

    let (status, body) = call(
        &state,
        "/api/v2/detections/batch",
        Some(TOKEN),
        None,
        &batch_body("lock", "", &times),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"applied\":5"), "{body}");
    assert!(body.contains("\"failed\":0"), "{body}");
    assert_eq!(
        locked_and_total(&state),
        (5, 6),
        "five of the six detections should be locked and none deleted"
    );

    // And the reverse, so the gate is not satisfied by a handler that only ever
    // locks.
    let (status, body) = call(
        &state,
        "/api/v2/detections/batch",
        Some(TOKEN),
        None,
        &batch_body("unlock", "", &times[..3]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(locked_and_total(&state), (2, 6), "three were unlocked");

    let (status, body) = call(
        &state,
        "/api/v2/detections/batch",
        Some(TOKEN),
        None,
        &batch_body("delete", "", &times),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"applied\":5"), "{body}");
    assert_eq!(
        locked_and_total(&state).1,
        1,
        "only the original detection should remain"
    );
}

/// A batch does what it can and says what it could not.
///
/// The discrimination for the partial-results design. A handler that refused
/// the whole batch on the first bad key would leave the good ones untouched;
/// one that reported success for everything would leave `failed` at zero.
#[tokio::test]
async fn a_batch_does_the_rest_when_one_key_is_bad() {
    let (_dir, state) = station(true);
    let times = seed_many(&state, 3);

    let good: Vec<String> = times.clone();
    let keys: Vec<String> = good
        .iter()
        .map(|t| format!(r#"{{"date":"2026-09-03","time":"{t}","sci_name":"Pica pica"}}"#))
        .collect();
    let body_json = format!(
        r#"{{"op":"lock","detections":[{},{},{}]}}"#,
        keys.join(","),
        // Well-formed, matches nothing.
        r#"{"date":"2020-01-01","time":"00:00:00","sci_name":"Turdus merula"}"#,
        // Malformed: a date no query should ever see.
        r#"{"date":"yesterday","time":"00:00:00","sci_name":"Pica pica"}"#,
    );

    let (status, body) = call(
        &state,
        "/api/v2/detections/batch",
        Some(TOKEN),
        None,
        &body_json,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"requested\":5"), "{body}");
    assert!(
        body.contains("\"applied\":3"),
        "the three good keys should still have been applied: {body}"
    );
    assert!(body.contains("\"failed\":2"), "{body}");
    assert!(
        body.contains("no detection matches"),
        "the missing row should be named as such: {body}"
    );
    assert!(
        body.contains("date must be YYYY-MM-DD"),
        "the malformed key should say what is wrong with it: {body}"
    );
    assert_eq!(
        locked_and_total(&state),
        (3, 4),
        "exactly the three good keys were locked"
    );
}

/// The two whole-request refusals, and the cap.
#[tokio::test]
async fn a_batch_refuses_an_unknown_op_a_misplaced_status_and_an_oversized_list() {
    let (_dir, state) = station(true);
    let times = seed_many(&state, 2);

    // An op the endpoint does not implement. Answering 200 would report a
    // change that never happened.
    let (status, body) = call(
        &state,
        "/api/v2/detections/batch",
        Some(TOKEN),
        None,
        &batch_body("purge", "", &times),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("accepted"), "{body}");

    // `status` belongs to review. Silently ignoring it would tell a caller who
    // wrote `{"op":"delete","status":"confirmed"}` that both words did work.
    let (status, body) = call(
        &state,
        "/api/v2/detections/batch",
        Some(TOKEN),
        None,
        &batch_body("delete", r#","status":"confirmed""#, &times),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // Over the cap: refused before anything is written, not truncated.
    let over: Vec<String> = (0..=BATCH_MAX)
        .map(|i| format!("11:{:02}:{:02}", i / 60, i % 60))
        .collect();
    let (status, body) = call(
        &state,
        "/api/v2/detections/batch",
        Some(TOKEN),
        None,
        &batch_body("delete", "", &over),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("too many"), "{body}");

    assert_eq!(
        locked_and_total(&state),
        (0, 3),
        "a refused batch must not have changed anything"
    );

    // The counterpart: a handler that refused every batch would pass all three
    // assertions above.
    let (status, body) = call(
        &state,
        "/api/v2/detections/batch",
        Some(TOKEN),
        None,
        &batch_body("lock", "", &times),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        locked_and_total(&state),
        (2, 3),
        "a well-formed batch must still work after the refusals"
    );
}

/// A batch writes one audit row per detection it changed, and none for the
/// ones it did not.
///
/// The alternative — one row per batch — was rejected: the audit view is where
/// an operator asks "what happened to that recording?", and a single row
/// reading "deleted 40 detections" cannot answer it. The cost is that one call
/// can write a full page of history into a view capped at 500 rows, which is
/// what [`BATCH_MAX`] bounds.
#[tokio::test]
async fn a_batch_audits_every_detection_it_changed_and_no_others() {
    let (_dir, state) = station(true);
    let times = seed_many(&state, 3);

    let keys: Vec<String> = times
        .iter()
        .map(|t| format!(r#"{{"date":"2026-09-03","time":"{t}","sci_name":"Pica pica"}}"#))
        .collect();
    let body_json = format!(
        r#"{{"op":"delete","detections":[{},{}]}}"#,
        keys.join(","),
        r#"{"date":"2020-01-01","time":"00:00:00","sci_name":"Turdus merula"}"#,
    );

    let (status, body) = call(
        &state,
        "/api/v2/detections/batch",
        Some(TOKEN),
        None,
        &body_json,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let entries: Vec<(String, String, String)> = state.with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT action, COALESCE(target, ''), COALESCE(metadata, '') FROM audit_log")
            .expect("audit_log exists");
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query")
            .filter_map(Result::ok)
            .collect()
    });

    let deletes: Vec<&(String, String, String)> = entries
        .iter()
        .filter(|(action, _, _)| action == "detection.delete")
        .collect();
    assert_eq!(
        deletes.len(),
        3,
        "one row per detection actually deleted, and none for the key that \
         matched nothing; the log holds {entries:?}"
    );
    for (_, target, metadata) in &deletes {
        assert!(
            target.contains("Pica pica"),
            "the row must name which detection it was: {target}"
        );
        assert!(
            metadata.contains("via=api"),
            "a batch is still an API change, not a human one: {metadata}"
        );
    }
    assert!(
        !entries.iter().any(|(_, t, _)| t.contains("Turdus merula")),
        "the key that matched nothing must not be recorded as a deletion: {entries:?}"
    );
}
