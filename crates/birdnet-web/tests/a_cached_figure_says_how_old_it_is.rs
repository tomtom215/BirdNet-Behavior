//! A figure served from the analytics cache says how old it is (UIC-23).
//!
//! The analytic fragments are kept for ten minutes (`analytics_cache::DEFAULT_TTL`), and a
//! fragment's own "now" marks (the heat grid's current hour) are as old as that. Nothing said
//! so: a reader at 14:39 was shown a figure worked out at 14:30 as the present. Each cached
//! figure on Patterns now ends with "As of HH:MM", stamped when the cache stores it, so a later
//! reader is told the time it was worked out and not the time of its own request (that half is
//! held with a clock the test sets, in `analytics_cache`'s own tests).
//!
//! Seen failing before the stamp existed: "no stamp in /pages/hourly-totals?days=7".

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

async fn get(state: &AppState, uri: &str) -> String {
    let res = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
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

/// The `HH:MM` the stamp names, if there is one.
fn stamp(html: &str) -> Option<String> {
    let line = html.split(r#"<p class="bnb-meta ck-asof">"#).nth(1)?;
    let at = line.split(r#"data-clk=""#).nth(1)?;
    Some(at.split('"').next()?.to_owned())
}

#[tokio::test]
async fn each_cached_figure_on_patterns_says_when_it_was_worked_out() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        for (back, time) in [
            (0, "06:10:00"),
            (1, "06:40:00"),
            (2, "18:05:00"),
            (3, "07:00:00"),
        ] {
            for (com, sci) in [
                ("American Robin", "Turdus migratorius"),
                ("Blue Jay", "Cyanocitta cristata"),
            ] {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                     VALUES (DATE('now', 'localtime', ?1), ?2, ?3, ?4, 0.9, '')",
                    rusqlite::params![format!("-{back} days"), time, sci, com],
                )
                .unwrap();
            }
        }
    });
    for uri in [
        "/pages/hourly-totals?days=7",
        "/pages/heatmap-grid?days=7",
        "/pages/activity-streamgraph?days=7",
        "/pages/acoustic-network?days=7",
    ] {
        let first = get(&state, uri).await;
        let at = stamp(&first).unwrap_or_else(|| panic!("no stamp in {uri}: {first}"));
        assert_eq!(at.len(), 5, "{uri}: {at}");
        // Counterpart: the second request is served from the cache, and is told the same time,
        // the one the figure was worked out at.
        let second = get(&state, uri).await;
        assert_eq!(stamp(&second).as_deref(), Some(at.as_str()), "{uri}");
        assert_eq!(
            first, second,
            "{uri}: the second reader is served the stored copy"
        );
    }
}
