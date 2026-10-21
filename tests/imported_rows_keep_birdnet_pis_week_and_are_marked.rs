//! A BirdNET-Pi row keeps the `Week` BirdNET-Pi wrote, and the import says so.
//!
//! BirdNET-Pi stores the ISO week of the recording (8 for 16 February 2026, 53
//! for 31 December 2026). This station stores the week of its own 48-week year
//! (four per month): 7 and 48 for the same two days. The import decision is to
//! leave the imported number exactly as BirdNET-Pi wrote it, so an export of the
//! imported rows matches the old install's, and to mark the row as imported
//! (`import_batch_id` is not null) so a query that groups by week can leave the
//! imported rows out or convert them from `Date`.
//!
//! What this gate holds:
//!
//! * the imported `Week` is BirdNET-Pi's, not this station's;
//! * the two calendars really do disagree on those dates (so the guide's
//!   warning is about something, and a future change that made the importer
//!   convert would have to change this test on purpose);
//! * an imported row can be told from a native one by `import_batch_id`.
//!
//! The counterpart: a row this station recorded has no batch, so "imported" is
//! a discriminator and not a blanket mark.

use birdnet_core::civil::birdnet_week_from_date;
use birdnet_migrate::ImportOptions;
use birdnet_migrate::birdnet_pi::importer::BirdNetPiImporter;
use birdnet_migrate::progress::ProgressHandle;
use rusqlite::Connection;

/// `(date, the ISO week BirdNET-Pi wrote, the week this station would write)`.
const DAYS: [(&str, i64, u32); 2] = [("2026-02-16", 8, 7), ("2026-12-31", 53, 48)];

fn pi_install(path: &std::path::Path) {
    let conn = Connection::open(path).expect("source");
    conn.execute_batch(
        "CREATE TABLE detections (
            Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
            Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
            Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
    )
    .expect("schema");
    for (i, (date, week, _)) in DAYS.iter().enumerate() {
        conn.execute(
            "INSERT INTO detections VALUES (?1, '06:00:00', 'Pica pica', 'Eurasian Magpie', \
             0.912, 51.5, -0.1, 0.7, ?2, 1.25, 0.0, ?3)",
            rusqlite::params![date, week, format!("magpie-{i}.wav")],
        )
        .expect("row");
    }
}

#[test]
fn the_imported_week_is_birdnet_pis_and_the_row_says_it_was_imported() {
    let dir = tempfile::tempdir().expect("dir");
    let src = dir.path().join("birds.db.pi");
    let dst = dir.path().join("birds.db");
    pi_install(&src);

    // Precondition: the two calendars disagree on both days, or the rest of
    // this test would pass whatever the importer did.
    for (date, iso, ours) in DAYS {
        assert_eq!(birdnet_week_from_date(date), Some(ours), "{date}");
        assert_ne!(i64::from(ours), iso, "{date}: the calendars must differ here");
    }

    let summary = BirdNetPiImporter
        // The entry the admin page uses: it tags what it imports. (The bare
        // `Migrator::migrate` does not, and is not what the page calls.)
        .migrate_with_options(
            &src,
            &dst,
            &ProgressHandle::new(),
            &ImportOptions::default(),
            (None, None),
        )
        .expect("import");
    assert_eq!(summary.imported_rows, 2);

    let conn = Connection::open(&dst).expect("dest");
    for (date, iso, _) in DAYS {
        let (week, batch): (i64, Option<i64>) = conn
            .query_row(
                "SELECT Week, import_batch_id FROM detections WHERE Date = ?1",
                [date],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("imported row");
        assert_eq!(week, iso, "{date}: the importer must keep BirdNET-Pi's week");
        assert!(batch.is_some(), "{date}: an imported row carries its batch");
    }

    // The counterpart: this station's own row has no batch.
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, \
         Sens, Overlap, File_Name, chunk_offset_secs) \
         VALUES ('2026-02-16', '07:00:00', 'Pica pica', 'Eurasian Magpie', 0.9, 0.7, 7, 1.25, 0.0, \
         'native.wav', 0)",
        [],
    )
    .expect("native row");
    let natives: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM detections WHERE import_batch_id IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(natives, 1, "only the row this station recorded has no batch");
}
