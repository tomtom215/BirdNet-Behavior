//! Today's "comparative phrase" partial — answers *how* today compares to baseline.
//!
//! Drop this into `crates/birdnet-web/src/routes/pages/today.rs` and add the
//! route in that file's `router()`:
//!
//! ```rust,ignore
//! .route("/pages/today-phrase", get(today_phrase_partial))
//! ```
//!
//! Pure read; no schema changes; uses only `detections` table.

// Percentile/tiering math with int<->float casts.
#![allow(clippy::pedantic, clippy::nursery)]

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;

use crate::routes::pages::{escape_html, today_date_string};
use crate::state::AppState;

/// Tier breakpoints — percentiles of the rolling 30-day count distribution.
///
/// Tuned for "feels right" rather than strict statistics. Adjust to taste.
const TIERS: &[(f64, &str, &str)] = &[
    (0.10, "quiet", "fg-3"), // bottom 10%
    (0.35, "calm", "fg-2"),
    (0.65, "steady", "fg"), // middle band — no accent
    (0.85, "busy", "moss-ink"),
    (0.97, "loud", "moss-ink"),
    (1.01, "record", "rare"), // top 3%  — uses rare hue
];

/// Fewer baseline days than this and the phrase makes no comparison at all.
///
/// With one or two days of history every count is either "well below typical"
/// or "your busiest day yet", and with none the percentile defaulted to the
/// middle and the hero said "right around typical vs your last 30 days" on a
/// station that was an hour old.
const MIN_BASELINE_DAYS: usize = 7;

pub async fn today_phrase_partial(State(state): State<AppState>) -> impl IntoResponse {
    let today = today_date_string();
    let now = now_time_string();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let today_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM detections_analytic WHERE Date = ?1",
                [&today],
                |r| r.get(0),
            )?;
            let today_species: i64 = conn.query_row(
                "SELECT COUNT(DISTINCT Com_Name) FROM detections_analytic WHERE Date = ?1",
                [&today],
                |r| r.get(0),
            )?;
            let baseline = baseline_counts(conn, &today, &now)?;
            Ok::<_, rusqlite::Error>((today_count, today_species, baseline))
        })
    })
    .await;

    let (count, species, baseline) = match result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "today phrase: query failed");
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                static_fallback(),
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "today phrase: task failed");
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                static_fallback(),
            );
        }
    };

    let html = phrase_html(count, species, &baseline, morning_or_day());
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// The station's local wall-clock time, `HH:MM:SS`.
fn now_time_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let t = (secs + super::local_utc_offset_secs()).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

/// Detections on each of the last 30 days **up to this time of day**, silent
/// days included as zero, from the station's first day of history onwards.
///
/// Three corrections to what this compared against:
///
/// * It counted each past day **in full**, so a morning's partial count was
///   ranked against whole days, and every station read "quiet" — "well below
///   typical" — at breakfast. A past day now counts only what it had heard by
///   the same local time.
/// * A day with no detections had no row, so it silently left the baseline,
///   and the silent days are exactly the ones a busy day should beat.
/// * Days before the station's first detection are not "silent days" — the
///   station did not exist — so the zero-fill starts from its first record.
fn baseline_counts(
    conn: &rusqlite::Connection,
    today: &str,
    now_time: &str,
) -> Result<Vec<i64>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE days(d) AS ( \
             SELECT date(?1, '-30 days') \
             UNION ALL SELECT date(d, '+1 day') FROM days WHERE d < date(?1, '-1 day') \
         ) \
         SELECT (SELECT COUNT(*) FROM detections_analytic \
                 WHERE Date = days.d AND Time <= ?2) \
         FROM days \
         WHERE days.d >= (SELECT MIN(Date) FROM detections_analytic) \
         ORDER BY days.d",
    )?;
    let rows = stmt.query_map(rusqlite::params![today, now_time], |r| r.get::<_, i64>(0))?;
    rows.collect()
}

/// The hero's two lines, from today's numbers and the baseline.
fn phrase_html(count: i64, species: i64, baseline: &[i64], time_phrase: &str) -> String {
    let counts = format!(
        r#"<span class="mono tabular">{count}</span> detections ·
  <span class="mono tabular">{species}</span> species"#
    );
    if baseline.len() < MIN_BASELINE_DAYS {
        return format!(
            r#"<h1 class="display td-h1">
Getting to know your <em class="tp-c-moss-ink">yard</em>.
</h1>
<p class="bnb-meta td-sub">
  {counts} so far today · after a week of listening, each day is compared with the ones before it ({days} so far).
</p>"#,
            days = baseline.len(),
        );
    }
    let pct = percentile(baseline, count);
    let (verb, color) = tier_for(pct);
    format!(
        r#"<h1 class="display td-h1">
A <em class="tp-c-{color}">{verb}</em> {time_phrase}.
</h1>
<p class="bnb-meta td-sub">
  {counts} ·
  {pct_str} for this time of day, over your last {days} days.
</p>"#,
        verb = escape_html(verb),
        pct_str = percentile_phrase(pct),
        days = baseline.len(),
    )
}

/// What the hero says when its numbers could not be read. It used to say
/// "You're listening." — a claim about the microphone this path knows
/// nothing about.
fn static_fallback() -> String {
    format!(
        r#"<h1 class="display td-h1">
Today at the station.
</h1>
{}"#,
        super::error_states::inline("today's summary")
    )
}

/// 0..1 percentile of `value` within `samples`. Empty samples → 0.5 (middle).
fn percentile(samples: &[i64], value: i64) -> f64 {
    if samples.is_empty() {
        return 0.5;
    }
    let below = samples.iter().filter(|&&v| v < value).count() as f64;
    below / samples.len() as f64
}

fn tier_for(pct: f64) -> (&'static str, &'static str) {
    for (bound, verb, color) in TIERS {
        if pct <= *bound {
            return (verb, color);
        }
    }
    ("record", "rare")
}

fn percentile_phrase(pct: f64) -> String {
    let n = (pct * 100.0).round() as i32;
    match n {
        0..=5 => "well below typical".into(),
        6..=25 => format!("{} percentile", ordinal(n)),
        26..=74 => "right around typical".into(),
        75..=89 => format!("{} percentile", ordinal(n)),
        90..=98 => format!("{} percentile — well above typical", ordinal(n)),
        _ => "your busiest day yet".into(),
    }
}

/// `21` → `21st`. This wrote `{n}th`, so a fifth of the percentiles it can
/// print came out as "21th", "22th", "83th".
fn ordinal(n: i32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn morning_or_day() -> &'static str {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    part_of_day(secs, super::local_utc_offset_secs())
}

/// The part of the station's **local** day. This read the UTC hour, so a
/// UTC+10 station's dawn chorus was "A busy evening."
fn part_of_day(unix_secs: i64, offset_secs: i64) -> &'static str {
    let hour = (unix_secs + offset_secs).rem_euclid(86_400) / 3600;
    match hour {
        4..=10 => "morning",
        11..=15 => "midday",
        16..=20 => "evening",
        _ => "night",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_of_day_is_the_stations_own() {
        // 20:00 UTC is 06:00 in Sydney and 16:00 in New York.
        let t = 1_790_280_000;
        assert_eq!(part_of_day(t, 36_000), "morning");
        assert_eq!(part_of_day(t, -14_400), "evening");
        assert_eq!(part_of_day(t, 0), "evening");
    }
    #[test]
    fn ordinals_are_english() {
        let got: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 83, 98]
            .iter()
            .map(|n| ordinal(*n))
            .collect();
        assert_eq!(
            got,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd", "83rd",
                "98th"
            ]
        );
        assert_eq!(percentile_phrase(0.21), "21st percentile");
    }

    #[test]
    fn a_station_without_a_week_of_history_makes_no_comparison() {
        for baseline in [vec![], vec![40, 50, 60]] {
            let html = phrase_html(12, 3, &baseline, "morning");
            assert!(!html.contains("typical"), "{html}");
            assert!(!html.contains("percentile"), "{html}");
            assert!(html.contains("Getting to know"), "{html}");
        }
        // Counterpart: a week of history is compared, and says over how many days.
        let html = phrase_html(55, 3, &[10, 20, 30, 40, 50, 60, 70], "morning");
        assert!(html.contains("over your last 7 days"), "{html}");
    }

    fn station(rows: &[(&str, &str)]) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        for (date, time) in rows {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
                 VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9)",
                [date, time],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn past_days_are_counted_up_to_this_time_of_day() {
        // Each of the last ten days: one detection at 06:00, one at 18:00.
        let mut rows = Vec::new();
        let dates: Vec<String> = (10..=19).map(|d| format!("2026-05-{d}")).collect();
        for d in &dates {
            rows.push((d.as_str(), "06:00:00"));
            rows.push((d.as_str(), "18:00:00"));
        }
        let conn = station(&rows);
        let at_seven = baseline_counts(&conn, "2026-05-20", "07:00:00").unwrap();
        assert_eq!(at_seven, vec![1; 10], "a morning is compared with mornings");
        let at_night = baseline_counts(&conn, "2026-05-20", "23:00:00").unwrap();
        assert_eq!(at_night, vec![2; 10]);
    }

    #[test]
    fn a_silent_day_counts_as_zero_but_days_before_the_station_do_not() {
        // First record on the 10th, silent on the 15th, today is the 20th.
        let conn = station(&[
            ("2026-05-10", "06:00:00"),
            ("2026-05-11", "06:00:00"),
            ("2026-05-12", "06:00:00"),
            ("2026-05-13", "06:00:00"),
            ("2026-05-14", "06:00:00"),
            ("2026-05-16", "06:00:00"),
            ("2026-05-17", "06:00:00"),
            ("2026-05-18", "06:00:00"),
            ("2026-05-19", "06:00:00"),
        ]);
        let days = baseline_counts(&conn, "2026-05-20", "23:59:59").unwrap();
        assert_eq!(days, vec![1, 1, 1, 1, 1, 0, 1, 1, 1, 1]);
    }

    #[test]
    fn percentile_basic() {
        let s = vec![10, 20, 30, 40, 50];
        assert!((percentile(&s, 35) - 0.6).abs() < 0.01);
        assert_eq!(percentile(&s, 5), 0.0);
        assert_eq!(percentile(&s, 100), 1.0);
    }
    #[test]
    fn tier_boundaries() {
        assert_eq!(tier_for(0.05).0, "quiet");
        assert_eq!(tier_for(0.50).0, "steady");
        assert_eq!(tier_for(0.99).0, "record");
    }
}
