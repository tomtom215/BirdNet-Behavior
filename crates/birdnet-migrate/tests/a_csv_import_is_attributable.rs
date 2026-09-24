//! A CSV import is as attributable as a database import.
//!
//! `run_migration_with_options` — the path both web upload handlers take —
//! sent any file that was not SQLite to `CsvImporter::migrate` and dropped the
//! options and station on the floor. So a `BirdDB.txt` from another site
//! arrived with no `import_batches` row and every row's `import_batch_id`
//! NULL: counted as this station's own recordings, not excluded by
//! `analytics_exclude_imports`, impossible to remove with
//! `delete_import_batch`, and with the operator's clock shift and label
//! silently ignored.

use birdnet_migrate::birdnet_pi::run_migration_with_options;
use birdnet_migrate::progress::ProgressHandle;
use birdnet_migrate::provenance::ImportOptions;
use rusqlite::Connection;
use tempfile::TempDir;

const BIRDDB: &str = "\
Date;Time;Sci_Name;Com_Name;Confidence;Lat;Lon;Cutoff;Week;Sens;Overlap;File_Name
2026-03-01;06:30:00;Turdus merula;Eurasian Blackbird;0.9;48.8566;2.3522;0.7;9;1.0;0.0;a.wav
2026-03-02;06:30:00;Turdus merula;Eurasian Blackbird;0.9;48.8566;2.3522;0.7;9;1.0;0.0;b.wav
";

/// `(id, source_label, row_count, source_lat, distance_km)` of a batch row.
type BatchRow = (i64, Option<String>, i64, Option<f64>, Option<f64>);

#[test]
fn a_csv_from_another_site_is_tagged_shifted_and_located() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("BirdDB.txt");
    std::fs::write(&src, BIRDDB).unwrap();
    let dst = dir.path().join("birds.db");
    {
        let conn = birdnet_db::sqlite::open_or_create(&dst).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-03-01','05:00:00','Erithacus rubecula','European Robin',0.8)",
            [],
        )
        .unwrap();
    }
    let options = ImportOptions {
        shift_secs: 3600,
        label: Some("Paris garden".to_owned()),
        ..ImportOptions::default()
    };
    let summary = run_migration_with_options(
        &src,
        &dst,
        false,
        &ProgressHandle::new(),
        &options,
        (Some(51.5074), Some(-0.1278)),
    )
    .expect("import");
    assert_eq!(summary.imported_rows, 2);

    let conn = Connection::open(&dst).unwrap();
    let batches: Vec<BatchRow> = conn
        .prepare("SELECT id, source_label, row_count, source_lat, distance_km FROM import_batches")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(batches.len(), 1, "one batch for the import: {batches:?}");
    let (id, label, rows, lat, km) = batches[0].clone();
    assert_eq!(label.as_deref(), Some("Paris garden"));
    assert_eq!(rows, 2);
    assert!((lat.expect("the source's site") - 48.857).abs() < 0.001);
    assert!((km.expect("distance") - 343.0).abs() < 10.0, "{km:?}");

    let tagged: Vec<(String, Option<i64>)> = conn
        .prepare("SELECT Time, import_batch_id FROM detections WHERE Sci_Name = 'Turdus merula'")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        tagged,
        vec![
            ("07:30:00".to_owned(), Some(id)),
            ("07:30:00".to_owned(), Some(id))
        ],
        "rows tagged with the batch and moved by the shift"
    );
    // The station's own row is untouched: NULL means "recorded here".
    let own: Option<i64> = conn
        .query_row(
            "SELECT import_batch_id FROM detections WHERE Sci_Name = 'Erithacus rubecula'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(own, None);
}
