//! A row that names no point in time is left out of a query, never fatal to it.
//!
//! `Date` and `Time` are free-form text, and the BirdNET-Pi importer copies
//! malformed values through, so `detections_ts` can hold a row with a date but
//! no timestamp (a `Time` that does not parse) or with neither. Three queries
//! grouped or reported on the missing value and handed a NULL to a decoder
//! expecting a `String` or `i64` — `InvalidColumnType(0, "window_start",
//! Null)` — so one such row took the whole card down for every day:
//! `hourly_activity`, `species_peak_hours` and `accumulation_curve`.
//!
//! Every public query is driven here, so the next one to group on a nullable
//! clock fails this file rather than a station's dashboard.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_timeseries::executor::TimeSeriesDb;
use birdnet_timeseries::types::params::{
    AnomalyParams, DailyParams, DiversityParams, HourlyParams, PeakParams, SessionParams,
    TrendParams, WeeklyParams,
};

#[test]
fn every_query_answers_with_an_unplaceable_row_present() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    // Today and yesterday each hold one placed detection at 06:00 and one
    // whose Time does not parse; one more row has a Date that does not parse.
    // Yesterday matters: the heatmap counts complete days only.
    db.conn()
        .execute_batch(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             SELECT strftime(d, '%Y-%m-%d'), '06:00:00', 's', 'Placed', 0.9,
                    epoch(d::TIMESTAMP + INTERVAL 6 HOUR)::BIGINT
               FROM (VALUES (CURRENT_DATE), (CURRENT_DATE - INTERVAL 1 DAY)) t(d)
             UNION ALL
             SELECT strftime(d, '%Y-%m-%d'), '', 's', 'Placed', 0.9, NULL
               FROM (VALUES (CURRENT_DATE), (CURRENT_DATE - INTERVAL 1 DAY)) t(d)
             UNION ALL
             SELECT 'garbage', '06:00:00', 'g', 'Undated', 0.9, NULL;",
        )
        .expect("seed");
    // Precondition: the fixture holds both kinds of unplaceable row.
    let (no_ts, no_date): (i64, i64) = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FILTER (WHERE detection_date IS NOT NULL
                                       AND detection_timestamp IS NULL),
                    COUNT(*) FILTER (WHERE detection_date IS NULL)
             FROM detections_ts",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("count");
    assert_eq!((no_ts, no_date), (2, 1));

    let ts = TimeSeriesDb::new(db.conn()).expect("the ts view builds");
    let today: String = db
        .conn()
        .query_row("SELECT strftime(CURRENT_DATE, '%Y-%m-%d')", [], |r| {
            r.get(0)
        })
        .expect("today");

    let hourly = ts
        .hourly_activity(&HourlyParams::default())
        .expect("hourly_activity");
    assert_eq!(
        hourly.iter().map(|w| w.detection_count).sum::<i64>(),
        2,
        "{hourly:?}"
    );
    let heatmap = ts
        .hourly_heatmap(&HourlyParams::default())
        .expect("hourly_heatmap");
    assert_eq!(
        heatmap
            .iter()
            .map(|h| (h.hour_of_day, h.total_detections))
            .collect::<Vec<_>>(),
        vec![(6, 1)],
        "{heatmap:?}"
    );
    let peak = ts
        .species_peak_hours("Placed", 30)
        .expect("species_peak_hours");
    assert_eq!(peak.len(), 1, "{peak:?}");
    assert_eq!(peak[0].detection_count, 2, "{peak:?}");
    let accum = ts
        .accumulation_curve(None, None)
        .expect("accumulation_curve");
    assert_eq!(
        accum.last().map(|a| a.cumulative_species),
        Some(1),
        "{accum:?}"
    );

    // The rest already answered; they stay in so a regression in any of them
    // fails here too.
    ts.daily_activity(&DailyParams::default())
        .expect("daily_activity");
    ts.weekly_activity(&WeeklyParams::default())
        .expect("weekly_activity");
    ts.daily_richness(&DiversityParams::default())
        .expect("daily_richness");
    ts.top_species(30, 10).expect("top_species");
    ts.peak_windows(&PeakParams::default())
        .expect("peak_windows");
    ts.activity_sessions(&SessionParams::default())
        .expect("activity_sessions");
    ts.intraday_gaps(&today, 30).expect("intraday_gaps");
    ts.quiet_days(5, 30).expect("quiet_days");
    ts.daily_max_gaps(30, 30).expect("daily_max_gaps");
    ts.moving_average(&TrendParams::default())
        .expect("moving_average");
    ts.year_over_year(&WeeklyParams::default())
        .expect("year_over_year");
    ts.anomalies(&AnomalyParams::default()).expect("anomalies");
}
