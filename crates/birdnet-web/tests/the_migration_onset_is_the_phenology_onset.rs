//! N05-02. The Migration page's "Earliest vs last year" tile reads the `SQLite` store,
//! where `phenology_timing_sql`'s `arrival_q05` (`quantile_disc(detection_date, 0.05)` over
//! a year's distinct detection days, in `DuckDB`) cannot run. The tile restates that rule
//! as the `(n + 19) / 20`-th of the `n` days, which is `ceil(0.05 n)` in whole numbers.
//! This holds the restatement to the engine: for each `n` here a species is heard on
//! 1 Jan, 2 Jan, … of 2025 for `n` days, so its 5th percentile is the `k`-th of January,
//! `k = (n + 19) / 20`. The counts sit either side of the multiples of twenty, where a
//! floating-point `0.05 · n` and the engine could part.

#![cfg(feature = "analytics")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use serde_json::Value;
use tower::ServiceExt as _;

const COUNTS: [u32; 10] = [1, 19, 20, 21, 39, 40, 41, 100, 120, 121];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_tiles_rule_is_the_engines_fifth_percentile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sqlite = dir.path().join("birds.db");
    {
        let conn = birdnet_db::sqlite::open_or_create(&sqlite).expect("sqlite");
        for n in COUNTS {
            for day in 0..n {
                // Day `day` after 1 Jan 2025 (121 days reach 1 May).
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                     VALUES (date('2025-01-01', '+' || ?1 || ' days'), '06:00:00',
                             'Sci ' || ?2, 'Bird ' || ?2, 0.9, ?2 || '-' || ?1 || '.wav')",
                    rusqlite::params![day, n],
                )
                .expect("detection");
            }
        }
    }
    let state =
        AppState::new_with_analytics(sqlite, &dir.path().join("analytics.duckdb")).expect("state");
    let resp = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/v2/analytics/phenology?year=2025&min_detections=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router responds");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), 1 << 22)
            .await
            .expect("body"),
    )
    .expect("json");
    let rows = body["phenology"].as_array().expect("rows");
    assert_eq!(rows.len(), COUNTS.len(), "{body}");
    for n in COUNTS {
        let row = rows
            .iter()
            .find(|r| r["species"] == format!("Bird {n}"))
            .unwrap_or_else(|| panic!("a row for {n} days: {body}"));
        let k = n.div_ceil(20);
        assert_eq!(
            row["arrival_q05"],
            format!("2025-01-{k:02}"),
            "{n} detection days: {row}"
        );
    }
}
