//! A species page for a bird this station has not heard keeps its summary row.
//!
//! The summary partial answered a bare `<p>Species not found.</p>`. It lands
//! inside `.stat-row`, a four-column bordered grid, which collapsed around it
//! to a hairline box across the page — and "not found" is false for a real
//! species reached from search or a shared link. The visual sweep never saw
//! it on a species with data, because its species route named a bird the
//! fixture had never heard.
//!
//! Observed failing with the old arm restored: the body was
//! `<p class="spp-muted">Species not found.</p>`.

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-05-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'b.wav')",
        [],
    )
    .expect("seed");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn summary(name: &str) -> String {
    let res = birdnet_web::server::build_router(station())
        .oneshot(
            Request::get(format!("/pages/species-summary?name={name}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn an_unheard_species_gets_four_empty_tiles_not_a_not_found() {
    let html = summary("European%20Robin").await;
    assert_eq!(html.matches(r#"class="stat-card""#).count(), 4, "{html}");
    assert!(
        html.contains(r#"<div class="value">0</div><div class="label">Detections"#),
        "{html}"
    );
    assert!(!html.contains("not found"), "{html}");
}

/// Its hour-of-day chart covers every date, so its empty state cannot say
/// "today", which is the dashboard's wording for the same chart.
#[tokio::test]
async fn an_unheard_species_hourly_chart_does_not_talk_about_today() {
    let res = birdnet_web::server::build_router(station())
        .oneshot(
            Request::get("/pages/species-hourly?name=European%20Robin")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("Not heard at this station yet."), "{html}");
    assert!(!html.contains("today"), "{html}");
}

/// The counterpart: a heard species still gets its real numbers.
#[tokio::test]
async fn a_heard_species_gets_its_numbers() {
    let html = summary("Eurasian%20Blackbird").await;
    assert_eq!(html.matches(r#"class="stat-card""#).count(), 4, "{html}");
    assert!(
        html.contains(r#"<div class="value">1</div><div class="label">Detections"#),
        "{html}"
    );
    assert!(html.contains("2026-05-01"), "{html}");
}

/// The trend is the last 14 calendar days. A species heard only in May has
/// none of them, and says so in a sentence rather than fourteen empty bars;
/// one heard today gets its chart, today's bar last.
#[tokio::test]
async fn the_trend_is_the_last_fourteen_calendar_days() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name) VALUES
           ('2026-05-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'b.wav'),
           (DATE('now','localtime'), '06:00:00', 'Pica pica', 'Eurasian Magpie', 0.9, 'm.wav');",
    )
    .expect("seed");
    let today: String = conn
        .query_row("SELECT DATE('now','localtime')", [], |r| r.get(0))
        .expect("today");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let get = |name: &'static str| {
        let state = state.clone();
        async move {
            let res = birdnet_web::server::build_router(state)
                .oneshot(
                    Request::get(format!("/pages/species-daily?name={name}"))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .expect("body");
            String::from_utf8_lossy(&bytes).into_owned()
        }
    };
    let blackbird = get("Eurasian%20Blackbird").await;
    assert!(
        blackbird.contains("Not heard in the last 14 days."),
        "{blackbird}"
    );
    let magpie = get("Eurasian%20Magpie").await;
    assert_eq!(magpie.matches("<rect ").count(), 14, "{magpie}");
    assert!(
        magpie.contains(&format!(">{}</text></svg>", &today[5..])),
        "{magpie}"
    );
}
