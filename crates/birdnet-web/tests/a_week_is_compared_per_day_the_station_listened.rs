//! "A loud week" is said of a week the birds were louder in, not of one held against a week the
//! station was partly off for.
//!
//! The weekly report compared this week's detections with the same days of the last in total,
//! so a week that followed two days of power cut read "A loud week" and "N% busier than the
//! same 6 days last week" while the Trends tab hatched those two days as not listening. It now
//! compares per day the station listened (from the `recording_effort` ledger), says when either
//! week lost days, and drops the adjective when the two weeks' listened days differ by more than
//! a fifth of the days compared.
//!
//! The week is a fixed past one (`?week=2026-03-09`), so the test does not depend on the day it
//! runs: a past week is compared whole, seven days against seven.

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// This week: Monday 9 March 2026 to Sunday the 15th. Last week: 2 to 8 March.
const WEEK: &str = "2026-03-09";

fn seed(conn: &rusqlite::Connection, date: &str, n: usize) {
    for i in 0..n {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs)
             VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9, 0.7, 1, 1.25, 0.0,
                     'x.wav', 0)",
            rusqlite::params![date, format!("06:{:02}:{:02}", i / 60 % 60, i % 60)],
        )
        .expect("seed");
    }
}

fn listened(conn: &rusqlite::Connection, date: &str) {
    conn.execute(
        "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, 'mic', 86400)",
        [date],
    )
    .expect("ledger");
}

/// 12 a day on each day of this week; 10 a day on each day of last week the station was on.
/// `off` are the days of last week (2 to 8 March) with neither audio nor detections.
fn station(off: &[u32], ledger: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for d in 2..=8 {
        let date = format!("2026-03-{d:02}");
        if off.contains(&d) {
            continue;
        }
        seed(&conn, &date, 10);
        if ledger {
            listened(&conn, &date);
        }
    }
    for d in 9..=15 {
        let date = format!("2026-03-{d:02}");
        seed(&conn, &date, 12);
        if ledger {
            listened(&conn, &date);
        }
    }
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(
        n,
        84 + 10 * (7 - i64::try_from(off.len()).expect("a few days")),
        "the fixture seeded what it says"
    );
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn weekly(state: &AppState) -> String {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/pages/weekly-content?week={WEEK}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The headline and the lead, without the chart and the lists.
fn words(html: &str) -> &str {
    let from = html.find("wk-headline").expect("a headline");
    let to = html[from..].find("rp-stats").expect("a stat band") + from;
    &html[from..to]
}

/// Last week the station was off on Monday and Tuesday. 84 against 50 is 68% in total, and
/// "loud"; per listened day it is 12 against 10, 20%, and two days of seven lost is too many to
/// call the week anything.
#[tokio::test]
async fn a_week_after_an_outage_is_compared_per_listened_day_and_names_the_gap() {
    let html = weekly(&station(&[2, 3], true)).await;
    let w = words(&html);
    assert!(
        w.contains("20% busier per listened day than the week before"),
        "the percentage is per listened day: {w}"
    );
    assert!(
        w.contains("last week the station was off for 2 of those 7 days"),
        "the sentence names the gap: {w}"
    );
    assert!(!w.contains("68%"), "not the raw totals: {w}");
    assert!(
        !w.contains("<em>loud</em>") && !w.contains("<em>quieter</em>"),
        "no adjective a gap produced: {w}"
    );
    // The detections tile says the same thing in its small print.
    assert!(html.contains("↑ 20% per listened day"), "{html}");
}

/// Counterpart: one lost day in seven is within the share, so the week is still called what
/// it was, per listened day, and the gap is still named.
#[tokio::test]
async fn one_lost_day_in_seven_is_named_and_keeps_the_adjective() {
    let html = weekly(&station(&[2], true)).await;
    let w = words(&html);
    assert!(w.contains("<em>loud</em> week"), "{w}");
    assert!(
        w.contains("20% busier per listened day than the week before"),
        "{w}"
    );
    assert!(
        w.contains("last week the station was off for 1 of those 7 days"),
        "{w}"
    );
}

/// Counterpart: two whole weeks of listening compare as they always did; nothing is said about
/// gaps there were not.
#[tokio::test]
async fn two_whole_weeks_compare_in_total_and_say_nothing_about_gaps() {
    // 84 against 70: 20%, a loud week.
    let html = weekly(&station(&[], true)).await;
    let w = words(&html);
    assert!(w.contains("<em>loud</em> week"), "{w}");
    assert!(w.contains("20% busier than the week before"), "{w}");
    assert!(!w.contains("per listened day"), "{w}");
    assert!(!w.contains("was off"), "{w}");
}

/// Counterpart: a station with no ledger (imported from BirdNET-Pi) cannot say which days it
/// listened, and says nothing about it: the comparison is the totals, as before.
#[tokio::test]
async fn without_a_ledger_the_totals_are_compared_and_no_gap_is_claimed() {
    let html = weekly(&station(&[2, 3], false)).await;
    let w = words(&html);
    assert!(w.contains("68% busier than the week before"), "{w}");
    assert!(!w.contains("per listened day"), "{w}");
    assert!(!w.contains("was off"), "{w}");
}
