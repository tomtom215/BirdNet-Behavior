//! `GET /pages/species-reporting`: the species page's reporting-frequency figure, through the
//! real handler, the real sync into the analytics store and the real query (CHT-16 / G-01).
//!
//! The render and the query have their own tests; this one holds the wiring: that the ledger
//! reaches the analytics copy, that the handler passes the trend queries' floor, and that what
//! the card prints is what the counts are. Every date is fixed (2025), so the run on the 1st of
//! a month is the run on any other day.
//!
//! * 1–7 May 2025: listened all seven days; a Song Thrush on 2, 4 and 6 May.
//! * 8–14 May 2025: no ledger row; a Thrush on the 9th.
//! * A Wallcreeper is in the species table nowhere: never heard.

#![cfg(feature = "analytics")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station(dir: &std::path::Path) -> AppState {
    let sqlite = dir.join("birds.db");
    {
        let conn = birdnet_db::sqlite::open_or_create(&sqlite).expect("sqlite");
        let add = |day: &str, com: &str| {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, detected_at_utc)
                 VALUES (?1, '05:30:00', 'Sci ' || ?2, ?2, 0.9, ?2 || ?1 || '.wav',
                         CAST(strftime('%s', ?1 || ' 05:30:00') AS INTEGER))",
                rusqlite::params![day, com],
            )
            .expect("seed detection");
        };
        for d in 1..=7 {
            let day = format!("2025-05-0{d}");
            conn.execute(
                "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, 'local', 86400)",
                [&day],
            )
            .expect("seed effort");
            add(&day, "Blackbird");
        }
        for day in ["2025-05-02", "2025-05-04", "2025-05-06", "2025-05-09"] {
            add(day, "Song Thrush");
        }
    }
    AppState::new_with_analytics(sqlite, &dir.join("analytics.duckdb")).expect("state")
}

async fn card(state: AppState, uri: &str) -> String {
    let resp = build_router(state)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    assert_eq!(resp.status(), StatusCode::OK, "{uri}");
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 22)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The table row for `label`, from its header cell to its end.
fn row<'a>(html: &'a str, label: &str) -> &'a str {
    let at = html
        .find(&format!(r#"<th scope="row">{label}</th>"#))
        .unwrap_or_else(|| panic!("a row for {label} in {html}"));
    let end = html[at..].find("</tr>").expect("row end") + at;
    &html[at..end]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_card_reads_three_of_seven_with_its_interval_and_no_value_where_nobody_listened() {
    let dir = tempfile::tempdir().expect("tempdir");
    let html = card(
        station(dir.path()),
        "/pages/species-reporting?name=Song%20Thrush",
    )
    .await;
    // 3 of 7: 43%, Wilson 0.1582198 to 0.7495416 (worked by hand in
    // birdnet_behavioral::interval), printed as whole percentages.
    let may = row(&html, "1–7 May");
    assert!(
        may.contains(r#"<td class="num">7</td><td class="num">3</td><td class="num">43%</td><td>16% to 75%</td>"#),
        "{may}"
    );
    // 8–14 May has no ledger row: no value, and the Thrush heard on the 9th is said, not counted.
    let off = row(&html, "8–14 May");
    assert!(off.contains("not listening") && !off.contains('%'), "{off}");
    assert!(
        html.contains("1 of 4 (the ledger does not count them as listened)"),
        "{html}"
    );
    // The reading is the first thing in the card.
    assert!(
        html.starts_with(r#"<p class="fig-take">Heard on 3 of 7 listened days (43%); most surely 1–7 May, on 3 of 7 (95% interval 16% to 75%)."#),
        "{html}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_species_never_heard_has_the_documented_empty_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let html = card(
        station(dir.path()),
        "/pages/species-reporting?name=Wallcreeper",
    )
    .await;
    assert_eq!(
        html,
        r#"<p class="cht-empty">Never heard by this station, so there is no year to draw for it.</p>"#
    );
}
