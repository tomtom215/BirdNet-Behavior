//! The settings form must not accept a number that silently switches the
//! station off.
//!
//! # The defect
//!
//! `birdnet_core::config::validate` has always checked these ranges, and
//! `--doctor` runs it from `ExecStartPre`. But it only ever sees the config
//! **file**: `src/app.rs` validates, and *then* `overlay_db_settings` lays the
//! settings table over the result. A number typed into `/admin/settings` goes
//! into that table, so it arrives after the only check that would have caught
//! it and is never looked at again.
//!
//! The specific mistake is the percentage slip. The field is labelled
//! "Minimum Confidence (0–1)", the score it is compared against is a
//! probability, and the app's own notification templates offer
//! `$confidencepct` alongside `$confidence` — so `75` gets typed where `0.75`
//! was meant. It parses. It stores. `resolve_confidence` hands `75.0` to the
//! detection pipeline, every three-second window scores below it, and the
//! station records nothing ever again. No error is raised anywhere: the
//! station looks healthy, the microphone is live, and the birds stop.
//!
//! A probe against the shipped code confirmed the whole chain before this gate
//! was written:
//!
//! ```text
//! stored CONFIDENCE = "75"; parsed = Some(75.0); validate() would say
//! ["Error: CONFIDENCE=75 is outside the valid range 0 to 1", …]
//! ```
//!
//! — the validator knows, and is never asked.
//!
//! # What is guarded
//!
//! Three things, and all three are needed:
//!
//! 1. Every bounded field rejects a value outside its range, and the whole
//!    submission is dropped rather than half-written.
//! 2. A value **inside** the range still saves. Without this a validator that
//!    rejected everything would pass.
//! 3. A comma decimal (`0,82`) still saves. The form documents that either
//!    separator works, and a check placed before normalisation would break
//!    that while looking correct.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Every field whose value the station compares against a model score or a
/// bounded parameter, with a value outside its range.
///
/// Stated here independently of the handler's own table so that dropping an
/// entry from that table fails this test rather than quietly widening what the
/// form will accept.
const OUT_OF_RANGE: &[(&str, &str)] = &[
    ("confidence_threshold", "75"),
    ("sensitivity", "3"),
    ("overlap", "9"),
    ("sf_thresh", "50"),
    ("privacy_threshold", "2"),
    ("notify_confidence", "80"),
    ("email_min_confidence", "90"),
];

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

async fn post_settings(state: &AppState, body: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/admin/settings")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.to_owned()))
        .expect("request");
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
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
async fn a_value_outside_its_range_is_refused_and_nothing_is_written() {
    for &(field, bad) in OUT_OF_RANGE {
        let (_dir, state) = station();
        // A second, perfectly good change rides along: the submission has to
        // be dropped whole, or the form and the station disagree afterwards.
        let (status, body) = post_settings(
            &state,
            &format!("{field}={bad}&site_name=Should+Not+Persist"),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{field}: handler should answer");
        assert!(
            body.contains("Nothing was saved"),
            "{field}={bad} was accepted; the station would stop detecting. Body: {body}"
        );
        assert_eq!(
            stored(&state, field),
            None,
            "{field}={bad} was refused but written anyway"
        );
        assert_eq!(
            stored(&state, "site_name"),
            None,
            "{field}={bad} was refused but the rest of the submission was written"
        );
    }
}

/// The counterpart. A gate that only ever sees rejection is satisfied by a
/// handler that rejects everything.
#[tokio::test]
async fn a_value_inside_its_range_still_saves() {
    let in_range = [
        ("confidence_threshold", "0.75"),
        ("sensitivity", "1.25"),
        ("overlap", "1.5"),
        ("sf_thresh", "0.03"),
        ("privacy_threshold", "0.02"),
        ("notify_confidence", "0.8"),
        ("email_min_confidence", "0.9"),
    ];
    for (field, good) in in_range {
        let (_dir, state) = station();
        let (_, body) = post_settings(&state, &format!("{field}={good}")).await;
        assert!(
            body.contains("Settings saved"),
            "{field}={good} is inside its range but was refused. Body: {body}"
        );
        assert_eq!(
            stored(&state, field).as_deref(),
            Some(good),
            "{field}={good} was accepted but not written"
        );
    }
}

/// The form tells the reader that either decimal separator works. A range
/// check run before normalisation would fail `0,82` as "not a number" and look
/// entirely correct while doing it.
#[tokio::test]
async fn a_comma_decimal_is_still_accepted() {
    let (_dir, state) = station();
    let (_, body) = post_settings(&state, "confidence_threshold=0,82").await;
    assert!(
        body.contains("Settings saved"),
        "a comma decimal was refused, breaking the separator the form documents. Body: {body}"
    );
    assert_eq!(
        stored(&state, "confidence_threshold").as_deref(),
        Some("0.82"),
        "the comma decimal was not normalised on the way in"
    );
}

/// A bad value that is *already stored* must be caught too.
///
/// `build_settings_items` drops any field whose submitted value matches the
/// database, so a check driven by that list waves through exactly the station
/// that is already broken — the one whose owner is on this page because the
/// birds stopped. The form renders the stored value, the reader saves, and
/// nothing happens.
#[tokio::test]
async fn a_value_already_in_the_database_is_caught_on_the_next_save() {
    let (_dir, state) = station();
    // Put it there behind the form's back, as a seed from a config file would.
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO settings (key, value, category) VALUES ('confidence_threshold', '75', 'detection')",
            [],
        )
    })
    .expect("seed the bad value");

    // The form re-submits what it rendered, so the value is unchanged.
    let (_, body) = post_settings(&state, "confidence_threshold=75").await;
    assert!(
        body.contains("Nothing was saved"),
        "a stored out-of-range value must be reported when the form is next \
         saved, not waved through as unchanged. Body: {body}"
    );
}

/// The message has to name the mistake, not just the rule. Someone who typed
/// `75` meaning 75% needs to be told what to type instead.
#[tokio::test]
async fn the_refusal_names_the_percentage_mistake() {
    let (_dir, state) = station();
    let (_, body) = post_settings(&state, "confidence_threshold=75").await;
    assert!(
        body.contains("0.75"),
        "the refusal must offer the value the reader meant. Body: {body}"
    );
}
