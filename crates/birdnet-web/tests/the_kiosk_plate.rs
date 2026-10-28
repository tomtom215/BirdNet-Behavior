//! The kiosk draws today's birds as a plate, and says so honestly when it cannot.
//!
//! `/kiosk` was three numbers and a list. It is now a sheet drawn from what the station heard
//! today (`dashboard/plate`), with the old display kept at `?mode=numbers`. What this holds:
//!
//! * the sheet is drawn from `detections_analytic`, most heard first, with its times marked
//!   for the reader's clock and no markup printed as text;
//! * a poll that already shows the current sheet is answered `204`, and one that does not
//!   (a different digest) gets the sheet: a wall display that redraws every thirty seconds
//!   re-engraves every picture, and one that never redraws shows yesterday for ever;
//! * a detection the operator rejected is not on the wall;
//! * before the first bird of the day, yesterday's sheet, and it says so;
//! * a database that cannot be read is an error on the wall, never an empty sheet: "Nothing
//!   heard yet" is a claim about the garden, and a broken view is not the garden;
//! * `mode=numbers` is still the old display, and its poll asks for it by name.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Today's local epoch day, the way the pages compute it.
fn today_epoch_day() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let local = i64::try_from(now).expect("fits") + birdnet_db::clock::local_utc_offset_secs();
    local.div_euclid(86_400)
}

fn date(offset_days: i64) -> String {
    birdnet_core::civil::date_string_from_days(today_epoch_day() + offset_days)
}

fn seed(conn: &rusqlite::Connection, day: &str, time: &str, sci: &str, com: &str, n: usize) {
    let base: usize = time[3..5].parse().expect("minutes");
    for i in 0..n {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs)
             VALUES (?1, ?2, ?3, ?4, 0.9, 0.7, 1, 1.25, 0.0, 'x.wav', 0)",
            rusqlite::params![
                day,
                format!("{}:{:02}:00", &time[..2], (base + i) % 60),
                sci,
                com
            ],
        )
        .expect("seed");
    }
}

fn state_with(seed_with: impl FnOnce(&rusqlite::Connection)) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    seed_with(&conn);
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// The `data-hash` the sheet carries.
fn hash_of(svg: &str) -> String {
    svg.split("data-hash=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("a data-hash")
        .to_owned()
}

/// The page measures the screen and sends `ar`, from a hidden control that starts at 1.6. A wall
/// display runs unattended for months, so a request that arrives with the value blank (a script
/// that failed, an address typed by hand) draws the default sheet and does not answer `400`. A
/// value that is not a number is still refused: turning it into "default" would be a silent guess.
#[tokio::test]
async fn a_blank_aspect_ratio_draws_the_default_sheet_but_a_garbled_one_is_refused() {
    let today = date(0);
    let state = state_with(|c| seed(c, &today, "06:10", "Turdus merula", "Eurasian Blackbird", 2));
    let (status, _, svg) = get(&state, "/pages/kiosk-content?ar=").await;
    assert_eq!(status, StatusCode::OK, "{svg}");
    assert!(svg.contains(r#"class="plate-svg""#), "{svg}");
    let (status, _, _) = get(&state, "/pages/kiosk-content?ar=wide").await;
    assert_ne!(
        status,
        StatusCode::OK,
        "a value that is not a number is refused"
    );
}

#[tokio::test]
async fn the_plate_is_drawn_from_what_was_heard_today_most_heard_first() {
    let today = date(0);
    let state = state_with(|c| {
        seed(
            c,
            &today,
            "06:10",
            "Erithacus rubecula",
            "European Robin",
            1,
        );
        seed(c, &today, "06:20", "Turdus merula", "Eurasian Blackbird", 3);
    });
    let (status, _, svg) = get(&state, "/pages/kiosk-content?mode=plate&ar=1.6").await;
    assert_eq!(status, StatusCode::OK);
    assert!(svg.contains(r#"class="plate-svg""#), "{svg}");
    // Both are on the sheet; the more-heard one is figure 1 and comes first in the document.
    let blackbird = svg.find(r#"data-sci="Turdus merula""#).expect("blackbird");
    let robin = svg.find(r#"data-sci="Erithacus rubecula""#).expect("robin");
    assert!(blackbird < robin, "most heard first");
    assert!(svg.contains("Heard this morning") || svg.contains("Heard this afternoon"));
    // Times are marked for the reader's clock, and no markup is printed as text.
    assert!(
        svg.contains(r#"<tspan data-clk="06:10">06:10</tspan>"#),
        "{svg}"
    );
    assert!(
        !svg.contains("&lt;span") && !svg.contains("&lt;tspan"),
        "markup printed as text: {svg}"
    );
}

#[tokio::test]
async fn a_poll_that_shows_the_current_sheet_gets_nothing_and_a_stale_one_gets_the_sheet() {
    let today = date(0);
    let state = state_with(|c| seed(c, &today, "07:00", "Turdus merula", "Eurasian Blackbird", 2));
    let (_, _, svg) = get(&state, "/pages/kiosk-content?ar=1.6").await;
    let hash = hash_of(&svg);

    let (status, _, body) = get(&state, &format!("/pages/kiosk-content?ar=1.6&h={hash}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "nothing changed: {body}");
    assert!(body.is_empty());

    // The counterpart: a digest that is not the current one is not mistaken for it.
    let (status, _, body) = get(&state, "/pages/kiosk-content?ar=1.6&h=0000000000000000").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("plate-svg"));

    // And a new bird changes the digest, so the wall redraws.
    {
        let (hash_before, state2) = (hash.clone(), state.clone());
        state2.with_db(|c| seed(c, &today, "07:05", "Parus major", "Great Tit", 1));
        let (status, _, body) = get(
            &state,
            &format!("/pages/kiosk-content?ar=1.6&h={hash_before}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "a new bird must redraw the sheet");
        assert_ne!(hash_of(&body), hash_before);
    }
}

#[tokio::test]
async fn a_bird_the_operator_rejected_is_not_on_the_wall() {
    let today = date(0);
    let state = state_with(|c| {
        seed(
            c,
            &today,
            "06:10",
            "Erithacus rubecula",
            "European Robin",
            2,
        );
        seed(c, &today, "06:20", "Turdus merula", "Eurasian Blackbird", 2);
        c.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Com_Name = 'European Robin'",
            [],
        )
        .expect("reject");
    });
    let (_, _, svg) = get(&state, "/pages/kiosk-content?ar=1.6").await;
    assert!(svg.contains(r#"data-sci="Turdus merula""#), "{svg}");
    assert!(
        !svg.contains("Erithacus rubecula"),
        "rejected, so not heard: {svg}"
    );
}

#[tokio::test]
async fn before_the_first_bird_of_the_day_the_wall_shows_yesterday_and_says_so() {
    let yesterday = date(-1);
    let state = state_with(|c| {
        seed(
            c,
            &yesterday,
            "18:00",
            "Turdus merula",
            "Eurasian Blackbird",
            4,
        );
    });
    let (_, _, svg) = get(&state, "/pages/kiosk-content?ar=1.6").await;
    assert!(svg.contains("Heard yesterday"), "{svg}");
    assert!(svg.contains(r#"data-sci="Turdus merula""#));

    // The counterpart: nothing yesterday either is an empty sheet, not an error.
    let empty = state_with(|_| {});
    let (status, _, svg) = get(&empty, "/pages/kiosk-content?ar=1.6").await;
    assert_eq!(status, StatusCode::OK);
    assert!(svg.contains("Nothing heard yet"), "{svg}");
    assert!(!svg.contains("couldn't load"));
}

/// A view that cannot be read is a fault on the wall, not an empty garden.
#[tokio::test]
async fn a_database_that_cannot_be_read_is_an_error_not_an_empty_sheet() {
    let today = date(0);
    let state = state_with(|c| {
        seed(c, &today, "06:10", "Turdus merula", "Eurasian Blackbird", 5);
        c.execute_batch("DROP VIEW detections_analytic")
            .expect("break the view");
    });
    let (status, _, body) = get(&state, "/pages/kiosk-content?ar=1.6").await;
    // 200, not 500: htmx discards a 5xx body, and the wall would go on showing the last good
    // sheet for ever.
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("couldn't load"), "{body}");
    assert!(
        !body.contains("Nothing heard yet"),
        "a fault is not an empty day: {body}"
    );
    assert!(!body.contains("plate-svg"));
}

#[tokio::test]
async fn the_numbers_face_is_still_there_and_polls_for_itself() {
    let today = date(0);
    let state = state_with(|c| seed(c, &today, "06:10", "Turdus merula", "Eurasian Blackbird", 2));
    let (status, _, page) = get(&state, "/kiosk?mode=numbers").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("/pages/kiosk-content?mode=numbers"),
        "the numbers page must ask for the numbers, or the poll would swap the plate into it"
    );
    let (_, _, body) = get(&state, "/pages/kiosk-content?mode=numbers").await;
    assert!(body.contains(r#"class="stats""#), "{body}");
    assert!(!body.contains("plate-svg"));
}

#[tokio::test]
async fn the_kiosk_page_is_the_plate_by_default_and_follows_the_sun_or_the_override() {
    let state = state_with(|_| {});
    let (status, _, page) = get(&state, "/kiosk").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("/static/css/plate.css"), "{page}");
    assert!(page.contains("/static/plate.js"));
    assert!(
        page.contains("/static/display-format.js"),
        "times must follow the reader's clock"
    );
    assert!(page.contains(r#"aria-current="true">Plate"#), "{page}");

    let (_, _, night) = get(&state, "/kiosk?night=1").await;
    assert!(night.contains(r#"data-night="1""#), "{night}");
    let (_, _, day) = get(&state, "/kiosk?night=0").await;
    assert!(day.contains(r#"data-night="0""#), "{day}");

    // The poll must carry the override too, or the first swap hands the sheet back to the sun.
    assert!(
        night.contains(r#"<input type="hidden" name="night" value="1">"#),
        "the form that drives the poll must carry ?night=1: {night}"
    );
    assert!(
        day.contains(r#"<input type="hidden" name="night" value="0">"#),
        "{day}"
    );
    assert!(
        !page.contains(r#"name="night""#) && !page.contains(r#"name="paper""#),
        "no override asked for, none carried: {page}"
    );
    let (_, _, plain) = get(&state, "/kiosk?paper=0").await;
    assert!(
        plain.contains(r#"<input type="hidden" name="paper" value="0">"#),
        "{plain}"
    );

    let (_, _, latest) = get(&state, "/kiosk?mode=latest").await;
    assert!(latest.contains(r#"aria-current="true">Latest"#), "{latest}");
}

#[tokio::test]
async fn the_plates_assets_are_served() {
    let state = state_with(|_| {});
    let (status, headers, css) = get(&state, "/static/css/plate.css").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/css; charset=utf-8");
    assert!(css.contains(".plate-page"));
    let (status, headers, js) = get(&state, "/static/plate.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/javascript");
    assert!(js.contains("bnbPlate"));
}

/// The plate engraves a photograph but draws the operator's own art as it was made, and the
/// header is the only thing that lets the script tell them apart.
#[tokio::test]
async fn an_operators_own_picture_is_marked_as_theirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("turdus_merula.png"), b"\x89PNG\r\n\x1a\n").expect("art");
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_custom_image_dir(dir.path().to_path_buf());
    let (status, headers, _) = get(&state, "/api/v2/species/image/Turdus%20merula/file").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-bnb-image-source"], "custom");
    assert_eq!(headers["content-type"], "image/png");

    // The counterpart: a species the operator has no file for is not marked, whatever answers.
    let (_, headers, _) = get(&state, "/api/v2/species/image/Parus%20major/file").await;
    assert!(headers.get("x-bnb-image-source").is_none());
}

/// What the page asks the poll for is what the sheet is drawn as: `?night=` and `?paper=0`
/// are honoured by the content route, in both directions.
#[tokio::test]
async fn the_sheet_is_drawn_as_asked_whatever_the_sun_says() {
    let state = state_with(|_| {});
    let (_, _, lamp) = get(&state, "/pages/kiosk-content?ar=1.6&night=1").await;
    assert!(lamp.contains(r#"data-night="1""#), "{lamp}");
    let (_, _, daylight) = get(&state, "/pages/kiosk-content?ar=1.6&night=0").await;
    assert!(daylight.contains(r#"data-night="0""#), "{daylight}");
    let (_, _, bare) = get(&state, "/pages/kiosk-content?ar=1.6&paper=0").await;
    assert!(bare.contains(r#"data-paper="0""#), "{bare}");
    let (_, _, papered) = get(&state, "/pages/kiosk-content?ar=1.6").await;
    assert!(papered.contains(r#"data-paper="1""#), "{papered}");
}
