//! The species list and the species page must agree about whose birds these are.
//!
//! # What was wrong
//!
//! `species_summary` (migration 30) is a rollup maintained by triggers, and its
//! triggers filter on one thing: `review_verdict IS NOT 'rejected'`. Migration
//! 34 then gave `detections_analytic` a second rule — imported detections are
//! excluded when the operator sets `analytics_exclude_imports` — which the
//! rollup could not learn, because the rule depends on a setting that can be
//! flipped at any time and the rollup's key has no provenance dimension.
//!
//! So every reader of the rollup went on counting another station's history
//! after the operator excluded it. Measured on the fixture below — two
//! detections of the station's own and three imported, exclusion on —
//! `detections_analytic` reported 1 species and 2 rows while `species_count`
//! reported 2 and `top_species` ranked the **imported** species first, at 3
//! detections: a bird that station never heard, presented as its commonest.
//!
//! `species_summary(conn, name)`, the per-species detail, reads
//! `detections_analytic` directly and was right throughout, so the list and the
//! detail page disagreed about the same species — and the list is the one an
//! operator reads first.
//!
//! `tests/provenance_filter_two_stores.rs` could not see any of it: it compares
//! the SQLite view against the DuckDB view, and both were correct. The rollup
//! is a third implementation of the same rule and was in neither comparison.
//!
//! # What this holds
//!
//! Both directions of the setting, and — the one that matters most — that the
//! fix did not simply abandon the rollup. A version of this that always read
//! `detections_analytic` would pass every correctness gate here and silently
//! undo migration 30, putting the species list back on a scan that grows with
//! the station's whole history.

use birdnet_db::sqlite::{
    search_species, species_count, species_hourly_activity, species_hourly_activity_batch,
    top_species,
};
use rusqlite::Connection;

/// Two detections this station heard, three imported from somewhere else.
fn station(exclude: Option<&str>) -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO import_batches (id, source_kind, row_count) VALUES (1, 'birdnet-pi', 3)",
        [],
    )
    .expect("batch");
    for i in 0..2 {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES (date('now','localtime'), ?1, 'Turdus merula', 'Eurasian Blackbird', 0.9)",
            rusqlite::params![format!("06:{i:02}:00")],
        )
        .expect("seed local");
    }
    for i in 0..3 {
        conn.execute(
            "INSERT INTO detections
               (Date, Time, Sci_Name, Com_Name, Confidence, import_batch_id)
             VALUES (date('now','localtime'), ?1, 'Parus major', 'Great Tit', 0.9, 1)",
            rusqlite::params![format!("07:{i:02}:00")],
        )
        .expect("seed import");
    }
    if let Some(value) = exclude {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('analytics_exclude_imports', ?1)",
            rusqlite::params![value],
        )
        .expect("setting");
    }
    conn
}

/// The reproduction, across every reader of the rollup.
#[test]
fn the_species_list_excludes_an_import_the_operator_excluded() {
    let conn = station(Some("true"));

    assert_eq!(
        species_count(&conn).expect("count"),
        1,
        "the imported species is still counted as one of this station's"
    );

    let top = top_species(&conn, 10).expect("top");
    assert_eq!(
        top.len(),
        1,
        "an excluded species is still in the species list: {top:?}"
    );
    assert_eq!(top[0].com_name, "Eurasian Blackbird");
    assert_eq!(top[0].count, 2);

    let found = search_species(&conn, "Tit", 10).expect("search");
    assert!(
        found.is_empty(),
        "search still finds an excluded species: {found:?}"
    );

    let hours = species_hourly_activity(&conn, "Great Tit").expect("hours");
    assert!(
        hours.is_empty(),
        "the hour histogram still has the excluded species: {hours:?}"
    );

    let batch =
        species_hourly_activity_batch(&conn, &["Great Tit".to_string()]).expect("hours batch");
    assert!(
        !batch.contains_key("Great Tit"),
        "the batched histogram still has the excluded species: {batch:?}"
    );
}

/// The counterpart, and the reason the fix cannot be "drop imported rows".
///
/// Including an import is the default and a legitimate choice — merging two
/// sites is a thing operators do, and only they know whether these are one site
/// with a moved GPS fix or two a county apart. Only the exact string `"true"`
/// may exclude, which is the same rule `detections_analytic` applies.
#[test]
fn the_species_list_keeps_an_import_the_operator_kept() {
    for setting in [None, Some("false"), Some("yes"), Some("TRUE")] {
        let conn = station(setting);
        assert_eq!(
            species_count(&conn).expect("count"),
            2,
            "setting {setting:?} is not \"true\", so the import counts"
        );
        let top = top_species(&conn, 10).expect("top");
        assert_eq!(top.len(), 2, "setting {setting:?}: {top:?}");
        assert_eq!(
            top[0].com_name, "Great Tit",
            "setting {setting:?}: the import has the most detections"
        );
        assert!(
            !species_hourly_activity(&conn, "Great Tit")
                .expect("hours")
                .is_empty(),
            "setting {setting:?}: the import's histogram must be there"
        );
    }
}

/// The discrimination, and the one this fix could most easily have got wrong.
///
/// A version that always read `detections_analytic` would satisfy every gate
/// above and silently undo migration 30, putting the species list back on a
/// scan of the whole detection history — 0.58 s at 2.76 M rows, growing.
///
/// The mechanical check: put a value in the rollup that the base table does not
/// support, and assert the readers report it. Only a reader that actually read
/// `species_summary` can. This is deliberately a wrong number — it is the only
/// way to tell the two sources apart when they otherwise agree.
#[test]
fn a_station_with_nothing_to_exclude_still_reads_the_rollup() {
    for setting in [
        None,
        Some("false"),
        // The setting is on, but this station has never imported anything, so
        // there is nothing for the substitute source to remove and the rollup
        // is still the right answer — and the cheap one.
        Some("true"),
    ] {
        let conn = Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES (date('now','localtime'), '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
            [],
        )
        .expect("seed");
        if let Some(value) = setting {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('analytics_exclude_imports', ?1)",
                rusqlite::params![value],
            )
            .expect("setting");
        }
        conn.execute(
            "UPDATE species_summary SET detections = 41 WHERE Com_Name = 'Eurasian Blackbird'",
            [],
        )
        .expect("tamper");

        let top = top_species(&conn, 10).expect("top");
        assert_eq!(
            top[0].count, 41,
            "setting {setting:?}: the species list stopped reading the rollup, \
             so migration 30's bounded cost is gone"
        );
    }
}

/// The two sources must be interchangeable when there is nothing to exclude.
///
/// If they are not, the switch above changes numbers for a reason that has
/// nothing to do with provenance — which is how a performance optimisation
/// becomes a data bug.
#[test]
fn the_two_sources_agree_when_there_is_nothing_to_exclude() {
    let conn = station(Some("false"));
    let from_rollup: Vec<(String, i64)> = top_species(&conn, 10)
        .expect("top")
        .into_iter()
        .map(|s| (s.com_name, s.count))
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT Com_Name, SUM(detections) AS count FROM
               (SELECT Com_Name, Sci_Name, SUBSTR(Time, 1, 2) AS hour,
                       COUNT(*) AS detections, SUM(Confidence) AS confidence_sum
                  FROM detections_analytic
                 GROUP BY Com_Name, Sci_Name, SUBSTR(Time, 1, 2))
             GROUP BY Com_Name, Sci_Name ORDER BY count DESC, Com_Name ASC LIMIT 10",
        )
        .expect("prepare");
    let from_view: Vec<(String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");

    assert_eq!(
        from_rollup, from_view,
        "the rollup and the substitute source disagree on a station with \
         nothing to exclude, so switching between them changes the answer"
    );
}
