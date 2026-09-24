//! Weekly detection report notification scheduler.
//!
//! Sends a weekly summary of bird detections via Apprise on a configured
//! weekday. The report includes the top 10 species by detection count and
//! the total number of detections for the past 7 days.
//!
//! BirdNET-Pi equivalent: `weekly_report.sh` cron job.

use std::fmt::Write as FmtWrite;
use std::sync::Arc;
use tokio::sync::Mutex;

use birdnet_integrations::apprise::{Client as AppriseClient, NotifyType};

/// Start the weekly report scheduler as a background tokio task.
///
/// Wakes up hourly, checks if today is the configured weekday and if the
/// report has already been sent this week (recorded in the database, so a
/// restart does not repeat it). If not, generates and sends the report.
///
/// `schedule` is one of: "monday", "tuesday", "wednesday", "thursday",
/// "friday", "saturday", "sunday", or "disabled".
pub fn start_weekly_report_scheduler(
    schedule: &str,
    apprise: Arc<Mutex<AppriseClient>>,
    state: birdnet_web::state::AppState,
) {
    let weekday = parse_weekday(schedule);
    let Some(target_weekday) = weekday else {
        if schedule != "disabled" {
            tracing::warn!(schedule, "unknown weekly report schedule, disabling");
        }
        return;
    };

    tracing::info!(
        schedule = %schedule,
        "weekly report scheduler started"
    );

    tokio::spawn(async move {
        weekly_report_loop(target_weekday, apprise, state).await;
    });
}

/// Weekday number (0 = Monday, 6 = Sunday), matching ISO 8601.
fn parse_weekday(schedule: &str) -> Option<u8> {
    match schedule.trim().to_lowercase().as_str() {
        "monday" => Some(0),
        "tuesday" => Some(1),
        "wednesday" => Some(2),
        "thursday" => Some(3),
        "friday" => Some(4),
        "saturday" => Some(5),
        "sunday" => Some(6),
        _ => None,
    }
}

/// The main loop: wakes up hourly, sends report on the right weekday.
async fn weekly_report_loop(
    target_weekday: u8,
    apprise: Arc<Mutex<AppriseClient>>,
    state: birdnet_web::state::AppState,
) {
    // Also held in memory, so a database that cannot record the send does not
    // turn into a report every hour for the rest of the day.
    let mut last_sent_date: Option<String> = None;

    loop {
        // Sleep 1 hour between checks.
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;

        let (today_str, weekday) = today_weekday();

        // Only send on the target weekday and only once per day.
        if weekday != target_weekday {
            continue;
        }

        if last_sent_date.as_deref() == Some(&today_str) {
            continue; // Already sent today.
        }
        let now_secs = unix_now();
        match already_sent(&state, now_secs) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => {
                // Unknown is not "not sent": try again next hour rather than
                // risk a duplicate.
                tracing::warn!(error = %e, "could not read when the weekly report was last sent");
                continue;
            }
        }

        tracing::info!(date = %today_str, "sending weekly detection report");

        match build_weekly_report(&state) {
            Ok((title, body)) => {
                let mut client = apprise.lock().await;
                // Operational, not routine: a report sent once a week must not
                // lose the minute's send budget to a dawn chorus. It used to,
                // and the skip came back as `Ok(())`, so `last_sent_date` was
                // stamped and that week's report was never attempted again.
                if let Err(e) = client
                    .send_operational_alert(&title, &body, NotifyType::Info)
                    .await
                {
                    tracing::warn!(error = %e, "weekly report notification failed");
                } else {
                    tracing::info!("weekly report sent");
                    if let Err(e) = record_sent(&state, now_secs) {
                        tracing::warn!(error = %e, "weekly report sent but not recorded; a restart today would send it again");
                    }
                    last_sent_date = Some(today_str);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to build weekly report");
            }
        }
    }
}

/// The `maintenance_runs` key a sent report is recorded under.
///
/// Not a `JOB_` in the maintenance catalogue: the report is a notification
/// with its own weekday and may be disabled, and the maintenance status would
/// then show it as a job that never ran.
const SENT_KEY: &str = "weekly_report_sent";

/// A report sent less than this long ago is this week's. Six days rather than
/// a calendar comparison, so a daylight-saving change between a send and the
/// next hourly check cannot move the send onto "yesterday".
const RESEND_AFTER_SECS: i64 = 6 * 86_400;

/// Whether this week's report has already gone out, as recorded in the
/// database — so a restart on report day does not send it again.
fn already_sent(
    state: &birdnet_web::state::AppState,
    now_secs: i64,
) -> Result<bool, birdnet_db::sqlite::DbError> {
    let last = state.with_db(|conn| birdnet_db::sqlite::last_run_unix(conn, SENT_KEY))?;
    // A clock that has gone backwards since the send (a Pi with no RTC before
    // NTP answers) reads as recent: suppressing one report beats sending two.
    Ok(last.is_some_and(|t| now_secs - t < RESEND_AFTER_SECS))
}

/// Record that the report went out at `now_secs`.
fn record_sent(
    state: &birdnet_web::state::AppState,
    now_secs: i64,
) -> Result<(), birdnet_db::sqlite::DbError> {
    state.with_db(|conn| birdnet_db::sqlite::record_run(conn, SENT_KEY, now_secs))
}

/// Build the weekly report title and body.
fn build_weekly_report(
    state: &birdnet_web::state::AppState,
) -> Result<(String, String), birdnet_db::sqlite::DbError> {
    // Compute the 7-day window ending today.
    let (week_end, week_start) = week_range_strings();

    let (total, top_species) = state.with_db(|conn| {
        let total = birdnet_db::sqlite::weekly_detection_count(conn, &week_start, &week_end)?;
        let top = birdnet_db::sqlite::weekly_top_species(conn, &week_start, &week_end, 10)?;
        Ok::<_, birdnet_db::sqlite::DbError>((total, top))
    })?;

    let title = format!("Weekly Bird Report: {total} detections ({week_start} – {week_end})");

    let mut body = format!(
        "Bird Detection Weekly Summary\n\nPeriod: {week_start} to {week_end}\nTotal detections: {total}\n\nTop species:\n"
    );
    for (i, (_, com_name, count)) in top_species.iter().enumerate() {
        writeln!(body, "{}. {} — {count} detections", i + 1, com_name).unwrap_or_default();
    }

    Ok((title, body))
}

/// Return `(today_str, seven_days_ago_str)` as ISO date strings.
fn week_range_strings() -> (String, String) {
    week_range_on(today_local_day())
}

/// The seven days ending on `day` (days since the epoch), inclusive.
fn week_range_on(day: i64) -> (String, String) {
    (
        birdnet_core::civil::date_string_from_days(day),
        birdnet_core::civil::date_string_from_days(day - 6),
    )
}

/// Seconds since the epoch.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Today, as days since the epoch.
fn today_local_day() -> i64 {
    local_day(unix_now(), birdnet_db::clock::local_utc_offset_secs())
}

/// The station's day containing `now_secs`, at `utc_offset_secs` east of UTC.
///
/// Local, because the `Date` column the report counts is local. It was
/// `secs / 86400` — UTC — so at UTC−8 a Monday report fired about 16:00 on
/// Sunday, with a window that ended "tomorrow".
const fn local_day(now_secs: i64, utc_offset_secs: i64) -> i64 {
    (now_secs + utc_offset_secs).div_euclid(86_400)
}

/// Return today's ISO date string and ISO weekday (0 = Mon, 6 = Sun).
fn today_weekday() -> (String, u8) {
    let day = today_local_day();
    (
        birdnet_core::civil::date_string_from_days(day),
        weekday_of(day),
    )
}

/// ISO weekday of `day` (days since the epoch), 0 = Monday. 1970-01-01 was a
/// Thursday, which is where the 3 comes from.
fn weekday_of(day: i64) -> u8 {
    u8::try_from((day + 3).rem_euclid(7)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report's day is the station's day.
    ///
    /// It was `secs / 86400`, UTC. At UTC−8 a Monday report fired about
    /// 16:00 local on Sunday, with a window ending "tomorrow" that covered
    /// six days and part of the current one — compared against the `Date`
    /// column, which is local.
    #[test]
    fn the_report_runs_on_the_stations_calendar() {
        // 2026-09-21 00:30 UTC is Sunday 2026-09-20 16:30 at UTC−8.
        let now = 1_789_950_600;
        let pacific = -8 * 3600;
        let day = super::local_day(now, pacific);
        assert_eq!(super::weekday_of(day), 6, "Sunday at the station");
        assert_eq!(
            super::week_range_on(day),
            ("2026-09-20".to_owned(), "2026-09-14".to_owned())
        );
        // Counterpart: at UTC it is Monday already.
        assert_eq!(super::weekday_of(super::local_day(now, 0)), 0);
    }

    #[test]
    fn parse_weekday_valid() {
        assert_eq!(parse_weekday("monday"), Some(0));
        assert_eq!(parse_weekday("TUESDAY"), Some(1));
        assert_eq!(parse_weekday("Sunday"), Some(6));
        assert_eq!(parse_weekday("disabled"), None);
        assert_eq!(parse_weekday("unknown"), None);
    }

    #[test]
    fn today_weekday_returns_valid_day() {
        let (date, wd) = today_weekday();
        assert_eq!(date.len(), 10); // "YYYY-MM-DD"
        assert!(wd <= 6);
    }

    #[test]
    fn days_to_date_str_known_values() {
        assert_eq!(birdnet_core::civil::date_string_from_days(0), "1970-01-01");
        assert_eq!(
            birdnet_core::civil::date_string_from_days(19_723),
            "2024-01-01"
        );
        assert_eq!(
            birdnet_core::civil::date_string_from_days(20_454),
            "2026-01-01"
        );
    }

    #[test]
    fn week_range_strings_is_a_seven_day_window() {
        let (end, start) = week_range_strings();
        assert_eq!(end.len(), 10);
        assert_eq!(start.len(), 10);
        // ISO date strings sort chronologically, and start is six days before end.
        assert!(start <= end);
    }

    fn seeded_state(rows: &[(&str, &str)]) -> birdnet_web::state::AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let (today, _) = week_range_strings();
        // Each seeded row is a *separate* detection, so each needs its own
        // timestamp. They previously all shared '06:00:00' and were only
        // distinct rows because a NULL `File_Name` made the UNIQUE key treat
        // them as unrelated — the same hole that let a re-imported BirdNET-Pi
        // database double itself. Two hits on one species at the very same
        // second, from no clip, are one detection recorded twice; counting them
        // as two is exactly the inflation this report should never show.
        for (i, (sci, com)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
                 VALUES (?1, ?2, ?3, ?4, 0.9)",
                rusqlite::params![today, format!("06:{:02}:00", i), sci, com],
            )
            .unwrap();
        }
        birdnet_web::state::AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    #[test]
    fn build_weekly_report_formats_title_and_top_species() {
        let state = seeded_state(&[
            ("Cardinalis cardinalis", "Northern Cardinal"),
            ("Cardinalis cardinalis", "Northern Cardinal"),
            ("Cyanocitta cristata", "Blue Jay"),
        ]);
        let (title, body) = build_weekly_report(&state).unwrap();
        assert!(title.contains("Weekly Bird Report"));
        assert!(
            title.contains('3'),
            "title should report 3 detections: {title}"
        );
        assert!(body.contains("Top species"));
        assert!(body.contains("Northern Cardinal"));
    }

    /// A sent report survives a restart. The sent date was a local variable
    /// in the loop, so a station restarted on report day — an update, a power
    /// cut, a settings change — sent the week's report again.
    #[test]
    fn a_sent_report_is_remembered_across_a_restart() {
        let now = 1_789_950_600;
        let state = seeded_state(&[]);
        assert!(!already_sent(&state, now).unwrap(), "nothing sent yet");
        record_sent(&state, now).unwrap();

        // The "restart": nothing in memory, only what the database holds.
        assert!(
            already_sent(&state, now + 3600).unwrap(),
            "an hour later the report would go out again"
        );
        assert!(already_sent(&state, now + 5 * 86_400).unwrap());
        // Counterpart: next week's report is not suppressed.
        assert!(!already_sent(&state, now + 7 * 86_400).unwrap());
    }

    #[test]
    fn build_weekly_report_handles_empty_window() {
        let state = seeded_state(&[]);
        let (title, body) = build_weekly_report(&state).unwrap();
        assert!(title.contains("0 detections"));
        assert!(body.contains("Top species"));
    }
}
