//! Undoing an import has to reach this store too.
//!
//! # What this is defending
//!
//! There are two stores. `SQLite` answers the species lists, the life list and
//! the heat map; this `DuckDB` copy answers sessionize, funnel, retention,
//! next-species, phenology and every time-series query. The sync between them
//! is *incremental* — it can only add rows newer than the ones it already holds
//! — so it has no way to notice a removal. That is why
//! `AnalyticsDb::delete_detection` exists for a single deleted false positive,
//! and it is why removing an import needs the same treatment.
//!
//! Without the mirror, "undo this import" would undo it on half the application
//! and leave the other half answering with the merged history — the exact
//! two-stores-two-answers failure `sync_from_sqlite`'s drift repair was written
//! for, except permanent, because nothing revisits a row once synced.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use tempfile::TempDir;

/// A store with the station's own detections and two imports.
fn seeded() -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).unwrap();

    let mut values = Vec::new();
    let mut push = |i: usize, sci: &str, com: &str, batch: &str| {
        values.push(format!(
            "('2026-06-15','{:02}:{:02}:00','{sci}','{com}',0.9,\
              NULL,NULL,NULL,NULL,NULL,NULL,'rec.wav',{batch},NULL)",
            i / 60,
            i % 60
        ));
    };
    for i in 0..10 {
        push(i, "Erithacus rubecula", "European Robin", "NULL");
    }
    for i in 10..17 {
        push(i, "Erithacus rubecula", "European Robin", "1");
    }
    for i in 17..21 {
        push(i, "Luscinia megarhynchos", "Common Nightingale", "2");
    }

    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections \
              (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, \
               Sens, Overlap, File_Name, import_batch_id, review_verdict) \
             VALUES {};",
            values.join(",")
        ))
        .expect("seed");
    (db, dir)
}

fn count(db: &AnalyticsDb, where_sql: &str) -> i64 {
    db.conn()
        .query_row(
            &format!("SELECT COUNT(*) FROM detections WHERE {where_sql}"),
            [],
            |r| r.get(0),
        )
        .expect("count")
}

/// Observed failing before `AnalyticsDb::delete_import_batch` existed — there
/// was no call to make, and an undone import stayed in every behavioural and
/// time-series dashboard for good.
#[test]
fn the_analytics_copy_loses_the_batch_and_nothing_else() {
    let (db, _dir) = seeded();
    assert_eq!(count(&db, "TRUE"), 21);

    let removed = db.delete_import_batch(1).expect("delete");

    assert_eq!(removed, 7);
    assert_eq!(
        count(&db, "import_batch_id IS NULL"),
        10,
        "a locally recorded detection must never be reachable from an import undo"
    );
    assert_eq!(
        count(&db, "import_batch_id = 2"),
        4,
        "the other import must be untouched"
    );
}

/// The counterpart: the analytic view every query actually reads must agree,
/// not just the underlying table. `detections_ts` is a view over `detections`,
/// so this would only diverge if a future change materialised it.
#[test]
fn the_view_every_query_reads_agrees_with_the_table() {
    let (db, _dir) = seeded();
    db.delete_import_batch(1).expect("delete");

    let via_view: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM detections_ts", [], |r| r.get(0))
        .expect("view count");
    assert_eq!(via_view, 14, "10 local + the other import's 4");

    let species: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(DISTINCT Com_Name) FROM detections_ts",
            [],
            |r| r.get(0),
        )
        .expect("species");
    assert_eq!(
        species, 2,
        "the removed batch's species is gone only if it had no other source; \
         here the robin stays because this station hears it too"
    );
}

/// Removing a batch that is not in this copy is a no-op. The two stores are
/// updated in sequence, so the analytics side is routinely asked about a batch
/// it may never have seen — a station that ran without `--analytics-db` for a
/// while, for instance.
#[test]
fn removing_an_unknown_batch_is_harmless() {
    let (db, _dir) = seeded();
    assert_eq!(db.delete_import_batch(99).expect("delete"), 0);
    assert_eq!(count(&db, "TRUE"), 21);
}

/// `SQLite` keeps an import's locked rows when the import is removed; this
/// copy has no lock column, so it is told which rows to keep. Deleting the
/// whole batch here left the two stores disagreeing until a restart.
#[test]
fn undoing_an_import_keeps_the_rows_sqlite_kept() {
    let (db, _dir) = seeded();
    // Batch 1 is 07 rows at 00:10..00:16; say SQLite kept the one at 00:12.
    let keep = vec![(
        "2026-06-15".to_owned(),
        "00:12:00".to_owned(),
        "Erithacus rubecula".to_owned(),
        Some("rec.wav".to_owned()),
    )];
    let removed = db.delete_import_batch_keeping(1, &keep).unwrap();
    assert_eq!(removed, 6);
    assert_eq!(count(&db, "import_batch_id = 1"), 1);
    assert_eq!(count(&db, "import_batch_id = 1 AND Time = '00:12:00'"), 1);
    // Nothing kept: the whole batch goes, as before.
    assert_eq!(db.delete_import_batch_keeping(2, &[]).unwrap(), 4);
}

/// A key with a clip names one row, where the triple names one per source.
#[test]
fn a_keyed_delete_removes_only_the_row_it_names() {
    let (db, _dir) = seeded();
    db.conn()
        .execute_batch(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-06-15','00:00:00','Erithacus rubecula','European Robin',0.8,'cam2.wav'),
                    ('2026-06-15','00:00:00','Erithacus rubecula','European Robin',0.8,NULL);",
        )
        .unwrap();
    let key = |f: Option<&str>| {
        db.delete_detection_at("2026-06-15", "00:00:00", "Erithacus rubecula", f)
            .unwrap()
    };
    assert_eq!(key(Some("cam2.wav")), 1);
    assert_eq!(key(None), 1, "None names the row with no clip");
    assert_eq!(
        count(&db, "Time = '00:00:00'"),
        1,
        "the seeded rec.wav row at 00:00 is untouched"
    );
}
