//! `detected_at_utc` — the instant a detection happened, beside the local wall
//! clock it is displayed in.
//!
//! # What these gates are for
//!
//! `Date`/`Time` are local wall clock with no offset. That pair is not a point
//! in time: one local hour repeats every autumn and one never happens every
//! spring, so ordering inside the repeated hour is wrong and every elapsed-time
//! calculation across either transition is an hour out. Migration 32 adds the
//! instant; these gates pin the three things that have to be true of it.
//!
//! Every test here runs under a **fixed** `TZ`, set before the process opens
//! any connection, because the whole point of the column is that it depends on
//! the host's timezone rules. A test that inherited the runner's zone would
//! pass in UTC CI and prove nothing about the stations this is for.

use birdnet_db::migration::MIGRATIONS;
use rusqlite::Connection;

/// Berlin: `CET` (+1) in winter, `CEST` (+2) in summer. Fall-back 2026-10-25,
/// spring-forward 2026-03-29.
const TZ: &str = "Europe/Berlin";

/// The two real instants that Berlin's local 02:30 on 2026-10-25 names.
///
/// The offset moves +2 -> +1 at 01:00 UTC, so the hour is lived through twice:
/// once at 00:30Z while the clock still reads `CEST`, and again at 01:30Z under
/// `CET`. Checked against the tz database rather than derived here — a fixture
/// that computed these the same way the code does could not disagree with it.
const CEST_READING: i64 = 1_792_888_200;
const CET_READING: i64 = 1_792_891_800;

/// A migrated in-memory database, with `TZ` pinned for the whole process.
///
/// Callers gate on [`ensure_tz`] first, so every test in this file runs under
/// a known zone rather than the runner's.
fn db() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn
}

/// Insert a pre-migration-32-shaped row directly, so the backfill has something
/// to convert that the write path did not stamp.
fn raw_insert(conn: &Connection, date: &str, time: &str, sci: &str) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES (?1, ?2, ?3, 'Test Bird', 0.9)",
        rusqlite::params![date, time, sci],
    )
    .expect("insert");
}

fn utc_of(conn: &Connection, date: &str, time: &str) -> Option<i64> {
    conn.query_row(
        "SELECT detected_at_utc FROM detections WHERE Date = ?1 AND Time = ?2",
        rusqlite::params![date, time],
        |r| r.get(0),
    )
    .expect("row exists")
}

/// The station is on Berlin time, and the conversion has to use the offset that
/// was in force **on the detection's own date** — not today's.
///
/// This is the property that makes backfilling a decade of history worth doing
/// at all. Verified directly against SQLite before the migration was written:
/// the `'utc'` modifier consults the tz database for the given date.
#[test]
fn the_offset_used_is_the_one_in_force_on_that_date() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    raw_insert(&conn, "2026-01-15", "12:00:00", "Winter");
    raw_insert(&conn, "2026-07-15", "12:00:00", "Summer");

    let winter = utc_of(&conn, "2026-01-15", "12:00:00").expect("winter stamped");
    let summer = utc_of(&conn, "2026-07-15", "12:00:00").expect("summer stamped");

    // Local noon minus the offset gives the UTC second-of-day.
    assert_eq!(
        winter.rem_euclid(86_400),
        11 * 3600,
        "Berlin is UTC+1 in January, so local 12:00 is 11:00Z"
    );
    assert_eq!(
        summer.rem_euclid(86_400),
        10 * 3600,
        "Berlin is UTC+2 in July, so local 12:00 is 10:00Z — a backfill that \
         used today's offset for every row would put both on the same one"
    );
}

/// The reason the column exists, stated as an assertion: a duration measured
/// across a daylight-saving boundary is an hour wrong on wall clock and right
/// on the instant.
#[test]
fn an_elapsed_hour_across_the_spring_transition_is_only_right_on_the_instant() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    // 2026-03-29: Berlin jumps 02:00 CET -> 03:00 CEST. Local 01:30 and 03:30
    // are *one* real hour apart; the wall clock says two.
    raw_insert(&conn, "2026-03-29", "01:30:00", "Before");
    raw_insert(&conn, "2026-03-29", "03:30:00", "After");

    let a = utc_of(&conn, "2026-03-29", "01:30:00").expect("stamped");
    let b = utc_of(&conn, "2026-03-29", "03:30:00").expect("stamped");
    assert_eq!(
        b - a,
        3600,
        "one real hour separates them; the instant must say so"
    );

    // What the old arithmetic would have said, computed the same way the
    // analytics did — this is the number a 30-minute session gap was compared
    // against, and it is double.
    let wall: f64 = conn
        .query_row(
            "SELECT (julianday('2026-03-29 03:30:00') - julianday('2026-03-29 01:30:00')) * 86400.0",
            [],
            |r| r.get(0),
        )
        .expect("wall clock delta");
    assert!(
        (wall - 7200.0).abs() < 1.0,
        "wall clock reads two hours for a one-hour gap; got {wall}"
    );
}

/// The autumn transition, and the honest limit of a backfill.
///
/// Local 02:30 on 2026-10-25 in Berlin is two real instants, 00:30Z and 01:30Z.
/// A row that carries only `Date`/`Time` contains nothing that says which, so
/// the backfill picks one — `strftime` returns the later, standard-time
/// reading. This gate pins that it produces *a valid instant on that day*
/// rather than NULL or a nonsense value, and records which one, so a future
/// reader knows it was chosen rather than overlooked.
#[test]
fn the_repeated_autumn_hour_resolves_to_one_of_its_two_real_instants() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    raw_insert(&conn, "2026-10-25", "02:30:00", "Ambiguous");
    let v = utc_of(&conn, "2026-10-25", "02:30:00").expect("stamped, not NULL");

    assert!(
        v == CEST_READING || v == CET_READING,
        "must be one of the hour's two real instants; got {v}"
    );
    assert_eq!(
        v, CET_READING,
        "SQLite resolves the ambiguity to the standard-time reading — recorded \
         so a change in that behaviour is noticed rather than absorbed"
    );
}

/// A row whose `Date`/`Time` name no point in time must stay unplaceable, not
/// acquire a plausible-looking instant.
///
/// `Date`/`Time` are `TEXT NOT NULL`, which forbids NULL and not nonsense. The
/// BirdNET-Pi importer turns a NULL source date into `""`. What must not happen
/// is that such a row acquires a plausible-looking instant — being unplaceable
/// is information, and a fabricated timestamp would put the row into orderings
/// and gap calculations it has no business being in.
#[test]
fn rows_that_name_no_point_in_time_stay_null() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    raw_insert(&conn, "", "", "Empty");
    raw_insert(&conn, "not-a-date", "25:99:99", "Garbage");
    raw_insert(&conn, "2026-05-01", "06:00:00", "Fine");

    assert_eq!(utc_of(&conn, "", ""), None);
    assert_eq!(utc_of(&conn, "not-a-date", "25:99:99"), None);
    assert!(utc_of(&conn, "2026-05-01", "06:00:00").is_some());

    // And the rows are still there — unplaceable in, unplaceable out, never
    // dropped.
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(n, 3);
}

/// The trigger is the safety net for the write path nobody has written yet.
///
/// Every path in the tree today stamps the column itself. This asserts that one
/// which does not still gets a value, because the alternative — a row that is
/// silently absent from every ordering that uses the column — is exactly the
/// failure `species_summary`'s triggers were introduced to prevent.
#[test]
fn a_write_path_that_forgets_the_column_is_covered_by_the_trigger() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    // `raw_insert` names no `detected_at_utc` at all — the shape a future
    // caller would write.
    raw_insert(&conn, "2026-07-15", "12:00:00", "Forgotten");
    let v = utc_of(&conn, "2026-07-15", "12:00:00").expect("the trigger stamped it");
    assert_eq!(v.rem_euclid(86_400), 10 * 3600, "and stamped it correctly");
}

/// An explicit value must win over the trigger.
///
/// The live write path can do better than a tz-database lookup: it knows the
/// offset that was actually in force when the audio was captured, which is the
/// only way to tell the two passes of the repeated hour apart. That is worth
/// nothing if the trigger overwrites it.
#[test]
fn an_explicitly_stamped_instant_is_not_overwritten() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    // The *earlier* reading of the ambiguous hour — the one SQLite does not
    // pick — as a live capture would have recorded it.
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
         VALUES ('2026-10-25', '02:30:00', 'Live', 'Test Bird', 0.9, ?1)",
        rusqlite::params![CEST_READING],
    )
    .expect("insert");
    assert_eq!(
        utc_of(&conn, "2026-10-25", "02:30:00"),
        Some(CEST_READING),
        "the trigger must not clobber a caller that knew better"
    );
}

/// …and the production write path must actually carry it there.
///
/// [`an_explicitly_stamped_instant_is_not_overwritten`] inserts raw SQL, so it
/// proves the *trigger* yields — and would stay green if `insert_detection`
/// stopped binding the column at all, because the trigger would then stamp a
/// plausible value and nothing would look wrong. What would be lost is the only
/// thing the live path can do that the trigger cannot: tell the two passes of
/// the repeated autumn hour apart. So this goes through `insert_detection` and
/// asks for the reading SQLite does *not* pick.
#[test]
fn the_write_path_carries_an_explicit_instant_through_to_the_row() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    // `CEST_READING` is local 02:30 on its *first* pass, while Berlin is still
    // +2. The trigger's tz lookup resolves the same wall clock to `CET_READING`.
    let record = birdnet_db::sqlite::DetectionRecord {
        date: "2026-10-25",
        time: "02:30:00",
        sci_name: "Turdus merula",
        com_name: "Eurasian Blackbird",
        confidence: 0.9,
        lat: None,
        lon: None,
        cutoff: None,
        week: None,
        sensitivity: None,
        overlap: None,
        file_name: "live.wav",
        // NOT NULL in the schema — the fixture has to be a row the table will
        // actually take, or this tests the constraint and not the column.
        chunk_offset_secs: Some(0.0),
        correlation_id: None,
        source: None,
        duration_secs: None,
        detected_at_utc: Some(CEST_READING),
    };
    birdnet_db::sqlite::insert_detection(&conn, &record).expect("insert");

    let v = utc_of(&conn, "2026-10-25", "02:30:00").expect("stamped");
    assert_ne!(
        v, CET_READING,
        "the write path dropped the caller's instant and let the trigger guess"
    );
    assert_eq!(
        v, CEST_READING,
        "the caller's instant reached the row intact"
    );
}

// ---------------------------------------------------------------------------
// The backfill — history that predates the column
// ---------------------------------------------------------------------------

/// The migration that introduces `detected_at_utc`.
const UTC_MIGRATION: u32 = 32;

/// A database migrated only as far as *before* [`UTC_MIGRATION`].
///
/// # Why this is not just `db()` with an extra step
///
/// Every other test in this file inserts into a fully migrated database, which
/// exercises the **trigger** and would pass even if the backfill did not exist.
/// Two mutations proved exactly that: dropping `'utc'` from the backfill, and
/// removing its `datetime(...) IS NOT NULL` guard, both left the whole file
/// green — because the backfill never ran. History that predates the column is
/// the only thing that reaches it, so the only way to gate it is to build a
/// database that has some.
fn db_before_the_column() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .expect("pragma");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
             version INTEGER PRIMARY KEY,
             description TEXT NOT NULL,
             applied_at TEXT NOT NULL DEFAULT (datetime('now')));",
    )
    .expect("version table");
    let pre: Vec<_> = MIGRATIONS
        .iter()
        .take_while(|m| m.version < UTC_MIGRATION)
        .collect();
    assert_eq!(
        u32::try_from(pre.len()).expect("migration count"),
        UTC_MIGRATION - 1,
        "the migration list is not the contiguous 1..{UTC_MIGRATION} this assumes"
    );
    for m in pre {
        conn.execute_batch(m.up_sql)
            .unwrap_or_else(|e| panic!("migration {} failed: {e}", m.version));
        conn.execute(
            "INSERT INTO schema_version (version, description) VALUES (?1, ?2)",
            rusqlite::params![m.version, m.description],
        )
        .expect("stamp version");
    }
    assert!(
        conn.query_row("SELECT detected_at_utc FROM detections LIMIT 1", [], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .is_err(),
        "the column must not exist yet, or this proves nothing"
    );
    conn
}

/// The backfill converts pre-existing history with the offset that was in force
/// **on each row's own date** — the property that makes converting a decade of
/// it worth doing rather than stamping everything with today's offset.
#[test]
fn the_backfill_uses_each_rows_own_offset_not_todays() {
    if !ensure_tz() {
        return;
    }
    let conn = db_before_the_column();
    raw_insert(&conn, "2026-01-15", "12:00:00", "Winter");
    raw_insert(&conn, "2026-07-15", "12:00:00", "Summer");

    birdnet_db::migration::migrate(&conn).expect("migrate the rest");

    let winter = utc_of(&conn, "2026-01-15", "12:00:00").expect("winter backfilled");
    let summer = utc_of(&conn, "2026-07-15", "12:00:00").expect("summer backfilled");
    assert_eq!(
        winter.rem_euclid(86_400),
        11 * 3600,
        "Berlin is UTC+1 in January"
    );
    assert_eq!(
        summer.rem_euclid(86_400),
        10 * 3600,
        "Berlin is UTC+2 in July — a backfill that dropped the 'utc' modifier, \
         or used one offset for every row, would put these on the same one"
    );
    assert_ne!(
        winter.rem_euclid(86_400),
        summer.rem_euclid(86_400),
        "the two must not agree, or the conversion is not date-aware"
    );
}

/// A pre-existing row that names no point in time keeps a NULL instant through
/// the backfill, and the migration completes rather than aborting on it.
#[test]
fn the_backfill_leaves_unplaceable_history_unplaceable() {
    if !ensure_tz() {
        return;
    }
    let conn = db_before_the_column();
    raw_insert(&conn, "", "", "Empty");
    raw_insert(&conn, "not-a-date", "25:99:99", "Garbage");
    raw_insert(&conn, "2026-05-01", "06:00:00", "Fine");

    birdnet_db::migration::migrate(&conn).expect("migrate the rest");

    assert_eq!(utc_of(&conn, "", ""), None);
    assert_eq!(utc_of(&conn, "not-a-date", "25:99:99"), None);
    assert!(utc_of(&conn, "2026-05-01", "06:00:00").is_some());
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(n, 3, "no row is dropped by the conversion");
}

/// Elapsed time across the spring transition, on history that predates the
/// column — the case an existing station actually has.
#[test]
fn the_backfill_makes_an_old_dst_gap_measurable() {
    if !ensure_tz() {
        return;
    }
    let conn = db_before_the_column();
    raw_insert(&conn, "2026-03-29", "01:30:00", "Before");
    raw_insert(&conn, "2026-03-29", "03:30:00", "After");

    birdnet_db::migration::migrate(&conn).expect("migrate the rest");

    let a = utc_of(&conn, "2026-03-29", "01:30:00").expect("stamped");
    let b = utc_of(&conn, "2026-03-29", "03:30:00").expect("stamped");
    assert_eq!(b - a, 3600, "one real hour, on rows the trigger never saw");
}

// ---------------------------------------------------------------------------
// The queries that moved onto it
// ---------------------------------------------------------------------------

/// The deadman's freshness must be measured between two real instants.
///
/// It used to subtract two local wall clocks. Inside one offset regime that is
/// exact; across a daylight-saving transition it is not, and the error runs in
/// both directions — the station reads an hour fresher than it is on the autumn
/// night and an hour staler on the spring one.
///
/// This pins the arithmetic rather than the wording: a detection stamped an
/// exact hour ago must read as an hour, whatever the local clock did in between.
#[test]
fn freshness_is_measured_between_instants_not_wall_clocks() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    // A detection whose *wall clock* is 2026-10-25 02:30 — the repeated hour —
    // but whose instant is exactly one hour before now. The wall clock says
    // nothing useful about how long ago that was; the instant says everything.
    let now: i64 = conn
        .query_row("SELECT CAST(strftime('%s','now') AS INTEGER)", [], |r| {
            r.get(0)
        })
        .expect("now");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
         VALUES ('2026-10-25','02:30:00','Turdus merula','Blackbird',0.9, ?1)",
        rusqlite::params![now - 3600],
    )
    .expect("insert");

    let secs = birdnet_db::sqlite::seconds_since_last_detection(&conn)
        .expect("query")
        .expect("a value");
    assert!(
        (3595..=3605).contains(&secs),
        "an hour ago must read as an hour, not as whatever the wall clock implies; got {secs}s"
    );
}

/// A detection with no instant cannot date the station's last activity, and
/// saying nothing is better than saying something wrong — which is the
/// function's existing contract, restated now that the source column changed.
#[test]
fn freshness_says_nothing_when_nothing_placeable_has_been_heard() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    raw_insert(&conn, "not-a-date", "25:99:99", "Garbage");
    assert_eq!(
        birdnet_db::sqlite::seconds_since_last_detection(&conn).expect("query"),
        None
    );
}

/// "Most recent" is a chronological claim, and lexical order on `Date` is not
/// chronological — a garbage date sorts above every real one.
///
/// A single unplaceable imported row used to make itself the station's latest
/// detection, on the dashboard and in the freshness signal.
#[test]
fn the_latest_detection_is_the_newest_one_not_the_lexically_largest() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    raw_insert(&conn, "2026-05-01", "06:00:00", "Real");
    raw_insert(&conn, "not-a-date", "25:99:99", "Garbage");
    assert!(
        "not-a-date" > "2026-05-01",
        "the premise: the garbage date sorts higher lexically"
    );

    let (date, _time, _com) = birdnet_db::sqlite::latest_detection(&conn)
        .expect("query")
        .expect("a row");
    assert_eq!(
        date, "2026-05-01",
        "the newest real detection, not the largest string"
    );
}

/// The analytic view is `SELECT *`, and SQLite re-expands that at query time —
/// verified directly before relying on it. So `detections_analytic` picks up
/// `detected_at_utc` with no change to migration 26.
///
/// Gated because a later migration that pins the view's column list would break
/// `latest_detection` silently: the ORDER BY would fail to resolve and every
/// caller would see an error where it used to see a detection.
#[test]
fn the_analytic_view_carries_the_instant() {
    if !ensure_tz() {
        return;
    }
    let conn = db();
    raw_insert(&conn, "2026-05-01", "06:00:00", "Real");
    let v: Option<i64> = conn
        .query_row(
            "SELECT detected_at_utc FROM detections_analytic LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("the view exposes the column");
    assert!(v.is_some());
}

// ---------------------------------------------------------------------------
// TZ harness
// ---------------------------------------------------------------------------

/// Guards the re-exec below, so it happens once per process rather than once
/// per test thread.
static RE_EXEC: std::sync::Once = std::sync::Once::new();

/// Ensure the process is running under [`TZ`], re-exec'ing once if it is not.
///
/// # Why a re-exec, of all things
///
/// The column's whole purpose is that it depends on the host's timezone rules,
/// so a test that inherited the runner's zone would pass under CI's UTC and
/// prove nothing about the stations this is for. Pinning the zone means setting
/// `TZ` before the process calls `tzset`, and `std::env::set_var` is `unsafe`
/// in edition 2024 while this workspace sets `unsafe_code = "forbid"`. That
/// leaves being *launched* with it.
///
/// So on the first call in a process with the wrong zone, this re-runs the test
/// binary with `TZ` set, forwarding the same arguments, and exits with the
/// child's status. The child sees the right zone and returns immediately.
///
/// [`RE_EXEC`] matters: without it every test thread re-execs, and the suite
/// runs once per thread. Threads that arrive after the first block inside
/// `call_once` and never leave it, because the closure ends in
/// [`std::process::exit`] — which is the intent, not an oversight.
///
/// Returns `false` only if the re-exec could not be attempted at all, in which
/// case the callers skip rather than assert against an unknown timezone.
fn ensure_tz() -> bool {
    if std::env::var("TZ").as_deref() == Ok(TZ) {
        return true;
    }
    RE_EXEC.call_once(|| {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let status = std::process::Command::new(exe)
            .args(std::env::args().skip(1))
            .env("TZ", TZ)
            .status();
        if let Ok(status) = status {
            std::process::exit(i32::from(!status.success()));
        }
    });
    false
}
