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

/// The label the chart's tooltip gives a day: `Wed 21 Oct`, weekday first and
/// the day of the month without a leading zero.
///
/// Asked of SQLite rather than computed here, so the expectation comes from the
/// same calendar the page's own query used, `n` days before today.
fn tooltip_day(conn: &rusqlite::Connection, days_ago: u32) -> String {
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (weekday, day, month): (i64, i64, i64) = conn
        .query_row(
            "SELECT CAST(strftime('%w', d) AS INTEGER), CAST(strftime('%d', d) AS INTEGER), \
                    CAST(strftime('%m', d) AS INTEGER) \
             FROM (SELECT DATE('now', 'localtime', '-' || ?1 || ' days') AS d)",
            [days_ago],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("calendar day");
    format!(
        "{} {day} {}",
        WEEKDAYS[usize::try_from(weekday).expect("weekday")],
        MONTHS[usize::try_from(month - 1).expect("month")]
    )
}

/// The trend is the last 14 calendar days. A species heard only in October has
/// none of them, and says so in a sentence rather than fourteen empty bars;
/// one heard today gets its chart, today's bar last.
///
/// The chart is the chart kit's: one `ck-hit` rectangle per day carries that
/// day's tooltip, so the days a reader can reach are read from those, in order,
/// and the bar actually drawn is the one at the last index.
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
    // Fourteen calendar days, oldest first, ending today.
    let expected: Vec<String> = (0..14).rev().map(|n| tooltip_day(&conn, n)).collect();
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
    assert!(!blackbird.contains("ck-hit"), "{blackbird}");

    let page = get("Eurasian%20Magpie").await;
    // The chart is drawn twice, a wide drawing for a card and a narrow one for a phone
    // (`.viz-chart--narrow`); CSS shows one. Each must tell the same fourteen days.
    let (wide, narrow) = page
        .split_once("<div class=\"viz-chart ck viz-chart--narrow\">")
        .unwrap_or_else(|| panic!("no narrow drawing: {page}"));
    for magpie in [wide, narrow] {
        // One reachable day per calendar day: 14 of them, in order, each at its own
        // index, each saying the date it is.
        let tips: Vec<(usize, &str)> = magpie
            .split("<rect class=\"ck-hit\"")
            .skip(1)
            .map(|rect| {
                let index = rect
                    .split("data-ck-i=\"")
                    .nth(1)
                    .and_then(|r| r.split('"').next())
                    .and_then(|n| n.parse::<usize>().ok())
                    .expect("each day carries its index");
                let tip = rect
                    .split("data-tip=\"")
                    .nth(1)
                    .and_then(|r| r.split('"').next())
                    .expect("each day carries its tooltip");
                (index, tip)
            })
            .collect();
        assert_eq!(tips.len(), 14, "{magpie}");
        for (position, (index, tip)) in tips.iter().enumerate() {
            assert_eq!(*index, position, "days are out of order: {tips:?}");
            assert!(
                tip.starts_with(&format!("{}&#10;", expected[position])),
                "day {position} should be {:?}, the tooltip says {tip:?}",
                expected[position]
            );
            // The magpie was heard once, today, and on no other day.
            let heard = u8::from(position == 13);
            assert!(
                tip.contains(&format!("Detections&#9;{heard}&#10;")),
                "day {position} should show {heard} detection(s): {tip:?}"
            );
        }
        // Today's bar is the one drawn, and the last: the only bar is at index 13.
        assert_eq!(magpie.matches("class=\"ck-grow\"").count(), 1, "{magpie}");
        assert!(
            magpie.contains("data-ck-i=\"13\" data-style=\"--i:13\"/>"),
            "{magpie}"
        );
        assert!(magpie.contains("data-ck-start=\"13\""), "{magpie}");
    }
}
