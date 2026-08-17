//! "Today" must mean the station's today, not UTC's.
//!
//! Every detection's `Date` column is **local civil time**: capture stamps
//! recording filenames from the system's local clock (see
//! `birdnet_core::audio::capture::types::LocalOffset`) and the detection rows
//! inherit that. Queries therefore have to ask SQLite for the local date —
//! `date('now','localtime')` — and `birdnet_db::clock` says so in as many
//! words.
//!
//! Five queries asked for `date('now')` instead, which is **UTC**. The result
//! is a day-boundary skew whose size and direction depend on the station's
//! offset and the time of day:
//!
//! * **East of UTC** (Europe/Asia/Oceania) — from local midnight until the UTC
//!   day catches up, `date('now')` is *behind* the local date, so "today" is
//!   still yesterday. A UTC+13 station is a day behind for most of its day.
//! * **West of UTC** (the Americas) — after local evening the UTC day has
//!   already rolled over, so `date('now')` is *ahead*: `WHERE Date =
//!   date('now')` matches a date no detection carries yet and the RSS feed
//!   returns **nothing** for the last hours of every evening.
//!
//! # Why this test re-executes itself
//!
//! SQLite's `localtime` modifier reads the process timezone through libc, and
//! the process timezone can only be changed before the process starts:
//! `std::env::set_var` is `unsafe` in edition 2024 and this workspace forbids
//! `unsafe`. So the test runs twice — once as itself, to spawn a child with
//! `TZ` set, and once as that child, to make the assertions.
//!
//! The offset is chosen from the current UTC hour so the local and UTC dates
//! are guaranteed to differ whenever this runs: UTC+14 differs from 10:00 UTC
//! onward, UTC−12 differs before 12:00 UTC, and between them they cover the
//! clock. POSIX `TZ` strings (`XXX-14`) are used rather than zone names so the
//! test does not depend on tzdata being installed.

use std::process::Command;

/// Env var marking the child run. Its value is the offset under test.
const CHILD_MARKER: &str = "BNB_TZ_PROBE";

/// A `TZ` value whose local date differs from the UTC date *right now*.
///
/// POSIX `TZ` sign convention is inverted relative to the usual one: `XXX-14`
/// is UTC **+**14.
fn skewed_tz() -> &'static str {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let utc_hour = (secs / 3600) % 24;
    if utc_hour >= 10 { "XXX-14" } else { "XXX+12" }
}

/// Re-run this test binary with `TZ` set, and fail if the child fails.
fn in_skewed_timezone(test_name: &str) -> bool {
    if std::env::var_os(CHILD_MARKER).is_some() {
        return true; // we *are* the child; carry on and assert.
    }
    let exe = std::env::current_exe().expect("test binary path");
    let tz = skewed_tz();
    let status = Command::new(exe)
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env("TZ", tz)
        .env(CHILD_MARKER, tz)
        .status()
        .expect("re-exec the test binary");
    assert!(
        status.success(),
        "the {test_name} assertions failed under TZ={tz}"
    );
    false
}

/// Open an in-memory station holding one detection at local noon today.
fn conn_with_local_noon_detection() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    // Stamped exactly the way capture stamps it: the *local* civil date.
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES (date('now','localtime'), '12:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
        [],
    )
    .expect("insert");
    conn
}

/// Guard the fixture itself: if local and UTC dates agreed, every assertion
/// below would pass for the wrong reason.
fn assert_dates_really_differ(conn: &rusqlite::Connection) {
    let (utc, local): (String, String) = conn
        .query_row("SELECT date('now'), date('now','localtime')", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("dates");
    assert_ne!(
        utc, local,
        "fixture is inert: the chosen TZ does not skew the date at this hour"
    );
}

/// The RSS "today" feed must carry a detection recorded at local noon today.
///
/// This is the endpoint where the defect is total rather than partial: west of
/// UTC the query matches a date no row carries and the feed is simply empty.
#[test]
fn todays_feed_uses_the_stations_local_day() {
    if !in_skewed_timezone("todays_feed_uses_the_stations_local_day") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("birds.db");
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES (date('now','localtime'), '12:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
            [],
        )
        .expect("insert");
        assert_dates_really_differ(&conn);
    }

    // The real handler, not a re-implementation of its query: the whole defect
    // was that the handler's own SQL asked the wrong clock.
    let state = birdnet_web::state::AppState::new(db_path).expect("state");
    let body = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            use tower::ServiceExt as _;
            let response = birdnet_web::server::build_router(state)
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/feeds/today.rss")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .expect("feed responds");
            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .expect("feed body");
            String::from_utf8_lossy(&bytes).into_owned()
        });

    assert!(
        body.contains("Eurasian Blackbird"),
        "the today feed is empty for a station whose local day differs from \
         UTC's — west of UTC this is every evening. Body was:\n{body}"
    );
}

/// The species sparkline's window **and** its date axis must both be local.
///
/// The axis is the half that misleads even when the window happens to be wide
/// enough: it is built from `date('now', …)` while the counts are keyed by the
/// local `Date`, so the two are joined on dates that name different days.
#[test]
fn species_sparklines_are_keyed_to_the_local_day() {
    if !in_skewed_timezone("species_sparklines_are_keyed_to_the_local_day") {
        return;
    }
    let conn = conn_with_local_noon_detection();
    assert_dates_really_differ(&conn);

    let map = birdnet_db::sqlite::species_sparklines(&conn, 7).expect("sparklines");
    let series = map
        .get("Eurasian Blackbird")
        .expect("the species has a sparkline");
    assert_eq!(series.len(), 7, "seven days of axis");
    assert_eq!(
        series.iter().sum::<i64>(),
        1,
        "the detection recorded at local noon today is missing from its own \
         sparkline — the axis and the counts are keyed to different days"
    );
    assert_eq!(
        *series.last().expect("non-empty"),
        1,
        "the detection landed on a day other than the axis's last, so 'today' \
         on the sparkline is not the station's today"
    );
}
