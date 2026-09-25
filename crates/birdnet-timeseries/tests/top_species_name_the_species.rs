//! Top species name the species.
//!
//! `top_species` returned `PeakWindowRow`s, which have nowhere to put a species
//! name: the first- and last-seen dates went into `window_start`/`window_end`
//! and the name was dropped, so the list could not say whom it was ranking.
//! (Against the old code this file does not compile — `species` and
//! `first_seen` did not exist on the row — which is the defect.)

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_timeseries::executor::TimeSeriesDb;

#[test]
fn top_species_name_the_species() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    db.conn()
        .execute_batch(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             SELECT strftime(CURRENT_DATE - INTERVAL 1 DAY, '%Y-%m-%d'), t, 'Parus major',
                    'Great Tit', 0.9, 0
               FROM (VALUES ('06:00:00'), ('06:01:00'), ('06:02:00')) v(t);
             INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             SELECT strftime(CURRENT_DATE - INTERVAL 2 DAY, '%Y-%m-%d'), '07:00:00',
                    'Erithacus rubecula', 'Robin', 0.8, 0;",
        )
        .expect("seed");
    let expected_first: String = db
        .conn()
        .query_row(
            "SELECT strftime(CURRENT_DATE - INTERVAL 2 DAY, '%Y-%m-%d')",
            [],
            |r| r.get(0),
        )
        .expect("date");
    let top = TimeSeriesDb::new(db.conn())
        .expect("executor")
        .top_species(30, 10)
        .expect("top");
    assert_eq!(top.len(), 2, "{top:?}");
    assert_eq!(top[0].species, "Great Tit");
    assert_eq!(top[0].detection_count, 3);
    assert_eq!(top[1].species, "Robin");
    assert_eq!(top[1].first_seen, expected_first);
}
