//! A station that has heard nothing says so, and does not blame a filter.
//!
//! The Species list and its photo gallery answered an empty result with "No
//! species match this filter yet." — including on a station that had just been
//! installed, where no filter had been chosen and there was nothing to match.
//! `empty_states::no_species` ("No species heard yet. First detections usually
//! take a few minutes after the mic comes online", with a link to the mic's
//! status) existed for exactly that moment and was used only by the dashboard.
//! The visual sweep never showed it, because its fixture has detections.
//!
//! Three outcomes have to stay three: nothing heard (say so), a search that
//! matched nothing (name the search), a filter that matched nothing on a
//! station that has heard birds (say the filter did).

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station(with_a_detection: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    if with_a_detection {
        // Long ago, so "this week" is empty and the station is not.
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-01-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'b.wav')",
            [],
        )
        .expect("seed");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn page(with_a_detection: bool, uri: &str) -> String {
    let res = birdnet_web::server::build_router(station(with_a_detection))
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    assert!(res.status().is_success(), "{uri}: {}", res.status());
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn a_station_that_has_heard_nothing_says_so_in_the_list_and_the_gallery() {
    for uri in ["/species", "/species?view=photos"] {
        let html = page(false, uri).await;
        assert!(html.contains("No species heard yet."), "{uri}: {html}");
        assert!(
            !html.contains("match this filter"),
            "{uri} blames a filter on a station with no detections"
        );
    }
}

#[tokio::test]
async fn a_search_that_matches_nothing_names_the_search() {
    for uri in ["/species?q=zzzz", "/species?view=photos&q=zzzz"] {
        // On a station with no birds and on one with birds: the words are the
        // search's either way.
        for with_a_detection in [false, true] {
            let html = page(with_a_detection, uri).await;
            assert!(html.contains("No species match “zzzz”."), "{uri}: {html}");
            assert!(
                !html.contains("No species heard yet."),
                "{uri} calls a search that missed an empty station"
            );
        }
    }
}

#[tokio::test]
async fn a_filter_that_matches_nothing_on_a_station_with_birds_says_the_filter_did() {
    for uri in ["/species?filter=week", "/species?view=photos&filter=week"] {
        let html = page(true, uri).await;
        assert!(
            html.contains("No species match this filter yet."),
            "{uri}: {html}"
        );
        assert!(
            !html.contains("No species heard yet."),
            "{uri} says the station has heard nothing, which is not true of it"
        );
    }
}
