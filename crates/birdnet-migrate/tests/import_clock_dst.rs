//! Importing a history from another zone must survive daylight saving.
//!
//! # What this is defending
//!
//! BirdNET-Pi stores `Date`/`Time` as local wall clock with no offset recorded,
//! so a history from a station on a different clock has to be moved onto this
//! one at import — there is nowhere to keep the source's offset per row.
//!
//! The importer used to do that with a single number: `destination_offset_now −
//! source_offset`, added to every row. Both halves of that are frozen at the
//! moment of import, and a history is years long, so the result was an hour out
//! for however much of it fell under a different daylight-saving regime — which
//! for a multi-year import is roughly half of it.
//!
//! It now converts per row: source wall clock minus the source's offset is the
//! real instant, and SQLite renders that instant in the host's zone *for that
//! instant*. The destination half is then exact on both sides of every
//! transition. The source half is still one constant, because a constant is all
//! the operator gives us — see `to_local_here`.
//!
//! # Why this re-executes itself
//!
//! The behaviour under test is the *host's* zone rules, and CI runs in UTC,
//! where a January and a July timestamp get the same offset and any assertion
//! about daylight saving would pass vacuously. A `#[ignore]`d worker test does
//! the real work, and the visible test re-runs this same binary with
//! `TZ=Europe/Berlin` set on the child. That keeps the zone out of the parent
//! process — `std::env::set_var` is `unsafe` on this edition and the workspace
//! forbids `unsafe` — and, more importantly, means the gate cannot quietly
//! degrade to a skip on the machine that actually runs it.

use birdnet_migrate::birdnet_pi::BirdNetPiImporter;
use birdnet_migrate::progress::ProgressHandle;
use birdnet_migrate::provenance::ImportOptions;
use rusqlite::Connection;

/// The zone the worker needs: one that observes daylight saving, with offsets
/// this file can state exactly.
const WORKER_TZ: &str = "Europe/Berlin";

/// Run this binary's `#[ignore]`d worker with `TZ` set, and fail with its output.
fn run_worker(name: &str) {
    let exe = std::env::current_exe().expect("test binary path");
    let out = std::process::Command::new(exe)
        .env("TZ", WORKER_TZ)
        .args([
            "--exact",
            name,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .output()
        .expect("re-run this test binary");

    assert!(
        out.status.success(),
        "worker `{name}` failed under TZ={WORKER_TZ}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    // A worker that ran zero tests would "succeed" without asserting anything —
    // the precise failure this file's whole structure exists to avoid.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1 passed"),
        "worker `{name}` did not actually run:\n{stdout}"
    );
}

/// The host's UTC offset at a given instant, as SQLite sees it.
fn offset_at(instant: &str) -> i64 {
    let conn = Connection::open_in_memory().expect("open");
    conn.query_row(
        "SELECT CAST(ROUND((julianday(?1, 'localtime') - julianday(?1)) * 86400.0) AS INTEGER)",
        [instant],
        |r| r.get(0),
    )
    .expect("offset")
}

/// Import `rows` from a source on `src_offset`, and return the stored
/// `Date Time` strings. Goes through the real importer, not a reimplementation.
fn imported(dir: &std::path::Path, rows: &[(&str, &str)], src_offset: i64) -> Vec<String> {
    let src_path = dir.join("source.db");
    let dst_path = dir.join("birds.db");

    let src = Connection::open(&src_path).expect("source");
    src.execute_batch(
        "CREATE TABLE detections (
            Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT, Confidence REAL,
            Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER, Sens REAL, Overlap REAL,
            File_Name TEXT);",
    )
    .expect("schema");
    for (i, (d, t)) in rows.iter().enumerate() {
        src.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES (?1, ?2, 'Erithacus rubecula', 'European Robin', 0.9, ?3)",
            rusqlite::params![d, t, format!("clip-{i}.wav")],
        )
        .expect("seed");
    }
    drop(src);
    drop(birdnet_db::sqlite::open_or_create(&dst_path).expect("dest"));

    BirdNetPiImporter
        .migrate_with_options(
            &src_path,
            &dst_path,
            &ProgressHandle::new(),
            &ImportOptions {
                label: Some("elsewhere".to_string()),
                source_utc_offset_secs: Some(src_offset),
                ..Default::default()
            },
            (None, None),
        )
        .expect("import");

    let dst = Connection::open(&dst_path).expect("reopen");
    let mut stmt = dst
        .prepare("SELECT Date || ' ' || Time FROM detections ORDER BY File_Name")
        .expect("prepare");
    stmt.query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

// ── The visible tests: run the workers under a daylight-saving zone ─────────

#[test]
fn a_history_crossing_daylight_saving_is_converted_per_row() {
    run_worker("worker_dst_conversion");
}

#[test]
fn the_flat_shift_this_replaced_cannot_do_it() {
    run_worker("worker_flat_shift_is_wrong");
}

// ── Workers ────────────────────────────────────────────────────────────────

/// Observed failing against the flat shift the importer used to apply: on a
/// `Europe/Berlin` host importing a UTC+0 source, the January row came back as
/// 08:00 when the truth is 07:00. `worker_flat_shift_is_wrong` keeps that fact
/// asserted rather than only recorded here.
#[test]
#[ignore = "run by a_history_crossing_daylight_saving_is_converted_per_row, under TZ"]
fn worker_dst_conversion() {
    assert_eq!(offset_at("2024-01-15 12:00:00"), 3600, "January is CET");
    assert_eq!(offset_at("2024-07-15 12:00:00"), 7200, "July is CEST");

    let tmp = tempfile::tempdir().unwrap();
    let out = imported(
        tmp.path(),
        &[
            ("2024-01-15", "06:00:00"), // deep winter
            ("2024-07-15", "06:00:00"), // deep summer
            ("2024-03-31", "00:30:00"), // the hour before spring forward
            ("2024-10-27", "01:30:00"), // inside the repeated autumn hour
        ],
        0,
    );

    assert_eq!(
        out,
        vec![
            "2024-01-15 07:00:00".to_string(),
            "2024-07-15 08:00:00".to_string(),
            "2024-03-31 01:30:00".to_string(),
            "2024-10-27 02:30:00".to_string(),
        ],
        "each row must be converted with the offset in force on its own date"
    );

    // The property behind the strings, stated on its own so a future zone-rule
    // change fails with the reason rather than with four opaque diffs.
    let hour = |s: &str| -> i64 { s[11..13].parse().expect("hour") };
    assert_ne!(
        hour(&out[0]),
        hour(&out[1]),
        "a winter and a summer detection from one source clock cannot land on \
         the same local hour — that is exactly what a constant shift does"
    );
}

/// The counterpart to the conversion, kept as its own worker so the claim in the
/// module doc is asserted rather than asserted-about: a constant shift gives
/// winter and summer the same offset, whatever constant you pick.
#[test]
#[ignore = "run by the_flat_shift_this_replaced_cannot_do_it, under TZ"]
fn worker_flat_shift_is_wrong() {
    let conn = Connection::open_in_memory().expect("open");
    let flat = |date: &str, secs: i64| -> String {
        conn.query_row(
            "SELECT strftime('%Y-%m-%d %H:%M:%S', datetime(?1 || ' 06:00:00', ?2))",
            rusqlite::params![date, format!("{secs} seconds")],
            |r| r.get(0),
        )
        .expect("flat")
    };
    // Whatever "today's offset" happens to be when the import runs.
    for shift in [3600, 7200] {
        let winter = flat("2024-01-15", shift);
        let summer = flat("2024-07-15", shift);
        assert_eq!(
            &winter[11..13],
            &summer[11..13],
            "a flat shift of {shift}s must give both the same hour"
        );
        // And therefore exactly one of them is wrong, because the truth differs.
        let truth_differs = offset_at("2024-01-15 12:00:00") != offset_at("2024-07-15 12:00:00");
        assert!(
            truth_differs,
            "this worker is only meaningful in a zone that observes daylight saving"
        );
    }
}

// ── Zone-independent behaviour, asserted in the parent ──────────────────────

/// A source on the *same* clock as this station is not moved at all — true in
/// every zone, so it needs no worker. Without this the conversion could be
/// "always shifts something" and still pass the gates above.
#[test]
fn a_source_on_this_stations_clock_is_left_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let here = offset_at("2024-01-15 06:00:00");
    let out = imported(tmp.path(), &[("2024-01-15", "06:00:00")], here);
    assert_eq!(
        out,
        vec!["2024-01-15 06:00:00".to_string()],
        "a source keeping this station's clock must import unchanged"
    );
}

/// A row whose wall clock names no point in time survives the import rather than
/// being dropped or turned into nonsense. BirdNET-Pi's columns are free-form
/// `TEXT` and real databases carry values like these — migration 32's comment
/// records the same fact from the other end.
#[test]
fn an_unparseable_timestamp_is_carried_through_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let out = imported(
        tmp.path(),
        &[("not-a-date", "25:99:99"), ("2024-01-15", "06:00:00")],
        3600,
    );
    assert!(
        out.iter().any(|r| r.starts_with("not-a-date")),
        "the unplaceable row must still be imported: {out:?}"
    );
    assert_eq!(out.len(), 2);
}
