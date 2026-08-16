//! The analytics dashboards must survive a station's real history.
//!
//! Every fixture in this suite is clean, and that is exactly how a defect that
//! emptied every analytics dashboard on 0.13.x reached a release: `Date` and
//! `Time` are free-form `TEXT NOT NULL` in SQLite, the BirdNET-Pi importer
//! turns a NULL `Date` into `""` and copies malformed values through verbatim,
//! and `detections_ts` cast them with a plain `CAST`. DuckDB raises
//! `Conversion Error` for the *whole query*, so a single unplaceable row —
//! anywhere in a multi-year import — took down every behavioural and
//! time-series dashboard at once.
//!
//! Nothing reported it. The pages render a muted "Analytics temporarily
//! unavailable" placeholder because each handler maps a query error to `None`
//! (`Some(Ok(rows)) => …, _ => None`), the detection lists kept working because
//! they are served from SQLite, and `COUNT(*)` over the view kept working
//! because DuckDB never evaluates a projected column it does not need — so the
//! health endpoint stayed green throughout.
//!
//! These tests seed one bad row alongside good ones and assert the dashboards
//! still render real content.

#![cfg(feature = "analytics")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// The muted placeholder every time-series partial falls back to on a query
/// error. Asserting its *absence* is the point: it is what a broken dashboard
/// looked like, and it is indistinguishable from "no data yet" to the eye.
const TS_FALLBACK_TEXT: &str = "Analytics temporarily unavailable";

/// Build a station whose history contains rows that name no point in time,
/// mixed in with 30 days of ordinary detections.
fn state_with_dirty_history(dir: &std::path::Path) -> AppState {
    let db_path = dir.join("birds.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();

    // 30 days of ordinary, well-formed detections ending today.
    //
    // Relative to today, not fixed calendar dates: every dashboard filters on a
    // look-back from `CURRENT_DATE`, so a pinned fixture would drift out of
    // range and render an empty state that this test could not tell from a
    // working one.
    let today: String = conn
        .query_row("SELECT date('now')", [], |r| r.get(0))
        .expect("today");
    for back in 0..30 {
        for (hour, sci, com) in [
            (6, "Turdus merula", "Eurasian Blackbird"),
            (7, "Erithacus rubecula", "European Robin"),
            (8, "Parus major", "Great Tit"),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (date(?1, ?2), ?3, ?4, ?5, 0.85)",
                rusqlite::params![
                    &today,
                    format!("-{back} days"),
                    format!("{hour:02}:15:00"),
                    sci,
                    com
                ],
            )
            .unwrap();
        }
    }

    // The rows a real import leaves behind: a NULL Date arrives as "", and
    // genuinely malformed values pass through untouched.
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES ('','','Parus major','Great Tit',0.80),
                ('not-a-date','25:99:99','Corvus corax','Northern Raven',0.60);",
    )
    .unwrap();
    drop(conn);

    let state = AppState::new_with_analytics(db_path, &dir.join("analytics.duckdb"))
        .expect("analytics state opens");
    state
        .resync_analytics_full()
        .expect("analytics is configured")
        .expect("the dirty rows must not break the sync itself");
    state
}

async fn fetch(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// Every time-series dashboard partial renders real content despite the dirty
/// rows.
///
/// These partials are pure SQL over `detections_ts` — no *behavioral*
/// extension involved — which is what makes them the honest gate for the
/// unplaceable-row defect.
///
/// They do need `icu`, though. Every one of them filters on a look-back from
/// `CURRENT_DATE`, and `CURRENT_DATE` lives in ICU. This comment used to claim
/// they "need no DuckDB extension, so they run identically on an air-gapped
/// station and in CI"; a no-network run of the suite falsified that — all six
/// ICU-dependent tests in the workspace fail together, these two among them.
/// On a station and in the release images ICU is embedded at build time, so
/// the guarantee holds there; it is a local build with nothing embedded that
/// quietly reaches the network instead.
#[tokio::test]
async fn timeseries_dashboards_render_despite_unplaceable_rows() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_router(state_with_dirty_history(dir.path()));

    for uri in [
        "/pages/ts-daily",
        "/pages/ts-diversity",
        "/pages/ts-heatmap",
        "/pages/ts-sessions",
        "/pages/ts-anomalies",
        "/pages/ts-peak",
    ] {
        let (status, body) = fetch(&app, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri} should answer");
        assert!(
            !body.contains(TS_FALLBACK_TEXT),
            "{uri} fell back to the error placeholder. Two causes reach this \
             line and they are not the same problem:\n  \
             (a) the defect this test exists for — one unplaceable row aborting \
             the whole query;\n  \
             (b) ICU is simply not available, so `CURRENT_DATE` will not bind \
             and every dashboard fails alike.\n  \
             Check the captured log for `could not load DuckDB's ICU extension`: \
             if it is there this is (b), a build with no embedded ICU and no \
             network, not a regression. `--verify-extension` says so \
             directly.\n{body}"
        );
        assert!(
            !body.trim().is_empty(),
            "{uri} rendered nothing at all:\n{body}"
        );
    }
}

/// The good rows are actually counted, not merely "not an error".
///
/// A view that silently dropped *every* row would pass the check above while
/// showing an empty dashboard, so pin the arithmetic: 90 placeable detections
/// go in, and the daily-trend table has to show them.
#[tokio::test]
async fn placeable_rows_still_reach_the_dashboards() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_dirty_history(dir.path());

    let (total, unplaceable) = state
        .with_analytics(|db| {
            (
                db.detection_count().unwrap(),
                db.unplaceable_detection_count().unwrap(),
            )
        })
        .expect("analytics configured");
    assert_eq!(total, 92, "all rows sync, including the unplaceable ones");
    assert_eq!(unplaceable, 2);

    let app = build_router(state);
    let (status, body) = fetch(&app, "/pages/ts-daily").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("<table"),
        "the daily trend must render a real table, not an empty state:\n{body}"
    );
    // A row for today proves the well-formed history survived rather than being
    // filtered out alongside the unplaceable rows.
    let today: String = rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row("SELECT date('now')", [], |r| r.get(0))
        .unwrap();
    assert!(
        body.contains(&today),
        "today's detections are missing from the trend table (looking for \
         {today}):\n{body}"
    );
}
