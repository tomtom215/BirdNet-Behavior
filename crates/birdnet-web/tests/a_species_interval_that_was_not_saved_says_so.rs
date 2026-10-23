//! The per-species minimum interval (`/admin/species/intervals/*`): what the
//! form refuses says why, a write the database refused is neither shown nor
//! audited as done, and a good value is saved, listed and audited.
//!
//! htmx never swaps a 4xx/5xx body, so every refusal is a 200 carrying a toast
//! and `HX-Reswap: none` (the same contract as the confidence thresholds).

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    body: String,
) -> (u16, Option<String>, String) {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let reswap = resp
        .headers()
        .get("hx-reswap")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, reswap, String::from_utf8_lossy(&body).into_owned())
}

async fn set(state: &AppState, seconds: &str) -> (u16, Option<String>, String) {
    call(
        state,
        "POST",
        "/admin/species/intervals/set",
        format!("sci_name=Strix+aluco&seconds={seconds}"),
    )
    .await
}

fn audited(state: &AppState, action: &str) -> i64 {
    state.with_db(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = ?1",
            [action],
            |r| r.get(0),
        )
        .unwrap()
    })
}

fn stored(state: &AppState) -> Vec<(String, i64)> {
    state
        .with_db(birdnet_db::sqlite::get_species_intervals)
        .unwrap()
        .into_iter()
        .map(|i| (i.sci_name, i.min_interval_secs))
        .collect()
}

#[tokio::test]
async fn a_good_value_is_saved_listed_and_audited() {
    let st = station();
    let (status, reswap, body) = set(&st, "90").await;
    assert_eq!(status, 200);
    assert_eq!(reswap, None, "{body}");
    assert!(body.contains("Strix aluco"), "{body}");
    assert!(body.contains("90 s"), "{body}");
    assert_eq!(stored(&st), vec![("Strix aluco".to_owned(), 90)]);
    assert_eq!(audited(&st, "species.interval.set"), 1);
}

#[tokio::test]
async fn every_slip_is_refused_with_a_toast_and_nothing_stored() {
    for (raw, needle) in [
        ("0", "means no interval"),
        ("-5", "whole number"),
        ("1.5", "whole number"),
        ("abc", "whole number"),
        ("", "whole number"),
        ("86401", "unit slip"),
        ("3600000", "unit slip"),
    ] {
        let st = station();
        let (status, reswap, body) = set(&st, raw).await;
        assert_eq!(status, 200, "htmx discards a 4xx body: {raw:?}");
        assert_eq!(reswap.as_deref(), Some("none"), "{raw:?}: {body}");
        assert!(body.contains(needle), "{raw:?} lacked {needle:?}: {body}");
        assert!(stored(&st).is_empty(), "{raw:?} was stored");
        assert_eq!(audited(&st, "species.interval.set"), 0, "{raw:?}");
    }
}

/// The boundaries themselves are accepted (the counterpart of the slips).
#[tokio::test]
async fn one_second_and_one_day_are_accepted() {
    for raw in ["1", "86400"] {
        let st = station();
        let (_, reswap, body) = set(&st, raw).await;
        assert_eq!(reswap, None, "{raw}: {body}");
        assert_eq!(stored(&st).len(), 1, "{raw}");
    }
}

#[tokio::test]
async fn a_missing_species_is_refused() {
    let st = station();
    let (status, reswap, body) = call(
        &st,
        "POST",
        "/admin/species/intervals/set",
        "sci_name=+&seconds=60".to_owned(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(reswap.as_deref(), Some("none"), "{body}");
    assert!(stored(&st).is_empty());
}

#[tokio::test]
async fn a_refused_write_is_not_reported_or_audited_as_done() {
    let st = station();
    st.with_db(|c| c.execute_batch("DROP TABLE species_intervals"))
        .unwrap();
    let (status, reswap, body) = set(&st, "60").await;
    assert_eq!(status, 200);
    assert_eq!(reswap.as_deref(), Some("none"), "shown as saved: {body}");
    assert_eq!(audited(&st, "species.interval.set"), 0);
}

#[tokio::test]
async fn an_unreadable_table_is_not_rendered_as_an_empty_one() {
    let st = station();
    st.with_db(|c| c.execute_batch("DROP TABLE species_intervals"))
        .unwrap();
    let (status, _, body) = call(&st, "GET", "/admin/species/intervals", String::new()).await;
    assert_eq!(status, 200);
    assert!(
        !body.contains("No species has an interval of its own"),
        "a database fault was shown as an empty list: {body}"
    );
}

#[tokio::test]
async fn an_interval_can_be_removed_and_the_removal_is_audited() {
    let st = station();
    set(&st, "60").await;
    let (status, reswap, body) = call(
        &st,
        "POST",
        "/admin/species/intervals/delete",
        "sci_name=Strix+aluco".to_owned(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(reswap, None, "{body}");
    assert!(stored(&st).is_empty());
    assert_eq!(audited(&st, "species.interval.delete"), 1);
    assert!(
        body.contains("No species has an interval of its own"),
        "{body}"
    );
}
