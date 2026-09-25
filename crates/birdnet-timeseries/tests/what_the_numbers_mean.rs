//! Each gate here pins what a number on the Trends page *means*, against a
//! store whose answer is known in advance.
//!
//! Every one of them was a query that ran, returned plausible rows, and passed
//! the `*_sql_*` substring tests — while answering a different question from
//! the one its card asks. Silent days skipped by a moving average, a baseline
//! of two days scoring a z of forty, a 29-minute gap counted as thirty, a
//! session cut in two by midnight, this week's four days against last year's
//! seven. None of that is visible in SQL text; all of it is visible in the
//! numbers, so these execute against a real `AnalyticsDb` and assert on them.
//!
//! Where a gate covers a discrimination, its counterpart is here too, so a
//! query that simply stopped returning anything cannot pass for a fixed one.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_timeseries::executor::TimeSeriesDb;
use birdnet_timeseries::types::params::{
    AnomalyParams, DailyParams, HourlyParams, PeakParams, SessionParams, TrendParams, WeeklyParams,
};
use tempfile::TempDir;

/// Days since the epoch of `CURRENT_DATE`, as the engine under test sees it.
fn today_days(db: &AnalyticsDb) -> i64 {
    let t: String = db
        .conn()
        .query_row("SELECT CAST(CURRENT_DATE AS VARCHAR)", [], |r| r.get(0))
        .expect("current date");
    let p: Vec<u32> = t.split('-').map(|x| x.parse().unwrap()).collect();
    birdnet_core::civil::days_from_civil(p[0], p[1], p[2])
}

/// The date `back` days before today, `YYYY-MM-DD`.
fn day(db: &AnalyticsDb, back: i64) -> String {
    let (y, m, d) = birdnet_core::civil::civil_from_days(today_days(db) - back);
    format!("{y:04}-{m:02}-{d:02}")
}

fn open() -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("ts.duckdb")).expect("open");
    (db, dir)
}

/// Insert `(date, time, common name, confidence)` rows. The fixture is UTC, so
/// the instant is the wall clock read as UTC.
fn seed(db: &AnalyticsDb, rows: &[(String, String, &str, f64)]) {
    if rows.is_empty() {
        return;
    }
    let values: Vec<String> = rows
        .iter()
        .map(|(d, t, com, conf)| {
            format!("('{d}','{t}','Sci {com}','{com}',{conf},epoch(TIMESTAMP '{d} {t}'))")
        })
        .collect();
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             VALUES {};",
            values.join(",")
        ))
        .expect("seed");
}

/// `n` Great Tit detections on the day `back` days ago, a minute apart from 06:00.
fn day_of(db: &AnalyticsDb, back: i64, n: u32) -> Vec<(String, String, &'static str, f64)> {
    let date = day(db, back);
    (0..n)
        .map(|k| {
            (
                date.clone(),
                format!("{:02}:{:02}:00", 6 + k / 60, k % 60),
                "Great Tit",
                0.9,
            )
        })
        .collect()
}

fn ts(db: &AnalyticsDb) -> TimeSeriesDb<'_> {
    TimeSeriesDb::new(db.conn()).expect("executor")
}

// ---------------------------------------------------------------------------
// Moving average
// ---------------------------------------------------------------------------

/// A day the station heard nothing is a zero in a moving average, not a gap
/// the window steps over; and today, a few hours in, is not a day yet.
///
/// Seven detections a day for ten days, a silent day three days ago, and fifty
/// so far today. The trailing 7-day average for yesterday covers the six
/// seven-detection days and the silent one: 42/7 = 6.0. Grouping only the
/// days that had rows skipped the silent day entirely, and the window reached
/// forward into today's partial count.
#[test]
fn a_silent_day_is_a_zero_in_the_moving_average() {
    let (db, _dir) = open();
    let mut rows = Vec::new();
    for back in 1..=10 {
        if back != 3 {
            rows.extend(day_of(&db, back, 7));
        }
    }
    rows.extend(day_of(&db, 0, 50));
    seed(&db, &rows);

    let trend = ts(&db)
        .moving_average(&TrendParams {
            window_days: 7,
            from_date: Some("CURRENT_DATE - INTERVAL 30 DAYS".into()),
            to_date: None,
            species: None,
        })
        .expect("moving_average");
    let row = |back: i64| trend.iter().find(|r| r.date == day(&db, back));

    assert!(row(0).is_none(), "today is not over: {trend:?}");
    let silent = row(3).expect("the silent day is a row");
    assert_eq!(silent.daily_detections, 0, "{trend:?}");
    let yesterday = row(1).expect("yesterday");
    assert_eq!(
        yesterday.moving_avg_detections,
        Some(6.0),
        "seven days of 7 with one of them silent: {trend:?}"
    );
}

/// A window that runs off the start of the data has no average, rather than an
/// average over however many days happened to be in reach.
///
/// Four days of history with a 7-day window: no day has seven days behind it.
/// A `RANGE` frame silently averaged the two or three it found and drew them
/// on the same line as the complete windows.
#[test]
fn an_incomplete_window_has_no_average() {
    let (db, _dir) = open();
    let mut rows = Vec::new();
    for back in 1..=4 {
        rows.extend(day_of(&db, back, 3 * u32::try_from(back).unwrap()));
    }
    seed(&db, &rows);
    let trend = ts(&db)
        .moving_average(&TrendParams {
            window_days: 7,
            from_date: Some("CURRENT_DATE - INTERVAL 30 DAYS".into()),
            to_date: None,
            species: None,
        })
        .expect("moving_average");
    assert_eq!(trend.len(), 4, "the four days the station ran: {trend:?}");
    assert!(
        trend.iter().all(|r| r.moving_avg_detections.is_none()),
        "no day has a full window behind it: {trend:?}"
    );
    // Counterpart: the raw counts are still reported.
    assert_eq!(
        trend.iter().map(|r| r.daily_detections).collect::<Vec<_>>(),
        [12, 9, 6, 3]
    );
}

/// A station with no detections has no trend, not ninety days of zeros.
#[test]
fn an_empty_station_has_no_trend() {
    let (db, _dir) = open();
    let trend = ts(&db)
        .moving_average(&TrendParams::default())
        .expect("moving_average");
    assert!(trend.is_empty(), "{trend:?}");
}

// ---------------------------------------------------------------------------
// Anomalies
// ---------------------------------------------------------------------------

/// A baseline with no spread cannot score a day, so it cannot flag one.
///
/// Ten days of exactly five, then six. The sample deviation of the baseline is
/// zero, the z-score is undefined — and the query flagged the day `high` with
/// a blank z, because `6 > 5 + 2 × 0`.
#[test]
fn a_flat_baseline_flags_nothing() {
    let (db, _dir) = open();
    let mut rows = Vec::new();
    for back in 2..=11 {
        rows.extend(day_of(&db, back, 5));
    }
    rows.extend(day_of(&db, 1, 6));
    seed(&db, &rows);
    let out = ts(&db)
        .anomalies(&AnomalyParams {
            z_threshold: 2.0,
            window_days: 30,
            lookback_days: 30,
        })
        .expect("anomalies");
    let flagged: Vec<_> = out.iter().filter(|r| r.anomaly_flag != "normal").collect();
    assert!(flagged.is_empty(), "flagged without a z-score: {flagged:?}");
}

/// Two days are not a baseline.
///
/// Days of 1 and 3, then 50. `STDDEV_SAMP` is defined from two points, so the
/// third day of a new station scored z ≈ 34 and was flagged `high`.
#[test]
fn two_days_of_history_is_not_a_baseline() {
    let (db, _dir) = open();
    let mut rows = day_of(&db, 3, 1);
    rows.extend(day_of(&db, 2, 3));
    rows.extend(day_of(&db, 1, 50));
    seed(&db, &rows);
    let out = ts(&db)
        .anomalies(&AnomalyParams {
            z_threshold: 2.0,
            window_days: 30,
            lookback_days: 30,
        })
        .expect("anomalies");
    let yesterday = out
        .iter()
        .find(|r| r.date == day(&db, 1))
        .expect("yesterday is a row");
    assert_eq!(yesterday.anomaly_flag, "normal", "{out:?}");
}

/// Counterpart: with a real baseline, a real outlier is still flagged.
#[test]
fn a_real_outlier_on_a_real_baseline_is_still_flagged() {
    let (db, _dir) = open();
    let mut rows = Vec::new();
    for back in 2..=15 {
        rows.extend(day_of(&db, back, if back % 2 == 0 { 9 } else { 11 }));
    }
    rows.extend(day_of(&db, 1, 50));
    seed(&db, &rows);
    let out = ts(&db)
        .anomalies(&AnomalyParams {
            z_threshold: 2.0,
            window_days: 30,
            lookback_days: 30,
        })
        .expect("anomalies");
    let yesterday = out.iter().find(|r| r.date == day(&db, 1)).expect("row");
    assert_eq!(yesterday.anomaly_flag, "high", "{out:?}");
    assert!(yesterday.z_score.is_some());
}

// ---------------------------------------------------------------------------
// Sessions and gaps: elapsed minutes, not minute boundaries
// ---------------------------------------------------------------------------

const MAY: &str = "2026-05-01";

fn at(time: &str) -> (String, String, &'static str, f64) {
    (MAY.to_string(), time.to_string(), "Great Tit", 0.9)
}

fn by_date(db: &AnalyticsDb, gap: u32) -> Vec<birdnet_timeseries::types::results::SessionRow> {
    ts(db)
        .activity_sessions(&SessionParams {
            gap_minutes: gap,
            date_filter: Some(MAY.into()),
            ..SessionParams::default()
        })
        .expect("sessions by date")
}

fn by_range(db: &AnalyticsDb, gap: u32) -> Vec<birdnet_timeseries::types::results::SessionRow> {
    ts(db)
        .activity_sessions(&SessionParams {
            gap_minutes: gap,
            date_filter: None,
            lookback_days: 40_000,
            limit: 100,
        })
        .expect("sessions by range")
}

/// 05:00:59 to 05:30:00 is 29 minutes and one second of silence — under a
/// 30-minute threshold — so it is one session, 29 minutes long.
///
/// `date_diff('minute', …)` counts minute *boundaries* crossed, not minutes
/// elapsed, so it read thirty, split the session, and would have reported its
/// duration as thirty too.
#[test]
fn a_gap_is_elapsed_minutes_not_minute_boundaries() {
    let (db, _dir) = open();
    seed(&db, &[at("05:00:59"), at("05:30:00")]);
    for (builder, s) in [("date", by_date(&db, 30)), ("range", by_range(&db, 30))] {
        assert_eq!(s.len(), 1, "{builder}: 29m01s is under 30: {s:?}");
        assert_eq!(s[0].duration_minutes, 29, "{builder}: {s:?}");
        assert_eq!(s[0].max_internal_gap_minutes, Some(29), "{builder}: {s:?}");
    }
    let gaps = ts(&db).intraday_gaps(MAY, 30).expect("gaps");
    assert!(gaps.is_empty(), "no 30-minute gap here: {gaps:?}");
}

/// The threshold is the one the behavioral extension's `sessionize` applies:
/// a new session only when the silence is *longer* than the gap. Exactly
/// thirty minutes stays one session; thirty minutes and a second does not.
///
/// This is the counterpart that keeps the gate above honest — a builder that
/// never split anything would pass it.
#[test]
fn the_threshold_splits_only_a_longer_silence() {
    let (db, _dir) = open();
    seed(&db, &[at("05:00:00"), at("05:30:00"), at("06:00:01")]);
    for (builder, s) in [("date", by_date(&db, 30)), ("range", by_range(&db, 30))] {
        assert_eq!(
            s.iter().map(|r| r.detection_count).collect::<Vec<_>>(),
            [2, 1],
            "{builder}: {s:?}"
        );
    }
}

/// A session that runs through midnight is one session.
///
/// 23:50 and 00:10 are twenty minutes apart; grouping by session *and date*
/// reported them as two one-detection sessions.
#[test]
fn a_session_through_midnight_is_one_session() {
    let (db, _dir) = open();
    let d1 = day(&db, 2);
    let d2 = day(&db, 1);
    seed(
        &db,
        &[
            (d1.clone(), "23:50:00".into(), "Tawny Owl", 0.9),
            (d2, "00:10:00".into(), "Tawny Owl", 0.9),
        ],
    );
    let s = by_range(&db, 30);
    assert_eq!(s.len(), 1, "{s:?}");
    assert_eq!(s[0].detection_count, 2);
    assert_eq!(s[0].duration_minutes, 20);
    assert_eq!(s[0].date, d1, "a session is dated by its start: {s:?}");
}

/// The longest gap of a day is reported with the instants that bound it.
///
/// `gap_end` was filled with the *date*, and `gap_start` with nothing, so the
/// daily-gaps API reported "a 120-minute gap ending on 2026-05-01".
#[test]
fn a_daily_max_gap_says_when_it_was() {
    let (db, _dir) = open();
    let d = day(&db, 1);
    seed(
        &db,
        &[
            (d.clone(), "05:00:00".into(), "Great Tit", 0.9),
            (d.clone(), "05:10:00".into(), "Great Tit", 0.9),
            (d.clone(), "07:10:00".into(), "Great Tit", 0.9),
        ],
    );
    let gaps = ts(&db).daily_max_gaps(7, 30).expect("gaps");
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert_eq!(gaps[0].gap_minutes, 120);
    assert_eq!(
        gaps[0].gap_start.as_deref(),
        Some(&*format!("{d} 05:10:00"))
    );
    assert_eq!(gaps[0].gap_end, format!("{d} 07:10:00"));
}

// ---------------------------------------------------------------------------
// Look-back windows: N days is N dates
// ---------------------------------------------------------------------------

/// "The last 3 days" is three dates — today and the two before it — not four.
///
/// `detection_date >= CURRENT_DATE - INTERVAL 3 DAYS` includes both ends.
#[test]
fn three_days_is_three_dates() {
    let (db, _dir) = open();
    let mut rows = Vec::new();
    for back in 0..=5 {
        rows.extend(day_of(&db, back, 1));
    }
    seed(&db, &rows);
    let daily = ts(&db)
        .daily_activity(&DailyParams {
            lookback_days: 3,
            species: None,
        })
        .expect("daily");
    assert_eq!(
        daily
            .iter()
            .map(|r| r.window_start.clone())
            .collect::<Vec<_>>(),
        [day(&db, 2), day(&db, 1), day(&db, 0)]
    );
}

/// An hour's "average per day" is over complete days: today, a few hours in,
/// is left out of both the numerator and the denominator.
///
/// Ten days of one 06:00 detection, and five already today. The average is
/// 1.0; with today counted it was 15/11.
#[test]
fn todays_partial_day_is_not_averaged() {
    let (db, _dir) = open();
    let mut rows = Vec::new();
    for back in 1..=10 {
        rows.extend(day_of(&db, back, 1));
    }
    rows.extend(
        day_of(&db, 0, 5)
            .into_iter()
            .map(|(d, _, c, f)| (d, "06:00:30".into(), c, f)),
    );
    seed(&db, &rows);
    let heat = ts(&db)
        .hourly_heatmap(&HourlyParams {
            lookback_days: 10,
            species: None,
        })
        .expect("heatmap");
    let six = heat.iter().find(|r| r.hour_of_day == 6).expect("06:00");
    assert!(
        (six.avg_detections_per_day - 1.0).abs() < 1e-9,
        "{}",
        six.avg_detections_per_day
    );
    assert_eq!(six.total_detections, 10);
}

/// The heat map honours the species it is asked about.
///
/// `HourlyParams::species` was dropped on the way to the query, so every
/// species' clock was the whole station's.
#[test]
fn the_heatmap_honours_its_species_filter() {
    let (db, _dir) = open();
    let mut rows = Vec::new();
    for back in 1..=4 {
        let d = day(&db, back);
        rows.push((d.clone(), "06:00:00".to_string(), "Great Tit", 0.9));
        rows.push((d, "22:00:00".to_string(), "Tawny Owl", 0.9));
    }
    seed(&db, &rows);
    let heat = ts(&db)
        .hourly_heatmap(&HourlyParams {
            lookback_days: 10,
            species: Some("Tawny Owl".into()),
        })
        .expect("heatmap");
    assert_eq!(
        heat.iter().map(|r| r.hour_of_day).collect::<Vec<_>>(),
        [22],
        "{heat:?}"
    );
    // The denominator is still the days the station was listening.
    assert!((heat[0].avg_detections_per_day - 1.0).abs() < 1e-9);
}

/// "The last 4 weeks" is four weekly buckets, each a whole week except the one
/// still running — not a stub of a week at the far end.
#[test]
fn four_weeks_is_four_buckets_starting_on_a_week_boundary() {
    let (db, _dir) = open();
    let mut rows = Vec::new();
    for back in 0..60 {
        rows.extend(day_of(&db, back, 1));
    }
    seed(&db, &rows);
    let weeks = ts(&db)
        .weekly_activity(&WeeklyParams { lookback_weeks: 4 })
        .expect("weekly");
    assert_eq!(weeks.len(), 4, "{weeks:?}");
    assert!(
        weeks[..3].iter().all(|w| w.detection_count == 7),
        "every bucket but the running week is whole: {weeks:?}"
    );
}

// ---------------------------------------------------------------------------
// Year over year: equal spans, and silence is a zero
// ---------------------------------------------------------------------------

/// This week so far is compared with the same days of the same week last year,
/// and a week the station heard nothing in is a zero, not a missing row.
///
/// A detection every day for 800 days, except one whole week, three weeks ago,
/// of silence. Every delta is 0 except that week's, which is −7. The running
/// week was compared at its partial count against last year's whole seven
/// days, and the silent week was not in the result at all.
#[test]
fn year_over_year_compares_like_with_like() {
    let (db, _dir) = open();
    // The Monday three weeks before this one.
    let silent_monday: String = db
        .conn()
        .query_row(
            "SELECT CAST((date_trunc('week', CURRENT_DATE) - INTERVAL 3 WEEKS)::DATE AS VARCHAR)",
            [],
            |r| r.get(0),
        )
        .expect("monday");
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             SELECT strftime(d, '%Y-%m-%d'), '06:00:00', 'Turdus merula', 'Eurasian Blackbird',
                    0.9, epoch(d + INTERVAL 6 HOUR)
               FROM range(CAST(CURRENT_DATE - INTERVAL 800 DAY AS TIMESTAMP),
                          CAST(CURRENT_DATE + INTERVAL 1 DAY AS TIMESTAMP),
                          INTERVAL 1 DAY) t(d)
              WHERE d NOT BETWEEN TIMESTAMP '{silent_monday}'
                              AND TIMESTAMP '{silent_monday}' + INTERVAL 6 DAY"
        ))
        .expect("seed");
    let rows = ts(&db)
        .year_over_year(&WeeklyParams { lookback_weeks: 8 })
        .expect("yoy");
    assert_eq!(rows.len(), 8, "{rows:?}");
    let silent = rows
        .iter()
        .find(|r| r.week_start == silent_monday)
        .unwrap_or_else(|| panic!("the silent week is missing: {rows:?}"));
    assert_eq!(silent.current_year_count, 0, "{silent:?}");
    assert_eq!(silent.yoy_delta, Some(-7), "{silent:?}");
    for r in rows.iter().filter(|r| r.week_start != silent_monday) {
        assert_eq!(r.yoy_delta, Some(0), "equal spans in both years: {r:?}");
    }
}

// ---------------------------------------------------------------------------
// Peaks
// ---------------------------------------------------------------------------

/// The busiest windows are distinct stretches of time, not one burst seen
/// through overlapping windows.
///
/// Ten detections between 05:00 and 05:09, and four at 18:00. With 15-minute
/// windows hopping every 5, the burst sits in three overlapping windows, which
/// filled the whole top three before the evening peak could appear.
#[test]
fn peak_windows_do_not_overlap() {
    let (db, _dir) = open();
    let d = day(&db, 1);
    let mut rows: Vec<_> = (0..10)
        .map(|m| (d.clone(), format!("05:{m:02}:00"), "Great Tit", 0.9))
        .collect();
    rows.extend((0..4).map(|m| (d.clone(), format!("18:0{m}:00"), "Robin", 0.8)));
    seed(&db, &rows);
    let peaks = ts(&db)
        .peak_windows(&PeakParams {
            window_minutes: 15,
            hop_minutes: 5,
            lookback_days: 1,
            limit: 3,
        })
        .expect("peaks");
    for (i, a) in peaks.iter().enumerate() {
        for b in &peaks[i + 1..] {
            assert!(
                a.window_end <= b.window_start || b.window_end <= a.window_start,
                "overlapping peaks {a:?} and {b:?}"
            );
        }
    }
    assert_eq!(peaks[0].detection_count, 10, "{peaks:?}");
    assert!(
        peaks.iter().any(|p| p.detection_count == 4),
        "the evening peak: {peaks:?}"
    );
}

/// A species' peak hours honour the look-back, report the loudest detection
/// as the peak confidence, and print both ends of the hour the same way.
#[test]
fn species_peak_hours_mean_what_they_say() {
    let (db, _dir) = open();
    seed(
        &db,
        &[
            (day(&db, 1), "06:10:00".into(), "Great Tit", 0.6),
            (day(&db, 2), "06:20:00".into(), "Great Tit", 0.9),
            // Outside a 30-day look-back; inside the 90 days it used.
            (day(&db, 60), "08:00:00".into(), "Great Tit", 0.9),
        ],
    );
    let hours = ts(&db)
        .species_peak_hours("Great Tit", 30)
        .expect("species peak");
    assert_eq!(hours.len(), 1, "the 60-day-old detection is out: {hours:?}");
    assert_eq!(hours[0].window_start, "06:00");
    assert_eq!(hours[0].window_end, "07:00", "{hours:?}");
    assert_eq!(hours[0].peak_confidence, Some(0.9), "{hours:?}");
}
