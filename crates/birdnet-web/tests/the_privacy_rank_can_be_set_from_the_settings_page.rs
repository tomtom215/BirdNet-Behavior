//! `PRIVACY_RANK` is a setting a person can see and change, and what they type
//! is held to what the daemon can use.
//!
//! # What was missing
//!
//! The daemon has read `PRIVACY_RANK` since the human-sound rule moved to
//! BirdNET-Pi's own (a human class in the model's top ten masks the window and
//! its neighbours) and made it **on by default**. The only way to change it was
//! a line in `birdnet.conf`, an environment variable or a command-line flag, so
//! a station whose owner heard a conversation get through, or who wanted the
//! rule off, had no field to find. Settings → Detection carried only the
//! other rule, `PRIVACY_THRESHOLD`, which is on a different scale and which the
//! page labels as not being BirdNET-Pi's.
//!
//! # What this gate holds
//!
//! 1. A rank saved on the page is stored, trimmed, and shown back.
//! 2. The mistakes a person makes with a count — a fraction (`0.02`, copied
//!    across from the threshold field next to it), a percent sign, a
//!    thousands separator, a sign, a number past the model's class count — are
//!    refused, with a message that names the range and says what `0` does, and
//!    **nothing** from the submission is written. A fraction must not be
//!    truncated to `0`, which is the rule switched off with a green "saved".
//! 3. The counterparts: `0` itself is a real answer and is saved, the
//!    largest accepted value is saved, and a value that is already what is
//!    stored is not rewritten.
//! 4. A bad value already in the database is caught on the next save, because
//!    the page re-submits what it rendered and `build_settings_items` drops
//!    unchanged fields (validate the submission, not the diff).
//! 5. The REST settings API, which shares the form type and the check, says
//!    the same thing in JSON.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::api_token::ApiToken;
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

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

async fn get_page(state: &AppState) -> String {
    let (status, body) = send(
        state,
        Request::builder()
            .method("GET")
            .uri("/admin/settings")
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the settings page must render");
    body
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

/// The `<input>` element for the rank, so an assertion cannot be satisfied by
/// the hint text or a comment elsewhere on the page.
fn rank_input(page: &str) -> &str {
    let at = page
        .find(r#"<input id="privacy_rank""#)
        .expect("the settings page has a Privacy Rank field");
    let rest = &page[at..];
    &rest[..rest.find('>').expect("the input closes")]
}

#[tokio::test]
async fn a_rank_typed_on_the_page_is_saved_and_shown_back() {
    let (_dir, state) = station();

    // Nothing stored: the page shows the daemon's own default.
    let fresh = get_page(&state).await;
    let default = birdnet_core::config::DEFAULT_PRIVACY_RANK.to_string();
    assert!(
        rank_input(&fresh).contains(&format!(r#"value="{default}""#)),
        "with no row the field must show the rank the daemon runs on: {}",
        rank_input(&fresh)
    );

    // Padding comes with a copy from somewhere else; it must not reach the
    // table, where `PRIVACY_RANK=" 25"` would fail the daemon's integer read.
    let (_, body) = post_settings(&state, "privacy_rank=+25+").await;
    assert!(body.contains("Settings saved"), "{body}");
    assert_eq!(stored(&state, "privacy_rank").as_deref(), Some("25"));

    let page = get_page(&state).await;
    assert!(
        rank_input(&page).contains(r#"value="25""#),
        "the saved rank must come back on the page: {}",
        rank_input(&page)
    );
}

/// `0` is the rule switched off; it has to be accepted, and the same for the
/// top of the range. A check that refused everything would pass the next test.
#[tokio::test]
async fn zero_and_the_largest_rank_are_real_answers() {
    for good in ["0", "12000"] {
        let (_dir, state) = station();
        let (_, body) = post_settings(&state, &format!("privacy_rank={good}")).await;
        assert!(body.contains("Settings saved"), "{good}: {body}");
        assert_eq!(
            stored(&state, "privacy_rank").as_deref(),
            Some(good),
            "{good} was accepted but not written"
        );
    }
}

#[tokio::test]
async fn a_rank_that_is_not_a_count_in_range_is_refused_and_nothing_is_written() {
    // 0.02 and 0,02 are what the threshold field next to it takes. Read as a
    // number and cut to an integer they are 0: the rule off, reported as saved.
    for bad in [
        "0.02", "0%2C02", "10%25", "-5", "12001", "1%2C000", "1e3", "ten",
    ] {
        let (_dir, state) = station();
        let (status, body) = post_settings(
            &state,
            &format!("privacy_rank={bad}&site_name=Should+Not+Persist"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{bad}: the handler answers 200");
        assert!(
            body.contains("Nothing was saved"),
            "privacy_rank={bad} was accepted. Body: {body}"
        );
        assert!(
            body.contains("from 0 to 12000") && body.contains("0 turns the rule off"),
            "{bad}: the refusal must name the range and say what 0 does. Body: {body}"
        );
        assert_eq!(
            stored(&state, "privacy_rank"),
            None,
            "{bad} was refused but written"
        );
        assert_eq!(
            stored(&state, "site_name"),
            None,
            "{bad} was refused but the rest of the submission was written"
        );
    }
}

#[tokio::test]
async fn a_rank_that_is_already_stored_is_not_rewritten() {
    let (_dir, state) = station();
    state
        .with_db(|conn| {
            // A timestamp no rewrite can produce, so the assertion below
            // cannot be satisfied by a rewrite of the same value.
            conn.execute(
                "INSERT INTO settings (key, value, category, updated_at)
                 VALUES ('privacy_rank', '25', 'detection', '2001-01-01 00:00:00')",
                [],
            )
        })
        .expect("seed the rank");

    let (_, body) = post_settings(&state, "privacy_rank=25").await;
    assert!(body.contains("0 values updated"), "{body}");
    let at: String = state.with_db(|conn| {
        conn.query_row(
            "SELECT updated_at FROM settings WHERE key = 'privacy_rank'",
            [],
            |r| r.get(0),
        )
        .expect("the row is still there")
    });
    assert_eq!(at, "2001-01-01 00:00:00", "an unchanged rank was rewritten");
}

/// The page re-submits what it rendered, and the diff drops what matches the
/// table; a bad value already stored must still stop the save.
#[tokio::test]
async fn a_bad_rank_already_in_the_database_is_caught_on_the_next_save() {
    let (_dir, state) = station();
    state
        .with_db(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value, category) VALUES ('privacy_rank', '0.02', 'detection')",
                [],
            )
        })
        .expect("seed the bad rank");

    let (_, body) = post_settings(&state, "privacy_rank=0.02").await;
    assert!(
        body.contains("Nothing was saved"),
        "a stored rank that is not a count must be reported, not waved through as unchanged. Body: {body}"
    );
}

#[tokio::test]
async fn the_api_takes_and_refuses_the_same_ranks() {
    let (_dir, state) = station();
    let state = state.with_api_token(ApiToken::new(TOKEN).expect("long enough"));

    // JSON numbers, which is what a script sends.
    let (status, body) = put_api(&state, r#"{"privacy_rank": 25}"#).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&state, "privacy_rank").as_deref(), Some("25"));

    let (status, body) = put_api(&state, r#"{"privacy_rank": 0}"#).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "0 is the rule off and must be accepted: {body}"
    );
    assert_eq!(stored(&state, "privacy_rank").as_deref(), Some("0"));

    for bad in ["0.02", "12001", "-1", r#""10%""#] {
        let (status, body) = put_api(
            &state,
            &format!(r#"{{"privacy_rank": {bad}, "site_name": "Should Not Persist"}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        assert!(
            body.contains("from 0 to 12000"),
            "{bad}: the refusal must name the range: {body}"
        );
        assert_eq!(
            stored(&state, "privacy_rank").as_deref(),
            Some("0"),
            "{bad} was refused but overwrote the stored rank"
        );
        assert_eq!(stored(&state, "site_name"), None, "{bad}: partial write");
    }
}
