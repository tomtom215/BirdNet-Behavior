//! A webhook alert rule made in the admin form carries a cooldown, and the
//! number the person typed is the number the daemon enforces.
//!
//! # What was missing
//!
//! Migration 55 gave `alert_rules` a `cooldown_secs` column and the daemon
//! (`src/daemon/alert_cooldown.rs`) enforces it, because a calling owl is
//! detected in every 3 s chunk and a rule on `Strix*` posted once per chunk for
//! as long as it called. The admin form had no field for it. Its handler
//! wrote `cooldown_secs: 0`, so **every rule created in the UI** — the only way
//! most stations make one — kept the old behaviour: a post for every matching
//! detection. The protection existed and nobody who used the page could turn it
//! on.
//!
//! # What this gate holds
//!
//! 1. The create form offers the field, pre-filled with the default, **inside
//!    the webhook-only part of the form** — a Log or Suppress rule ignores a
//!    cooldown (a suppress rule that stopped suppressing after its first match
//!    would let every later notification through), so the page does not offer
//!    one there.
//! 2. A webhook rule stores what was typed, in seconds (the form asks in
//!    minutes). `0` is a real answer and is stored as `0`, not as the default.
//! 3. What is not a number of minutes in range — negative, fractional, text,
//!    blank, past the seven-day cap — is refused with a message that names the
//!    range, and **nothing is created**. It is not clamped.
//! 4. A rule whose action ignores the cooldown stores `0`, whatever the hidden
//!    field held.
//! 5. The list says what each webhook rule's cooldown is, and says nothing of
//!    one on a rule that ignores it.
//! 6. A cooldown survives export and import, including an explicit `0`, which
//!    must not come back as the form's default.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_db::alert_rules::{DEFAULT_COOLDOWN_SECS, MAX_COOLDOWN_SECS};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

/// Percent-encode a form value byte by byte, so a JSON document or a space in
/// a rule name survives `application/x-www-form-urlencoded`.
fn enc(value: &str) -> String {
    use std::fmt::Write as _;
    value.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
        out
    })
}

async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = build_router(state.clone())
        .oneshot(req)
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

async fn get(state: &AppState, uri: &str) -> String {
    let (status, _, body) = send(
        state,
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "GET {uri}");
    body
}

/// POST `/admin/rules` with the given extra `name=value` pairs.
async fn create(state: &AppState, pairs: &[(&str, &str)]) -> (axum::http::HeaderMap, String) {
    let body = pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", enc(v)))
        .collect::<Vec<_>>()
        .join("&");
    let (status, headers, text) = send(
        state,
        Request::builder()
            .method("POST")
            .uri("/admin/rules")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the handler answers 200: {text}");
    (headers, text)
}

fn webhook<'a>(name: &'a str, cooldown: Option<&'a str>) -> Vec<(&'a str, &'a str)> {
    let mut v = vec![
        ("name", name),
        ("action_type", "webhook"),
        ("action_webhook_url", "https://hooks.example.test/owl"),
        ("action_webhook_method", "POST"),
    ];
    if let Some(c) = cooldown {
        v.push(("cooldown_minutes", c));
    }
    v
}

/// `(name, cooldown_secs)` of every stored rule, oldest first.
fn stored(state: &AppState) -> Vec<(String, u32)> {
    state.with_db(|conn| {
        birdnet_db::alert_rules::list_rules(conn)
            .expect("rules list")
            .into_iter()
            .map(|r| (r.name, r.cooldown_secs))
            .collect()
    })
}

fn cooldown_of(state: &AppState, name: &str) -> u32 {
    stored(state)
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no rule named {name:?} was created"))
        .1
}

/// The markup of the `<div>` that opens at `start`, up to its matching close.
///
/// Counted by nesting, not by position: a field that is merely *after* the
/// webhook section's opening tag is not inside it, and a test that took
/// "between two landmarks" for "inside" passed with the field moved out.
fn div_at(page: &str, start: usize) -> &str {
    let mut depth = 0_i32;
    let mut at = start;
    while at < page.len() {
        let rest = &page[at..];
        if rest.starts_with("<div") {
            depth += 1;
            at += 4;
        } else if rest.starts_with("</div>") {
            depth -= 1;
            at += 6;
            if depth == 0 {
                return &page[start..at];
            }
        } else {
            at += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    panic!("the <div> at {start} never closes");
}

#[tokio::test]
async fn the_create_form_offers_a_cooldown_with_the_webhook_fields_and_nowhere_else() {
    let (_dir, state) = station();
    let page = get(&state, "/station/alerts").await;

    let section_start = page
        .find(r#"<div id="webhook-fields""#)
        .expect("the form has a webhook-only section");
    let section = div_at(&page, section_start);
    let field = r#"name="cooldown_minutes""#;
    assert_eq!(
        page.matches(field).count(),
        1,
        "exactly one cooldown field on the page"
    );
    assert!(
        section.contains(field),
        "the cooldown field is outside the webhook-only section, so a Log or Suppress rule \
         is offered a setting that does nothing"
    );
    // The probe itself: the section really does hold the webhook's own fields,
    // so "inside" cannot be satisfied by a section that closed early.
    assert!(
        section.contains(r#"name="action_webhook_url""#),
        "{section}"
    );

    // Pre-filled with the default the database layer exports, in minutes.
    let at = section.find(field).expect("the cooldown field");
    let input_start = section[..at].rfind("<input").expect("an <input>");
    let input = &section[input_start..at + section[at..].find('>').expect("closes")];
    let default_minutes = DEFAULT_COOLDOWN_SECS / 60;
    assert!(
        input.contains(&format!(r#"value="{default_minutes}""#)),
        "a new rule must start with the default cooldown ({default_minutes} min): {input}"
    );
    // And the page says what the field does not do.
    assert!(
        section.contains("Log and Suppress rules"),
        "the page must say the cooldown is for webhooks only"
    );
}

#[tokio::test]
async fn a_webhook_rule_stores_the_cooldown_typed_in_seconds_and_zero_stays_zero() {
    let (_dir, state) = station();
    create(&state, &webhook("ten", Some("10"))).await;
    create(&state, &webhook("none", Some("0"))).await;
    create(
        &state,
        &webhook("longest", Some(&(MAX_COOLDOWN_SECS / 60).to_string())),
    )
    .await;
    assert_eq!(cooldown_of(&state, "ten"), 600);
    assert_eq!(
        cooldown_of(&state, "none"),
        0,
        "an explicit 0 is 'every match' and must not become the default"
    );
    assert_eq!(cooldown_of(&state, "longest"), MAX_COOLDOWN_SECS);

    // A request that does not mention the field at all — a script, or a tab
    // opened before the upgrade — gets the protective default, not 0.
    create(&state, &webhook("absent", None)).await;
    assert_eq!(cooldown_of(&state, "absent"), DEFAULT_COOLDOWN_SECS);
}

#[tokio::test]
async fn a_cooldown_that_is_not_minutes_in_range_is_refused_and_nothing_is_created() {
    let past = (MAX_COOLDOWN_SECS / 60 + 1).to_string();
    for bad in [
        "-5",
        "abc",
        "1.5",
        "1,5",
        "5 min",
        "",
        "   ",
        past.as_str(),
        "99999999999999999999",
    ] {
        let (_dir, state) = station();
        let (headers, body) = create(&state, &webhook("rejected", Some(bad))).await;
        assert!(
            stored(&state).is_empty(),
            "cooldown {bad:?} was refused but a rule was created anyway"
        );
        assert_eq!(
            headers.get("hx-reswap").and_then(|v| v.to_str().ok()),
            Some("none"),
            "cooldown {bad:?}: a refusal must leave the page alone and say why in a toast"
        );
        assert!(
            body.to_lowercase().contains("cooldown")
                && body.contains(&format!("0 to {}", MAX_COOLDOWN_SECS / 60)),
            "cooldown {bad:?}: the refusal must name the field and the range. Body: {body}"
        );
    }
}

#[tokio::test]
async fn a_rule_that_ignores_a_cooldown_stores_none_whatever_the_hidden_field_held() {
    for (action, junk) in [("log", "10"), ("suppress", "10"), ("log", "garbage")] {
        let (_dir, state) = station();
        create(
            &state,
            &[
                ("name", "quiet"),
                ("action_type", action),
                ("cooldown_minutes", junk),
            ],
        )
        .await;
        assert_eq!(
            cooldown_of(&state, "quiet"),
            0,
            "a {action} rule ignores a cooldown and must not store one ({junk:?})"
        );
    }
}

#[tokio::test]
async fn the_list_says_what_each_webhook_rules_cooldown_is() {
    let (_dir, state) = station();
    create(&state, &webhook("fiveish", Some("5"))).await;
    create(&state, &webhook("hourly", Some("60"))).await;
    create(&state, &webhook("every", Some("0"))).await;
    create(&state, &[("name", "just-log"), ("action_type", "log")]).await;

    let list = get(&state, "/admin/rules/list").await;
    let row = |name: &str| -> String {
        let at = list
            .find(&format!("<strong>{name}</strong>"))
            .unwrap_or_else(|| panic!("{name} is not listed: {list}"));
        let end = list[at..].find("</tr>").expect("row ends");
        list[at..at + end].to_owned()
    };
    assert!(
        row("fiveish").contains("cooldown 5 min"),
        "{}",
        row("fiveish")
    );
    assert!(row("hourly").contains("cooldown 1 h"), "{}", row("hourly"));
    assert!(row("every").contains("no cooldown"), "{}", row("every"));
    // The counterpart: a rule that ignores a cooldown claims none, in either
    // direction ("no cooldown" would say it posts for every match).
    assert!(
        !row("just-log").to_lowercase().contains("cooldown"),
        "a Log rule has no cooldown to describe: {}",
        row("just-log")
    );
}

#[tokio::test]
async fn a_cooldown_survives_export_and_import_and_an_explicit_zero_stays_zero() {
    let (_a_dir, a) = station();
    create(&a, &webhook("twelve", Some("12"))).await;
    create(&a, &webhook("every", Some("0"))).await;
    create(&a, &[("name", "just-log"), ("action_type", "log")]).await;

    // `?secrets=1` so the webhook address survives the trip; a redacted one
    // imports switched off, which is a different gate's business.
    let exported = get(&a, "/admin/rules/export?secrets=1").await;
    let doc: serde_json::Value = serde_json::from_str(&exported).expect("the export is JSON");
    let by_name = |n: &str| {
        doc["rules"]
            .as_array()
            .expect("rules")
            .iter()
            .find(|r| r["name"] == n)
            .unwrap_or_else(|| panic!("{n} missing from the export"))
            .clone()
    };
    assert_eq!(by_name("twelve")["cooldown_secs"], 720, "{exported}");
    assert!(
        by_name("every").get("cooldown_secs").is_none(),
        "a zero cooldown is omitted from the file: {exported}"
    );

    let (_b_dir, b) = station();
    let (status, _, body) = send(
        &b,
        Request::builder()
            .method("POST")
            .uri("/admin/rules/import")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!("rules_json={}", enc(&exported))))
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Imported 3 rule(s)"), "{body}");

    assert_eq!(cooldown_of(&b, "twelve"), 720);
    assert_eq!(
        cooldown_of(&b, "every"),
        0,
        "an explicit 0 came back from the import as something else"
    );
    assert_eq!(cooldown_of(&b, "just-log"), 0);
}
