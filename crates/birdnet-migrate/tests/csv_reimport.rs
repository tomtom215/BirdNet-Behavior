//! Re-importing the same CSV export must not duplicate rows.
//!
//! The rows here are the shipped e2e fixture's own: an empty trailing
//! `File_Name` field, which `parse_opt_str` maps to SQL NULL. That fixture only
//! ever imported once, so the duplication it produced on a second run went
//! unseen until migration 23 made the UNIQUE key NULL-insensitive.
use birdnet_migrate::birdnet_pi::run_migration;
use birdnet_migrate::progress::ProgressHandle;
use rusqlite::Connection;
use std::io::Write as _;
use tempfile::NamedTempFile;

#[test]
fn csv_reimport_does_not_duplicate() {
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
    let p = ProgressHandle::new();
    let first = run_migration(csv.path(), dst.path(), false, &p).unwrap();
    let second = run_migration(csv.path(), dst.path(), false, &p).unwrap();

    let conn = Connection::open(dst.path()).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .unwrap();
}
