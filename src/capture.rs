//! Audio capture manager startup with recording schedule integration.
//!
//! Resolves capture source from CLI flags or config, then starts the
//! `CaptureManager` subprocess lifecycle. The recording-schedule parsing and
//! the time-gate monitor loop live in the [`schedule`] submodule; this module
//! owns the capture-source resolution and process orchestration.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use birdnet_core::audio::capture::{AudioFormat, CaptureManager, CaptureSource, RecordingConfig};
use birdnet_scheduler::DailySchedule;

use crate::cli::Cli;

mod schedule;

/// Handle returned from [`start_capture_manager`] that keeps recording alive
/// and manages schedule-based pausing.
#[derive(Debug)]
pub struct CaptureHandle {
    /// The underlying capture manager (keeps recording alive until dropped).
    _manager: CaptureManager,
    /// Shared flag to stop the schedule loop.
    stop_signal: Arc<AtomicBool>,
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

/// Start managed audio capture processes from CLI/config settings.
///
/// Returns a `Vec<CaptureHandle>` (keeps recording alive until dropped).
/// Multiple RTSP URLs each get their own independent capture pipeline,
/// with filenames prefixed `RTSP_1-`, `RTSP_2-`, etc.
///
/// When a recording schedule is configured, a background task periodically
/// checks whether recording should be active and pauses/resumes accordingly.
#[allow(clippy::too_many_lines)]
pub fn start_capture_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Vec<CaptureHandle> {
    // Determine output directory (same as watch_dir).
    let output_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp/StreamData"));

    let pipewire_device = cli.pipewire_device.clone();

    let alsa_device = cli
        .alsa_device
        .clone()
        .or_else(|| config?.get("ALSA_CARD").map(String::from));

    let rtsp_urls = resolve_rtsp_urls(cli, config);

    // Build the list of capture sources.
    // Priority: PipeWire > ALSA > RTSP
    let sources: Vec<CaptureSource> = if let Some(device) = pipewire_device {
        // PipeWire/PulseAudio microphone (ffmpeg -f pulse).
        let mut srcs = vec![CaptureSource::PipeWire {
            device,
            sample_rate: 48_000,
            channels: 1,
        }];
        for (i, url) in rtsp_urls.into_iter().enumerate() {
            srcs.push(CaptureSource::Rtsp {
                url,
                stream_id: format!("RTSP_{}", i + 1),
            });
        }
        srcs
    } else if let Some(device) = alsa_device {
        // ALSA microphone; RTSP streams are additional.
        let mut srcs = vec![CaptureSource::Microphone {
            device,
            sample_rate: 48_000,
            channels: 1,
        }];
        for (i, url) in rtsp_urls.into_iter().enumerate() {
            srcs.push(CaptureSource::Rtsp {
                url,
                stream_id: format!("RTSP_{}", i + 1),
            });
        }
        srcs
    } else if !rtsp_urls.is_empty() {
        rtsp_urls
            .into_iter()
            .enumerate()
            .map(|(i, url)| {
                let stream_id = if i == 0 && cli.rtsp_urls.is_empty() {
                    // Single --rtsp-url: use plain "rtsp" for backward compat.
                    "rtsp".to_string()
                } else {
                    format!("RTSP_{}", i + 1)
                };
                CaptureSource::Rtsp { url, stream_id }
            })
            .collect()
    } else {
        return Vec::new();
    };

    // Parse schedule configuration.
    let schedule_config = schedule::parse_schedule_config(cli, config);
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
    let (year, month, day, minutes_now) = schedule::utc_now();
    let daily = DailySchedule::for_date(&schedule_config, year, month, day);
    let should_start = daily.is_allowed(minutes_now);

    let mut handles = Vec::new();

    for source in sources {
        let source_label = match &source {
            CaptureSource::Microphone { device, .. } => format!("mic:{device}"),
            CaptureSource::PipeWire { device, .. } => format!(
                "pulse:{}",
                if device.is_empty() {
                    "default"
                } else {
                    device.as_str()
                }
            ),
            CaptureSource::Rtsp { stream_id, .. } => stream_id.clone(),
        };

        let recording_config = RecordingConfig {
            source,
            output_dir: output_dir.clone(),
            segment_duration_secs: cli.segment_duration,
            format: AudioFormat::Wav,
        };

        let mut manager = CaptureManager::new(recording_config);

        if should_start {
            match manager.start() {
                Ok(()) => {
                    tracing::info!(source = %source_label, "audio capture started");
                }
                Err(e) => {
                    tracing::warn!(source = %source_label, error = %e, "audio capture not started (non-fatal)");
                    continue;
                }
            }
        } else {
            tracing::info!(
                source = %source_label,
                minutes_now,
                "audio capture deferred — outside recording schedule"
            );
        }

        let stop_flag = Arc::new(AtomicBool::new(false));
        handles.push(CaptureHandle {
            _manager: manager,
            stop_signal: stop_flag,
        });
    }

    // Spawn a single schedule monitor task (only if not all-day).
    if !is_all_day && !handles.is_empty() {
        let stop = Arc::clone(&handles[0].stop_signal);
        let sched = schedule_config;
        std::thread::spawn(move || {
            schedule::schedule_monitor_loop(stop, sched);
        });
    }

    if handles.len() > 1 {
        tracing::info!(count = handles.len(), "multi-stream capture active");
    }

    handles
}
