//! "Year in review · 2026" is 2026.
//!
//! The page mixed three scopes under that one label: all-time totals, species
//! count, leaderboard and first day; a trailing 366 days for the busiest day;
//! and the calendar year for "new to your list". A station with three years of
//! records was told all three years' detections were "your year". Every
//! figure is now the calendar year so far.
//!
//! Observed failing against the old page: the headline read "9 detections
//! across 3 species" for a year with three, and last year's only species led
//! the leaderboard.

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// The station's local calendar year, computed the way the page computes it.
fn this_year() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let local = i64::try_from(now).expect("fits") + birdnet_db::clock::local_utc_offset_secs();
    i64::from(birdnet_core::civil::civil_from_days(local.div_euclid(86_400)).0)
}

fn station(rows: &[(&str, &str, &str)]) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for (i, (date, sci, com)) in rows.iter().enumerate() {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs)
             VALUES (?1, ?2, ?3, ?4, 0.9, 0.7, 1, 1.25, 0.0, 'x.wav', 0)",
            rusqlite::params![date, format!("06:{:02}:00", i % 60), sci, com],
        )
        .expect("seed");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn year_page(state: &AppState) -> String {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/reports?tab=year")
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
async fn last_years_birds_are_not_this_years() {
    let y = this_year();
    let last = format!("{}-06-15", y - 1);
    let jan = format!("{y}-01-01");
    let mut rows: Vec<(&str, &str, &str)> = Vec::new();
    // Last year: five Magpies and one Robin.
    for _ in 0..5 {
        rows.push((last.as_str(), "Pica pica", "Eurasian Magpie"));
    }
    rows.push((last.as_str(), "Erithacus rubecula", "European Robin"));
    // This year: two Blackbirds — new — and the Robin again, which is not.
    rows.push((jan.as_str(), "Turdus merula", "Eurasian Blackbird"));
    rows.push((jan.as_str(), "Turdus merula", "Eurasian Blackbird"));
    rows.push((jan.as_str(), "Erithacus rubecula", "European Robin"));
    let html = year_page(&station(&rows)).await;

    // The shared page head's eyebrow ("Reports · year in review · 2026"); it is small caps by CSS.
    assert!(
        html.contains(&format!("year in review · {y}")),
        "the masthead names the year"
    );
    assert!(
        !html.contains(&format!("year in review · {}", y - 1)),
        "the masthead does not name last year"
    );
    assert!(
        html.contains("<b>3 detections</b> across <b>2 species</b>"),
        "the headline counts this year only"
    );
    assert!(
        html.contains("— and <b>1</b> heard for the very first time"),
        "only the Blackbird is new"
    );
    assert!(
        !html.contains("Eurasian Magpie"),
        "a species heard only last year is not on this year's page"
    );
    assert!(
        html.contains(&format!("{jan} · the first day heard in {y}")),
        "the first voice is this year's"
    );
    assert!(
        html.contains("across 1 days of listening in"),
        "days of listening are this year's"
    );
}

/// The counterpart: a station whose birds were all heard last year gets an
/// empty year, said as one, and not an error or last year's page.
#[tokio::test]
async fn a_year_with_nothing_heard_yet_says_so() {
    let y = this_year();
    let last = format!("{}-06-15", y - 1);
    let html = year_page(&station(&[(last.as_str(), "Pica pica", "Eurasian Magpie")])).await;
    assert!(html.contains("<b>0 detections</b> across <b>0 species</b>"));
    assert!(html.contains(&format!("Nothing heard yet in {y}.")));
    assert!(!html.contains("Eurasian Magpie"));
    assert!(!html.contains("We couldn't load your year in review"));
}
