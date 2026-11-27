//! UIC-10: the detection log can take away what it found. The CSV for a
//! search holds exactly the rows the search counts, every page of them, and
//! the page links to it with its own query string.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        for i in 0..120 {
            // 120 rows over three hours, scores 0.30..0.89; a page is 50.
            let hour = 22 + (i % 3);
            let hour = if hour >= 24 { hour - 24 } else { hour };
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES ('2026-05-01', ?1, 'Strix aluco', 'Tawny Owl', ?2, ?3)",
                rusqlite::params![
                    format!("{hour:02}:{:02}:{:02}", i / 60, i % 60),
                    0.30 + f64::from(i % 60) / 100.0,
                    format!("c{i}.wav")
                ],
            )
            .expect("insert");
        }
    });
    (dir, state)
}

async fn get(state: &AppState, uri: &str, htmx: bool) -> (StatusCode, String) {
    let mut req = Request::builder().uri(uri);
    if htmx {
        req = req.header("hx-request", "true");
    }
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn the_csv_holds_every_row_the_search_counts_and_only_those() {
    let (_d, state) = station();
    let query = "from=2026-05-01&to=2026-05-01&hour_from=22&hour_to=0&conf_max=40";
    let filter = birdnet_web::routes::pages::search::SearchParams {
        from: Some("2026-05-01".into()),
        to: Some("2026-05-01".into()),
        hour_from: Some("22".into()),
        hour_to: Some("0".into()),
        conf_max: Some("40".into()),
        ..Default::default()
    }
    .to_filter();
    let (expected, keys) = state.with_db(|conn| {
        let n = birdnet_db::sqlite::search_detection_count(conn, &filter).unwrap();
        let rows = birdnet_db::sqlite::search_detections(conn, &filter, 10_000, 0).unwrap();
        (n, rows.into_iter().map(|r| r.time).collect::<Vec<_>>())
    });
    assert!(
        expected > 0 && expected < 120,
        "the filter narrows: {expected}"
    );

    let (status, csv) = get(
        &state,
        &format!("/api/v2/detections/export/search?{query}"),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{csv}");
    let lines: Vec<&str> = csv.lines().skip(1).collect();
    assert_eq!(
        i64::try_from(lines.len()).unwrap(),
        expected,
        "the CSV has the rows the search counts"
    );
    let times: Vec<&str> = lines.iter().map(|l| l.split(',').nth(1).unwrap()).collect();
    assert_eq!(times, keys, "the same rows in the same order");

    // Counterpart: a search that matches everything exports everything.
    let (_, all) = get(&state, "/api/v2/detections/export/search", false).await;
    assert_eq!(all.lines().count(), 121);
}

#[tokio::test]
async fn the_results_link_to_their_own_csv() {
    let (_d, state) = station();
    let (status, html) = get(
        &state,
        "/pages/search-results?from=2026-05-01&to=2026-05-01&conf_max=40",
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("/api/v2/detections/export/search?from=2026-05-01&amp;to=2026-05-01"),
        "{html}"
    );
}
