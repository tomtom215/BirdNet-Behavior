//! `species_summary` must agree with `detections` after every kind of write.
//!
//! Migration 30 stops the species screens from re-aggregating the whole
//! detection history on every page load by maintaining the aggregate on write.
//! That trade is only worth making if the maintained copy cannot drift: a
//! materialised aggregate that silently disagrees with its source is worse than
//! a slow query, because nothing about a wrong number looks wrong.
//!
//! The maintenance is done by SQLite triggers rather than by a Rust function
//! the write paths call, so that a path which forgets to call it cannot exist.
//! This file is the check on that reasoning. It drives every class of mutation
//! the codebase performs on `detections` — including the ones performed from
//! other crates, spelled here exactly as they are spelled there — and after
//! each one asserts the summary still matches a freshly computed aggregate.
//!
//! Each assertion goes through `species_summary_drift`, which is the same
//! function `--doctor` and the maintenance job call, so this also exercises the
//! detector operators depend on.

use birdnet_db::migration::{MIGRATIONS, migrate};
use birdnet_db::sqlite::queries::species::{
    rebuild_species_summary, species_summary_drift, top_species,
};
use rusqlite::Connection;

/// The migration that introduces `species_summary`.
///
/// Named rather than spelled inline so the upgrade test below reads as "the
/// state just before the summary existed" and moves with the schema instead of
/// silently testing the wrong boundary when migration 31 lands.
const SUMMARY_MIGRATION: u32 = 30;

/// A migrated in-memory database.
fn db() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .expect("pragma");
    migrate(&conn).expect("migrate");
    conn
}

/// Insert through the same statement the capture pipeline uses.
fn insert(conn: &Connection, date: &str, time: &str, sci: &str, com: &str, confidence: f64) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES (?1, ?2, ?3, ?4, ?5, 'rec.wav')",
        rusqlite::params![date, time, sci, com, confidence],
    )
    .expect("insert");
}

/// Fail with the disagreeing buckets named, not just a count.
#[track_caller]
fn assert_agrees(conn: &Connection, after: &str) {
    let drift = species_summary_drift(conn).expect("drift check");
    assert!(
        drift.is_empty(),
        "species_summary disagrees with detections after {after}: {drift:#?}"
    );
}

/// Total detections the summary claims, across every bucket.
fn summary_total(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COALESCE(SUM(detections), 0) FROM species_summary",
        [],
        |r| r.get(0),
    )
    .expect("summary total")
}

/// Every mutation class the codebase performs, each followed by a full
/// comparison against a recomputed aggregate.
#[test]
fn species_summary_is_maintained_by_every_write_path() {
    let conn = db();
    assert_agrees(&conn, "migration (empty database)");

    // --- plain INSERT: the capture pipeline -------------------------------
    insert(
        &conn,
        "2026-01-01",
        "06:15:00",
        "Turdus merula",
        "Blackbird",
        0.90,
    );
    insert(
        &conn,
        "2026-01-01",
        "06:45:00",
        "Turdus merula",
        "Blackbird",
        0.80,
    );
    insert(
        &conn,
        "2026-01-01",
        "18:05:00",
        "Turdus merula",
        "Blackbird",
        0.70,
    );
    insert(
        &conn,
        "2026-01-02",
        "06:05:00",
        "Parus major",
        "Great Tit",
        0.60,
    );
    assert_agrees(&conn, "plain inserts");
    assert_eq!(summary_total(&conn), 4, "four analytic detections inserted");

    // --- INSERT OR IGNORE, accepted: the BirdNET-Pi importer --------------
    conn.execute(
        "INSERT OR IGNORE INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-01-03', '07:00:00', 'Erithacus rubecula', 'Robin', 0.55, 'i.wav')",
        [],
    )
    .expect("import insert");
    assert_agrees(&conn, "INSERT OR IGNORE (accepted)");

    // --- INSERT OR IGNORE, ignored ----------------------------------------
    // An ignored row must add nothing: no row was created, so no contribution
    // exists to count. This arm is why the summary survives a re-run of an
    // import, which is the normal way an operator recovers from a partial one.
    let before = summary_total(&conn);
    let n = conn
        .execute(
            "INSERT OR IGNORE INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-01-03', '07:00:00', 'Erithacus rubecula', 'Robin', 0.55, 'i.wav')",
            [],
        )
        .expect("duplicate import insert");
    assert_eq!(
        n, 0,
        "the duplicate must be ignored for this arm to mean anything"
    );
    assert_eq!(
        before,
        summary_total(&conn),
        "an ignored insert changed the summary"
    );
    assert_agrees(&conn, "INSERT OR IGNORE (ignored)");

    // --- rejected on arrival ----------------------------------------------
    // A detection inserted already carrying a rejection is outside
    // `detections_analytic` from the start and must never be counted.
    let before = summary_total(&conn);
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, review_verdict)
         VALUES ('2026-01-04', '09:00:00', 'Corvus corax', 'Raven', 0.51, 'r.wav', 'rejected')",
        [],
    )
    .expect("pre-rejected insert");
    assert_eq!(
        before,
        summary_total(&conn),
        "a pre-rejected insert was counted"
    );
    assert_agrees(&conn, "insert of an already-rejected detection");

    // --- UPDATE review_verdict -> rejected: applying a verdict ------------
    conn.execute(
        "UPDATE detections SET review_verdict = 'rejected'
          WHERE Date = '2026-01-01' AND Time = '06:15:00' AND Sci_Name = 'Turdus merula'",
        [],
    )
    .expect("reject");
    assert_agrees(&conn, "verdict applied (rejected)");

    // --- UPDATE review_verdict -> NULL: undoing a verdict -----------------
    conn.execute(
        "UPDATE detections SET review_verdict = NULL
          WHERE Date = '2026-01-01' AND Time = '06:15:00' AND Sci_Name = 'Turdus merula'",
        [],
    )
    .expect("undo reject");
    assert_agrees(&conn, "verdict undone");

    // --- UPDATE review_verdict -> confirmed -------------------------------
    // Confirmed is *not* rejected, so this must leave the count alone. An
    // implementation that keyed on "verdict changed" rather than on "rejected
    // changed" would drop a row here.
    let before = summary_total(&conn);
    conn.execute(
        "UPDATE detections SET review_verdict = 'confirmed'
          WHERE Date = '2026-01-02' AND Time = '06:05:00' AND Sci_Name = 'Parus major'",
        [],
    )
    .expect("confirm");
    assert_eq!(
        before,
        summary_total(&conn),
        "confirming a detection changed its count"
    );
    assert_agrees(&conn, "verdict applied (confirmed)");

    // --- UPDATE Sci_Name/Com_Name: relabelling ----------------------------
    // The row moves from one bucket to another. Both sides have to move.
    conn.execute(
        "UPDATE detections SET Sci_Name = 'Turdus philomelos', Com_Name = 'Song Thrush'
          WHERE Date = '2026-01-01' AND Time = '18:05:00' AND Sci_Name = 'Turdus merula'",
        [],
    )
    .expect("relabel");
    assert_agrees(&conn, "relabel to a different species");

    // --- UPDATE of a column the summary does not depend on ----------------
    // `maintenance.rs` sets `Clip_Pruned_At` in bulk and the lock handlers set
    // `is_locked`. Neither can change the summary, and the trigger's WHEN
    // clause exists so neither *costs* anything either.
    //
    // Asserting the summary is unchanged would not test that: an unguarded
    // trigger withdraws the row's contribution and immediately re-admits it,
    // arriving back at the same number by doing two index writes per row. So
    // this counts the writes instead. `total_changes()` includes writes made
    // inside a trigger body — probed, not assumed — which makes the guard
    // directly observable: guarded, the delta is exactly the rows the statement
    // touched; unguarded, it is three times that.
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count detections");
    let before_summary = summary_total(&conn);

    let before = conn.total_changes();
    conn.execute("UPDATE detections SET is_locked = 1", [])
        .expect("lock all");
    let locked_writes = i64::try_from(conn.total_changes() - before).expect("write count");
    assert_eq!(
        locked_writes, rows,
        "setting is_locked on {rows} rows performed {locked_writes} writes — the summary \
         triggers are doing work for an update that cannot change the summary"
    );

    let before = conn.total_changes();
    conn.execute(
        "UPDATE detections SET Clip_Pruned_At = '2026-02-01T00:00:00Z'",
        [],
    )
    .expect("prune clips");
    let pruned_writes = i64::try_from(conn.total_changes() - before).expect("write count");
    assert_eq!(
        pruned_writes, rows,
        "the bulk clip prune performed {pruned_writes} writes over {rows} rows"
    );

    assert_eq!(
        before_summary,
        summary_total(&conn),
        "an unrelated column update moved the summary"
    );
    assert_agrees(&conn, "updates to columns the summary does not depend on");

    // --- UPDATE Time: the row moves between hour buckets ------------------
    conn.execute(
        "UPDATE detections SET Time = '23:59:59'
          WHERE Date = '2026-01-02' AND Time = '06:05:00' AND Sci_Name = 'Parus major'",
        [],
    )
    .expect("retime");
    assert_agrees(&conn, "a detection moved to a different hour");

    // --- UPDATE Confidence -------------------------------------------------
    conn.execute(
        "UPDATE detections SET Confidence = 0.99
          WHERE Date = '2026-01-03' AND Sci_Name = 'Erithacus rubecula'",
        [],
    )
    .expect("reconfidence");
    assert_agrees(&conn, "confidence changed");
}

/// Every way a detection can be removed, each followed by the same comparison.
///
/// Split from the write-path test above rather than continued in it: a failure
/// here says the *withdrawal* half of the maintenance is wrong, which is a
/// different defect from the admission half, and a single 200-line timeline
/// made the two indistinguishable in a failure report.
#[test]
fn species_summary_is_maintained_by_every_delete_path() {
    let conn = db();

    // The same shape the write-path test ends with: several species, one of
    // them present in a single bucket, and one rejected detection that was
    // never counted.
    insert(
        &conn,
        "2026-01-01",
        "06:15:00",
        "Turdus merula",
        "Blackbird",
        0.90,
    );
    insert(
        &conn,
        "2026-01-01",
        "06:45:00",
        "Turdus merula",
        "Blackbird",
        0.80,
    );
    insert(
        &conn,
        "2026-01-01",
        "18:05:00",
        "Turdus philomelos",
        "Song Thrush",
        0.70,
    );
    insert(
        &conn,
        "2026-01-02",
        "06:05:00",
        "Parus major",
        "Great Tit",
        0.60,
    );
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, review_verdict)
         VALUES ('2026-01-04', '09:00:00', 'Corvus corax', 'Raven', 0.51, 'r.wav', 'rejected')",
        [],
    )
    .expect("pre-rejected insert");
    assert_agrees(&conn, "setup");

    // --- DELETE one row: the admin delete ---------------------------------
    conn.execute(
        "DELETE FROM detections
          WHERE Date = '2026-01-01' AND Time = '06:45:00' AND Sci_Name = 'Turdus merula'",
        [],
    )
    .expect("delete one");
    assert_agrees(&conn, "single-row delete");

    // --- DELETE the last row of a bucket ----------------------------------
    // The bucket has to disappear, not linger at zero: a zero-count row would
    // make the species list report a species with no detections, and its
    // average confidence would be a division by zero.
    conn.execute(
        "DELETE FROM detections WHERE Sci_Name = 'Turdus philomelos'",
        [],
    )
    .expect("delete last of species");
    let lingering: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM species_summary WHERE Sci_Name = 'Turdus philomelos'",
            [],
            |r| r.get(0),
        )
        .expect("count lingering");
    assert_eq!(lingering, 0, "an emptied bucket was left behind at zero");
    assert!(
        !top_species(&conn, 50)
            .expect("top species")
            .iter()
            .any(|s| s.sci_name == "Turdus philomelos"),
        "a species with no remaining detections is still on the species list"
    );
    assert_agrees(&conn, "delete of a bucket's last row");

    // --- DELETE a rejected row --------------------------------------------
    // It was never counted, so removing it must not decrement anything.
    let before = summary_total(&conn);
    conn.execute(
        "DELETE FROM detections WHERE review_verdict = 'rejected'",
        [],
    )
    .expect("delete rejected");
    assert_eq!(
        before,
        summary_total(&conn),
        "deleting a rejected row decremented the summary"
    );
    assert_agrees(&conn, "delete of a rejected detection");

    // --- DELETE everything: the store reset -------------------------------
    conn.execute("DELETE FROM detections", [])
        .expect("delete all");
    assert_eq!(
        summary_total(&conn),
        0,
        "the summary outlived every detection"
    );
    assert_agrees(&conn, "wholesale delete");
}

/// The backfill has to see history that was already there when it ran.
///
/// A station upgrading into migration 30 has years of detections and no
/// summary. If the backfill only caught rows written after it, every species
/// screen would read zero on the machines that need it most — and the drift
/// check would be the only thing that noticed.
#[test]
fn the_backfill_summarises_history_that_predates_it() {
    // Build the pre-migration-30 state by replaying the migration list itself
    // up to 29, then let the real `migrate()` apply only 30. Loading history
    // into a fully migrated database instead would exercise the *triggers* and
    // pass even if no backfill existed at all, which is the whole point of this
    // test.
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
    let pre = MIGRATIONS
        .iter()
        .take_while(|m| m.version < SUMMARY_MIGRATION)
        .collect::<Vec<_>>();
    assert_eq!(
        u32::try_from(pre.len()).expect("migration count"),
        SUMMARY_MIGRATION - 1,
        "the migration list is not the contiguous 1..{SUMMARY_MIGRATION} this test assumes"
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
        conn.query_row("SELECT COUNT(*) FROM species_summary", [], |r| r
            .get::<_, i64>(0))
            .is_err(),
        "species_summary must not exist yet, or this test proves nothing"
    );

    for i in 0..50 {
        insert(
            &conn,
            "2024-05-01",
            &format!("06:{:02}:00", i % 60),
            "Turdus merula",
            "Blackbird",
            0.5 + f64::from(i % 10) / 100.0,
        );
    }
    conn.execute(
        "UPDATE detections SET review_verdict = 'rejected' WHERE Time = '06:07:00'",
        [],
    )
    .expect("reject one");

    let applied = migrate(&conn).expect("migrate the rest");
    // Everything from the summary migration onward, not "exactly one". This
    // asserted `applied == 1` and broke the day migration 31 was added — an
    // assertion about the *length of the migration list* standing in for one
    // about the backfill, which is what this test is actually for. The real
    // property is that migration 30 was among those applied and its backfill
    // ran; that is what the two assertions below check.
    let expected = MIGRATIONS
        .iter()
        .filter(|m| m.version >= SUMMARY_MIGRATION)
        .count();
    let expected = u32::try_from(expected).expect("migration count");
    assert_eq!(
        applied, expected,
        "migration {SUMMARY_MIGRATION} and everything after it should have been applied"
    );

    assert_agrees(&conn, "backfill over pre-existing history");
    assert_eq!(
        summary_total(&conn),
        49,
        "50 detections were loaded before migration 30 and one was rejected"
    );
}

/// `species_summary_drift` must report a disagreement, not just the absence of
/// one.
///
/// Without this arm the checker could be a constant `Ok(vec![])` and every
/// assertion above would still pass. The repair is checked in the same test,
/// because a detector with no working repair leaves an operator worse off than
/// no detector.
#[test]
fn the_drift_check_reports_a_summary_that_has_been_corrupted() {
    let conn = db();
    insert(
        &conn,
        "2026-03-01",
        "06:00:00",
        "Turdus merula",
        "Blackbird",
        0.9,
    );
    insert(
        &conn,
        "2026-03-01",
        "06:01:00",
        "Turdus merula",
        "Blackbird",
        0.8,
    );
    assert_agrees(&conn, "setup");

    // Corrupt the summary behind the triggers' back — a stand-in for whatever
    // a future bypassing write path would do.
    conn.execute("UPDATE species_summary SET detections = detections + 7", [])
        .expect("corrupt");
    let drift = species_summary_drift(&conn).expect("drift check");
    assert_eq!(
        drift.len(),
        1,
        "one bucket was corrupted, so one must be reported"
    );
    assert_eq!(drift[0].summary_count, 9);
    assert_eq!(drift[0].actual_count, 2);

    // A bucket that exists only in the summary must also be reported: an
    // inner join would miss it and report nothing.
    conn.execute(
        "INSERT INTO species_summary VALUES ('Ghost', 'Nonexistent avis', '04', 3, 2.1)",
        [],
    )
    .expect("phantom bucket");
    let drift = species_summary_drift(&conn).expect("drift check");
    assert_eq!(
        drift.len(),
        2,
        "a summary-only bucket was not reported: {drift:#?}"
    );

    // And a bucket that exists only in `detections`. Reached by deleting the
    // summary rows rather than by inserting detections, so the triggers cannot
    // put it back.
    conn.execute("DELETE FROM species_summary", [])
        .expect("wipe summary");
    let drift = species_summary_drift(&conn).expect("drift check");
    assert_eq!(
        drift.len(),
        1,
        "a detections-only bucket was not reported: {drift:#?}"
    );
    assert_eq!(drift[0].summary_count, 0);
    assert_eq!(drift[0].actual_count, 2);

    // Repair.
    let buckets = rebuild_species_summary(&conn).expect("rebuild");
    assert_eq!(buckets, 1);
    assert_agrees(&conn, "rebuild");
}

/// The summary must reproduce what the aggregate it replaced returned.
///
/// Switching a read from `detections_analytic` to `species_summary` is only
/// safe if the two give the same answer, and "the same answer" is a claim about
/// counts *and* average confidence, not counts alone.
#[test]
fn the_summary_reproduces_the_aggregate_it_replaced() {
    let conn = db();
    let species = [
        ("Turdus merula", "Blackbird"),
        ("Parus major", "Great Tit"),
        ("Erithacus rubecula", "Robin"),
    ];
    for day in 1..=9 {
        for (i, (sci, com)) in species.iter().enumerate() {
            for k in 0..=i {
                insert(
                    &conn,
                    &format!("2026-04-0{day}"),
                    &format!("{:02}:{:02}:00", (day * 2 + k) % 24, k * 5),
                    sci,
                    com,
                    0.5 + f64::from(u32::try_from(day).unwrap()) / 100.0,
                );
            }
        }
    }
    conn.execute(
        "UPDATE detections SET review_verdict = 'rejected' WHERE Date = '2026-04-03'",
        [],
    )
    .expect("reject a day");

    // The pre-migration-30 statement, run directly against the view.
    let mut stmt = conn
        .prepare(
            "SELECT Com_Name, Sci_Name, COUNT(*), AVG(Confidence)
               FROM detections_analytic GROUP BY Com_Name, Sci_Name
              ORDER BY COUNT(*) DESC, Com_Name ASC",
        )
        .expect("prepare reference");
    let reference: Vec<(String, String, i64, f64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("run reference")
        .collect::<Result<_, _>>()
        .expect("collect reference");

    let via_summary = top_species(&conn, 100).expect("top species");
    assert_eq!(
        via_summary.len(),
        reference.len(),
        "different number of species"
    );
    for (got, want) in via_summary.iter().zip(&reference) {
        assert_eq!(
            (&got.com_name, &got.sci_name),
            (&want.0, &want.1),
            "species order differs"
        );
        assert_eq!(got.count, want.2, "count differs for {}", want.0);
        assert!(
            (got.avg_confidence - want.3).abs() < 1e-9,
            "average confidence differs for {}: {} vs {}",
            want.0,
            got.avg_confidence,
            want.3
        );
    }

    // Same for the per-species hour histogram, against its own old statement.
    for (_, com) in species {
        let mut stmt = conn
            .prepare(
                "SELECT SUBSTR(Time, 1, 2), COUNT(*) FROM detections_analytic
                  WHERE Com_Name = ?1 GROUP BY 1 ORDER BY 1",
            )
            .expect("prepare hourly reference");
        let reference: Vec<(String, i64)> = stmt
            .query_map([com], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("run hourly reference")
            .collect::<Result<_, _>>()
            .expect("collect hourly reference");
        let got = birdnet_db::sqlite::queries::species::species_hourly_activity(&conn, com)
            .expect("hourly via summary");
        let got: Vec<(String, i64)> = got.into_iter().map(|h| (h.hour, h.count)).collect();
        assert_eq!(got, reference, "hour histogram differs for {com}");
    }
}
