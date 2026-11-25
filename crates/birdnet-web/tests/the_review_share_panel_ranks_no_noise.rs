//! The Data tab's reviewer-verdict panel prints what the verdicts can support
//! and no more (ACC-07).
//!
//! It used to set the classifier's mean confidence over every detection beside
//! the share of reviewed detections confirmed, over as few as three verdicts,
//! rank species by the gap, and call the top of the list "the most likely
//! overconfident false positives". A mean of uncalibrated scores is not
//! comparable to a confirmation rate; with three verdicts, 2 of 3 confirmed
//! has a 95 % Wilson interval of 21 % to 94 %, so the ranking mostly ranked
//! noise; and reviews are not a random sample, because people review what
//! looks wrong. These read the panel as a reader gets it, from the router.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_db::sqlite::{DetectionKey, ReviewStatus, set_detection_review};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// `(scientific, common, confirmed, rejected)`; each verdict is on its own
/// detection, and every detection has one.
type Species = (&'static str, &'static str, u32, u32);

const BLACKBIRD: Species = ("Turdus merula", "Eurasian Blackbird", 16, 8);
const OWL: Species = ("Strix aluco", "Tawny Owl", 2, 1);
const TIT: Species = ("Parus major", "Great Tit", 10, 10);
const ROBIN: Species = ("Erithacus rubecula", "European Robin", 120, 80);

fn station(species: &[Species]) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for &(sci, com, confirmed, rejected) in species {
        for i in 0..confirmed + rejected {
            let time = format!("06:{:02}:{:02}", i / 60, i % 60);
            let clip = format!("{sci}-{i}.wav");
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES ('2026-09-08', ?1, ?2, ?3, 0.9, ?4)",
                rusqlite::params![time, sci, com, clip],
            )
            .expect("seed");
            let status = if i < confirmed {
                ReviewStatus::Confirmed
            } else {
                ReviewStatus::Rejected
            };
            let key = DetectionKey {
                date: "2026-09-08",
                time: &time,
                sci_name: sci,
                file_name: Some(&clip),
            };
            assert!(set_detection_review(&conn, &key, com, status, None).expect("review"));
        }
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn data_tab(state: &AppState) -> String {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/station/data")
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    String::from_utf8_lossy(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned()
}

/// The `<section>` that names the blackbird: the panel. No other panel on the
/// tab lists a species with confident detections.
fn panel(page: &str) -> &str {
    let at = page
        .find("Eurasian Blackbird")
        .unwrap_or_else(|| panic!("the panel names no species: {page}"));
    let start = page[..at].rfind("<section").expect("in a section");
    let end = at + page[at..].find("</section>").expect("closed");
    &page[start..end]
}

/// The ranked list: the `<ol>` of the panel, or empty when it has none.
fn ranked(panel: &str) -> &str {
    panel.find("<ol").map_or("", |start| {
        let end = start + panel[start..].find("</ol>").expect("closed");
        &panel[start..end]
    })
}

/// The defect: a species with three verdicts is ranked, no interval is
/// printed, the share is not said to be of reviewed detections, and the gap is
/// called overconfidence.
#[tokio::test]
async fn the_panel_prints_an_interval_and_does_not_rank_three_verdicts() {
    let page = data_tab(&station(&[BLACKBIRD, OWL])).await;
    let panel = panel(&page);

    // 16 of 24: 67 %, Wilson 95 % 46.7 % to 82.0 %.
    assert!(panel.contains("16 of 24"), "{panel}");
    assert!(panel.contains("47–82%"), "no interval printed: {panel}");
    assert!(
        panel.contains("not a random sample"),
        "the share must be said to be of reviewed detections: {panel}"
    );

    // The owl, 2 of 3, is shown with its interval and not ranked.
    assert!(ranked(panel).contains("Eurasian Blackbird"), "{panel}");
    assert!(
        !ranked(panel).contains("Tawny Owl"),
        "three verdicts are ranked: {panel}"
    );
    assert!(panel.contains("Tawny Owl"), "{panel}");
    assert!(panel.contains("21–94%"), "{panel}");

    assert!(
        !page.contains("overconfident false positives"),
        "the claim is still printed"
    );
}

/// The counterpart, and the order: with enough verdicts a species is ranked,
/// and the one whose interval reaches least high comes first. 120 of 200
/// (60 %) tops out at 67 %, 10 of 20 (50 %) at 70 % and 16 of 24 at 82 %, so
/// the robin leads although the tit's share is lower: ranking by the share
/// itself would put the thinner evidence first, which is the noise this
/// panel used to rank.
#[tokio::test]
async fn enough_verdicts_are_ranked_lowest_interval_first() {
    let page = data_tab(&station(&[BLACKBIRD, TIT, ROBIN])).await;
    let list = ranked(panel(&page));
    let robin = list.find("European Robin").expect("ranked");
    let tit = list.find("Great Tit").expect("ranked");
    let blackbird = list.find("Eurasian Blackbird").expect("ranked");
    assert!(robin < tit && tit < blackbird, "{list}");
    assert!(list.contains("30–70%"), "{list}");
    assert!(list.contains("53–67%"), "{list}");
}
