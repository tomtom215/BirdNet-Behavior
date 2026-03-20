//! Weekly detection report notification scheduler.
//!
//! Sends a weekly summary of bird detections via Apprise on a configured
//! weekday. The report includes the top 10 species by detection count and
//! the total number of detections for the past 7 days.
//!
//! BirdNET-Pi equivalent: `weekly_report.sh` cron job.

use std::sync::Arc;
use tokio::sync::Mutex;

use birdnet_integrations::apprise::{Client as AppriseClient, NotifyType};

/// Start the weekly report scheduler as a background tokio task.
///
/// Wakes up hourly, checks if today is the configured weekday and if the
/// report has already been sent today. If not, generates and sends the report.
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
        "disabled" | "" => None,
        _ => None,
    }
}

/// The main loop: wakes up hourly, sends report on the right weekday.
async fn weekly_report_loop(
    target_weekday: u8,
    apprise: Arc<Mutex<AppriseClient>>,
    state: birdnet_web::state::AppState,
) {
    // Track the last date we sent a report to avoid duplicates.
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

        tracing::info!(date = %today_str, "sending weekly detection report");

        match build_weekly_report(&state) {
            Ok((title, body)) => {
                let client = apprise.lock().await;
                if let Err(e) = client
                    .send_notification(&title, &body, NotifyType::Info)
                    .await
                {
                    tracing::warn!(error = %e, "weekly report notification failed");
                } else {
                    tracing::info!("weekly report sent");
                    last_sent_date = Some(today_str);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to build weekly report");
            }
        }
    }
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
        body.push_str(&format!("{}. {} — {count} detections\n", i + 1, com_name));
    }

    Ok((title, body))
}

/// Return `(today_str, seven_days_ago_str)` as ISO date strings.
fn week_range_strings() -> (String, String) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let today_days = secs / 86400;
    let start_days = today_days.saturating_sub(6); // 7 days inclusive

    (days_to_date_str(today_days), days_to_date_str(start_days))
}

/// Convert days since Unix epoch to `"YYYY-MM-DD"`.
fn days_to_date_str(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let y_u32 = y as u32;
    format!("{y_u32:04}-{m:02}-{d:02}")
}

/// Return today's ISO date string and ISO weekday (0 = Mon, 6 = Sun).
fn today_weekday() -> (String, u8) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let days = secs / 86400;

    // Convert days since epoch to (year, month, day) — same algorithm as capture.rs.
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (year, month, day) = (y as u32, m, d);

    // ISO weekday: (days_since_epoch + 3) % 7, where 0=Mon.
    let weekday = ((days + 3) % 7) as u8;

    let date_str = format!("{year:04}-{month:02}-{day:02}");
    (date_str, weekday)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(date.len() == 10); // "YYYY-MM-DD"
        assert!(wd <= 6);
    }
}
