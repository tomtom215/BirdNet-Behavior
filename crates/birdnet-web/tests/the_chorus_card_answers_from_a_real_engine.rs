//! `GET /pages/analytics-assembly`: the Sequences tab's chorus card, from a real engine.
//!
//! The handler does four things the renderers' own tests cannot see: it asks the station's own
//! database which voices are its regular dawn voices of the season (at least ten songs in the dawn
//! hours, most heard first, then in the order they open), it runs `window_funnel` for them at the
//! ladder of windows, it puts the extension's live version in the Method panel, and it leaves out
//! mornings older than the season.
//!
//! The station is ten mornings in the last ten days, each the same by construction so the answer
//! can be worked out by hand: a Robin at 05:00 and 05:01, a Blackbird at 05:02, a Wren at 05:12.
//! The tightest chain is the 05:01 Robin, the 05:02 Blackbird and the 05:12 Wren, eleven minutes
//! from first to last: two voices from the first minute, all three from the twelve-minute rung and
//! not the eight. The Blackbird and the Wren are heard exactly ten times, the floor. One more
//! morning, a hundred days ago, has only the Robin and is outside the ninety days.

// Needs the DuckDB analytics store: without the feature `AppState` has no analytics methods.
#![cfg(feature = "analytics")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A station with the ten mornings above (and, when `with_wren`, the Wren on them).
fn station(dir: &std::path::Path, with_wren: bool) -> AppState {
    let sqlite = dir.join("birds.db");
    {
        let conn = birdnet_db::sqlite::open_or_create(&sqlite).expect("sqlite");
        let add = |ago: u32, time: &str, sci: &str, com: &str| {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, detected_at_utc)
                 SELECT d, ?2, ?3, ?4, 0.9, 'clip.wav', CAST(strftime('%s', d || ' ' || ?2) AS INTEGER)
                 FROM (SELECT date('now', 'localtime', '-' || ?1 || ' days') AS d)",
                rusqlite::params![ago, time, sci, com],
            )
            .expect("seed a detection");
        };
        for ago in 1..=10 {
            add(ago, "05:00:00", "Erithacus rubecula", "Robin");
            add(ago, "05:01:00", "Erithacus rubecula", "Robin");
            add(ago, "05:02:00", "Turdus merula", "Blackbird");
            if with_wren {
                add(ago, "05:12:00", "Troglodytes troglodytes", "Wren");
            }
        }
        // Outside the ninety days: it must not be counted.
        add(100, "05:00:00", "Erithacus rubecula", "Robin");
    }
    AppState::new_with_analytics(sqlite, &dir.join("analytics.duckdb"))
        .expect("a state with the analytics store")
}

async fn card(state: AppState) -> (StatusCode, String) {
    let resp = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/pages/analytics-assembly")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router responds");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_card_follows_the_stations_own_voices_and_says_what_it_computed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (status, html) = card(station(dir.path(), true)).await;
    assert_eq!(status, StatusCode::OK);

    // The voices are the station's, most heard first by who opens: Robin, Blackbird, Wren.
    assert!(
        html.contains("Your dawn builds in this order: <strong>Robin → Blackbird → Wren</strong>"),
        "{html}"
    );
    // Ten mornings, not eleven: the one a hundred days ago is outside the season.
    assert!(
        html.contains("On half of your 10 mornings all 3 voices were together"),
        "{html}"
    );
    // Two voices from the first minute, all three from the twelve-minute rung (not the eight).
    assert!(
        html.contains("in order, within 12 min; two were together within 1 min."),
        "{html}"
    );
    // The 12 minute row: typical morning 3 voices, middle half 3 to 3, 10 and 100% of mornings both ways.
    let row = html
        .split(r#"<th scope="row" data-csv="12">"#)
        .nth(1)
        .and_then(|r| r.split("</tr>").next())
        .expect("the 12 minute row");
    assert!(
        row.contains(">3</td>")
            && row.matches(">100</td>").count() == 2
            && row.matches(">10</td>").count() == 2,
        "{row}"
    );
    let row8 = html
        .split(r#"<th scope="row" data-csv="8">"#)
        .nth(1)
        .and_then(|r| r.split("</tr>").next())
        .expect("the 8 minute row");
    assert!(
        row8.contains(">0</td>"),
        "no morning has all three in eight minutes: {row8}"
    );

    // The Method panel names the function, the extension's live version, and what was asked.
    assert!(html.contains("<code>window_funnel</code>"), "{html}");
    assert!(
        html.contains("version ") && !html.contains("version not reported"),
        "the live extension reports its version: {html}"
    );
    assert!(html.contains("Robin → Blackbird → Wren"), "{html}");
    assert!(
        html.contains("/api/v2/analytics/assembly?species=Robin,Blackbird,Wren&amp;since="),
        "{html}"
    );
    assert!(html.contains("&amp;hour_start=4&amp;hour_end=8"), "{html}");
    assert!(html.contains(r##"data-csv-table="#as-table""##), "{html}");
}

/// Two regular voices are a pair, not a chorus: the card says so in words and draws nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fewer_than_three_regular_voices_is_said_and_draws_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (status, html) = card(station(dir.path(), false)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("Fewer than 3 voices sing often enough at dawn here"),
        "{html}"
    );
    assert!(
        !html.contains("<svg") && !html.contains("bh-method"),
        "{html}"
    );
}
