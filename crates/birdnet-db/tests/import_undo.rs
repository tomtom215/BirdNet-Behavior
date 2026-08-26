//! Removing an import must remove exactly what the import brought in.
//!
//! # What this is defending
//!
//! `provenance.rs` warns before an import and says why it has to: the damage a
//! merged dataset does "is not detectable after the fact, so a dataset that
//! quietly merged two sites cannot be repaired, only discarded". Until
//! `delete_import_batch` existed there was no way to discard *an import* — only
//! every detection in the database — so the warning was the whole of the
//! safety net, and `/admin/migrate` told the operator the recorded shift meant
//! the import "stays reversible", which was true of the record and not the data.
//!
//! The property that makes removal safe is the one migration 25 bought:
//! `import_batch_id` is NULL for every detection this station heard itself, so a
//! `WHERE import_batch_id = ?` can never reach one. These pin that, plus the
//! consequences a rollup and a second store make easy to forget.

use rusqlite::Connection;

/// A station with its own history and two imports from elsewhere.
fn station_with_two_imports() -> (Connection, i64, i64) {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");

    let batch = |label: &str| -> i64 {
        conn.execute(
            "INSERT INTO import_batches (source_kind, source_label, row_count)
             VALUES ('birdnet-pi-sqlite', ?1, 0)",
            rusqlite::params![label],
        )
        .expect("batch");
        conn.last_insert_rowid()
    };
    let a = batch("Old garden station");
    let b = batch("A friend's woodland station");

    let mut n = 0;
    let mut add = |day: &str, com: &str, sci: &str, batch_id: Option<i64>| {
        n += 1;
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, import_batch_id)
             VALUES (?1, ?2, ?3, ?4, 0.9, ?5)",
            rusqlite::params![
                day,
                format!("{:02}:{:02}:00", n / 60, n % 60),
                sci,
                com,
                batch_id
            ],
        )
        .expect("insert");
    };

    // Heard here.
    for _ in 0..10 {
        add("2026-06-15", "European Robin", "Erithacus rubecula", None);
    }
    // From the old garden station — a species this station has also heard.
    for _ in 0..7 {
        add(
            "2024-05-01",
            "European Robin",
            "Erithacus rubecula",
            Some(a),
        );
    }
    // From the friend's station — a species only they ever heard.
    for _ in 0..4 {
        add(
            "2023-04-02",
            "Common Nightingale",
            "Luscinia megarhynchos",
            Some(b),
        );
    }
    (conn, a, b)
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}

/// Observed failing before `delete_import_batch` existed — there was no call to
/// make. The assertion it now carries is the one that matters: the batch's rows
/// go, and the station's own do not.
#[test]
fn removing_an_import_removes_its_rows_and_only_its_rows() {
    let (conn, a, b) = station_with_two_imports();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM detections"), 21);

    let removed = birdnet_db::sqlite::delete_import_batch(&conn, a).expect("delete");

    assert_eq!(removed, 7, "the batch's own rows");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM detections WHERE import_batch_id IS NULL"
        ),
        10,
        "a locally recorded detection must never be reachable from an import undo"
    );
    assert_eq!(
        count(
            &conn,
            &format!("SELECT COUNT(*) FROM detections WHERE import_batch_id = {b}")
        ),
        4,
        "the other import must be untouched"
    );
    assert_eq!(
        count(
            &conn,
            &format!("SELECT COUNT(*) FROM import_batches WHERE id = {a}")
        ),
        0,
        "the batch record goes with its rows"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM import_batches"),
        1,
        "and only that batch record"
    );
}

/// The counterpart that makes the gate above a discrimination rather than a
/// count: after the undo, the station's remaining history must be *exactly* what
/// it recorded itself — same species, same dates — not merely the right number
/// of rows.
#[test]
fn what_survives_is_the_stations_own_history() {
    let (conn, a, b) = station_with_two_imports();
    birdnet_db::sqlite::delete_import_batch(&conn, a).expect("delete a");
    birdnet_db::sqlite::delete_import_batch(&conn, b).expect("delete b");

    let dates: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT Date FROM detections ORDER BY Date")
            .expect("prepare");
        stmt.query_map([], |r| r.get(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    };
    assert_eq!(
        dates,
        vec!["2026-06-15".to_string()],
        "only local days remain"
    );

    let species: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT Com_Name FROM detections ORDER BY Com_Name")
            .expect("prepare");
        stmt.query_map([], |r| r.get(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    };
    assert_eq!(
        species,
        vec!["European Robin".to_string()],
        "the friend's nightingale must not still be on this station's life list"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM import_batches"), 0);
}

/// `species_summary` is maintained by triggers and every species count in the
/// application reads it rather than `detections`. A bulk delete that left it
/// stale would undo the import on the pages that read the table and not on the
/// ones that read the rollup — which is the same two-answers failure the
/// rollup's own drift check exists for.
#[test]
fn the_species_rollup_follows_the_undo() {
    let (conn, a, _b) = station_with_two_imports();

    let robins = |c: &Connection| -> i64 {
        c.query_row(
            "SELECT COALESCE(SUM(detections), 0) FROM species_summary
              WHERE Com_Name = 'European Robin'",
            [],
            |r| r.get(0),
        )
        .expect("rollup")
    };
    assert_eq!(robins(&conn), 17, "10 local + 7 imported before the undo");

    birdnet_db::sqlite::delete_import_batch(&conn, a).expect("delete");

    assert_eq!(robins(&conn), 10, "the rollup must lose the imported seven");
    assert!(
        birdnet_db::sqlite::queries::species::species_summary_drift(&conn)
            .expect("drift")
            .is_empty(),
        "and must agree with the detections table afterwards"
    );
}

/// The live count a confirmation dialog states, rather than the number recorded
/// when the import ran. The two diverge the moment anything is deleted, and the
/// question is how much is about to disappear.
#[test]
fn the_row_count_is_counted_live_not_recalled() {
    let (conn, a, _b) = station_with_two_imports();
    assert_eq!(
        birdnet_db::sqlite::import_batch_row_count(&conn, a).expect("count"),
        7
    );
    // `import_batches.row_count` was never filled in by this fixture, so if the
    // implementation read it instead this would be 0.
    conn.execute(
        "DELETE FROM detections WHERE import_batch_id = ?1 AND Time LIKE '%:1%'",
        rusqlite::params![a],
    )
    .expect("partial delete");
    let after = birdnet_db::sqlite::import_batch_row_count(&conn, a).expect("count");
    assert!(after < 7, "the count must track the table, not the record");
}

/// Removing a batch that is not there is a no-op, not an error. An operator
/// double-clicking the button, or two tabs open on the same page, must not get a
/// failure for work that is already done.
#[test]
fn removing_a_batch_twice_is_harmless() {
    let (conn, a, _b) = station_with_two_imports();
    assert_eq!(
        birdnet_db::sqlite::delete_import_batch(&conn, a).expect("first"),
        7
    );
    assert_eq!(
        birdnet_db::sqlite::delete_import_batch(&conn, a).expect("second"),
        0,
        "the second removal finds nothing and says so"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM detections"), 14);
}

/// A station that has never imported anything has nothing to remove, and asking
/// must not touch its history.
#[test]
fn a_station_with_no_imports_is_unaffected() {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES ('2026-06-15', '06:00:00', 'Erithacus rubecula', 'European Robin', 0.9)",
        [],
    )
    .expect("insert");

    assert_eq!(
        birdnet_db::sqlite::delete_import_batch(&conn, 1).expect("delete"),
        0
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM detections"), 1);
}
