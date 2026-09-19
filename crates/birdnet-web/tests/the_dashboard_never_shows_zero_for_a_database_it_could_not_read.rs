//! The front page must not answer "0 birds" when the question was never asked.
//!
//! # The defect
//!
//! The same shape as `a_query_that_failed_is_never_reported_as_a_quiet_year`,
//! on three more surfaces — and one of them is the first screen anyone opens.
//! `dashboard/stats.rs`, `dashboard/kiosk.rs` and `admin/overview.rs` each ran
//! their counts with `.unwrap_or(0)` **inside** the `with_db` closure, which
//! returned a plain tuple, so the `else`/`Err` arm below could only ever fire
//! on a task panic. A database that could not be read rendered:
//!
//! ```text
//! Detections 0 · Species 0 · Today 0 · Last hour 0
//! ```
//!
//! as an ordinary dashboard, a kiosk on a wall showing three zeroes and an
//! empty list, and an admin overview reporting a station that has never heard
//! anything. Nobody is reading a log beside a kiosk.
//!
//! # What is guarded, and what is deliberately left defaulted
//!
//! Every number that is a claim about the reader's own birds propagates. The
//! 12-day sparkline on the Detections tile does not: it is decoration beside a
//! number rather than a claim of its own, and `dashboard/partials.rs` already
//! drew that line correctly — primary query with `?`, `first_seen` and
//! `sparklines` defaulted — which is the pattern these three now follow.
//!
//! The counterpart matters as much as the check: a station that genuinely has
//! no detections must still get its dashboard, or "always render the error"
//! would pass.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Surfaces that answer with counts, and the phrase each must not invent.
const SURFACES: [&str; 2] = ["/pages/stats", "/pages/kiosk-content"];

/// A station whose `detections` table has been dropped: the connection opens
/// and every read of it fails, which is what a corrupt or partially-restored
/// database looks like from a handler.
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
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn a_failed_read_is_not_a_station_that_has_heard_nothing() {
    let state = station(true);
    for uri in SURFACES {
        let (status, body) = get(&state, uri).await;
        assert!(
            body.contains("We couldn't load") || !status.is_success(),
            "{uri} rendered a failed read as ordinary content. Body: {}",
            body.chars().take(240).collect::<String>()
        );
        assert!(
            !body.contains(r#"<div class="value tabular">0</div>"#),
            "{uri} printed a zero it never measured"
        );
    }
}

/// The worst of the family, and the reason it is not just a wrong number.
///
/// `today.rs` computed `firstrun = total_ever == 0` from a defaulted count, and
/// `firstrun` chooses the hero copy, the aside, the rail and a template flag.
/// A database that could not be read therefore did not print "0" — it replaced
/// a station with years of records with the **first-run setup experience**, and
/// told its owner to go and set up a microphone they had been using for years.
#[tokio::test]
async fn a_failed_read_does_not_turn_a_running_station_into_a_new_one() {
    let (_status, body) = get(&station(true), "/").await;
    assert!(
        !body.contains("waking up") && !body.contains("Let's get you listening"),
        "a failed read was rendered as a brand-new station. Body: {}",
        body.chars().take(300).collect::<String>()
    );
    assert!(
        body.contains("We couldn't load your dashboard"),
        "and it must say the read failed instead. Body: {}",
        body.chars().take(300).collect::<String>()
    );
}

/// The JSON API has the same rule and a sharper reason: its consumer is a
/// dashboard or a script, and it stores what it is told. `detections: 0` with
/// HTTP 200 becomes a recorded fact about a station that was never asked.
#[tokio::test]
async fn the_stats_api_refuses_rather_than_reporting_a_zero_it_did_not_measure() {
    let (status, body) = get(&station(true), "/api/v2/stats").await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a failed read must not answer 200. Body: {body}"
    );
    assert!(
        !body.contains(r#""total_detections":0"#),
        "the API reported a zero it never measured: {body}"
    );
}

/// The monitoring that exists to report a broken station must not report a
/// healthy one.
///
/// `routes/health.rs` states the rule itself, for acoustic drift: a source with
/// no baseline "exports the level and omits the drift, rather than exporting a
/// drift of zero, which would read as measured, and unchanged". Its own three
/// counts did export the zero. `birdnet_detections_stored 0` on a station with
/// years of records is a counter that fell to nothing, which is exactly what a
/// Prometheus alert is built to catch — so it fires for the wrong reason and
/// hides the real fault behind it. The series is now omitted, and the process
/// metrics, which are measured from this process, still publish.
#[tokio::test]
async fn the_metrics_endpoint_omits_a_count_it_could_not_take() {
    let (status, body) = get(&station(true), "/api/v2/metrics").await;
    assert_eq!(status, StatusCode::OK, "the scrape must still succeed");
    assert!(
        !body.contains("birdnet_detections_stored"),
        "a count that could not be read must be omitted, not exported as 0:\n{body}"
    );
    assert!(
        body.contains("birdnet_process_resident_memory_bytes"),
        "metrics measured from the process itself must survive:\n{body}"
    );
}

/// The counterpart for the exposition: a working station publishes the series.
#[tokio::test]
async fn the_metrics_endpoint_publishes_counts_it_could_take() {
    let (_status, body) = get(&station(false), "/api/v2/metrics").await;
    assert!(
        body.contains("birdnet_detections_stored 1"),
        "a working station must export its counts:\n{body}"
    );
}

/// The life list is the page a birdwatcher is most invested in, and "0 species"
/// there is not an empty list but a lost one.
#[tokio::test]
async fn the_life_list_does_not_report_a_lost_list_as_an_empty_one() {
    let (_status, body) = get(&station(true), "/species?view=lifelist").await;
    assert!(
        body.contains("We couldn't load your life list"),
        "a failed read must say so. Body: {}",
        body.chars().take(240).collect::<String>()
    );
}

/// The counterpart. A station whose database works must still get its numbers,
/// or "always render the error" would satisfy the check above.
#[tokio::test]
async fn a_working_station_still_gets_its_numbers() {
    let state = station(false);
    let (status, body) = get(&state, "/api/v2/stats").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the API must answer a working station"
    );
    assert!(
        body.contains(r#""total_detections":1"#),
        "and report the detection it has: {body}"
    );
    for uri in SURFACES {
        let (status, body) = get(&state, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri} must answer");
        assert!(
            !body.contains("We couldn't load"),
            "{uri} reported a failure against a working database. Body: {}",
            body.chars().take(240).collect::<String>()
        );
        assert!(
            body.contains('1'),
            "{uri} must show the one detection it has. Body: {}",
            body.chars().take(240).collect::<String>()
        );
    }
}
