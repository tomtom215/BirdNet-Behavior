//! `/species?rank=order&taxon=…` narrows the grids to one taxonomic rank.
//!
//! The pieces are tested where they live — the rank parsing, the predicate and
//! the chip row in `species_pages`, the taxonomy map in `state`, the label
//! column in `birdnet-core`. What none of those covers is that they are
//! *connected*: a version of `list_view` that builds the chips and then never
//! calls `retain` passes every one of them, and renders a page whose controls
//! do nothing.
//!
//! Observed failing against the pre-feature page, which had no `rank` or
//! `taxon` parameter at all: `axum`'s `Query<HomeParams>` ignored both, so the
//! filtered request returned the same three species as the unfiltered one and
//! `only_the_woodpeckers` failed on `Barred Owl`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::{AppState, Taxon};
use tower::ServiceExt as _;

/// Three species over two orders, so a filter that keeps everything and one
/// that keeps nothing both fail.
const SPECIES: [(&str, &str, &str, &str); 3] = [
    (
        "Dryobates villosus",
        "Hairy Woodpecker",
        "Piciformes",
        "Dryobates",
    ),
    (
        "Dryobates pubescens",
        "Downy Woodpecker",
        "Piciformes",
        "Dryobates",
    ),
    ("Strix varia", "Barred Owl", "Strigiformes", "Strix"),
];

fn state() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db"))
        .expect("state")
        .with_taxonomy(SPECIES.map(|(sci, _, order, genus)| {
            (
                sci,
                Taxon {
                    class: Some("Aves".to_owned()),
                    order: Some(order.to_owned()),
                    genus: Some(genus.to_owned()),
                },
            )
        }));
    state.with_db(|conn| {
        for (sci, com, _, _) in SPECIES {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                      File_Name, chunk_offset_secs)
                 VALUES ('2026-05-01', '06:00:00', ?1, ?2, 0.9, 0.7, 18, 1.25, 0.0, 'x.wav', 0)",
                rusqlite::params![sci, com],
            )
            .expect("insert");
        }
    });
    (dir, state)
}

async fn page(state: &AppState, uri: &str) -> String {
    let app = birdnet_web::server::build_router(state.clone());
    let res = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    assert_eq!(res.status(), StatusCode::OK, "GET {uri}");
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

#[tokio::test]
async fn a_chosen_order_narrows_the_list_and_the_grid_to_it() {
    let (_dir, state) = state();

    for view in ["list", "photos"] {
        let all = page(&state, &format!("/species?view={view}")).await;
        for (_, com, _, _) in SPECIES {
            assert!(
                all.contains(com),
                "{view}: every species shows without a filter, but {com} is missing"
            );
        }

        let owls = page(
            &state,
            &format!("/species?view={view}&rank=order&taxon=Strigiformes"),
        )
        .await;
        assert!(owls.contains("Barred Owl"), "{view}: the owl must survive");
        assert!(
            !owls.contains("Hairy Woodpecker") && !owls.contains("Downy Woodpecker"),
            "{view}: choosing Strigiformes must drop the woodpeckers"
        );

        let woodpeckers = page(
            &state,
            &format!("/species?view={view}&rank=order&taxon=Piciformes"),
        )
        .await;
        assert!(
            woodpeckers.contains("Hairy Woodpecker") && woodpeckers.contains("Downy Woodpecker"),
            "{view}: both woodpeckers must survive"
        );
        assert!(
            !woodpeckers.contains("Barred Owl"),
            "{view}: choosing Piciformes must drop the owl"
        );
    }
}

/// The genus is reachable the same way, from the detail panel's link.
#[tokio::test]
async fn a_chosen_genus_narrows_the_list_too() {
    let (_dir, state) = state();
    let html = page(&state, "/species?view=list&rank=genus&taxon=Dryobates").await;
    assert!(html.contains("Hairy Woodpecker") && html.contains("Downy Woodpecker"));
    assert!(!html.contains("Barred Owl"), "{html}");
}

/// A rank the page does not know must not empty the station's species list —
/// the counterpart that stops "filter everything out" passing the gates above.
#[tokio::test]
async fn an_unknown_rank_shows_every_species_rather_than_none() {
    let (_dir, state) = state();
    let html = page(&state, "/species?view=list&rank=family&taxon=Picidae").await;
    for (_, com, _, _) in SPECIES {
        assert!(html.contains(com), "{com} must still be listed");
    }
}

/// A station with no label file has no taxonomy, and its species pages are
/// exactly as they were: no chip row, and nothing filtered away.
#[tokio::test]
async fn a_station_without_a_label_file_is_unaffected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        for (sci, com, _, _) in SPECIES {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                      File_Name, chunk_offset_secs)
                 VALUES ('2026-05-01', '06:00:00', ?1, ?2, 0.9, 0.7, 18, 1.25, 0.0, 'x.wav', 0)",
                rusqlite::params![sci, com],
            )
            .expect("insert");
        }
    });

    let html = page(&state, "/species?view=list").await;
    for (_, com, _, _) in SPECIES {
        assert!(html.contains(com));
    }
    assert!(
        !html.contains("Taxonomic order"),
        "no taxonomy means no chip row: {html}"
    );
}
