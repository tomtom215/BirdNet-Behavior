//! The provenance filter has to mean the same thing in both stores.
//!
//! # What this is defending
//!
//! Migration 25 tagged every imported detection with its batch. Until migration
//! 34 nothing read the tag, so the life list, first-of-year, species richness,
//! phenology, the heat map, co-occurrence and the dawn chorus all counted
//! another site's records as this station's — which `provenance.rs` warns about
//! before an import, saying the damage "is not detectable after the fact".
//!
//! The filter is implemented twice, because the data lives twice:
//!
//! * **SQLite** — a subquery in `detections_analytic` against the `settings`
//!   table. Every SQLite-side analytic reads that view.
//! * **DuckDB** — a literal baked into `detections_ts`, recreated when the flag
//!   changes, because that store has no settings table. Every behavioural and
//!   time-series query reads that view.
//!
//! Two implementations of one rule is exactly the shape this repository keeps
//! paying for, so the point of this file is not that either works — each crate
//! tests its own — but that they **agree**, including in the cases where they
//! could most easily diverge: the default, an unrecognised value, and a station
//! with no imports at all.
//!
//! This lives in the binary's test directory rather than either crate's because
//! it is the only place both are in scope.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::queries::EXCLUDE_IMPORTS_SETTING;
use rusqlite::Connection;
use tempfile::TempDir;

/// Ten detections this station heard, seven imported from one site, four from
/// another — the same fixture in both stores.
fn two_stores() -> (Connection, AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let sqlite = Connection::open_in_memory().expect("sqlite");
    birdnet_db::migration::migrate(&sqlite).expect("migrate");
    let duck = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).expect("duckdb");

    let rows: Vec<(usize, &str, &str, Option<i64>)> = (0..10)
        .map(|i| (i, "Erithacus rubecula", "European Robin", None))
        .chain((10..17).map(|i| (i, "Erithacus rubecula", "European Robin", Some(1))))
        .chain((17..21).map(|i| (i, "Luscinia megarhynchos", "Common Nightingale", Some(2))))
        .collect();

    for id in [1_i64, 2] {
        sqlite
            .execute(
                "INSERT INTO import_batches (id, source_kind, row_count) VALUES (?1, 'x', 0)",
                rusqlite::params![id],
            )
            .expect("batch");
    }

    let mut duck_values = Vec::new();
    for (i, sci, com, batch) in &rows {
        let time = format!("{:02}:{:02}:00", i / 60, i % 60);
        sqlite
            .execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, import_batch_id)
                 VALUES ('2026-06-15', ?1, ?2, ?3, 0.9, ?4)",
                rusqlite::params![time, sci, com, batch],
            )
            .expect("sqlite insert");
        duck_values.push(format!(
            "('2026-06-15','{time}','{sci}','{com}',0.9,NULL,NULL,NULL,NULL,NULL,NULL,\
              'rec.wav',{},NULL)",
            batch.map_or("NULL".to_string(), |b| b.to_string())
        ));
    }
    duck.conn()
        .execute_batch(&format!(
            "INSERT INTO detections \
              (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, \
               Sens, Overlap, File_Name, import_batch_id, review_verdict) \
             VALUES {};",
            duck_values.join(",")
        ))
        .expect("duckdb insert");

    (sqlite, duck, dir)
}

fn sqlite_visible(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM detections_analytic", [], |r| r.get(0))
        .expect("sqlite count")
}

fn duck_visible(db: &AnalyticsDb) -> i64 {
    db.conn()
        .query_row("SELECT COUNT(*) FROM detections_ts", [], |r| r.get(0))
        .expect("duckdb count")
}

/// Set the flag the way the handler does, and propagate it to `DuckDB` the way
/// the handler does.
fn set_flag(sqlite: &Connection, duck: &AnalyticsDb, value: &str) {
    birdnet_db::settings::set(
        sqlite,
        EXCLUDE_IMPORTS_SETTING,
        value,
        birdnet_db::settings::SettingsCategory::System,
    )
    .expect("set");
    duck.refresh_view_from(sqlite).expect("refresh view");
}

/// Observed failing before migration 34 and `detections_ts_view_sql`: with the
/// flag on, both stores still reported all 21 rows, because nothing read
/// `import_batch_id`.
#[test]
fn both_stores_hide_imports_when_the_filter_is_on() {
    let (sqlite, duck, _dir) = two_stores();
    set_flag(&sqlite, &duck, "true");

    assert_eq!(
        sqlite_visible(&sqlite),
        10,
        "SQLite must show only local rows"
    );
    assert_eq!(duck_visible(&duck), 10, "DuckDB must show only local rows");
}

/// The counterpart, and the one that keeps an upgrade honest: absent means
/// included, so no existing station's numbers move.
#[test]
fn both_stores_include_imports_by_default() {
    let (sqlite, duck, _dir) = two_stores();
    // Nothing is written to `settings` at all — the state every station is in
    // immediately after the upgrade.
    assert_eq!(sqlite_visible(&sqlite), 21);
    assert_eq!(duck_visible(&duck), 21);
}

/// And explicitly off is the same as absent, in both.
#[test]
fn both_stores_include_imports_when_the_filter_is_off() {
    let (sqlite, duck, _dir) = two_stores();
    set_flag(&sqlite, &duck, "false");
    assert_eq!(sqlite_visible(&sqlite), 21);
    assert_eq!(duck_visible(&duck), 21);
}

/// A value neither store recognises must mean "include" in both, rather than one
/// engine's truthiness rules diverging from the other's. `"1"`, `"yes"` and
/// `"TRUE"` are all things an operator or a future config importer could write.
#[test]
fn an_unrecognised_value_means_include_in_both_stores() {
    for value in ["1", "yes", "TRUE", "", "  true  "] {
        let (sqlite, duck, _dir) = two_stores();
        set_flag(&sqlite, &duck, value);
        assert_eq!(
            sqlite_visible(&sqlite),
            21,
            "SQLite treated {value:?} as on"
        );
        assert_eq!(duck_visible(&duck), 21, "DuckDB treated {value:?} as on");
    }
}

/// Flipping it back restores every row in both stores — the setting hides rows,
/// it does not delete them. That distinction is the whole reason this exists
/// alongside `delete_import_batch` rather than instead of it.
#[test]
fn the_filter_hides_rather_than_deletes() {
    let (sqlite, duck, _dir) = two_stores();
    set_flag(&sqlite, &duck, "true");
    assert_eq!(sqlite_visible(&sqlite), 10);

    set_flag(&sqlite, &duck, "false");
    assert_eq!(sqlite_visible(&sqlite), 21, "SQLite rows came back");
    assert_eq!(duck_visible(&duck), 21, "DuckDB rows came back");

    // And the rows were there the whole time.
    let raw: i64 = sqlite
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("raw");
    assert_eq!(raw, 21);
}

/// A station that has never imported anything sees no difference either way,
/// which is what the `import_batch_id IS NULL` short-circuit is for.
#[test]
fn a_station_with_no_imports_is_unaffected_in_both_stores() {
    let dir = TempDir::new().expect("tempdir");
    let sqlite = Connection::open_in_memory().expect("sqlite");
    birdnet_db::migration::migrate(&sqlite).expect("migrate");
    let duck = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).expect("duckdb");

    for i in 0..5 {
        let time = format!("06:{i:02}:00");
        sqlite
            .execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-06-15', ?1, 'Erithacus rubecula', 'European Robin', 0.9)",
                rusqlite::params![time],
            )
            .expect("insert");
        duck.conn()
            .execute_batch(&format!(
                "INSERT INTO detections \
                  (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, \
                   Sens, Overlap, File_Name, import_batch_id, review_verdict) \
                 VALUES ('2026-06-15','{time}','Erithacus rubecula','European Robin',0.9,\
                         NULL,NULL,NULL,NULL,NULL,NULL,'rec.wav',NULL,NULL);"
            ))
            .expect("insert");
    }

    for value in ["true", "false"] {
        set_flag(&sqlite, &duck, value);
        assert_eq!(sqlite_visible(&sqlite), 5, "SQLite, filter {value}");
        assert_eq!(duck_visible(&duck), 5, "DuckDB, filter {value}");
    }
}

/// A rejected detection stays hidden whichever way the provenance filter is set.
/// The two rules compose rather than replacing one another, and migration 26's
/// verdict filter is the one that must not be lost.
#[test]
fn the_verdict_filter_survives_the_provenance_filter() {
    let (sqlite, duck, _dir) = two_stores();
    sqlite
        .execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Time = '00:00:00'",
            [],
        )
        .expect("reject");
    duck.conn()
        .execute_batch("UPDATE detections SET review_verdict = 'rejected' WHERE Time = '00:00:00';")
        .expect("reject");

    set_flag(&sqlite, &duck, "true");
    assert_eq!(
        sqlite_visible(&sqlite),
        9,
        "10 local minus the rejected one"
    );
    assert_eq!(duck_visible(&duck), 9);

    set_flag(&sqlite, &duck, "false");
    assert_eq!(sqlite_visible(&sqlite), 20, "21 minus the rejected one");
    assert_eq!(duck_visible(&duck), 20);
}
