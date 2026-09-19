//! A database that cannot be read must not be rendered as a station that
//! heard nothing.
//!
//! # The defect
//!
//! Two surfaces defaulted every one of their queries to zero *inside* the
//! closure that ran them, so the closure returned a plain tuple and the `Err`
//! arm below it could only ever fire on a task panic. A real database error —
//! a locked file, a bad sector, a full disk mid-read — produced a complete,
//! ordinary-looking page:
//!
//! * **Year in review** rendered "0 detections across 0 species", "busiest
//!   day —", for a station with three years of records. Nothing on that page
//!   is anything other than a claim about the reader's own birds.
//! * **Station Health** rendered "no microphone or camera is set up yet", "No
//!   detections yet today" and a total of `0`: a brand-new, unconfigured
//!   station, shown to someone whose own has been running for years. That page
//!   is where every other error message in the app sends its reader, and its
//!   own module comment promises "Everything shown is real".
//!
//! The failure mode is worse than an error page, because there is nothing to
//! act on. The reader concludes the birds stopped.
//!
//! # What is guarded
//!
//! Both directions, on both surfaces. A station with a broken database says
//! so; a station that is genuinely quiet still says *that*, because a page
//! that cried failure at every empty result would be just as useless and would
//! hide the real thing.
//!
//! Health keeps rendering its vitals either way: CPU, memory and disk are
//! measured from the system, not the database, and are exactly what someone
//! diagnosing a broken database needs to see.

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A station whose detection table has been dropped: the schema is there, the
/// connection opens, and every read of it fails — the shape of a database that
/// has gone bad under a running station.
fn station(break_reads: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    conn.execute(
        "INSERT INTO detections
             (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
              File_Name, chunk_offset_secs)
         VALUES ('2026-09-19', '09:00:00', 'Pica pica', 'Eurasian Magpie',
                 0.9, 0.7, 38, 1.25, 0.0, 'x.wav', 0)",
        [],
    )
    .expect("seed a detection");
    if break_reads {
        conn.execute("DROP TABLE detections", [])
            .expect("drop detections");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn page(state: &AppState, uri: &str) -> String {
    let req = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn a_broken_database_is_not_a_year_with_no_birds_in_it() {
    let html = page(&station(true), "/reports?tab=year").await;
    assert!(
        html.contains("We couldn't load your year in review"),
        "a failed read must say so. Page: {}",
        excerpt(&html)
    );
    assert!(
        !html.contains("0 detections</b>"),
        "a failed read was rendered as a year in which nothing was heard"
    );
}

/// The counterpart. A station that genuinely has no detections must still get
/// its page, or the check above is satisfied by a surface that always errors.
#[tokio::test]
async fn a_station_with_one_bird_still_gets_its_year() {
    let html = page(&station(false), "/reports?tab=year").await;
    assert!(
        !html.contains("We couldn't load your year in review"),
        "a working database was reported as a failure. Page: {}",
        excerpt(&html)
    );
    assert!(
        html.contains("1 detections</b>") || html.contains("detections</b>"),
        "the year page must still render its headline. Page: {}",
        excerpt(&html)
    );
}

#[tokio::test]
async fn a_broken_database_is_not_a_station_nobody_configured() {
    let html = page(&station(true), "/station").await;
    assert!(
        html.contains("records could not be read"),
        "Health must say the read failed. Page: {}",
        excerpt(&html)
    );
    assert!(
        !html.contains("No detections yet today"),
        "a failed read was reported as a quiet yard"
    );
    assert!(
        !html.contains("microphone or camera is set up yet"),
        "a failed read was reported as an unconfigured station"
    );
    // The half that still works has to keep working: this is the page someone
    // opens *because* something is wrong.
    assert!(
        html.contains("Vitals"),
        "the system vitals are not read from the database and must survive a \
         failed read. Page: {}",
        excerpt(&html)
    );
}

/// The counterpart for Health.
#[tokio::test]
async fn a_working_station_health_page_makes_no_such_claim() {
    let html = page(&station(false), "/station").await;
    assert!(
        !html.contains("records could not be read"),
        "a working database was reported as unreadable. Page: {}",
        excerpt(&html)
    );
}

/// What the page *says*, for a failure message worth reading.
///
/// Comments, `<script>` and `<style>` bodies and every tag are dropped, so an
/// assertion shows the sentence the reader would see. Without the script skip
/// the excerpt was the update-banner's inline JavaScript, which is neither
/// visible nor informative.
fn excerpt(html: &str) -> String {
    let body = html.find("<main").map_or(html, |i| &html[i..]);
    let mut text = String::new();
    let mut rest = body;
    while !rest.is_empty() && text.len() < 600 {
        let Some(open) = rest.find('<') else {
            text.push_str(rest);
            break;
        };
        text.push_str(&rest[..open]);
        rest = &rest[open..];
        let skip_to = if rest.starts_with("<!--") {
            "-->"
        } else if rest.starts_with("<script") {
            "</script>"
        } else if rest.starts_with("<style") {
            "</style>"
        } else {
            ">"
        };
        match rest.find(skip_to) {
            Some(end) => rest = &rest[end + skip_to.len()..],
            None => break,
        }
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let joined = words.join(" ");
    joined.chars().take(320).collect()
}
