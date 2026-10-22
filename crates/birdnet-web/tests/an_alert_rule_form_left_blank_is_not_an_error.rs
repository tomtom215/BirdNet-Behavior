//! The alert-rule form says "blank = any" on its hour fields, and a blank one
//! is not an error.
//!
//! # What was wrong
//!
//! A browser submits every field of a form, an empty one as `name=`. `RuleForm`
//! read `hour_start` and `hour_end` as `Option<u8>`, and serde answers an empty
//! string for an integer with an error, so **Create Rule with the two hour
//! fields left as the page invites** was refused by the extractor with a 422
//! ("Failed to deserialize form body: hour_start: cannot parse integer from
//! empty string"). htmx never swaps a 4xx body, so the person saw nothing at
//! all: no rule, no message, no sign the click registered.
//!
//! # What this gate holds
//!
//! 1. The exact body a browser sends for the form as rendered, with every
//!    optional field blank, creates the rule, and the rule has no hour window.
//! 2. `0` and `23` are accepted as real answers (a blank is not the only way to
//!    say "midnight").
//! 3. What is not an hour (`24`, `-1`, `abc`, `1.5`) is refused with a toast
//!    that names the field, answered `200` (a 4xx is invisible), and nothing is
//!    created.
//! 4. One end of a window without the other is refused, not silently dropped:
//!    the matcher only applies a window when it has both ends.
//! 5. A confidence that is not a number, is outside 0..1, or is a minimum above
//!    the maximum is refused rather than quietly turned into "any".
//! 6. A day list that is not 1..7 is refused rather than stored as a rule that
//!    never fires.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

async fn post_form(state: &AppState, body: String) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/rules")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// The body a browser sends for the rule form as it renders, a Log rule with
/// the fields the person did not touch exactly as the page leaves them. The
/// hidden webhook fields are submitted too, empty, as a browser does.
fn browser_body(overrides: &[(&str, &str)]) -> String {
    let mut fields: Vec<(&str, &str)> = vec![
        ("name", "Blank hours"),
        ("species_pattern", ""),
        ("action_type", "log"),
        ("confidence_min", "0.70"),
        ("confidence_max", "1.00"),
        ("days_of_week", ""),
        ("hour_start", ""),
        ("hour_end", ""),
        ("action_webhook_url", ""),
        ("action_webhook_method", "POST"),
        ("action_webhook_body", ""),
        ("action_webhook_auth_kind", ""),
        ("action_webhook_auth_value", ""),
        ("action_webhook_header_name", ""),
        ("cooldown_minutes", "5"),
    ];
    for (k, v) in overrides {
        if let Some(slot) = fields.iter_mut().find(|(name, _)| name == k) {
            slot.1 = v;
        }
    }
    fields
        .into_iter()
        .map(|(k, v)| format!("{k}={}", v.replace(' ', "+")))
        .collect::<Vec<_>>()
        .join("&")
}

/// `(hour_start, hour_end, confidence_min, confidence_max, days)` of every rule.
type Stored = (Option<u8>, Option<u8>, f64, f64, Option<String>);

fn stored(state: &AppState) -> Vec<Stored> {
    state.with_db(|conn| {
        birdnet_db::alert_rules::list_rules(conn)
            .expect("rules list")
            .into_iter()
            .map(|r| {
                (
                    r.hour_start,
                    r.hour_end,
                    r.confidence_min,
                    r.confidence_max,
                    r.days_of_week,
                )
            })
            .collect()
    })
}

#[tokio::test]
async fn the_form_with_every_optional_field_blank_creates_the_rule() {
    let (_dir, state) = station();
    let (status, _, body) = post_form(&state, browser_body(&[])).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a blank hour is 'any', not a 422 that htmx will not show: {body}"
    );
    let rules = stored(&state);
    assert_eq!(rules.len(), 1, "the rule was not created. Body: {body}");
    assert_eq!(
        (rules[0].0, rules[0].1),
        (None, None),
        "blank hours mean no window"
    );
    assert!(body.contains("Rule created"), "{body}");
}

#[tokio::test]
async fn midnight_and_the_last_hour_are_real_answers() {
    let (_dir, state) = station();
    let (status, _, body) = post_form(
        &state,
        browser_body(&[("hour_start", "0"), ("hour_end", "23")]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rules = stored(&state);
    assert_eq!(rules.len(), 1, "{body}");
    assert_eq!((rules[0].0, rules[0].1), (Some(0), Some(23)));
}

#[tokio::test]
async fn an_hour_that_is_not_an_hour_is_refused_in_a_toast_and_nothing_is_created() {
    for (field, bad) in [
        ("hour_start", "24"),
        ("hour_end", "24"),
        ("hour_start", "-1"),
        ("hour_end", "abc"),
        ("hour_start", "1.5"),
        ("hour_end", "300"),
        ("hour_start", "99999999999999999999"),
    ] {
        let (_dir, state) = station();
        // Give the other end a value so only this field is at fault.
        let other = if field == "hour_start" {
            ("hour_end", "9")
        } else {
            ("hour_start", "5")
        };
        let (status, headers, body) = post_form(&state, browser_body(&[(field, bad), other])).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{field}={bad:?}: a 4xx is invisible in htmx; answer 200 with a toast. {body}"
        );
        assert_eq!(
            headers.get("hx-reswap").and_then(|v| v.to_str().ok()),
            Some("none"),
            "{field}={bad:?}: a refusal leaves the page alone"
        );
        assert!(
            stored(&state).is_empty(),
            "{field}={bad:?} was refused but a rule was created"
        );
        let label = if field == "hour_start" {
            "Hour Start"
        } else {
            "Hour End"
        };
        assert!(
            body.contains(label) && body.contains("0 to 23"),
            "{field}={bad:?}: the toast must name the field and the range. {body}"
        );
    }
}

#[tokio::test]
async fn one_end_of_a_window_without_the_other_is_refused() {
    for overrides in [[("hour_start", "5")], [("hour_end", "9")]] {
        let (_dir, state) = station();
        let (status, _, body) = post_form(&state, browser_body(&overrides)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            stored(&state).is_empty(),
            "{overrides:?}: the matcher applies a window only when it has both ends, \
             so a rule with one would quietly fire at every hour"
        );
        assert!(
            body.contains("Hour Start") && body.contains("Hour End"),
            "{body}"
        );
    }
}

#[tokio::test]
async fn a_confidence_that_cannot_be_used_is_refused_not_turned_into_any() {
    for (overrides, label) in [
        (vec![("confidence_min", "high")], "Min Confidence"),
        (vec![("confidence_max", "abc")], "Max Confidence"),
        (vec![("confidence_min", "70")], "Min Confidence"),
        (vec![("confidence_max", "1.5")], "Max Confidence"),
        (
            vec![("confidence_min", "0.9"), ("confidence_max", "0.5")],
            "Min Confidence",
        ),
    ] {
        let (_dir, state) = station();
        let (status, _, body) = post_form(&state, browser_body(&overrides)).await;
        assert_eq!(status, StatusCode::OK, "{overrides:?}: {body}");
        assert!(
            stored(&state).is_empty(),
            "{overrides:?} was refused but a rule was created"
        );
        assert!(body.contains(label), "{overrides:?}: {body}");
    }
}

#[tokio::test]
async fn a_comma_decimal_and_blank_confidences_still_work() {
    let (_dir, state) = station();
    let (status, _, body) = post_form(
        &state,
        browser_body(&[("confidence_min", "0,85"), ("confidence_max", "")]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rules = stored(&state);
    assert_eq!(rules.len(), 1, "{body}");
    assert!((rules[0].2 - 0.85).abs() < 1e-9, "min {}", rules[0].2);
    assert!(
        (rules[0].3 - 1.0).abs() < 1e-9,
        "blank max is 1.0: {}",
        rules[0].3
    );
}

#[tokio::test]
async fn days_are_one_to_seven_or_refused() {
    let (_dir, state) = station();
    let (status, _, body) = post_form(&state, browser_body(&[("days_of_week", "1, 2,7")])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&state).len(), 1, "a valid list is accepted: {body}");

    for bad in ["0", "8", "mon", "1;2"] {
        let (_dir, state) = station();
        let (status, _, body) = post_form(&state, browser_body(&[("days_of_week", bad)])).await;
        assert_eq!(status, StatusCode::OK, "{bad:?}: {body}");
        assert!(
            stored(&state).is_empty(),
            "days {bad:?} was refused but a rule was created"
        );
        assert!(body.contains("Days of Week"), "{bad:?}: {body}");
    }
}
