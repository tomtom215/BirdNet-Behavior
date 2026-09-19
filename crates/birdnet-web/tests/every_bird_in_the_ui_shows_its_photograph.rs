//! The avatar beside every bird is the bird, not a four-letter code.
//!
//! # What this guards
//!
//! `atoms::avatar` is called from fourteen places across seven modules, and
//! until 0.16.0 every one of them rendered the same thing: a coloured circle
//! with a banding code in it (`Eurasian Blackbird` → `EUBL`). The station had
//! photographs the whole time — `/api/v2/species/image/{sci}/file` has served
//! them to the species gallery and the species-detail hero since the image
//! cache was added — but nothing else in the app asked for one, so the live
//! feed, the Today log, Recordings, History, the weekly report, Year in
//! Review, the life list and the species tables all showed lettering.
//!
//! This walks the real router and checks, on every route that renders an
//! avatar, that each `bnb-avatar` chip contains a `bnb-avatar-img` pointing at
//! that species' image URL.
//!
//! # Why it counts as well as matches
//!
//! A route that rendered *no* avatars would satisfy "every avatar has a
//! photo" vacuously, and several of these routes render nothing at all unless
//! the fixture happens to put a row where they look (a date, a week, the
//! current year). So each route must produce at least one avatar, and the
//! failure names the route rather than reporting a total.
//!
//! # The counterpart
//!
//! An imported BirdNET-Pi database can carry a detection with no `Sci_Name`.
//! There is no image URL to build for such a row, so it must render the code
//! chip **and no `<img>` at all** — not an `<img>` pointing at
//! `/api/v2/species/image//file`, which could only ever 404. The counterpart
//! puts a nameless row and a named row on the same page and asserts the
//! difference, so "always emits an img" and "never emits an img" both fail.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A station with enough history that every avatar-bearing route has something
/// to draw: two species heard today and on two earlier days of the same week,
/// each with a clip file so Recordings is not empty.
///
/// `nameless` adds one row whose `Sci_Name` is blank, which is what the
/// counterpart needs and what every other test here must not see.
fn station(nameless: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");

    for (offset, sci, com) in [
        (0_i64, "Turdus merula", "Eurasian Blackbird"),
        (0, "Parus major", "Great Tit"),
        (1, "Turdus merula", "Eurasian Blackbird"),
        (2, "Parus major", "Great Tit"),
    ] {
        for i in 0..4 {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens,
                      Overlap, File_Name, chunk_offset_secs)
                 VALUES (date('now','localtime',?1), ?2, ?3, ?4, ?5, 0.7, 38, 1.25, 0.0, ?6, 0)",
                rusqlite::params![
                    format!("-{offset} day"),
                    format!("0{}:1{}:00", 5 + i, i),
                    sci,
                    com,
                    0.72 + f64::from(i) / 50.0,
                    format!("clip-{offset}-{i}-{}.wav", com.replace(' ', "_")),
                ],
            )
            .expect("seed detection");
        }
    }

    if nameless {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens,
                  Overlap, File_Name, chunk_offset_secs)
             VALUES (date('now','localtime'), '23:59:00', '', 'Unnamed Import',
                     0.91, 0.7, 38, 1.25, 0.0, 'orphan.wav', 0)",
            [],
        )
        .expect("seed a row with no scientific name");
    }

    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// Today's date as the database computes it, so the date-scoped routes below
/// are asked about the same day the rows were written on. Deriving it in Rust
/// instead would reintroduce the local-vs-UTC disagreement `unix_secs`'s doc
/// comment describes.
fn today(state: &AppState) -> String {
    state
        .with_db(|conn| {
            conn.query_row("SELECT date('now','localtime')", [], |r| {
                r.get::<_, String>(0)
            })
            .map_err(birdnet_db::sqlite::DbError::Sqlite)
        })
        .expect("today")
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The body of each `<span class="bnb-avatar…">…</span>` in `html`.
///
/// A chip's content is the four-character code plus, now, a void `<img>`, so
/// there is no nested `</span>` to confuse a scan this simple. Returning the
/// bodies rather than a count is what lets the caller say *which* chip is bare.
fn avatar_chips(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find(r#"<span class="bnb-avatar"#) {
        rest = &rest[start..];
        let Some(end) = rest.find("</span>") else {
            break;
        };
        out.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    out
}

/// Every route that renders an avatar, with the module and line the chip comes
/// from, so a failure points at the code rather than at the page.
fn routes(day: &str) -> Vec<(&'static str, String)> {
    vec![
        // The Today home is an HTMX shell; the cards arrive in this partial.
        (
            "today.rs render_detection_card",
            "/pages/today-list".to_string(),
        ),
        ("partials.rs live feed", "/pages/detections".to_string()),
        (
            "partials.rs best detections",
            "/pages/best-detections".to_string(),
        ),
        ("partials.rs top species", "/pages/top-species".to_string()),
        (
            "partials.rs species list",
            "/pages/species-list".to_string(),
        ),
        (
            "history.rs day panel",
            format!("/pages/history-chart?date={day}"),
        ),
        ("history.rs open day", format!("/reports/day?date={day}")),
        (
            "weekly_report.rs top + first-ever",
            "/pages/weekly-content".to_string(),
        ),
        (
            "year_in_review.rs leaderboard",
            "/reports?tab=year".to_string(),
        ),
        ("species_pages.rs table", "/species".to_string()),
        (
            "species_pages.rs life list",
            "/species?view=lifelist".to_string(),
        ),
        ("recordings.rs clip row", "/recordings".to_string()),
    ]
}

#[tokio::test]
async fn every_avatar_carries_the_species_photograph() {
    let state = station(false);
    let day = today(&state);

    for (origin, uri) in routes(&day) {
        let (status, body) = get(&state, &uri).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{origin}: {uri} did not answer ({status})"
        );

        let chips = avatar_chips(&body);
        assert!(
            !chips.is_empty(),
            "{origin}: {uri} rendered no avatar at all, so it proves nothing. \
             The fixture no longer reaches this code path."
        );
        for chip in &chips {
            assert!(
                chip.contains("bnb-avatar-img"),
                "{origin}: {uri} rendered a bare code chip: {chip}"
            );
            assert!(
                chip.contains("/api/v2/species/image/"),
                "{origin}: {uri} rendered an image that is not the species photo: {chip}"
            );
        }
    }
}

/// The scientific name reaches the URL intact, space-encoded. Checked on the
/// live feed because that is the most-rendered avatar in the app.
#[tokio::test]
async fn the_photo_url_names_the_species_not_the_common_name() {
    let state = station(false);
    let (_, body) = get(&state, "/pages/detections").await;
    assert!(
        body.contains(r#"src="/api/v2/species/image/Turdus%20merula/file""#),
        "the feed must request the image by scientific name, space-encoded: {}",
        body.chars().take(400).collect::<String>()
    );
    assert!(
        !body.contains("/api/v2/species/image/Eurasian"),
        "the image endpoint is keyed by scientific name; a common name there \
         would 404 for every bird"
    );
}

/// The counterpart. A row with no scientific name has no image to ask for, so
/// it keeps the code chip and emits no `<img>` — while a named row on the same
/// page still gets one. Without both halves, "always emit" and "never emit"
/// would each pass one assertion.
#[tokio::test]
async fn a_detection_with_no_scientific_name_keeps_the_code_chip() {
    let state = station(true);
    let (status, body) = get(&state, "/pages/detections").await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        body.contains("Unnamed Import"),
        "precondition: the nameless row must be on the page, or this test \
         asserts nothing. Body: {}",
        body.chars().take(400).collect::<String>()
    );
    assert!(
        !body.contains("/api/v2/species/image//file"),
        "a blank scientific name must not become an image request that can \
         only 404"
    );

    let chips = avatar_chips(&body);
    let bare = chips
        .iter()
        .filter(|c| !c.contains("bnb-avatar-img"))
        .count();
    let photographed = chips.len() - bare;
    assert_eq!(
        bare,
        1,
        "exactly the nameless row should be bare; got {bare} bare of {} chips",
        chips.len()
    );
    assert!(
        photographed > 0,
        "the named rows on the same page must still carry their photograph"
    );
}
