//! Every play button on Today says which clip it plays (UIC-12, WCAG 2.4.6).
//!
//! The live feed, the best-recordings rail and the full-day list each wrote
//! `aria-label="Play clip"` on every button, so a screen-reader user tabbing
//! down the feed heard "Play clip" thirty times and could not tell one from
//! the next. Each is now "Play {the name shown}, {the time shown}", which is
//! distinct for two detections of one species, and escaped like the name it
//! repeats.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// The `aria-label` of every play button (`data-play-src`) in `html`.
fn play_labels(html: &str) -> Vec<String> {
    html.split("<button")
        .skip(1)
        .filter_map(|b| {
            let tag = &b[..b.find('>')?];
            if !tag.contains("data-play-src") {
                return None;
            }
            let at = tag.find("aria-label=\"")? + "aria-label=\"".len();
            Some(tag[at..at + tag[at..].find('"')?].to_owned())
        })
        .collect()
}

async fn get(state: &AppState, uri: &str) -> String {
    let token =
        birdnet_web::session::issue_token(&birdnet_web::session::generate_session_id(), 60_000);
    let res = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(
                    axum::http::header::COOKIE,
                    format!("{}={token}", birdnet_web::session::COOKIE_NAME),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{uri}");
    String::from_utf8(
        axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn todays_play_buttons_each_have_their_own_name() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        // Today is what these three surfaces show; nothing below depends on
        // which day it is.
        let today: String = conn
            .query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))
            .unwrap();
        for (time, sci, com, file, conf) in [
            ("06:10:00", "Erithacus rubecula", "Robin", "a.wav", 0.91),
            ("07:20:00", "Erithacus rubecula", "Robin", "b.wav", 0.92),
            (
                "08:30:00",
                "Turdus merula",
                "Blackbird & <Co>",
                "c.wav",
                0.93,
            ),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![today, time, sci, com, conf, file],
            )
            .unwrap();
        }
    });

    for uri in [
        "/pages/detections",
        "/pages/best-detections",
        "/pages/today-list",
    ] {
        let html = get(&state, uri).await;
        let labels = play_labels(&html);
        assert_eq!(
            labels.len(),
            3,
            "{uri}: precondition, one play button per detection with a clip: {labels:?}"
        );
        let mut distinct = labels.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            labels.len(),
            "{uri}: two play buttons share an accessible name: {labels:?}"
        );
        for (label, name) in labels
            .iter()
            .zip(["Robin", "Robin", "Blackbird &amp; &lt;Co&gt;"])
        {
            assert!(
                label.starts_with("Play ") && labels.iter().any(|l| l.contains(name)),
                "{uri}: a play button must name what it plays ({name}): {label:?}"
            );
        }
        assert!(
            labels.iter().all(|l| !l.contains("<Co>")),
            "{uri}: the name in a label is escaped: {labels:?}"
        );
    }
}
