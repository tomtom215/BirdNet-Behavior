//! `SEGMENT_DURATION` typed on the settings page is held to the range
//! `birdnet_core::config::validate` holds the config file to.
//!
//! # What was missing
//!
//! `validate()` has bounded `SEGMENT_DURATION` to 3–600 seconds since the key
//! was first checked, and `--doctor` runs it — against the config **file**. The
//! settings form is where the number is actually typed, and nothing there looked
//! at it: `segment_duration=0`, `2`, `100000` or `15.5` was stored, laid over the
//! file configuration at start-up *after* validation had already run
//! (`src/app.rs`: `validate`, then `overlay_db_settings`), and handed to the
//! capture loop as the length of every recording chunk. The page's own
//! `min`/`max` attributes are the browser's, not the server's: a script, the
//! REST API or a hand-made request goes straight past them.
//!
//! The same shape as `PRIVACY_RANK` (`the_privacy_rank_can_be_set_from_the_settings_page.rs`),
//! and held the same way: one set of bounds, read from `birdnet-core`, checked on
//! the **submission** at the boundary the value crosses.
//!
//! # What this gate holds
//!
//! 1. A length outside 3–600, a fraction, a sign, a unit and a word are refused,
//!    with a message that names the range, and nothing from the submission is
//!    written.
//! 2. The counterparts: both ends of the range are saved, a padded value is
//!    saved trimmed (a stray space would make the daemon's integer read fail and
//!    fall back to 15), and a value that is already what is stored is not
//!    rewritten.
//! 3. A bad value already in the database is caught on the next save, because
//!    the page re-submits what it rendered and the diff drops unchanged fields.
//! 4. The REST settings API, which shares the form type and the check, says the
//!    same thing in JSON.

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

/// The edges are stated here, independently of the handler's table, so a
/// widened bound there fails this test instead of moving the goalposts with it.
#[test]
fn the_range_is_the_validators() {
    assert_eq!(
        (
            birdnet_core::config::validate::SEGMENT_DURATION_MIN,
            birdnet_core::config::validate::SEGMENT_DURATION_MAX
        ),
        (3, 600)
    );
}

#[tokio::test]
async fn a_length_the_station_cannot_use_is_refused_and_nothing_is_written() {
    // 0, 1 and 2 are shorter than one analysis window; 601 and 100000 are
    // chunks that would hold a station deaf for ten minutes or a day. The rest
    // are the shapes a person types: a fraction, a unit, a sign, a word.
    for bad in [
        "0", "2", "601", "100000", "15.5", "15%2C5", "15s", "-15", "%2B15", "1e2", "fifteen",
    ] {
        let (_dir, state) = station();
        let (status, body) = post_settings(
            &state,
            &format!("segment_duration={bad}&site_name=Should+Not+Persist"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{bad}: the handler answers 200");
        assert!(
            body.contains("Nothing was saved"),
            "segment_duration={bad} was accepted. Body: {body}"
        );
        assert!(
            body.contains("Segment Duration must be a whole number from 3 to 600"),
            "{bad}: the refusal must name the field and the range. Body: {body}"
        );
        assert_eq!(
            stored(&state, "segment_duration"),
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

/// The ends of the range are real answers, and a padded value is trimmed on
/// its way in. A check that refused everything would pass the test above.
#[tokio::test]
async fn both_ends_of_the_range_save_and_padding_is_trimmed() {
    for (typed, kept) in [("3", "3"), ("600", "600"), ("30", "30"), ("+45+", "45")] {
        let (_dir, state) = station();
        let (_, body) = post_settings(&state, &format!("segment_duration={typed}")).await;
        assert!(body.contains("Settings saved"), "{typed}: {body}");
        assert_eq!(
            stored(&state, "segment_duration").as_deref(),
            Some(kept),
            "{typed} was accepted but not written as {kept}"
        );
    }
}

#[tokio::test]
async fn a_length_that_is_already_stored_is_not_rewritten() {
    let (_dir, state) = station();
    state
        .with_db(|conn| {
            // A timestamp no rewrite can produce, so the assertion below cannot
            // be satisfied by a rewrite of the same value.
            conn.execute(
                "INSERT INTO settings (key, value, category, updated_at)
                 VALUES ('segment_duration', '30', 'audio', '2001-01-01 00:00:00')",
                [],
            )
        })
        .expect("seed the length");

    let (_, body) = post_settings(&state, "segment_duration=30").await;
    assert!(body.contains("0 values updated"), "{body}");
    let at: String = state.with_db(|conn| {
        conn.query_row(
            "SELECT updated_at FROM settings WHERE key = 'segment_duration'",
            [],
            |r| r.get(0),
        )
        .expect("the row is still there")
    });
    assert_eq!(
        at, "2001-01-01 00:00:00",
        "an unchanged length was rewritten"
    );
}

/// The page re-submits what it rendered, and the diff drops what matches the
/// table; a bad value already stored must still stop the save. This is the
/// station whose owner is on the settings page because recordings went wrong.
#[tokio::test]
async fn a_bad_length_already_in_the_database_is_caught_on_the_next_save() {
    let (_dir, state) = station();
    state
        .with_db(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value, category) VALUES ('segment_duration', '2', 'audio')",
                [],
            )
        })
        .expect("seed the bad length");

    let (_, body) = post_settings(&state, "segment_duration=2").await;
    assert!(
        body.contains("Nothing was saved"),
        "a stored length the station cannot use must be reported, not waved through as unchanged. Body: {body}"
    );
}

#[tokio::test]
async fn the_api_takes_and_refuses_the_same_lengths() {
    let (_dir, state) = station();
    let state = state.with_api_token(ApiToken::new(TOKEN).expect("long enough"));

    // JSON numbers, which is what a script sends.
    let (status, body) = put_api(&state, r#"{"segment_duration": 30}"#).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&state, "segment_duration").as_deref(), Some("30"));

    for bad in ["2", "601", "15.5", "-1", r#""15s""#] {
        let (status, body) = put_api(
            &state,
            &format!(r#"{{"segment_duration": {bad}, "site_name": "Should Not Persist"}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        assert!(
            body.contains("from 3 to 600"),
            "{bad}: the refusal must name the range: {body}"
        );
        assert_eq!(
            stored(&state, "segment_duration").as_deref(),
            Some("30"),
            "{bad} was refused but overwrote the stored length"
        );
        assert_eq!(stored(&state, "site_name"), None, "{bad}: partial write");
    }
}
