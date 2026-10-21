//! The reads `tests/a_failed_read_is_not_defaulted_into_a_fact.rs` found, and the
//! pages that printed a default for each.
//!
//! The root crate's sweep lists every place a read error is turned into a value
//! inside the closure that made the read. Reading the 225 hits, these were the
//! ones where the value then said something to the operator that the station had
//! not measured:
//!
//! * the species page's "Best detection" said "No recording captured for this
//!   species yet" for a bird whose recordings the database could not list;
//! * "No per-species thresholds configured" for a list it could not read, which
//!   invites adding a row that a failed read made look absent;
//! * the backup panel said a scheduled backup had never run;
//! * a blacklisted image was served because the blacklist could not be asked;
//! * the live source picker said no source was configured; `/stream` said "no such
//!   audio source" (404) or "no audio source configured" (503) for a table it could
//!   not read;
//! * the Today pill said "recording" because the freshness read had failed;
//! * the BirdWeather test said "token not configured" for a settings table it could
//!   not read;
//! * the sign-in said "Incorrect username or password" for a users table it could
//!   not read;
//! * the Migration cards said "No arrivals yet" and "Quiet week";
//! * a share link said the sighting did not exist.
//!
//! Each test drives a station whose table has gone, beside the working station
//! with the same data and the empty station, because the discrimination matters
//! as much as the alarm: a station that truly has nothing must still say so.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A station with `statements` run against it after it is seeded.
fn station(statements: &[&str], seed: impl FnOnce(&rusqlite::Connection)) -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    seed(&conn);
    for sql in statements {
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("could not break the station with {sql:?}: {e}"));
    }
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

const fn nothing(_: &rusqlite::Connection) {}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    form: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let (status, _, body) = send(app, "GET", uri, "").await;
    (status, body)
}

fn one_clip(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-05-01', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.55, 'b.wav')",
        [],
    )
    .expect("seed detection");
}

/// A detection whose clip retention has reclaimed: the row is kept, the audio is not.
fn one_reclaimed_clip(conn: &rusqlite::Connection) {
    one_clip(conn);
    conn.execute(
        "UPDATE detections SET Clip_Pruned_At = '2026-05-02 00:00:00'",
        [],
    )
    .expect("reclaim");
}

// ── the species page's best recording ────────────────────────────────────

#[tokio::test]
async fn the_species_hero_does_not_say_no_recording_when_it_could_not_look() {
    let uri = "/pages/species-hero?name=Eurasian+Blackbird";

    let (status, body) = get(&station(&["DROP TABLE detections"], one_clip), uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.contains("No recording captured"),
        "a failed read was reported as a species with no recording: {body}"
    );
    assert!(
        !body.contains("clips reclaimed"),
        "a failed read was reported as reclaimed clips: {body}"
    );
    assert!(body.contains("couldn't load"), "{body}");

    // Counterparts: the same station working says each of its three true things.
    let (_, body) = get(&station(&[], one_clip), uri).await;
    assert!(body.contains("b.wav"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(
        body.contains("No recording captured for this species yet"),
        "{body}"
    );
    assert!(!body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], one_reclaimed_clip), uri).await;
    assert!(body.contains("reclaimed"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");
}
