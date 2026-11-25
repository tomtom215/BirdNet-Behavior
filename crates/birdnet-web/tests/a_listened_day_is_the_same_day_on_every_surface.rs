//! RES-26. A "listened" day had two definitions on one station: the pages that read the
//! `SQLite` ledger (`pages/listening.rs`) counted any date with 60 seconds of audio, and the
//! reporting frequency and the trend queries count a date with at least half the station's
//! typical day (`EFFORT_FLOOR_SHARE` of the median day, summed over sources). A 61-second day,
//! a station that was up for a minute before a power cut, was a whole day of listening on the
//! weekly report and Year in review and not one on the species page. There is one rule now,
//! the share of the typical day.
//!
//! The station (every date fixed, in the past, so the run on the 1st of a month is the run on
//! any other day; the period crosses a year boundary, three month boundaries and a leap day):
//!
//! * 25 Dec 2023 to 10 Mar 2024, 77 dates, a whole day (86 400 s) each, except
//!   * 31 Dec 2023 and 29 Feb 2024: 61 s each (up for a minute), and
//!   * 15 Jan 2024: two sources of 30 000 s each, 60 000 s for the day.
//! * The typical day is the median of the 77 daily sums: 74 of them are 86 400, so 86 400.
//!   The floor is half of it, 43 200 s. 61 s is under it; 60 000 is over it, though each
//!   source alone (30 000) would not be: a day is summed over sources.
//! * A Blackbird: 30 a day on every whole day up to 3 Mar, 60 a day from 4 Mar (Monday) to
//!   10 Mar, none on the two short days.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Every date of the fixture, `2023-12-25` to `2024-03-10`.
fn dates() -> Vec<String> {
    let mut out = Vec::new();
    for d in 25..=31 {
        out.push(format!("2023-12-{d:02}"));
    }
    for d in 1..=31 {
        out.push(format!("2024-01-{d:02}"));
    }
    for d in 1..=29 {
        out.push(format!("2024-02-{d:02}"));
    }
    for d in 1..=10 {
        out.push(format!("2024-03-{d:02}"));
    }
    out
}

const SHORT_DAYS: [&str; 2] = ["2023-12-31", "2024-02-29"];

fn seed(conn: &rusqlite::Connection) {
    let all = dates();
    assert_eq!(all.len(), 77, "the fixture's own arithmetic");
    for date in &all {
        let short = SHORT_DAYS.contains(&date.as_str());
        if date == "2024-01-15" {
            for source in ["local", "rtsp_1"] {
                conn.execute(
                    "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, ?2, 30000)",
                    rusqlite::params![date, source],
                )
                .expect("effort");
            }
        } else {
            conn.execute(
                "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, 'local', ?2)",
                rusqlite::params![date, if short { 61.0 } else { 86_400.0 }],
            )
            .expect("effort");
        }
        if short {
            continue;
        }
        let n = if date.as_str() >= "2024-03-04" {
            60
        } else {
            30
        };
        for i in 0..n {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                      File_Name, chunk_offset_secs, Source, detected_at_utc)
                 VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9, 0.7, 10, 1.25, 0.0,
                         ?3, 0, 'local', CAST(strftime('%s', ?1 || ' ' || ?2) AS INTEGER))",
                rusqlite::params![
                    date,
                    format!("06:{:02}:{:02}", i / 60, i % 60),
                    format!("{date}-{i}.wav")
                ],
            )
            .expect("detection");
        }
    }
}

async fn get(state: AppState, uri: &str) -> String {
    let res = birdnet_web::server::build_router(state)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    assert_eq!(res.status(), StatusCode::OK, "{uri}");
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The weekly report for the week of Monday 4 March 2024, held against 26 Feb – 3 Mar, which
/// holds the 61-second 29th.
///
/// Worked by hand: this week 7 × 60 = 420 on 7 listened days, 60 a day; the week before
/// 6 × 30 = 180 on 6 listened days, 30 a day: +100 % per day listened, and 1 of 7 days off.
/// With the 29th counted as listened (the 60-second rule) the weeks were both whole and the
/// raw counts compared: (420 − 180) / 180 = +133 %, and no day was said to be lost.
#[tokio::test]
async fn the_weekly_report_does_not_count_a_minute_as_a_day() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(seed);
    let page = get(state, "/pages/weekly-content?week=2024-03-04").await;
    assert!(
        page.contains(
            "100% busier per day listened than the week before \
             (the station was off for 1 of those 7 days the week before)"
        ),
        "the 61-second day was counted as a day of listening: {page}"
    );
    assert!(!page.contains("133%"), "{page}");
}

/// Counterpart: two whole weeks either side of the 29th compare by raw count, with no note.
/// 19 – 25 Feb against 12 – 18 Feb: 210 and 210, a steady week, nothing lost.
#[tokio::test]
async fn whole_weeks_still_compare_as_whole_weeks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(seed);
    let page = get(state, "/pages/weekly-content?week=2024-02-19").await;
    assert!(page.contains("A <em>steady</em> week."), "{page}");
    assert!(!page.contains("was off for"), "{page}");
}

/// The reporting frequency, which already used the share, says the same of the same day:
/// 22–29 Feb holds eight dates, and the 29th is not one the station listened on, so the
/// Blackbird was heard on 7 of 7 listened days.
#[cfg(feature = "analytics")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reporting_frequency_agrees() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sqlite = dir.path().join("birds.db");
    {
        let conn = birdnet_db::sqlite::open_or_create(&sqlite).expect("sqlite");
        seed(&conn);
    }
    let state =
        AppState::new_with_analytics(sqlite, &dir.path().join("analytics.duckdb")).expect("state");
    let html = get(state, "/pages/species-reporting?name=Eurasian%20Blackbird").await;
    let at = html
        .find(r#"<th scope="row">22–29 Feb</th>"#)
        .unwrap_or_else(|| panic!("a row for 22–29 Feb in {html}"));
    let row = &html[at..at + html[at..].find("</tr>").expect("row end")];
    assert!(
        row.contains(r#"<td class="num">7</td><td class="num">7</td>"#),
        "{row}"
    );
}
