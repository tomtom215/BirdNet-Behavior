//! The History page's reads are bounded by what it draws (UIC-21).
//!
//! A month of the calendar read every day the station had ever recorded
//! (`detections_per_day`, `GROUP BY Date` over the whole table) and a day page
//! read every detection of the day. The month is now read through the date
//! index for its own dates, the page's two ends from either end of the index,
//! and the day's log a page at a time.

use birdnet_db::sqlite::{
    detection_date_bounds, detections_by_date_page, detections_per_day, detections_per_day_between,
};
use rusqlite::Connection;

fn station() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "WITH RECURSIVE d(n) AS (SELECT 0 UNION ALL SELECT n + 1 FROM d WHERE n < 399)
         INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         SELECT date('2025-01-01', '+' || (n / 4) || ' days'),
                printf('06:%02d:00', n % 4), 'Turdus merula',
                CASE n % 2 WHEN 0 THEN 'Eurasian Blackbird' ELSE 'Song Thrush' END,
                0.9, 'f' || n || '.wav'
           FROM d;",
    )
    .expect("seed");
    conn
}

fn plan(conn: &Connection, sql: &str) -> String {
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("plan");
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(3))
        .expect("rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    rows.join("\n")
}

/// The month read agrees, day for day, with the whole-table read it replaces.
#[test]
fn a_month_read_is_the_whole_read_cut_to_the_month() {
    let conn = station();
    let all = detections_per_day(&conn).expect("all");
    let month = detections_per_day_between(&conn, "2025-02-01", "2025-02-28").expect("month");
    let expected: Vec<_> = all
        .iter()
        .filter(|d| d.date.as_str() >= "2025-02-01" && d.date.as_str() <= "2025-02-28")
        .map(|d| (d.date.clone(), d.count, d.species))
        .collect();
    let got: Vec<_> = month
        .iter()
        .map(|d| (d.date.clone(), d.count, d.species))
        .collect();
    assert_eq!(got.len(), 28, "every day of February has detections");
    assert_eq!(got, expected);
    assert_eq!(got[0], ("2025-02-01".into(), 4, 2));
}

/// The month is found through the date index, not by a scan of the table.
#[test]
fn a_month_read_searches_the_date_index() {
    let conn = station();
    let p = plan(
        &conn,
        "SELECT Date, COUNT(*), COUNT(DISTINCT Com_Name) FROM detections_analytic \
         WHERE Date BETWEEN '2025-02-01' AND '2025-02-28' GROUP BY Date ORDER BY Date",
    );
    assert!(
        p.contains("SEARCH detections USING") && p.contains("Date>? AND Date<?"),
        "a month must be read through the date index:\n{p}"
    );
    // The counterpart: the old read scans the table.
    let old = plan(
        &conn,
        "SELECT Date, COUNT(*), COUNT(DISTINCT Com_Name) FROM detections_analytic \
         GROUP BY Date ORDER BY Date",
    );
    assert!(!old.contains("Date>? AND Date<?"), "{old}");
}

/// The two ends, and a rejected detection is not one of them (the view the calendar counts).
#[test]
fn the_ends_are_the_first_and_last_counted_days() {
    let conn = station();
    assert_eq!(
        detection_date_bounds(&conn).expect("bounds"),
        Some(("2025-01-01".into(), "2025-04-10".into()))
    );
    conn.execute(
        "UPDATE detections SET review_verdict = 'rejected' WHERE Date = '2025-04-10'",
        [],
    )
    .expect("reject the last day");
    assert_eq!(
        detection_date_bounds(&conn).expect("bounds").map(|b| b.1),
        Some("2025-04-09".into())
    );
    let empty = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&empty).expect("migrate");
    assert_eq!(detection_date_bounds(&empty).expect("bounds"), None);
}

/// Pages of one day's log neither repeat nor skip a row, newest first, even where two rows
/// share a time.
#[test]
fn the_pages_of_a_day_cover_it_once() {
    let conn = station();
    let day = "2025-01-01";
    let mut seen = Vec::new();
    for page in 0..3 {
        let rows = detections_by_date_page(&conn, day, 3, page * 3).expect("page");
        seen.extend(rows.into_iter().map(|r| (r.time, r.com_name)));
    }
    assert_eq!(seen.len(), 4, "{seen:?}");
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 4, "no row twice: {seen:?}");
    assert_eq!(seen[0].0, "06:03:00", "newest first");
}
