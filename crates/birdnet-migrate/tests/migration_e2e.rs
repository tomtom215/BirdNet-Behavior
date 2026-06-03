//! End-to-end integration tests for the BirdNET-Pi → BirdNet-Behavior import.
//!
//! These exercise the public migration API exactly as an external caller (the
//! web admin endpoint, the CLI) would: build a *fixture legacy database*,
//! run the full detect → validate → import pipeline, then **open the
//! destination and assert the rows actually landed**. The in-crate unit tests
//! check the orchestration and summary counts; these tests close the gap by
//! verifying the destination contents, schema completeness, idempotency, value
//! normalisation, and the CSV path — as a black box.
//!
//! `birdnet-migrate` is pure SQLite (no DuckDB / ONNX), so this whole file
//! compiles and runs in CI without the analytics feature or the model.

use birdnet_migrate::DetectedSchema;
use birdnet_migrate::birdnet_pi::{self, run_migration};
use birdnet_migrate::progress::ProgressHandle;
use rusqlite::{Connection, params};
use tempfile::NamedTempFile;

/// One fixture detection row. `confidence` is a raw SQL fragment (`"0.9"`,
/// `"NULL"`, `"1.5"`) so tests can exercise NULL and out-of-range values that a
/// real BirdNET-Pi export might contain.
type LegacyRow<'a> = (&'a str, &'a str, &'a str, &'a str, &'a str);

/// Build a minimal BirdNET-Pi `detections` SQLite database (the 12-column
/// legacy schema the detector recognises) populated with `rows`.
fn write_legacy_pi_db(rows: &[LegacyRow]) -> NamedTempFile {
    let tmp = NamedTempFile::new().unwrap();
    let conn = Connection::open(tmp.path()).unwrap();
    conn.execute_batch(
        "CREATE TABLE detections (
            Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
            Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
            Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
    )
    .unwrap();
    for (date, time, sci, com, confidence) in rows {
        // `confidence` is a trusted in-test literal (incl. `NULL`); the rest are
        // bound parameters. A non-null `File_Name` is set deliberately: real
        // BirdNET-Pi rows always carry the clip filename, and it is part of the
        // destination's UNIQUE key — leaving it NULL would make SQLite treat
        // every row as distinct (NULLs never compare equal in a UNIQUE index),
        // so `INSERT OR IGNORE` could not dedupe and idempotency would not hold.
        conn.execute(
            &format!(
                "INSERT INTO detections
                   (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES (?1, ?2, ?3, ?4, {confidence}, ?5)"
            ),
            params![date, time, sci, com, format!("{date}/{time}.wav")],
        )
        .unwrap();
    }
    drop(conn);
    tmp
}

/// Count rows in the destination `detections` table.
fn dest_count(path: &std::path::Path) -> i64 {
    let conn = Connection::open(path).unwrap();
    conn.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn sqlite_import_lands_rows_in_destination() {
    let src = write_legacy_pi_db(&[
        (
            "2026-01-05",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            "0.91",
        ),
        (
            "2026-01-05",
            "06:31:00",
            "Erithacus rubecula",
            "European Robin",
            "0.88",
        ),
        (
            "2026-01-06",
            "07:00:00",
            "Turdus merula",
            "Eurasian Blackbird",
            "0.77",
        ),
    ]);
    let dst = NamedTempFile::new().unwrap();
    let progress = ProgressHandle::new();

    let summary =
        run_migration(src.path(), dst.path(), false, &progress).expect("migration failed");
    assert_eq!(summary.source_rows, 3);
    assert_eq!(summary.imported_rows, 3);
    assert_eq!(summary.skipped_rows, 0);
    assert_eq!(summary.schema_name, "BirdNET-Pi");

    // The rows actually landed in the destination, values intact.
    assert_eq!(dest_count(dst.path()), 3);
    let dst_conn = Connection::open(dst.path()).unwrap();
    let (com, conf): (String, f64) = dst_conn
        .query_row(
            "SELECT Com_Name, Confidence FROM detections
             WHERE Sci_Name = ?1 ORDER BY Confidence DESC LIMIT 1",
            params!["Turdus merula"],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(com, "Eurasian Blackbird");
    assert!((conf - 0.91).abs() < 1e-9);

    // The destination is the FULL migrated schema (open_or_create runs every
    // migration), not a bare copy of the 12 legacy columns.
    let col_count: i64 = dst_conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('detections')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        col_count > 12,
        "destination should carry the full migrated schema (>12 columns), got {col_count}"
    );
}

#[test]
fn import_is_idempotent() {
    let src = write_legacy_pi_db(&[
        ("2026-02-01", "06:00:00", "Parus major", "Great Tit", "0.8"),
        ("2026-02-01", "06:05:00", "Parus major", "Great Tit", "0.6"),
    ]);
    let dst = NamedTempFile::new().unwrap();
    let progress = ProgressHandle::new();

    let first = run_migration(src.path(), dst.path(), false, &progress).unwrap();
    assert_eq!(first.imported_rows, 2);
    assert_eq!(first.skipped_rows, 0);

    // Re-importing the same source must not duplicate (INSERT OR IGNORE).
    let second = run_migration(src.path(), dst.path(), false, &progress).unwrap();
    assert_eq!(
        second.imported_rows, 0,
        "re-import must not insert new rows"
    );
    assert_eq!(second.skipped_rows, 2);

    assert_eq!(
        dest_count(dst.path()),
        2,
        "row count must be stable across re-imports"
    );
}

#[test]
fn confidence_is_clamped_and_null_becomes_zero() {
    // A real BirdNET-Pi export can carry out-of-range or NULL confidences; the
    // importer normalises them to the [0, 1] contract the new schema expects.
    let src = write_legacy_pi_db(&[
        (
            "2026-03-01",
            "06:00:00",
            "Corvus corax",
            "Common Raven",
            "1.5",
        ),
        (
            "2026-03-01",
            "06:01:00",
            "Corvus corax",
            "Common Raven",
            "NULL",
        ),
        (
            "2026-03-01",
            "06:02:00",
            "Corvus corax",
            "Common Raven",
            "-0.2",
        ),
    ]);
    let dst = NamedTempFile::new().unwrap();
    let progress = ProgressHandle::new();
    run_migration(src.path(), dst.path(), false, &progress).unwrap();

    let dst_conn = Connection::open(dst.path()).unwrap();
    let mut stmt = dst_conn
        .prepare("SELECT Confidence FROM detections ORDER BY Time")
        .unwrap();
    let confs: Vec<f64> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<f64>>>()
        .unwrap();
    assert_eq!(confs.len(), 3);
    assert!((confs[0] - 1.0).abs() < 1e-9, "1.5 must clamp to 1.0");
    assert!((confs[1] - 0.0).abs() < 1e-9, "NULL must become 0.0");
    assert!((confs[2] - 0.0).abs() < 1e-9, "-0.2 must clamp to 0.0");
}

#[test]
fn csv_import_lands_rows_in_destination() {
    use std::io::Write as _;

    let mut csv = NamedTempFile::with_suffix(".csv").unwrap();
    writeln!(
        csv,
        "Date,Time,Sci_Name,Com_Name,Confidence,Lat,Lon,Cutoff,Week,Sens,Overlap,File_Name"
    )
    .unwrap();
    writeln!(
        csv,
        "2026-04-01,05:30:00,Luscinia megarhynchos,Common Nightingale,0.95,,,,,,,"
    )
    .unwrap();
    writeln!(
        csv,
        "2026-04-01,05:31:00,Luscinia megarhynchos,Common Nightingale,0.61,,,,,,,"
    )
    .unwrap();
    csv.flush().unwrap();

    let dst = NamedTempFile::new().unwrap();
    let progress = ProgressHandle::new();
    let summary =
        run_migration(csv.path(), dst.path(), false, &progress).expect("csv migration failed");
    assert_eq!(summary.imported_rows, 2);

    let dst_conn = Connection::open(dst.path()).unwrap();
    let count: i64 = dst_conn
        .query_row(
            "SELECT COUNT(*) FROM detections WHERE Sci_Name = ?1",
            params!["Luscinia megarhynchos"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn validate_source_previews_without_importing() {
    let src = write_legacy_pi_db(&[
        (
            "2026-05-01",
            "06:00:00",
            "Sturnus vulgaris",
            "Common Starling",
            "0.7",
        ),
        (
            "2026-05-02",
            "06:00:00",
            "Sturnus vulgaris",
            "Common Starling",
            "0.7",
        ),
    ]);

    let (schema, report, migration_report) =
        birdnet_pi::validate_source(src.path()).expect("validation failed");

    assert!(matches!(schema, DetectedSchema::BirdNetPi { row_count: 2 }));
    assert!(
        report.passed,
        "a clean 2-row fixture should pass validation"
    );
    assert_eq!(report.source_rows, 2);
    assert_eq!(migration_report.total_rows, 2);
}
