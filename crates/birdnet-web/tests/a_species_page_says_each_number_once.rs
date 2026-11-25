//! A species page says each of its numbers once, and its links go where their words say (UIR-24).
//!
//! The page drew "1,063 detections · First heard · Last heard · avg 84 %" as pills under the
//! name, and the same four numbers as tiles under them. The tiles stay; the pill row keeps only
//! what the tiles do not say, the range filter's "held back" link. The hour-of-day card was
//! titled "Circadian", a word no birder uses for it. And "When it's active" and "Who it sings
//! with" went to the general Patterns tabs, which take no species (`TabParam` reads `tab` and
//! nothing else), so a reader who asked about this bird landed on every bird: they go to this
//! page's own rhythm and companions cards now.

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

async fn get(uri: &str) -> String {
    let res = birdnet_web::server::build_router(station())
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn the_status_row_does_not_repeat_the_tiles() {
    let pills = get("/pages/species-status?name=Eurasian%20Blackbird").await;
    for repeated in [
        "detections</span>",
        "First heard",
        "Last heard",
        "confidence</span>",
    ] {
        assert!(
            !pills.contains(repeated),
            "the pill row repeats a tile ({repeated}): {pills}"
        );
    }
    // Counterpart: the tiles still carry all four.
    let tiles = get("/pages/species-summary?name=Eurasian%20Blackbird").await;
    for label in ["Detections", "Avg Confidence", "First Seen", "Last Seen"] {
        assert!(
            tiles.contains(&format!(r#"<div class="label">{label}</div>"#)),
            "{label} tile missing: {tiles}"
        );
    }
}

#[tokio::test]
async fn the_hour_card_is_called_daily_rhythm_and_the_links_stay_on_this_species() {
    let page = get("/species/detail?name=Eurasian%20Blackbird").await;
    assert!(!page.contains("Circadian"), "the card is still 'Circadian'");
    assert!(page.contains(">Daily rhythm<"), "no 'Daily rhythm' card");
    let actions = page
        .split(r#"<div class="sd-actions">"#)
        .nth(1)
        .and_then(|s| s.split("</div>").next())
        .unwrap_or_default();
    assert!(!actions.is_empty(), "no action row on the page: {page}");
    assert!(
        !actions.contains(r#"href="/patterns"#),
        "an action promises this species and opens every species: {actions}"
    );
    // Each in-page link names a card that is on the page.
    let mut anchors = 0;
    for part in actions.split(r##"href="#"##).skip(1) {
        let id = part.split('"').next().unwrap_or_default();
        anchors += 1;
        assert!(
            page.contains(&format!(r#"id="{id}""#)),
            "the link to #{id} names no element on the page"
        );
    }
    assert_eq!(anchors, 2, "the rhythm and companions links: {actions}");
}
