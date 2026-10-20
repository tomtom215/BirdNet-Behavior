//! The settings page and `PUT /api/v2/settings` hold a value to the same list of
//! checks, and that list is `birdnet_core::config::validate`'s for the keys it
//! knows.
//!
//! # What was missing
//!
//! `validate()` checks the recording window, the clip format and the
//! species-info site in the config **file**; a value typed on the page goes to
//! the settings table, which is laid over the file *after* that check has run, so
//! nothing looked at it again. The window is a free-text box, and an unknown one
//! is not rejected downstream but reinterpreted: `fixed: 06:00-20:00` (a space),
//! `sunrise-sunset`, or `fixed:20:00-06:00` (a night recorder's window, which the
//! scheduler cannot express) becomes "record all day" with one line in the log,
//! while the page goes on showing what was typed.
//!
//! The API had a second gap. The page ran the range, BirdWeather-token and
//! language checks; `PUT /api/v2/settings` ran only the ranges, so a script could
//! store a database language no species-name file exists for, or a token with a
//! `/` in it that makes every upload fail, which the page refuses.
//!
//! # What this gate holds
//!
//! 1. Each bad value is refused by the page and by the API, with the whole
//!    submission dropped (a good change riding along is not written).
//! 2. Each good value, including the ends of the grammar and the unset blank, is
//!    saved by both. A check that refused everything would pass (1).
//! 3. A bad window already stored is caught on the next save, because the page
//!    re-submits what it rendered and the diff drops unchanged fields.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::api_token::ApiToken;
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// `(field, value)` pairs no part of the station can run on as written.
const BAD: &[(&str, &str)] = &[
    ("recording_schedule", "sunrise-sunset"),
    ("recording_schedule", "allday"),
    ("recording_schedule", "fixed:25:99-20:00"),
    ("recording_schedule", "fixed: 06:00-20:00"),
    ("recording_schedule", "fixed:06:00-20:00 UTC"),
    // The grammar is fine; the scheduler refuses a window that does not end
    // after it starts, and the station would record all day.
    ("recording_schedule", "fixed:20:00-06:00"),
    ("recording_schedule", "fixed:06:00-06:00"),
    ("audio_format", "aiff"),
    ("audio_format", "S16_LE"),
    ("info_site", "wikipedia"),
    // The two the API did not check at all.
    ("database_lang", "xx"),
    ("birdweather_token", "abc/123"),
];

/// Values on the other side of each of the above.
const GOOD: &[(&str, &str)] = &[
    ("recording_schedule", "solar"),
    ("recording_schedule", "all-day"),
    ("recording_schedule", "fixed:06:00-20:00"),
    ("recording_schedule", "fixed:00:00-23:59"),
    ("audio_format", "flac"),
    ("audio_format", "ogg"),
    ("info_site", "allaboutbirds"),
    ("info_site", "none"),
    ("database_lang", "de"),
    ("birdweather_token", "abc123XYZ"),
];

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, String) {
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn encode(value: &str) -> String {
    use std::fmt::Write as _;
    value.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
        out
    })
}

async fn post_settings(state: &AppState, body: &str) -> (StatusCode, String) {
    send(
        state,
        Request::builder()
            .method("POST")
            .uri("/admin/settings")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body.to_owned()))
            .expect("request"),
    )
    .await
}

async fn put_api(state: &AppState, body: &str) -> (StatusCode, String) {
    send(
        state,
        Request::builder()
            .method("PUT")
            .uri("/api/v2/settings")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::from(body.to_owned()))
            .expect("request"),
    )
    .await
}

fn stored(state: &AppState, key: &str) -> Option<String> {
    state.with_db(|conn| {
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    })
}

#[tokio::test]
async fn the_page_refuses_each_bad_value_and_writes_nothing() {
    for &(field, bad) in BAD {
        let (_dir, state) = station();
        let (status, body) = post_settings(
            &state,
            &format!("{field}={}&site_name=Should+Not+Persist", encode(bad)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{field}={bad}: the handler answers 200"
        );
        assert!(
            body.contains("Nothing was saved"),
            "{field}={bad} was accepted by the page. Body: {body}"
        );
        assert_eq!(
            stored(&state, field),
            None,
            "{field}={bad} was refused but written"
        );
        assert_eq!(
            stored(&state, "site_name"),
            None,
            "{field}={bad} was refused but the rest of the submission was written"
        );
    }
}

#[tokio::test]
async fn the_api_refuses_each_bad_value_and_writes_nothing() {
    for &(field, bad) in BAD {
        let (_dir, state) = station();
        let state = state.with_api_token(ApiToken::new(TOKEN).expect("long enough"));
        let body = serde_json::json!({ field: bad, "site_name": "Should Not Persist" });
        let (status, answer) = put_api(&state, &body.to_string()).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{field}={bad} was accepted by the API. Body: {answer}"
        );
        assert!(
            answer.contains("nothing was written"),
            "{field}={bad}: {answer}"
        );
        assert_eq!(
            stored(&state, field),
            None,
            "{field}={bad} was refused but written"
        );
        assert_eq!(
            stored(&state, "site_name"),
            None,
            "{field}={bad}: partial write"
        );
    }
}

/// The counterpart. A gate that only ever sees refusal is satisfied by a
/// handler that refuses everything.
#[tokio::test]
async fn the_page_and_the_api_both_save_each_good_value() {
    for &(field, good) in GOOD {
        let (_dir, state) = station();
        let (_, body) = post_settings(&state, &format!("{field}={}", encode(good))).await;
        assert!(
            body.contains("Settings saved"),
            "page: {field}={good}: {body}"
        );
        // The page shows a key with no row at its default, so submitting that
        // default back is accepted and is not a change (saving_the_settings_form_
        // unchanged_writes_nothing); the API saw no form, so it writes it.
        let shown_as_default = (field, good) == ("recording_schedule", "all-day");
        assert_eq!(
            stored(&state, field).as_deref(),
            (!shown_as_default).then_some(good),
            "page: {field}={good} was accepted but not written as it should be"
        );

        let (_dir, state) = station();
        let state = state.with_api_token(ApiToken::new(TOKEN).expect("long enough"));
        let (status, answer) =
            put_api(&state, &serde_json::json!({ field: good }).to_string()).await;
        assert_eq!(status, StatusCode::OK, "api: {field}={good}: {answer}");
        assert_eq!(
            stored(&state, field).as_deref(),
            Some(good),
            "api: {field}={good} was accepted but not written"
        );
    }
}

/// Blank clears a setting and is never a problem, for any of the fields.
#[tokio::test]
async fn a_blank_value_is_not_a_bad_one() {
    let (_dir, state) = station();
    let (_, body) = post_settings(
        &state,
        "recording_schedule=&audio_format=&info_site=&database_lang=&birdweather_token=",
    )
    .await;
    assert!(!body.contains("Nothing was saved"), "{body}");
}

/// The page re-submits what it rendered, and the diff drops what matches the
/// table; a bad window already stored must still stop the save. This is the
/// station whose owner is on the page because it records at the wrong hours.
#[tokio::test]
async fn a_bad_window_already_in_the_database_is_caught_on_the_next_save() {
    let (_dir, state) = station();
    state
        .with_db(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value, category)
                 VALUES ('recording_schedule', 'fixed:20:00-06:00', 'location')",
                [],
            )
        })
        .expect("seed the bad window");

    let (_, body) = post_settings(&state, "recording_schedule=fixed%3A20%3A00-06%3A00").await;
    assert!(
        body.contains("Nothing was saved") && body.contains("does not end after it starts"),
        "a stored window the scheduler ignores must be reported, not waved through as unchanged. Body: {body}"
    );
}
