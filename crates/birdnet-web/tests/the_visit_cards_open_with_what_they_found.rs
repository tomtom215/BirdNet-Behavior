//! `GET /pages/analytics-sessions`, `-retention` and `-next`: each card opens with one sentence of
//! what its figure found, written from the same rows the figure draws.
//!
//! The sentences are pure functions with their own tests; what those cannot see is the wiring: that
//! the handler counts the right rows, names the right scope, puts the sentence first and keeps it
//! to what the engine returned. So this runs the real engine over a station whose answers can be
//! worked out by hand.
//!
//! Seven visit days, exactly a week apart (so they are seven distinct calendar weeks whatever day
//! it is today): every day a Robin pair at 06:00 and 06:01 and a lone Blackbird at 06:30; a Wren
//! trio (06:20 to 06:22) on four of them; a Jay trio (06:50 to 06:52) on two; and one day, the
//! twenty-second, a Finch five-in-a-row from 08:00. That gives:
//!
//! - bursts: Robin 7 (60 s), Wren 4 (120 s), Jay 2 (120 s), Finch 1 (240 s), so 14 bursts among 21
//!   sessions (the seven Blackbirds are the other sessions), Robin with the most, Finch's the
//!   longest;
//! - return rates: Robin and Blackbird in 7 of 7 weeks (residents), Wren in 4 (a regular), Jay in
//!   2 of 7 = 29% (a migrant), Finch in one day only (a rarity);
//! - neighbours of the latest bird, the Blackbird of the first day: before it a Wren on four days
//!   and a Robin on three; after it only a Jay, on two days, because the other five Blackbirds are
//!   followed by nothing within the hour. The Jay is therefore 2 of the 2 sessions that had a bird
//!   after (100%) and not 2 of 7: the engine's probability is of sessions with a neighbour.

// Needs the DuckDB analytics store: without the feature `AppState` has no analytics methods.
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
        let add = |ago: u32, time: &str, sci: &str, com: &str| {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, detected_at_utc)
                 SELECT d, ?2, ?3, ?4, 0.9, 'clip.wav', CAST(strftime('%s', d || ' ' || ?2) AS INTEGER)
                 FROM (SELECT date('now', 'localtime', '-' || ?1 || ' days') AS d)",
                rusqlite::params![ago, time, sci, com],
            )
            .expect("seed");
        };
        for ago in [1, 8, 15, 22, 29, 36, 43] {
            add(ago, "06:00:00", "Erithacus rubecula", "Robin");
            add(ago, "06:01:00", "Erithacus rubecula", "Robin");
            add(ago, "06:30:00", "Turdus merula", "Blackbird");
        }
        for ago in [1, 15, 29, 43] {
            for t in ["06:20:00", "06:21:00", "06:22:00"] {
                add(ago, t, "Troglodytes troglodytes", "Wren");
            }
        }
        for ago in [8, 43] {
            for t in ["06:50:00", "06:51:00", "06:52:00"] {
                add(ago, t, "Cyanocitta cristata", "Jay");
            }
        }
        for t in ["08:00:00", "08:01:00", "08:02:00", "08:03:00", "08:04:00"] {
            add(22, t, "Fringilla coelebs", "Finch");
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

/// The sentence is the first thing in the card, before the figure it describes.
fn first_sentence(html: &str, figure: &str) -> String {
    let at = html.find("<p class=\"fig-take\">").expect("a sentence");
    let end = html[at..].find("</p>").expect("its end") + at;
    let fig = html.find(figure).expect("the figure");
    assert!(at < fig, "the sentence leads the figure: {at} < {fig}");
    html[at + "<p class=\"fig-take\">".len()..end].to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_bursts_card_opens_with_its_count_its_busiest_and_its_longest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let html = card(station(dir.path()), "/pages/analytics-sessions").await;
    assert_eq!(
        first_sentence(&html, "bt-fig"),
        "14 bursts among all 21 sessions: Robin had the most (7), and the longest, 4m 0s, was Finch\u{2019}s."
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_ledger_opens_with_how_many_species_are_each_kind_of_visitor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let html = card(station(dir.path()), "/pages/analytics-retention").await;
    assert_eq!(
        first_sentence(&html, "rt-scroll"),
        "Of 5 species heard at least 5 times: 2 residents, 1 regular, 1 migrant and 1 rarity."
    );
    // The sentence agrees with the ledger under it: one group row per kind, the species in each.
    for (kind, n) in [
        ("Resident", 2),
        ("Regular", 1),
        ("Migrant", 1),
        ("Rarity", 1),
    ] {
        let at = html
            .find(&format!(">{kind}</span> <span class=\"bnb-meta\">{n} "))
            .unwrap_or_else(|| panic!("the ledger has {n} under {kind}"));
        assert!(at > 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_neighbour_card_says_what_its_share_is_of() {
    let dir = tempfile::tempdir().expect("tempdir");
    let html = card(station(dir.path()), "/pages/analytics-next").await;
    assert_eq!(
        first_sentence(&html, "fl-fig"),
        "Right after Blackbird, the next bird is most often Jay (2 sessions, 100% of those with a bird after); right before it, Wren (4 sessions, 57% of those with a bird before)."
    );
    // The table under it agrees: Jay 100% on two sessions, Wren 57% on four.
    assert!(
        html.contains("<td>Jay</td><td><span class=\"bnb-pill moss\">100%</span></td><td>2</td>")
    );
    assert!(
        html.contains("<td>Wren</td><td><span class=\"bnb-pill moss\">57%</span></td><td>4</td>")
    );
}
