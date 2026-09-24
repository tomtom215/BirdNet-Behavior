//! Today claims nothing about listening that it has not measured.
//!
//! The page served two green "live" pills as literal template text: the hero
//! pill row's placeholder read "recording" until `/pages/today-pills` answered
//! (and for good if that request failed or scripts were off), and the live
//! feed's header read "Listening" unconditionally. On a station whose
//! microphone had stopped, the feed said "Listening" a few centimetres from the
//! hero's own "not recording · the microphone has stopped".
//!
//! Capture state is known only to the partials that read the supervisor's
//! gauge. So the page as first served — before any of them has run — must not
//! show the animated live dot anywhere in its markup. Scripts are excluded:
//! the live-signal card's script builds that dot only once its socket is open.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// A station with history (so `/` is Today, not the first-run page) and no
/// audio source at all: nothing on it can be listening.
fn state() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES ('2026-05-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
        [],
    )
    .expect("seed");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// The document with every `<script>…</script>` removed.
fn markup_only(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        out.push_str(&rest[..open]);
        let close = rest[open..]
            .find("</script>")
            .map_or(rest.len(), |i| open + i + "</script>".len());
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

#[tokio::test]
async fn today_shows_no_live_dot_before_it_knows_the_station_is_listening() {
    let resp = build_router(state())
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    // Precondition: this is the Today page with its live feed, not a
    // first-run or onboarding page that happens to lack the pills.
    assert!(html.contains(r#"id="today-pills""#), "not the Today page");
    assert!(html.contains(r#"id="detections-table""#), "no live feed");

    let markup = markup_only(&html);
    let hits: Vec<&str> = markup
        .match_indices("bnb-dot live")
        .map(|(i, _)| &markup[i.saturating_sub(120)..(i + 60).min(markup.len())])
        .collect();
    assert!(
        hits.is_empty(),
        "an unmeasured live claim is served on a station with no microphone:\n{}",
        hits.join("\n---\n")
    );
}
