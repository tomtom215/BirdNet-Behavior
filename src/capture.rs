//! Audio capture supervision startup with recording-schedule integration.
//!
//! This module owns only the thread orchestration: it asks [`sources`] to
//! resolve the capture source(s) from the `audio_sources` table or CLI/config,
//! builds a [`CaptureManager`] per source, and spawns the [`runloop`] that
//! drives the [`supervisor`] — keeping each subprocess alive (restart-on-death
//! with backoff), pausing/resuming with the recording schedule, and driving the
//! `birdnet_audio_source_up` gauge from real process health.
//!
//! Submodules:
//! * [`sources`] — resolve CLI/config/`audio_sources` rows into capture sources.
//! * [`runloop`] — the supervisor loop and its per-tick schedule/clock gating.
//! * [`schedule`] — recording-schedule parsing and the hand-rolled civil clock.
//! * [`supervisor`] — restart/backoff/schedule decision logic (source-agnostic).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use birdnet_core::audio::capture::{
    AudioFormat, CaptureError, CaptureManager, CaptureStatusHandle, LiveAudioHubHandle,
    LocalOffset, RecordingConfig,
};
use birdnet_scheduler::ScheduleConfig;
use birdnet_web::metrics::SharedMetrics;

use crate::cli::Cli;

mod runloop;
pub mod schedule;

/// The resolved recording window for a given CLI + config pair.
///
/// Exposed for the settings-bridge test that proves a schedule chosen on
/// `/admin/settings` actually reaches the capture supervisor — the hop that was
/// broken until 0.12.0, when this resolver read the CLI default and ignored the
/// configured value.
#[cfg(test)]
pub fn schedule_config_for_test(
    cli: &crate::cli::Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_scheduler::ScheduleConfig {
    schedule::parse_schedule_config(cli, config)
}
mod sources;
mod supervisor;
mod uptime;

use runloop::run_supervisor;
use sources::{ResolvedSource, resolve_sources, resolve_sources_from_db, seed_sources_from_config};
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

    fn latest_output_age(&mut self) -> Option<Duration> {
        Self::latest_output_age(self)
    }
}

/// Silent-stall threshold for a source writing `segment_secs`-long segments.
///
/// Several consecutive segments must be overdue before the supervisor calls
/// a running process stalled — one slow write or a settling hiccup must not
/// bounce a healthy source — with a floor so short segments (3 s minimum)
/// don't make the verdict hair-triggered.
fn stall_threshold(segment_secs: u32) -> Duration {
    const STALL_SEGMENTS: u64 = 4;
    const STALL_FLOOR: Duration = Duration::from_secs(120);
    Duration::from_secs(u64::from(segment_secs).saturating_mul(STALL_SEGMENTS)).max(STALL_FLOOR)
}

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

/// Start the supervised audio-capture subsystem.
///
/// Returns a [`CaptureHandle`] that keeps recording alive until dropped, or
/// `None` when no capture source is configured. Each source is supervised
/// independently: a dead subprocess is restarted with exponential backoff,
/// and a recording schedule (solar / fixed window) pauses and resumes capture
/// instead of merely logging that it should.
///
/// ## Source resolution (O-13)
///
/// When `state` is `Some(_)`, an empty `audio_sources` table is first seeded
/// from the CLI/config sources (see [`sources::seed_sources_from_config`]) so
/// the table becomes the single source of truth — for the daemon and the web
/// surface (live `/stream`, Listen, `/admin/audio`). Then:
///
/// 1. **Database-driven** (the normal path): the non-disabled `audio_sources`
///    rows — what the admin UI's CRUD manages — are the source of truth.
/// 2. **CLI/config fallback**: only when there is no `state` (a headless
///    capture-only invocation) or the table is still empty (seeding found
///    nothing / failed) does the BirdNET-Pi-style `--rtsp-url` /
///    `--alsa-device` / `--pipewire-device` resolution drive capture directly.
///
/// A single info-level log line announces which path won, so the operator can
/// diagnose "why is my CLI arg being ignored" from the first lines of the
/// journal.
pub fn start_capture_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: Option<&birdnet_web::state::AppState>,
    metrics: SharedMetrics,
    status: CaptureStatusHandle,
    live_audio: Option<&LiveAudioHubHandle>,
) -> Option<CaptureHandle> {
    // O-13: seed an empty audio_sources table from CLI/config first, so the
    // table is the single source of truth for capture and the web surface.
    if let Some(state) = state {
        let seeded = seed_sources_from_config(state, cli, config);
        if seeded > 0 {
            tracing::info!(
                count = seeded,
                "seeded audio_sources from CLI/config (O-13)"
            );
        }
    }
    let sources: Vec<ResolvedSource> = if let Some(state) = state
        && let Some(db_sources) = resolve_sources_from_db(state)
        && !db_sources.is_empty()
    {
        tracing::info!(
            count = db_sources.len(),
            "capture sources resolved from audio_sources table (O-13)"
        );
        db_sources
    } else {
        let cli_sources = resolve_sources(cli, config);
        if !cli_sources.is_empty() {
            tracing::info!(
                count = cli_sources.len(),
                "capture sources resolved from CLI/config (no audio_sources rows)"
            );
        }
        // The bare CLI/config flags carry no gain or quiet window — those are
        // admin-UI / audio_sources concepts — so resolve to unity gain, none.
        cli_sources
            .into_iter()
            .map(|source| ResolvedSource {
                source,
                gain_db: 0.0,
                pipeline: birdnet_core::audio::capture::AudioPipeline::none(),
                eq_chain: birdnet_core::audio::eq::EqChain::default(),
                quiet: None,
            })
            .collect()
    };
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

    // Resolved, not read straight off the CLI: `--segment-duration` carries a
    // clap default, so the raw field always wins and the settings-page field
    // could never take effect. The stall threshold is derived from the same
    // value so the supervisor's patience tracks the real segment length.
    let segment_duration = crate::helpers::resolve::setting::<u32>(
        cli,
        "segment_duration",
        cli.segment_duration,
        config,
        "SEGMENT_DURATION",
    );
    let stall_after = stall_threshold(segment_duration);

    // Segment filenames carry **local** civil time, because that is what
    // `arecord --use-strftime` wrote and what `RecordingFile::parse` turns into
    // each detection's `Date`/`Time`. The in-process segment writer needs the
    // station's UTC offset to reproduce those names, so it is seeded here and
    // refreshed by the supervisor on every tick — a snapshot would be an hour
    // wrong for months after the next daylight-saving change.
    let local_offset = LocalOffset::new(birdnet_db::clock::local_utc_offset_secs());
    tracing::info!(
        utc_offset_secs = local_offset.get(),
        "capture will stamp segment filenames in local time"
    );

    let supervised: Vec<(
        CaptureManager,
        String,
        Option<supervisor::QuietWindow>,
        Duration,
    )> = sources
        .into_iter()
        .map(|resolved| {
            let ResolvedSource {
                source,
                gain_db,
                pipeline,
                eq_chain,
                quiet,
            } = resolved;
            let label = source_gauge_label(&source);
            tracing::info!(
                source = %label,
                gain_db,
                high_pass = pipeline.high_pass,
                dc_removal = pipeline.dc_removal,
                agc = pipeline.agc,
                eq = %eq_chain.to_spec(),
                quiet = quiet.is_some(),
                "audio source configured"
            );
            let recording_config = RecordingConfig {
                source,
                output_dir: output_dir.clone(),
                segment_duration_secs: segment_duration,
                format: AudioFormat::Wav,
                gain_db,
                pipeline,
                eq_chain,
                local_offset: local_offset.clone(),
                live_audio: live_audio.cloned(),
            };
            (
                CaptureManager::new(recording_config),
                label,
                quiet,
                stall_after,
            )
        })
        .collect();

    if supervised.len() > 1 {
        tracing::info!(count = supervised.len(), "multi-stream capture active");
    }

    let supervisor = Supervisor::new(supervised);
    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_for_thread = Arc::clone(&stop_signal);
    let thread = std::thread::spawn(move || {
        run_supervisor(
            supervisor,
            &schedule_config,
            &metrics,
            &status,
            &local_offset,
            &stop_for_thread,
        );
    });

    Some(CaptureHandle {
        stop_signal,
        thread: Some(thread),
    })
}
