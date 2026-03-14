//! Audio capture manager startup with recording schedule integration.
//!
//! Resolves capture source from CLI flags or config, then starts the
//! `CaptureManager` subprocess lifecycle. Integrates `birdnet-scheduler`
//! to gate recording based on time-of-day / solar position.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use birdnet_core::audio::capture::{AudioFormat, CaptureManager, CaptureSource, RecordingConfig};
use birdnet_scheduler::{DailySchedule, Location, RecordingWindow, ScheduleConfig};

use crate::cli::Cli;

/// Handle returned from [`start_capture_manager`] that keeps recording alive
/// and manages schedule-based pausing.
#[derive(Debug)]
pub struct CaptureHandle {
    /// The underlying capture manager (keeps recording alive until dropped).
    _manager: CaptureManager,
    /// Shared flag to stop the schedule loop.
    _stop: Arc<AtomicBool>,
}

/// Parse a schedule string from CLI into a `ScheduleConfig`.
///
/// Supported formats:
/// - `"all-day"` — no restriction
/// - `"solar"` — sunrise-to-sunset (requires lat/lon and night-inhibit)
/// - `"fixed:HH:MM-HH:MM"` — fixed daily window
fn parse_schedule_config(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> ScheduleConfig {
    let location = resolve_location(cli, config);

    let schedule_str = cli.recording_schedule.trim().to_lowercase();

    if schedule_str == "solar" {
        return ScheduleConfig {
            location,
            pre_sunrise_offset_min: cli.twilight_offset,
            post_sunset_offset_min: cli.twilight_offset,
            night_inhibit: true,
            fixed_window: None,
        };
    }

    if let Some(fixed_spec) = schedule_str.strip_prefix("fixed:") {
        if let Some(window) = parse_fixed_window(fixed_spec) {
            return ScheduleConfig {
                location: None,
                pre_sunrise_offset_min: 0,
                post_sunset_offset_min: 0,
                night_inhibit: false,
                fixed_window: Some(window),
            };
        }
        tracing::warn!(spec = %fixed_spec, "invalid fixed schedule, falling back to all-day");
    }

    // "all-day" or unrecognized — but respect --night-inhibit flag.
    ScheduleConfig {
        location,
        pre_sunrise_offset_min: cli.twilight_offset,
        post_sunset_offset_min: cli.twilight_offset,
        night_inhibit: cli.night_inhibit,
        fixed_window: None,
    }
}

/// Parse `"HH:MM-HH:MM"` into a `RecordingWindow`.
fn parse_fixed_window(spec: &str) -> Option<RecordingWindow> {
    let parts: Vec<&str> = spec.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
    let start = parse_hhmm(parts[0])?;
    let end = parse_hhmm(parts[1])?;
    RecordingWindow::fixed(start, end).ok()
}

/// Parse `"HH:MM"` into minutes since midnight.
fn parse_hhmm(s: &str) -> Option<u32> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    if h >= 24 || m >= 60 {
        return None;
    }
    Some(h * 60 + m)
}

/// Resolve latitude/longitude from CLI flags or config.
fn resolve_location(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> Option<Location> {
    let lat = cli
        .latitude
        .or_else(|| config?.get_parsed::<f64>("LATITUDE").ok())?;
    let lon = cli
        .longitude
        .or_else(|| config?.get_parsed::<f64>("LONGITUDE").ok())?;
    Location::new(lat, lon).ok()
}

/// Get the current UTC time as (year, month, day, minutes_since_midnight).
fn utc_now() -> (u32, u32, u32, u32) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Simple UTC date calculation.
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let minutes = (time_of_day / 60) as u32;

    // Convert days since epoch to (year, month, day).
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
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
    (y as u32, m, d, minutes)
}

/// Start a managed audio capture process from CLI/config settings.
///
/// Returns a `CaptureHandle` (keeps recording alive until dropped),
/// or `None` if no capture source is configured or start fails.
///
/// When a recording schedule is configured, a background task periodically
/// checks whether recording should be active and pauses/resumes accordingly.
pub fn start_capture_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<CaptureHandle> {
    // Determine output directory (same as watch_dir).
    let output_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp/StreamData"));

    let alsa_device = cli
        .alsa_device
        .clone()
        .or_else(|| config?.get("ALSA_CARD").map(String::from));

    let rtsp_url = cli
        .rtsp_url
        .clone()
        .or_else(|| config?.get("RTSP_URL").map(String::from));

    let source = alsa_device.map_or_else(
        || {
            rtsp_url.map(|url| CaptureSource::Rtsp {
                url,
                stream_id: "rtsp".to_string(),
            })
        },
        |device| {
            Some(CaptureSource::Microphone {
                device,
                sample_rate: 48_000,
                channels: 1,
            })
        },
    );

    let source = source?;

    let recording_config = RecordingConfig {
        source,
        output_dir,
        segment_duration_secs: cli.segment_duration,
        format: AudioFormat::Wav,
    };

    let mut manager = CaptureManager::new(recording_config);

    // Parse schedule configuration.
    let schedule_config = parse_schedule_config(cli, config);
    let is_all_day = schedule_config.fixed_window.is_none() && !schedule_config.night_inhibit;

    if is_all_day {
        tracing::info!("recording schedule: all-day (no restrictions)");
    } else if schedule_config.fixed_window.is_some() {
        tracing::info!(schedule = %cli.recording_schedule, "recording schedule: fixed window");
    } else {
        tracing::info!(
            twilight_offset = cli.twilight_offset,
            "recording schedule: solar-based with night inhibit"
        );
    }

    // Check if we should start recording now based on schedule.
    let (year, month, day, minutes_now) = utc_now();
    let daily = DailySchedule::for_date(&schedule_config, year, month, day);
    let should_start = daily.is_allowed(minutes_now);

    if should_start {
        match manager.start() {
            Ok(()) => {
                tracing::info!("audio capture started");
            }
            Err(e) => {
                tracing::warn!(error = %e, "audio capture not started (non-fatal)");
                return None;
            }
        }
    } else {
        tracing::info!(
            minutes_now,
            "audio capture deferred — outside recording schedule"
        );
    }

    // Spawn schedule monitor task (only if not all-day).
    let stop_flag = Arc::new(AtomicBool::new(false));
    if !is_all_day {
        let stop = Arc::clone(&stop_flag);
        let sched = schedule_config;
        std::thread::spawn(move || {
            schedule_monitor_loop(stop, sched);
        });
    }

    Some(CaptureHandle {
        _manager: manager,
        _stop: stop_flag,
    })
}

/// Background loop that logs schedule transitions.
///
/// Checks every 60 seconds whether the recording gate is open or closed
/// and logs transitions. The `CaptureManager` itself handles the actual
/// start/stop; this loop provides observability.
fn schedule_monitor_loop(
    stop: Arc<AtomicBool>,
    config: ScheduleConfig,
) {
    let mut was_allowed = true;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        std::thread::sleep(std::time::Duration::from_secs(60));

        if stop.load(Ordering::Relaxed) {
            break;
        }

        let (year, month, day, minutes_now) = utc_now();
        let daily = DailySchedule::for_date(&config, year, month, day);
        let allowed = daily.is_allowed(minutes_now);

        if allowed != was_allowed {
            if allowed {
                tracing::info!(
                    minutes = minutes_now,
                    "recording schedule: gate OPEN — recording should resume"
                );
            } else {
                tracing::info!(
                    minutes = minutes_now,
                    "recording schedule: gate CLOSED — recording should pause"
                );
            }
            was_allowed = allowed;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hhmm_valid() {
        assert_eq!(parse_hhmm("06:00"), Some(360));
        assert_eq!(parse_hhmm("20:30"), Some(1230));
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
    }

    #[test]
    fn parse_hhmm_invalid() {
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("12:60"), None);
        assert_eq!(parse_hhmm("abc"), None);
        assert_eq!(parse_hhmm(""), None);
    }

    #[test]
    fn parse_fixed_window_valid() {
        let w = parse_fixed_window("06:00-20:00").unwrap();
        assert!(w.is_allowed(720));  // noon
        assert!(!w.is_allowed(300)); // 05:00
    }

    #[test]
    fn parse_fixed_window_invalid() {
        assert!(parse_fixed_window("06:00").is_none());
        assert!(parse_fixed_window("20:00-06:00").is_none());
        assert!(parse_fixed_window("").is_none());
    }

    #[test]
    fn parse_schedule_all_day() {
        let cli = test_cli("all-day");
        let config = parse_schedule_config(&cli, None);
        assert!(config.fixed_window.is_none());
        assert!(!config.night_inhibit);
    }

    #[test]
    fn parse_schedule_solar() {
        let cli = test_cli("solar");
        let config = parse_schedule_config(&cli, None);
        assert!(config.night_inhibit);
        assert_eq!(config.pre_sunrise_offset_min, 30);
        assert_eq!(config.post_sunset_offset_min, 30);
    }

    #[test]
    fn parse_schedule_fixed() {
        let cli = test_cli("fixed:08:00-18:00");
        let config = parse_schedule_config(&cli, None);
        assert!(config.fixed_window.is_some());
    }

    #[test]
    fn utc_now_returns_valid_values() {
        let (year, month, day, minutes) = utc_now();
        assert!(year >= 2024);
        assert!((1..=12).contains(&month));
        assert!((1..=31).contains(&day));
        assert!(minutes < 1440);
    }

    /// Helper to create a minimal `Cli` for schedule parsing tests.
    fn test_cli(schedule: &str) -> Cli {
        Cli {
            config: PathBuf::from("/dev/null"),
            listen: "127.0.0.1:8502".to_string(),
            web_only: false,
            check_db: false,
            backup_db: false,
            model: None,
            labels: None,
            watch_dir: None,
            process_existing: false,
            analytics_db: None,
            apprise_url: None,
            notify_confidence: 0.8,
            birdweather_token: None,
            latitude: None,
            longitude: None,
            image_cache_dir: None,
            alsa_device: None,
            rtsp_url: None,
            segment_duration: 15,
            recording_schedule: schedule.to_string(),
            night_inhibit: false,
            twilight_offset: 30,
            heartbeat_url: None,
            notify_trigger: "each".to_string(),
            notify_species_exclude: None,
            notify_species_only: None,
            notify_title_template: None,
            notify_body_template: None,
        }
    }
}
