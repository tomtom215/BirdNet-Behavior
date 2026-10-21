//! What the station has learned about each species can be listed, counted and
//! reset (`P-11`), over the JSON API and from the admin page.
//!
//! The reads are public, like the rest of the read API; the reset is
//! bearer-gated (a station with no `BNB_API_TOKEN` answers `404`) and, on the
//! admin page, behind the admin sign-in and the CSRF guard. The reset has to
//! reach the running tracker as well as the table; that half is held where the
//! tracker lives, in the binary's `daemon::processor` tests.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_core::detection::dynamic_threshold::{DynamicThresholdConfig, DynamicThresholdHandle};
use birdnet_db::dynamic_thresholds::LearnedThreshold;
use birdnet_web::api_token::ApiToken;
use birdnet_web::state::AppState;
use serde_json::Value;
use tower::ServiceExt as _;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const HOUR_MS: i64 = 3_600_000;

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

const fn config() -> DynamicThresholdConfig {
    DynamicThresholdConfig {
        enabled: true,
        trigger: 0.8,
        min: 0.3,
        valid_hours: 24,
    }
}

fn row(name: &str, level: u8, expires_in_ms: i64) -> LearnedThreshold {
    let now = now_ms();
    LearnedThreshold {
        sci_name: name.to_owned(),
        level,
        confirmations: u32::from(level) + 1,
        expires_at_ms: now + expires_in_ms,
        first_learned_ms: now - 5 * HOUR_MS,
        last_confirmed_ms: now - HOUR_MS,
    }
}

/// One visible detection of `name`. The public reads list a learned row only
/// while its species still has one (`a_detection_the_owner_hid_is_not_confirmed_by_the_public_reads.rs`),
/// so a fixture that learned a species and never heard it would be asserting
/// a state the station cannot reach.
fn hear(state: &AppState, name: &str) {
    state
        .with_db(|c| {
            c.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, \
                 Sens, Overlap, File_Name, chunk_offset_secs)
                 VALUES ('2026-10-21', '06:30:12', ?1, 'Bird', 0.9, 0.7, 43, 1.25, 0.0, 'x.wav', 0)",
                [name],
            )
        })
        .unwrap();
}

/// A station with three learned rows (two in force, one lapsed), a daemon
/// handle, and the write API on.
fn station() -> (tempfile::TempDir, AppState, Arc<DynamicThresholdHandle>) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db"))
        .unwrap()
        .with_api_token(ApiToken::new(TOKEN).unwrap());
    state
        .with_db(|c| {
            birdnet_db::dynamic_thresholds::replace_all(
                c,
                &[
                    row("Pica pica", 1, 10 * HOUR_MS),
                    row("Strix aluco", 3, 10 * HOUR_MS),
                    row("Turdus merula", 2, -HOUR_MS),
                ],
            )
        })
        .unwrap();
    for name in ["Pica pica", "Strix aluco", "Turdus merula"] {
        hear(&state, name);
    }
    let handle = Arc::new(DynamicThresholdHandle::new(config()));
    state.set_dynamic_threshold_handle(Arc::clone(&handle));
    (dir, state, handle)
}

async fn call(
    state: &AppState,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: &str,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = bearer {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, json)
}

fn names(v: &Value) -> Vec<String> {
    v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["scientific_name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn the_list_is_public_and_says_what_is_in_force() {
    let (_d, state, _h) = station();
    let (status, v) = call(&state, "GET", "/api/v2/dynamic-thresholds", None, "").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["enabled"], true);
    assert_eq!(v["total"], 3);
    assert_eq!(names(&v), ["Pica pica", "Strix aluco", "Turdus merula"]);
    assert!((v["config"]["min"].as_f64().unwrap() - 0.3).abs() < 1e-6);
    let by = |n: &str| {
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["scientific_name"] == n)
            .unwrap()
            .clone()
    };
    assert_eq!(by("Strix aluco")["active"], true);
    assert_eq!(by("Strix aluco")["level"], 3);
    assert!((by("Strix aluco")["multiplier"].as_f64().unwrap() - 0.25).abs() < 1e-9);
    assert_eq!(by("Turdus merula")["active"], false);
    assert!(
        by("Turdus merula")["effective_threshold"].is_null(),
        "a lapsed level applies nothing"
    );
    // RFC 3339, not an epoch.
    assert!(
        by("Pica pica")["expires_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
}

#[tokio::test]
async fn the_effective_threshold_follows_the_trackers_own_rule() {
    let (_d, state, _h) = station();
    // The owl has a threshold of its own, 0.90, at level 3 (x0.25 = 0.225) with a
    // floor of 0.30: the floor wins. The magpie falls back to the station's.
    state
        .with_db(|c| birdnet_db::sqlite::set_species_threshold(c, "Strix aluco", 0.90))
        .unwrap();
    let (_, v) = call(
        &state,
        "GET",
        "/api/v2/dynamic-thresholds/Strix%20aluco",
        None,
        "",
    )
    .await;
    assert_eq!(v["base_is"], "species");
    assert!((v["base_threshold"].as_f64().unwrap() - 0.90).abs() < 1e-9);
    assert!(
        (v["effective_threshold"].as_f64().unwrap() - 0.30).abs() < 1e-9,
        "{v}"
    );
    let (_, m) = call(
        &state,
        "GET",
        "/api/v2/dynamic-thresholds/pica%20pica",
        None,
        "",
    )
    .await;
    assert_eq!(
        m["base_is"], "station",
        "a name in another case is the same species"
    );
    let base = m["base_threshold"].as_f64().unwrap();
    assert!((m["effective_threshold"].as_f64().unwrap() - (base * 0.75).max(0.3)).abs() < 1e-9);
    // Never raised above the base: a species easier than the floor stays so.
    state
        .with_db(|c| birdnet_db::sqlite::set_species_threshold(c, "Pica pica", 0.2))
        .unwrap();
    let (_, e) = call(
        &state,
        "GET",
        "/api/v2/dynamic-thresholds/Pica%20pica",
        None,
        "",
    )
    .await;
    assert!(
        (e["effective_threshold"].as_f64().unwrap() - 0.2).abs() < 1e-9,
        "{e}"
    );
}

#[tokio::test]
async fn paging_and_the_active_filter() {
    let (_d, state, _h) = station();
    let (_, v) = call(
        &state,
        "GET",
        "/api/v2/dynamic-thresholds?limit=1&offset=1",
        None,
        "",
    )
    .await;
    assert_eq!(names(&v), ["Strix aluco"]);
    assert_eq!(v["total"], 3, "the total is before paging");
    let (_, v) = call(
        &state,
        "GET",
        "/api/v2/dynamic-thresholds?active=true",
        None,
        "",
    )
    .await;
    assert_eq!(names(&v), ["Pica pica", "Strix aluco"]);
    let (_, v) = call(
        &state,
        "GET",
        "/api/v2/dynamic-thresholds?active=false",
        None,
        "",
    )
    .await;
    assert_eq!(names(&v), ["Turdus merula"]);
    for bad in [
        "limit=0",
        "limit=501",
        "limit=x",
        "offset=-1",
        "active=maybe",
    ] {
        let (status, v) = call(
            &state,
            "GET",
            &format!("/api/v2/dynamic-thresholds?{bad}"),
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
}

#[tokio::test]
async fn the_stats_count_what_is_in_force() {
    let (_d, state, _h) = station();
    let (status, v) = call(&state, "GET", "/api/v2/dynamic-thresholds/stats", None, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["total_count"], 3);
    assert_eq!(v["active_count"], 2);
    assert_eq!(v["at_maximum_count"], 1);
    let dist: Vec<(i64, i64)> = v["level_distribution"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["level"].as_i64().unwrap(), d["count"].as_i64().unwrap()))
        .collect();
    assert_eq!(
        dist,
        [(0, 0), (1, 1), (2, 0), (3, 1)],
        "the lapsed level 2 is not counted"
    );
}

#[tokio::test]
async fn an_unknown_species_is_404_and_an_unreadable_table_is_not_an_empty_list() {
    let (_d, state, _h) = station();
    let (status, _) = call(
        &state,
        "GET",
        "/api/v2/dynamic-thresholds/Aquila%20chrysaetos",
        None,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    state
        .with_db(|c| c.execute_batch("DROP TABLE dynamic_thresholds"))
        .unwrap();
    for path in [
        "/api/v2/dynamic-thresholds",
        "/api/v2/dynamic-thresholds/stats",
    ] {
        let (status, v) = call(&state, "GET", path, None, "").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{path}: {v}");
        assert!(
            !v.to_string().contains("dynamic_thresholds"),
            "{path} leaked the schema: {v}"
        );
    }
}

/// With no daemon there is no floor to clamp to, so no "threshold now" is
/// claimed: a number computed without the floor could be lower than the one the
/// station applies.
#[tokio::test]
async fn without_a_floor_no_effective_threshold_is_claimed() {
    let (_d, state, _h) = station();
    let dir = tempfile::tempdir().unwrap();
    let bare = AppState::new(dir.path().join("birds.db")).unwrap();
    bare.with_db(|c| {
        birdnet_db::dynamic_thresholds::replace_all(c, &[row("Strix aluco", 3, 10 * HOUR_MS)])
    })
    .unwrap();
    hear(&bare, "Strix aluco");
    let (_, v) = call(
        &bare,
        "GET",
        "/api/v2/dynamic-thresholds/Strix%20aluco",
        None,
        "",
    )
    .await;
    assert_eq!(v["level"], 3);
    assert!(v["effective_threshold"].is_null(), "{v}");
    // The counterpart: with a daemon's configuration it is a number.
    let (_, w) = call(
        &state,
        "GET",
        "/api/v2/dynamic-thresholds/Strix%20aluco",
        None,
        "",
    )
    .await;
    assert!(w["effective_threshold"].is_number(), "{w}");
}

#[tokio::test]
async fn without_a_daemon_the_list_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).unwrap();
    let (status, v) = call(&state, "GET", "/api/v2/dynamic-thresholds", None, "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["enabled"].is_null() && v["config"].is_null(), "{v}");
    assert_eq!(v["total"], 0);
}

// ── reset ───────────────────────────────────────────────────────────────

fn stored(state: &AppState) -> Vec<String> {
    state
        .with_db(birdnet_db::dynamic_thresholds::load_all)
        .unwrap()
        .into_iter()
        .map(|r| r.sci_name)
        .collect()
}

#[tokio::test]
async fn the_reset_needs_the_token() {
    let (_d, state, handle) = station();
    let body = r#"{"species":"Pica pica"}"#;
    let (status, _) = call(
        &state,
        "POST",
        "/api/v2/dynamic-thresholds/reset",
        None,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(
        &state,
        "POST",
        "/api/v2/dynamic-thresholds/reset",
        Some("fedcba9876543210fedcba9876543210"),
        body,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(stored(&state).len(), 3, "nothing was reset");
    assert!(handle.drain().is_empty(), "the daemon was told anyway");

    // And a station with no token at all has no such route.
    let dir = tempfile::tempdir().unwrap();
    let open = AppState::new(dir.path().join("birds.db")).unwrap();
    let (status, _) = call(
        &open,
        "POST",
        "/api/v2/dynamic-thresholds/reset",
        Some(TOKEN),
        body,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn one_species_is_reset_in_the_table_and_the_daemon_is_told() {
    use birdnet_core::detection::dynamic_threshold::Reset;
    let (_d, state, handle) = station();
    let (status, v) = call(
        &state,
        "POST",
        "/api/v2/dynamic-thresholds/reset",
        Some(TOKEN),
        r#"{"species":" Strix aluco "}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["rows_deleted"], 1);
    assert_eq!(v["daemon_notified"], true);
    assert_eq!(stored(&state), ["Pica pica", "Turdus merula"]);
    assert_eq!(handle.drain(), vec![Reset::Species("Strix aluco".into())]);
    let audited: i64 = state.with_db(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'species.learned_threshold.reset'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    });
    assert_eq!(audited, 1);
}

#[tokio::test]
async fn resetting_everything_needs_the_confirmation_upstream_also_asks_for() {
    use birdnet_core::detection::dynamic_threshold::Reset;
    let (_d, state, handle) = station();
    for body in [
        r#"{"all":true}"#,
        r#"{"all":true,"confirm":false}"#,
        "{}",
        r#"{"species":"","all":false}"#,
        r#"{"species":"Pica pica","all":true,"confirm":true}"#,
    ] {
        let (status, v) = call(
            &state,
            "POST",
            "/api/v2/dynamic-thresholds/reset",
            Some(TOKEN),
            body,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {v}");
    }
    assert_eq!(stored(&state).len(), 3, "a refused reset changed something");
    assert!(handle.drain().is_empty());

    let (status, v) = call(
        &state,
        "POST",
        "/api/v2/dynamic-thresholds/reset",
        Some(TOKEN),
        r#"{"all":true,"confirm":true}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["rows_deleted"], 3);
    assert!(stored(&state).is_empty());
    assert_eq!(handle.drain(), vec![Reset::All]);
}

#[tokio::test]
async fn a_reset_the_database_refuses_does_not_tell_the_daemon() {
    let (_d, state, handle) = station();
    state
        .with_db(|c| c.execute_batch("DROP TABLE dynamic_thresholds"))
        .unwrap();
    let (status, v) = call(
        &state,
        "POST",
        "/api/v2/dynamic-thresholds/reset",
        Some(TOKEN),
        r#"{"species":"Pica pica"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert!(
        handle.drain().is_empty(),
        "the tracker forgot what the table still says"
    );
}

#[tokio::test]
async fn resetting_a_species_with_nothing_stored_is_a_quiet_success() {
    let (_d, state, _h) = station();
    let (status, v) = call(
        &state,
        "POST",
        "/api/v2/dynamic-thresholds/reset",
        Some(TOKEN),
        r#"{"species":"Aquila chrysaetos"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["rows_deleted"], 0);
}

// ── the admin page ──────────────────────────────────────────────────────

async fn admin_post(state: &AppState, path: &str, body: &str) -> (StatusCode, String) {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_admin_card_lists_what_was_learned_and_resets_one_species() {
    let (_d, state, handle) = station();
    let (status, html) = {
        let res = birdnet_web::server::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/admin/species/learned")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let s = res.status();
        let b = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (s, String::from_utf8_lossy(&b).into_owned())
    };
    assert_eq!(status, StatusCode::OK);
    for expect in [
        "Strix aluco",
        "Pica pica",
        "level 3 of 3",
        "lapsed",
        "Reset all",
        "floor 30%",
    ] {
        assert!(html.contains(expect), "missing {expect:?}: {html}");
    }

    let (status, after) = admin_post(
        &state,
        "/admin/species/learned/reset",
        "sci_name=Strix+aluco",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !after.contains("Strix aluco"),
        "the card still lists the species it reset: {after}"
    );
    assert!(after.contains("Pica pica"));
    assert_eq!(stored(&state), ["Pica pica", "Turdus merula"]);
    assert_eq!(handle.drain().len(), 1);

    let (_, after) = admin_post(&state, "/admin/species/learned/reset-all", "").await;
    assert!(after.contains("Nothing has been learned yet"), "{after}");
    assert!(stored(&state).is_empty());
}

#[tokio::test]
async fn the_admin_reset_refuses_a_cross_site_post_and_a_blank_name() {
    let (_d, state, handle) = station();
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/species/learned/reset-all")
                .header("host", "localhost")
                .header("origin", "http://evil.example")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        stored(&state).len(),
        3,
        "a cross-site post reset the thresholds"
    );
    assert!(handle.drain().is_empty());

    let (status, body) = admin_post(&state, "/admin/species/learned/reset", "sci_name=+").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Choose a species"), "{body}");
    assert_eq!(stored(&state).len(), 3);
}

#[tokio::test]
async fn the_card_says_when_there_is_no_daemon_and_when_the_feature_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).unwrap();
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/admin/species/learned")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let b = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&b);
    assert!(html.contains("not running the detection service"), "{html}");

    let off = DynamicThresholdHandle::new(DynamicThresholdConfig {
        enabled: false,
        ..config()
    });
    state.set_dynamic_threshold_handle(Arc::new(off));
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/admin/species/learned")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let b = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&b);
    assert!(html.contains("are <b>off</b>"), "{html}");
}
