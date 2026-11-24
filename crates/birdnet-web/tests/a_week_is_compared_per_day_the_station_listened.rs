//! UIR-08. The weekly report compares a week with the one before it. When the
//! station was off for part of the earlier week, a raw count makes an outage
//! read as a loud week ("315 % busier"). The comparison is per listened day,
//! from the effort ledger, and the sentence says how many days were lost.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A Monday, so `?week=` names a whole past week.
const WEEK: &str = "2026-03-09";
const BEFORE: [&str; 7] = [
    "2026-03-02",
    "2026-03-03",
    "2026-03-04",
    "2026-03-05",
    "2026-03-06",
    "2026-03-07",
    "2026-03-08",
];
const THIS: [&str; 7] = [
    "2026-03-09",
    "2026-03-10",
    "2026-03-11",
    "2026-03-12",
    "2026-03-13",
    "2026-03-14",
    "2026-03-15",
];

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

/// `n` detections on `date`, a second apart, and a whole day of listening.
fn day(state: &AppState, date: &str, n: u32) {
    state.with_db(|conn| {
        for i in 0..n {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                      File_Name, chunk_offset_secs, Source)
                 VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9, 0.7, 10, 1.25, 0.0,
                         ?3, 0, 'local')",
                rusqlite::params![
                    date,
                    format!("06:{:02}:{:02}", i / 60, i % 60),
                    format!("{date}-{i}.wav")
                ],
            )
            .expect("insert detection");
        }
        conn.execute(
            "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, 'local', 86400)",
            rusqlite::params![date],
        )
        .expect("insert effort");
    });
}

async fn lead(state: &AppState) -> String {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/pages/weekly-content?week={WEEK}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router responds");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

/// The week before lost Monday and Tuesday: 30 a day over five days (150), against
/// 60 a day over seven (420). Per listened day that is twice as busy (+100 %);
/// by raw count it read +180 %.
#[tokio::test]
async fn an_outage_in_the_week_before_is_named_and_divided_out() {
    let (_dir, state) = station();
    for date in &BEFORE[2..] {
        day(&state, date, 30);
    }
    for date in THIS {
        day(&state, date, 60);
    }
    let page = lead(&state).await;
    assert!(
        page.contains("100% busier per day listened than the week before"),
        "the comparison is not per listened day: {page}"
    );
    assert!(
        page.contains("the station was off for 2 of those 7 days the week before"),
        "the lost days are not named: {page}"
    );
    assert!(
        !page.contains("180%"),
        "the raw comparison is still shown: {page}"
    );
}

/// Counterpart: two whole weeks compare as before, with no note.
#[tokio::test]
async fn two_whole_weeks_compare_as_they_always_did() {
    let (_dir, state) = station();
    for date in BEFORE {
        day(&state, date, 30);
    }
    for date in THIS {
        day(&state, date, 60);
    }
    let page = lead(&state).await;
    assert!(page.contains("100% busier than the week before"), "{page}");
    assert!(!page.contains("was off for"), "{page}");
}
