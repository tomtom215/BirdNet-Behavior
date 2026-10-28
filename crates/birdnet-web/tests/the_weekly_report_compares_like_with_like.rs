//! "A quieter week" is said of a quieter week, not of an unfinished one.
//!
//! The weekly report held this week's detections so far against the whole of last week's, so
//! until the week was over every station was told its week was quieter than the one before, and
//! by how much, whatever it had actually heard. The comparison is now over the days that are
//! done, against the same days of last week.
//!
//! Observed failing against the old page, run on a Wednesday: with every complete day of this
//! week matching the same day of last week, the headline read "A quieter week." and the body
//! "69% quieter than the week before" instead of "steady".

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// The station's local epoch day, computed the way the page computes it.
fn today_epoch_day() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let local = i64::try_from(now).expect("fits") + birdnet_db::clock::local_utc_offset_secs();
    local.div_euclid(86_400)
}

fn ymd(epoch_day: i64) -> String {
    let (y, m, d) = birdnet_core::civil::civil_from_days(epoch_day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// 0 for Monday: 1970-01-01 was a Thursday.
const fn weekday(epoch_day: i64) -> i64 {
    (epoch_day + 3).rem_euclid(7)
}

fn seed(conn: &rusqlite::Connection, day: i64, n: usize) {
    for i in 0..n {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs)
             VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9, 0.7, 1, 1.25, 0.0,
                     'x.wav', 0)",
            rusqlite::params![ymd(day), format!("06:{:02}:{:02}", i / 60 % 60, i % 60)],
        )
        .expect("seed");
    }
}

async fn weekly(state: &AppState) -> String {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/pages/weekly-content")
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

#[tokio::test]
async fn a_week_that_matches_last_weeks_same_days_is_steady_not_quieter() {
    let today = today_epoch_day();
    let monday = today - weekday(today);
    let done = weekday(today); // Monday to yesterday
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    // Last week: 20 a day, every day. This week: 20 a day on each day that is over, and a
    // few so far today.
    for d in 0..7 {
        seed(&conn, monday - 7 + d, 20);
    }
    for d in 0..done {
        seed(&conn, monday + d, 20);
    }
    seed(&conn, today, 3);
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let html = weekly(&state).await;

    if done == 0 {
        // It is Monday: nothing is over yet, and nothing is compared.
        assert!(html.contains("just <em>beginning</em>"), "{html}");
        assert!(
            !html.contains('%'),
            "no percentage on a Monday morning: {html}"
        );
        return;
    }
    assert!(
        html.contains("<em>steady</em> week so far"),
        "the same days as last week are steady, said as so far: {html}"
    );
    assert!(!html.contains("quieter"), "never quieter: {html}");
}

/// The counterpart: the same page still says quieter when the days that are over were.
#[tokio::test]
async fn a_week_that_really_is_down_on_the_same_days_is_still_called_quieter() {
    let today = today_epoch_day();
    let monday = today - weekday(today);
    let done = weekday(today);
    if done == 0 {
        return; // Monday: no complete day to compare; the test above covers it.
    }
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for d in 0..7 {
        seed(&conn, monday - 7 + d, 40);
    }
    for d in 0..done {
        seed(&conn, monday + d, 10); // a quarter of last week's, day for day
    }
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let html = weekly(&state).await;
    assert!(html.contains("<em>quieter</em> week so far"), "{html}");
    assert!(html.contains("75% quieter than the same"), "{html}");
}
