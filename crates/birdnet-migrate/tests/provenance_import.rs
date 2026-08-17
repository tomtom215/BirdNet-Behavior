//! Importing another station's history must stay attributable afterwards.
//!
//! The failure this guards is not a crash — it is a dataset that looks fine.
//! Before migration 25 and `migrate_with_options`, importing a BirdNET-Pi
//! database from a different site produced one table holding two sites and two
//! clocks, with no column able to separate them and no check that mentioned
//! either. Every location- and hour-dependent analytic then read the union as a
//! single station.
//!
//! For a research station the damage is unrecoverable rather than merely wrong:
//! once the rows are indistinguishable, no later query can undo the merge. These
//! tests hold the line at the only moment it can be held — import time.

use birdnet_migrate::birdnet_pi::BirdNetPiImporter;
use birdnet_migrate::progress::ProgressHandle;
use birdnet_migrate::provenance::{ImportOptions, SourceProfile};
use rusqlite::Connection;
use tempfile::TempDir;

/// A BirdNET-Pi source recorded at `(lat, lon)`, one detection per day at the
/// given local hour.
fn source_at(
    dir: &TempDir,
    name: &str,
    lat: f64,
    lon: f64,
    hour: u32,
    days: u32,
) -> std::path::PathBuf {
    let path = dir.path().join(name);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
         Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
         Sens REAL, Overlap REAL, File_Name TEXT);",
    )
    .unwrap();
    for d in 1..=days {
        conn.execute(
            "INSERT INTO detections VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird',
             0.9, ?3, ?4, 0.7, 1, 1.0, 0.0, ?5)",
            rusqlite::params![
                format!("2026-03-{d:02}"),
                format!("{hour:02}:30:00"),
                lat,
                lon,
                format!("rec-{name}-{d}.wav")
            ],
        )
        .unwrap();
    }
    path
}

fn dest(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().join("birds.db");
    let conn = birdnet_db::sqlite::open_or_create(&path).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    path
}

/// Imported rows carry a batch; locally-recorded rows stay NULL.
///
/// The NULL half is the load-bearing one. If the column defaulted to anything
/// else, every row already on the station would be reclassified by an upgrade.
#[test]
fn imported_rows_are_tagged_and_local_rows_are_not() {
    let dir = TempDir::new().unwrap();
    let dst = dest(&dir);
    {
        let conn = Connection::open(&dst).unwrap();
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-03-01','05:00:00','Erithacus rubecula','European Robin',0.8)",
            [],
        )
        .unwrap();
    }
    let src = source_at(&dir, "far.db", 48.8566, 2.3522, 6, 3);

    BirdNetPiImporter
        .migrate_with_options(
            &src,
            &dst,
            &ProgressHandle::new(),
            &ImportOptions {
                label: Some("Paris garden".into()),
                ..Default::default()
            },
            (Some(51.5074), Some(-0.1278)),
        )
        .unwrap();

    let conn = Connection::open(&dst).unwrap();
    let tagged: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM detections WHERE import_batch_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let local: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM detections WHERE import_batch_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tagged, 3, "imported rows must be attributable");
    assert_eq!(local, 1, "the station's own recording must stay untagged");

    let (label, km_apart, source_latitude): (String, f64, f64) = conn
        .query_row(
            "SELECT source_label, distance_km, source_lat FROM import_batches",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(label, "Paris garden");
    assert!(
        (km_apart - 343.0).abs() < 5.0,
        "the batch records how far apart the two sites are: {km_apart}"
    );
    assert!((source_latitude - 48.857).abs() < 0.01);
}

/// A clock offset given at import is applied once, to every row.
///
/// BirdNET-Pi records local wall-clock with no offset, so without this a
/// history from UTC−5 imported into a UTC+1 station lands six hours out and
/// every hour-of-day analytic silently averages two clocks.
#[test]
fn a_clock_offset_is_applied_to_every_imported_timestamp() {
    let dir = TempDir::new().unwrap();
    let dst = dest(&dir);
    // Source recorded at 06:30 local, six hours behind this station.
    let src = source_at(&dir, "west.db", 40.7128, -74.006, 6, 3);

    BirdNetPiImporter
        .migrate_with_options(
            &src,
            &dst,
            &ProgressHandle::new(),
            &ImportOptions {
                shift_secs: 6 * 3600,
                source_utc_offset_secs: Some(-5 * 3600),
                ..Default::default()
            },
            (Some(51.5074), Some(-0.1278)),
        )
        .unwrap();

    let conn = Connection::open(&dst).unwrap();
    let times: Vec<String> = conn
        .prepare("SELECT Time FROM detections ORDER BY Date")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(
        times,
        vec!["12:30:00", "12:30:00", "12:30:00"],
        "06:30 at the source, +6h, must be 12:30 in this station's clock"
    );

    let applied: i64 = conn
        .query_row("SELECT applied_shift_secs FROM import_batches", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        applied,
        6 * 3600,
        "the shift is recorded, so it is reversible"
    );
}

/// A shift that crosses midnight must move the date too.
///
/// The case a hand-rolled hour-only adjustment gets wrong, and the reason the
/// shift goes through SQLite's own date arithmetic.
#[test]
fn a_shift_across_midnight_moves_the_date() {
    let dir = TempDir::new().unwrap();
    let dst = dest(&dir);
    // 22:30 local at the source, shifted +3h → 01:30 the *next* day.
    let src = source_at(&dir, "late.db", 40.7128, -74.006, 22, 1);

    BirdNetPiImporter
        .migrate_with_options(
            &src,
            &dst,
            &ProgressHandle::new(),
            &ImportOptions {
                shift_secs: 3 * 3600,
                ..Default::default()
            },
            (None, None),
        )
        .unwrap();

    let conn = Connection::open(&dst).unwrap();
    let (date, time): (String, String) = conn
        .query_row("SELECT Date, Time FROM detections", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(
        (date.as_str(), time.as_str()),
        ("2026-03-02", "01:30:00"),
        "22:30 on the 1st plus three hours is 01:30 on the 2nd"
    );
}

/// Rows whose Date/Time name no point in time survive a shift unchanged.
///
/// They exist in real BirdNET-Pi databases (a NULL `Date` arrives as `""`) and
/// are already excluded from every time-bucketed analytic. Rewriting them to
/// some epoch would turn "unplaceable" into "placed, wrongly" — a worse state,
/// because it is no longer visible as a gap.
#[test]
fn unplaceable_rows_are_not_invented_into_existence_by_a_shift() {
    let dir = TempDir::new().unwrap();
    let dst = dest(&dir);
    let src = dir.path().join("dirty.db");
    {
        let conn = Connection::open(&src).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
             Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections VALUES ('','','Parus major','Great Tit',0.8,NULL,NULL,NULL,NULL,NULL,NULL,'a.wav');
             INSERT INTO detections VALUES ('2026-03-01','06:30:00','Turdus merula','Blackbird',0.9,NULL,NULL,NULL,NULL,NULL,NULL,'b.wav');",
        )
        .unwrap();
    }

    BirdNetPiImporter
        .migrate_with_options(
            &src,
            &dst,
            &ProgressHandle::new(),
            &ImportOptions {
                shift_secs: 3600,
                ..Default::default()
            },
            (None, None),
        )
        .unwrap();

    let conn = Connection::open(&dst).unwrap();
    let bad: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections WHERE Date = ''", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        bad, 1,
        "the unplaceable row is still unplaceable, not relocated"
    );
    let good: String = conn
        .query_row(
            "SELECT Time FROM detections WHERE Sci_Name = 'Turdus merula'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(good, "07:30:00", "the placeable row still shifted");
}

/// Profiling a source reports the site it was actually recorded at.
#[test]
fn a_source_profile_names_the_site_and_the_distance() {
    let dir = TempDir::new().unwrap();
    let src = source_at(&dir, "profile.db", 48.8566, 2.3522, 6, 4);
    let p = SourceProfile::read(&src).unwrap();
    assert_eq!(p.located_rows, 4);
    assert_eq!(p.distinct_sites, 1);
    assert_eq!(p.first_date.as_deref(), Some("2026-03-01"));
    let d = p.distance_km_to(Some(51.5074), Some(-0.1278)).unwrap();
    assert!((d - 343.0).abs() < 5.0, "d={d}");
}

/// The default path shifts nothing and still records provenance.
#[test]
fn a_same_site_import_shifts_nothing_but_is_still_attributable() {
    let dir = TempDir::new().unwrap();
    let dst = dest(&dir);
    let src = source_at(&dir, "same.db", 51.5074, -0.1278, 6, 2);

    BirdNetPiImporter
        .migrate_with_options(
            &src,
            &dst,
            &ProgressHandle::new(),
            &ImportOptions::default(),
            (Some(51.5074), Some(-0.1278)),
        )
        .unwrap();

    let conn = Connection::open(&dst).unwrap();
    let time: String = conn
        .query_row("SELECT Time FROM detections LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(time, "06:30:00", "no shift means no change");
    let (km_apart, rows): (f64, i64) = conn
        .query_row(
            "SELECT distance_km, row_count FROM import_batches",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(km_apart < 0.1, "same site, distance ~0: {km_apart}");
    assert_eq!(rows, 2, "the batch records how many rows it brought");
}
