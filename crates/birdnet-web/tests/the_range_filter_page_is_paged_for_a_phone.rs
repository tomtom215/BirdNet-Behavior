//! The list of birds the station admits is a page of 25, and the next 25 arrive
//! in place.
//!
//! "Why isn't a bird heard here?" lists the species the range filter admits this
//! week beneath its answer. It listed 100 before "Show the next 100": one column
//! of two-line rows on a phone, about 4,900 px before the first control, for a
//! list nobody reads end to end (a reader looking for one bird has the lookup
//! above it).
//!
//! The gate is on what the browser is sent, because that is what sets the height
//! of the page: the list holds a page of rows and no more; "Show the next" asks
//! htmx for the following rows alone and appends them to the list; and it is still
//! a plain link, so without script it reaches the same rows as a page of their own.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_core::inference::species_filter::OccurrenceScore;
use birdnet_web::occurrence::{FixedOccurrence, OccurrenceReport};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Rows per page of the list. The page's constant is private; this is what the
/// reader is promised, and a change to it is a change to this file.
const PAGE: usize = 25;

/// Admitted species the station is made to hold: five pages and a half.
const ADMITTED: usize = 137;

fn station() -> (tempfile::TempDir, AppState) {
    let mut species: Vec<OccurrenceScore> = (1..=ADMITTED)
        .map(|n| OccurrenceScore {
            scientific_name: format!("Fixturus avis{n:03}"),
            common_name: format!("Fixture warbler {n:03}"),
            // Most likely first, as the real filter orders them.
            probability: 0.9 - f32::from(u16::try_from(n).expect("small")) * 0.005,
            admitted: true,
        })
        .collect();
    species.push(OccurrenceScore {
        scientific_name: "Spiza americana".to_owned(),
        common_name: "Dickcissel".to_owned(),
        probability: 0.0004,
        admitted: false,
    });
    let source = Arc::new(FixedOccurrence::answering(OccurrenceReport {
        latitude: 52.52,
        longitude: 13.405,
        week: 1,
        threshold: 0.03,
        species,
    }));
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.set_occurrence_source(source);
    (dir, state)
}

async fn get(state: &AppState, uri: &str, htmx: bool) -> (StatusCode, String) {
    let mut req = Request::builder().uri(uri);
    if htmx {
        req = req.header("hx-request", "true");
    }
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Rows in the admitted list: one `<li>` per species.
fn rows(body: &str) -> usize {
    body.matches(r#"<li><span class="occ-li-name">"#).count()
}

#[tokio::test]
async fn the_list_is_one_page_of_rows_and_not_a_hundred() {
    let (_dir, state) = station();
    let (status, page) = get(&state, "/species/occurrence?week=22", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        rows(&page),
        PAGE,
        "the list should hold one page of {PAGE} rows"
    );
    // The first page is the most likely birds, and the 26th is not on it.
    assert!(page.contains("Fixture warbler 001"), "{page}");
    assert!(page.contains("Fixture warbler 025"), "{page}");
    assert!(!page.contains("Fixture warbler 026"), "{page}");
    // The count says how many there are, so a short list does not read as a short year.
    assert!(
        page.contains(&format!("{ADMITTED} of the {} species", ADMITTED + 1)),
        "{page}"
    );
    // The control names the size of the step, and carries both ways to take it.
    assert!(page.contains(&format!("Show the next {PAGE}")), "{page}");
    assert!(page.contains(r#"hx-target="#), "{page}");
    assert!(page.contains("offset=25&amp;more=1"), "{page}");
    assert!(page.contains(r#"id="occ-list""#), "{page}");
}

#[tokio::test]
async fn the_next_rows_arrive_alone_for_htmx_to_append() {
    let (_dir, state) = station();
    let (status, part) = get(&state, "/species/occurrence?week=22&offset=25&more=1", true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows(&part), PAGE, "{part}");
    assert!(part.contains("Fixture warbler 026"), "{part}");
    assert!(part.contains("Fixture warbler 050"), "{part}");
    assert!(!part.contains("Fixture warbler 025"), "{part}");
    // Rows alone: no document, no heading, no second copy of the form.
    assert!(!part.contains("<html") && !part.contains("<h1"), "{part}");
    // The control moves on, out of band, and the count is announced.
    assert!(
        part.contains(r#"id="occ-more" hx-swap-oob="true""#),
        "{part}"
    );
    assert!(part.contains("offset=50&amp;more=1"), "{part}");
    assert!(
        part.contains(r#"hx-swap-oob="innerHTML:#occ-status""#),
        "{part}"
    );
    assert!(part.contains("Showing 50 of 137"), "{part}");
}

#[tokio::test]
async fn the_last_page_has_no_next_control() {
    let (_dir, state) = station();
    let last = ADMITTED / PAGE * PAGE; // 125: the page with 12 rows
    let (_, part) = get(
        &state,
        &format!("/species/occurrence?week=22&offset={last}&more=1"),
        true,
    )
    .await;
    assert_eq!(rows(&part), ADMITTED - last, "{part}");
    assert!(
        !part.contains("Show the next"),
        "a control that leads nowhere: {part}"
    );
    // The control's slot is emptied rather than left holding the old link.
    assert!(
        part.contains(r#"id="occ-more" hx-swap-oob="true""#),
        "{part}"
    );

    // And the page itself, at that offset, offers nothing further either.
    let (_, page) = get(
        &state,
        &format!("/species/occurrence?week=22&offset={last}"),
        false,
    )
    .await;
    assert!(!page.contains("Show the next"), "{page}");
}

/// Without script the link is a link: the same rows, as a page.
#[tokio::test]
async fn without_script_the_next_page_is_a_page() {
    let (_dir, state) = station();
    // `more=1` without htmx asking is not a fragment request.
    let (_, page) = get(
        &state,
        "/species/occurrence?week=22&offset=25&more=1",
        false,
    )
    .await;
    assert!(page.contains("<h1"), "a whole page: {page}");
    assert_eq!(rows(&page), PAGE, "{page}");
    assert!(page.contains("Fixture warbler 026"), "{page}");
    assert!(!page.contains("Fixture warbler 025"), "{page}");
}
