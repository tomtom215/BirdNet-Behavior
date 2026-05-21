//! Audio capture supervision startup with recording-schedule integration.
//!
//! Resolves the capture source(s) from CLI flags or config, builds a
//! [`CaptureManager`] per source, and hands them to the [`supervisor`] —
//! a background thread that keeps each subprocess alive (restart-on-death
//! with backoff), pauses/resumes them with the recording schedule, and
//! drives the `birdnet_audio_source_up` gauge from real process health.
//!
//! The recording-schedule parsing and the hand-rolled UTC clock live in the
//! [`schedule`] submodule; the restart/backoff/schedule decision logic lives
//! in [`supervisor`]. This module owns only source resolution and thread
//! orchestration.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use birdnet_core::audio::capture::{
    AudioFormat, CaptureError, CaptureManager, CaptureSource, RecordingConfig,
};
use birdnet_scheduler::{DailySchedule, ScheduleConfig};
use birdnet_web::metrics::SharedMetrics;

use crate::cli::Cli;

mod schedule;
mod supervisor;

use supervisor::{Source, Supervisor, source_gauge_label};

/// Bridge the real [`CaptureManager`] into the supervisor's `Source`
/// abstraction. This trivial delegation is the supervisor's only contact with
/// a live subprocess, so it lives here in the orchestration module rather than
/// in `supervisor.rs`, whose restart/backoff/schedule logic is exhaustively
/// fake-source-tested under the mutation gate (a real subprocess can't be
/// driven from a unit test).
impl Source for CaptureManager {
    fn is_running(&mut self) -> bool {
        Self::is_running(self)
    }

    fn start(&mut self) -> Result<(), CaptureError> {
        Self::start(self)
    }

    fn stop(&mut self) {
        Self::stop(self);
    }
}

/// How often the supervisor reconciles each source toward its desired state.
/// Short enough to notice a dead subprocess and resume after a scheduled
/// pause promptly; the per-source backoff timers (not this cadence) govern
/// restart spacing.
const SUPERVISE_TICK: Duration = Duration::from_secs(2);

/// Handle that keeps audio capture alive. Dropping it signals the supervisor
/// thread to stop and joins it, which tears down every capture subprocess
/// (each [`CaptureManager`] kills its child on drop).
#[derive(Debug)]
pub struct CaptureHandle {
    stop_signal: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop_signal.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Resolve all RTSP URLs from CLI flags and config.
///
/// Priority: `--rtsp-urls` (multi) > `--rtsp-url` (single) > config `RTSP_URL`.
fn resolve_rtsp_urls(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> Vec<String> {
    if !cli.rtsp_urls.is_empty() {
        return cli.rtsp_urls.clone();
    }
    let single = cli
        .rtsp_url
        .clone()
        .or_else(|| config?.get("RTSP_URL").map(String::from));
    single.into_iter().collect()
}

/// Resolve the configured capture sources from CLI flags and config.
///
/// Priority: `PipeWire` > ALSA > RTSP. A local microphone (PipeWire/ALSA) may
/// be combined with one or more RTSP streams; RTSP-only is also supported.
fn resolve_sources(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> Vec<CaptureSource> {
    let pipewire_device = cli.pipewire_device.clone();
    let alsa_device = cli
        .alsa_device
        .clone()
        .or_else(|| config.and_then(|c| c.get("ALSA_CARD").map(String::from)));
    let rtsp_urls = resolve_rtsp_urls(cli, config);

    let rtsp_sources = |urls: Vec<String>, mixed: bool| -> Vec<CaptureSource> {
        urls.into_iter()
            .enumerate()
            .map(|(i, url)| {
                let stream_id = if i == 0 && !mixed && cli.rtsp_urls.is_empty() {
                    // Single --rtsp-url with no local mic: keep the plain
                    // "rtsp" id for backward compatibility with filenames.
                    "rtsp".to_string()
                } else {
                    format!("RTSP_{}", i + 1)
                };
                CaptureSource::Rtsp { url, stream_id }
            })
            .collect()
    };

    if let Some(device) = pipewire_device {
        let mut srcs = vec![CaptureSource::PipeWire {
            device,
            sample_rate: 48_000,
            channels: 1,
        }];
        srcs.extend(rtsp_sources(rtsp_urls, true));
        srcs
    } else if let Some(device) = alsa_device {
        let mut srcs = vec![CaptureSource::Microphone {
            device,
            sample_rate: 48_000,
            channels: 1,
        }];
        srcs.extend(rtsp_sources(rtsp_urls, true));
        srcs
    } else {
        rtsp_sources(rtsp_urls, false)
    }
}

/// Log which recording schedule is in effect.
fn log_schedule(cli: &Cli, schedule_config: &ScheduleConfig) {
    if schedule_config.fixed_window.is_some() {
        tracing::info!(schedule = %cli.recording_schedule, "recording schedule: fixed window");
    } else if schedule_config.night_inhibit {
        tracing::info!(
            twilight_offset = cli.twilight_offset,
            "recording schedule: solar-based with night inhibit"
        );
    } else {
        tracing::info!("recording schedule: all-day (no restrictions)");
    }
}

/// Start the supervised audio-capture subsystem from CLI/config settings.
///
/// Returns a [`CaptureHandle`] that keeps recording alive until dropped, or
/// `None` when no capture source is configured. Each source is supervised
/// independently: a dead subprocess is restarted with exponential backoff,
/// and a recording schedule (solar / fixed window) pauses and resumes capture
/// instead of merely logging that it should.
pub fn start_capture_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    metrics: SharedMetrics,
) -> Option<CaptureHandle> {
    let sources = resolve_sources(cli, config);
    if sources.is_empty() {
        return None;
    }

    let output_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config.and_then(|c| c.get("RECS_DIR").map(PathBuf::from)))
        .unwrap_or_else(|| PathBuf::from("/tmp/StreamData"));

    let schedule_config = schedule::parse_schedule_config(cli, config);
    log_schedule(cli, &schedule_config);

    let supervised: Vec<(CaptureManager, String)> = sources
        .into_iter()
        .map(|source| {
            let label = source_gauge_label(&source);
            tracing::info!(source = %label, "audio source configured");
            let recording_config = RecordingConfig {
                source,
                output_dir: output_dir.clone(),
                segment_duration_secs: cli.segment_duration,
                format: AudioFormat::Wav,
            };
            (CaptureManager::new(recording_config), label)
        })
        .collect();

    if supervised.len() > 1 {
        tracing::info!(count = supervised.len(), "multi-stream capture active");
    }

    let supervisor = Supervisor::new(supervised);
    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_for_thread = Arc::clone(&stop_signal);
    let thread = std::thread::spawn(move || {
        run_supervisor(supervisor, &schedule_config, &metrics, &stop_for_thread);
    });

    Some(CaptureHandle {
        stop_signal,
        thread: Some(thread),
    })
}

/// The supervisor's background loop: reconcile every source on a fixed
/// cadence until asked to stop.
///
/// Each tick re-checks whether the system clock looks NTP-synced. A Raspberry
/// Pi has no battery-backed RTC, so at boot the clock can read the epoch (or a
/// stale value) until `systemd-timesyncd`/NTP catches up. While the clock is
/// untrustworthy we **fail open** — record continuously regardless of the
/// solar/fixed window — because trusting a bogus date could drop us into a
/// "night" window and silently lose a whole session. Normal scheduling
/// resumes automatically once the clock becomes plausible.
fn run_supervisor(
    mut supervisor: Supervisor<CaptureManager>,
    schedule_config: &ScheduleConfig,
    metrics: &SharedMetrics,
    stop: &AtomicBool,
) {
    tracing::info!("capture supervisor started");
    // Start from "synced" so an unsynced clock at boot trips the warning on
    // the very first tick.
    let mut clock_synced = true;
    while !stop.load(Ordering::Relaxed) {
        let secs = now_unix_secs();
        let synced = schedule::secs_look_synced(secs);
        if synced != clock_synced {
            log_clock_sync_change(synced);
            clock_synced = synced;
        }
        supervisor.tick(
            Instant::now(),
            recording_allowed(schedule_config, secs),
            metrics,
        );
        sleep_with_stop(SUPERVISE_TICK, stop);
    }
    tracing::info!("capture supervisor stopped");
}

/// Current Unix time in seconds (0 if the clock is somehow before the epoch).
fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether recording is allowed for `secs`, failing **open** while the clock
/// looks unsynced so a bogus boot-time date can't silence the station.
fn recording_allowed(config: &ScheduleConfig, secs: u64) -> bool {
    !schedule::secs_look_synced(secs) || schedule_allows_at(config, secs)
}

/// Evaluate the recording schedule for a given (trusted) Unix timestamp.
fn schedule_allows_at(config: &ScheduleConfig, secs: u64) -> bool {
    let (year, month, day, minutes_now) = schedule::civil_from_unix_secs(secs);
    DailySchedule::for_date(config, year, month, day).is_allowed(minutes_now)
}

/// Log a one-line notice when the clock's apparent sync state changes.
fn log_clock_sync_change(synced: bool) {
    if synced {
        tracing::info!("system clock looks NTP-synced; honouring the recording schedule");
    } else {
        tracing::warn!(
            "system clock looks UNSYNCED (no RTC, NTP not ready) — recording continuously so no \
             session is missed; detection timestamps may be wrong until time syncs"
        );
    }
}

/// Sleep up to `total`, waking early when `stop` is set so shutdown stays
/// responsive — without a busy loop.
fn sleep_with_stop(total: Duration, stop: &AtomicBool) {
    const STEP: Duration = Duration::from_millis(200);
    let mut elapsed = Duration::ZERO;
    while elapsed < total {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(STEP);
        elapsed += STEP;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    #[test]
    fn no_source_configured_returns_empty() {
        assert!(resolve_sources(&cli(), None).is_empty());
    }

    #[test]
    fn single_rtsp_url_keeps_plain_id() {
        let mut c = cli();
        c.rtsp_url = Some("rtsp://cam.local/stream".to_string());
        let sources = resolve_sources(&c, None);
        assert_eq!(sources.len(), 1);
        match &sources[0] {
            CaptureSource::Rtsp { stream_id, .. } => assert_eq!(stream_id, "rtsp"),
            other => panic!("expected RTSP source, got {other:?}"),
        }
    }

    #[test]
    fn alsa_plus_rtsp_yields_mic_and_numbered_stream() {
        let mut c = cli();
        c.alsa_device = Some("plughw:1,0".to_string());
        c.rtsp_url = Some("rtsp://cam.local/stream".to_string());
        let sources = resolve_sources(&c, None);
        assert_eq!(sources.len(), 2);
        assert!(matches!(sources[0], CaptureSource::Microphone { .. }));
        match &sources[1] {
            // Mixed with a local mic → the stream is numbered, not "rtsp".
            CaptureSource::Rtsp { stream_id, .. } => assert_eq!(stream_id, "RTSP_1"),
            other => panic!("expected RTSP source, got {other:?}"),
        }
    }

    #[test]
    fn multiple_rtsp_urls_are_numbered() {
        let mut c = cli();
        c.rtsp_urls = vec![
            "rtsp://a.local/s".to_string(),
            "rtsp://b.local/s".to_string(),
        ];
        let sources = resolve_sources(&c, None);
        assert_eq!(sources.len(), 2);
        let ids: Vec<_> = sources
            .iter()
            .map(|s| match s {
                CaptureSource::Rtsp { stream_id, .. } => stream_id.clone(),
                other => panic!("expected RTSP, got {other:?}"),
            })
            .collect();
        assert_eq!(ids, vec!["RTSP_1", "RTSP_2"]);
    }

    #[test]
    fn pipewire_takes_priority_over_alsa() {
        let mut c = cli();
        c.pipewire_device = Some(String::new());
        c.alsa_device = Some("plughw:1,0".to_string());
        let sources = resolve_sources(&c, None);
        assert!(matches!(sources[0], CaptureSource::PipeWire { .. }));
    }

    fn fixed_window_config(spec: &str) -> ScheduleConfig {
        let mut c = cli();
        c.recording_schedule = spec.to_string();
        schedule::parse_schedule_config(&c, None)
    }

    #[test]
    fn recording_allowed_fails_open_when_clock_unsynced() {
        // A window that excludes 00:00. At the epoch (an unset-RTC reading) the
        // clock is untrustworthy, so we record anyway rather than believe the
        // bogus 1970 date and stay silent. Without the fail-open guard the
        // 06:00–07:00 window would reject 00:00 and this would be false.
        let config = fixed_window_config("fixed:06:00-07:00");
        assert!(recording_allowed(&config, 0));
    }

    #[test]
    fn recording_allowed_honours_schedule_once_clock_synced() {
        let config = fixed_window_config("fixed:06:00-07:00");
        let midnight_2024 = 1_704_067_200; // 2024-01-01 00:00:00 UTC — clock looks synced
        // 06:30 UTC is inside the window.
        assert!(recording_allowed(
            &config,
            midnight_2024 + 6 * 3600 + 30 * 60
        ));
        // 12:00 UTC is outside — and the clock is trusted, so we honour it.
        assert!(!recording_allowed(&config, midnight_2024 + 12 * 3600));
    }
}
