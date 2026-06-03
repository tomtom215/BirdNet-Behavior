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

/// Resolve all local ALSA microphone devices from CLI flags and config.
///
/// Priority: `--alsa-devices` (multi) > `--alsa-device` (single) > config
/// `ALSA_CARDS` (multi) > config `ALSA_CARD` (single). Devices are separated by
/// `;` — not `,` — because ALSA names contain commas (`plughw:1,0`). Blank
/// entries are dropped so a trailing separator or empty config value can't
/// spawn a mic on `""`.
fn resolve_alsa_devices(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> Vec<String> {
    let split = |s: &str| -> Vec<String> {
        s.split(';')
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    };

    if !cli.alsa_devices.is_empty() {
        return cli.alsa_devices.clone();
    }
    if let Some(device) = cli.alsa_device.clone().filter(|d| !d.trim().is_empty()) {
        return vec![device];
    }
    if let Some(config) = config {
        if let Some(multi) = config
            .get("ALSA_CARDS")
            .map(split)
            .filter(|v| !v.is_empty())
        {
            return multi;
        }
        if let Some(single) = config.get("ALSA_CARD").filter(|d| !d.trim().is_empty()) {
            return vec![single.trim().to_owned()];
        }
    }
    Vec::new()
}

/// Resolve capture sources from the `audio_sources` SQLite table.
///
/// Returns `None` when the DB read errors (the caller treats that as
/// "fall back to CLI/config"); returns `Some(empty)` when the table is
/// present but every row is disabled (the caller also falls back); and
/// `Some(non-empty)` only when at least one row is active.
///
/// O-13 stage 1 — additive layer over the existing CLI/config path.
/// Subsequent stages will retire the legacy path once every deployed
/// station has migrated its sources into the table.
fn resolve_sources_from_db(state: &birdnet_web::state::AppState) -> Option<Vec<CaptureSource>> {
    use birdnet_db::audio_sources::AudioSourceStore;
    let rows = state.with_db(|conn| AudioSourceStore::list(conn).ok())?;
    let active: Vec<_> = rows
        .into_iter()
        .filter(|s| s.disabled_at.is_none())
        .collect();
    if active.is_empty() {
        return None;
    }

    // Group by kind so we can apply the same `stream_id` "single-mic
    // keeps id-less filename" heuristic the legacy resolver uses. If
    // there's exactly one local-mic row (UsbAlsa or PipeWire), it gets
    // a `None` stream_id (BirdNET-Pi-compatible filename). Otherwise
    // every local mic carries its own id.
    let local_count = active
        .iter()
        .filter(|s| {
            use birdnet_db::audio_sources::SourceKind;
            matches!(s.kind, SourceKind::UsbAlsa | SourceKind::PipeWire)
        })
        .count();

    let mut out = Vec::with_capacity(active.len());
    let mut rtsp_index = 0_usize;
    for row in active {
        out.push(audio_source_to_capture_source(
            &row,
            local_count > 1,
            &mut rtsp_index,
        ));
    }
    Some(out)
}

/// Translate one [`birdnet_db::audio_sources::AudioSource`] row to a
/// [`CaptureSource`] enum variant the supervisor consumes.
///
/// In the DB-driven path the `stream_id` is **always** `row.id` so the
/// `audio_source_up{source}` gauge label matches the audio-source row
/// the admin UI manipulates. The probe-pill handler reads the same
/// `row.id` back, making `/admin/audio` show honest per-source liveness
/// instead of the synthetic "first row Capturing, rest Down" stub.
///
/// The `_multi_local` / `_rtsp_index` parameters are retained for
/// backwards-compatibility with the call shape from #102, but they're
/// no longer consulted — every row gets its own stable `row.id`-labelled
/// gauge, including lone-mic and RTSP rows that used to fall back to
/// `local` / `RTSP_N`.
fn audio_source_to_capture_source(
    row: &birdnet_db::audio_sources::AudioSource,
    _multi_local: bool,
    _rtsp_index: &mut usize,
) -> CaptureSource {
    use birdnet_db::audio_sources::{Channels, SourceKind};
    // Mono / Left / Right all collapse to one channel — the daemon
    // only consumes one channel today; an ffmpeg `pan` filter could
    // pick the half for Left/Right later without changing this surface.
    let channels: u16 = match row.channels {
        Channels::Mono | Channels::Left | Channels::Right => 1,
        Channels::Stereo => 2,
    };
    match row.kind {
        SourceKind::UsbAlsa => CaptureSource::Microphone {
            device: row.device_id.clone(),
            sample_rate: row.sample_rate,
            channels,
            stream_id: Some(row.id.clone()),
        },
        SourceKind::PipeWire => CaptureSource::PipeWire {
            device: row.device_id.clone(),
            sample_rate: row.sample_rate,
            channels,
            stream_id: Some(row.id.clone()),
        },
        SourceKind::Rtsp => CaptureSource::Rtsp {
            url: row.device_id.clone(),
            stream_id: row.id.clone(),
        },
    }
}

/// Resolve the configured capture sources from CLI flags and config.
///
/// Priority: `PipeWire` > ALSA > RTSP. One or more local microphones
/// (PipeWire, or one/several ALSA devices) may be combined with one or more
/// RTSP streams; RTSP-only is also supported. A lone local mic keeps the
/// historical id-less filename and `local` metrics label; when several local
/// mics are configured each is labelled `MIC_1`, `MIC_2`, … so its recordings
/// and health metric stay distinct.
fn resolve_sources(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> Vec<CaptureSource> {
    let pipewire_device = cli.pipewire_device.clone();
    let alsa_devices = resolve_alsa_devices(cli, config);
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
            stream_id: None,
        }];
        srcs.extend(rtsp_sources(rtsp_urls, true));
        srcs
    } else if !alsa_devices.is_empty() {
        let multi = alsa_devices.len() > 1;
        let mut srcs: Vec<CaptureSource> = alsa_devices
            .into_iter()
            .enumerate()
            .map(|(i, device)| CaptureSource::Microphone {
                device,
                sample_rate: 48_000,
                channels: 1,
                // A single mic keeps `None` (id-less filename, `local` label);
                // several mics each get a stable `MIC_n` id.
                stream_id: multi.then(|| format!("MIC_{}", i + 1)),
            })
            .collect();
        srcs.extend(rtsp_sources(rtsp_urls, true));
        srcs
    } else {
        rtsp_sources(rtsp_urls, false)
    }
}

/// O-13: seed the `audio_sources` table from CLI/config when it is empty.
///
/// Makes the table the single source of truth for both the capture daemon and
/// the web surface (live `/stream`, the Listen page, the `/admin/audio` pills).
/// Idempotent — it inserts nothing when the table already holds a row
/// (including one migration 15 seeded from a legacy `settings.audio_source`),
/// so an operator's later edits or deletions through the admin UI are never
/// re-seeded on the next start. Returns the number of rows seeded.
fn seed_sources_from_config(
    state: &birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> usize {
    use birdnet_db::audio_sources::AudioSourceStore;
    // Only ever seed a completely empty table. A read error is treated as
    // "already populated" so a transient failure can't double-seed — the
    // CLI/config fallback in `start_capture_manager` still drives capture.
    if !matches!(
        state.with_db(|conn| AudioSourceStore::list(conn).map(|rows| rows.is_empty())),
        Ok(true)
    ) {
        return 0;
    }
    let mut seeded = 0_usize;
    for (i, source) in resolve_sources(cli, config).into_iter().enumerate() {
        let new = capture_source_to_new(format!("src_seed_{}", i + 1), source);
        match state.with_db(|conn| conn.insert(&new)) {
            Ok(_) => seeded += 1,
            Err(e) => tracing::warn!(error = %e, id = %new.id, "audio_sources seed failed"),
        }
    }
    seeded
}

/// Map a CLI/config-resolved [`CaptureSource`] to a `NewAudioSource` row,
/// preserving device, sample rate and channel count so the seeded row
/// reconstructs the same capture stream. The inverse of
/// [`audio_source_to_capture_source`].
fn capture_source_to_new(
    id: String,
    source: CaptureSource,
) -> birdnet_db::audio_sources::NewAudioSource {
    use birdnet_db::audio_sources::{Channels, NewAudioSource, SourceKind};
    let channels_of = |channels: u16| {
        if channels >= 2 {
            Channels::Stereo
        } else {
            Channels::Mono
        }
    };
    match source {
        CaptureSource::Microphone {
            device,
            sample_rate,
            channels,
            ..
        } => {
            let mut new = NewAudioSource::defaults(id, SourceKind::UsbAlsa, device);
            new.sample_rate = sample_rate;
            new.channels = channels_of(channels);
            new
        }
        CaptureSource::PipeWire {
            device,
            sample_rate,
            channels,
            ..
        } => {
            let mut new = NewAudioSource::defaults(id, SourceKind::PipeWire, device);
            new.sample_rate = sample_rate;
            new.channels = channels_of(channels);
            new
        }
        CaptureSource::Rtsp { url, .. } => NewAudioSource::defaults(id, SourceKind::Rtsp, url),
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
/// from the CLI/config sources (see [`seed_sources_from_config`]) so the table
/// becomes the single source of truth — for the daemon and the web surface
/// (live `/stream`, Listen, `/admin/audio`). Then:
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
    let sources = if let Some(state) = state
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
        cli_sources
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

    #[test]
    fn resolve_rtsp_urls_falls_back_to_config() {
        use birdnet_core::config::Config;
        // No CLI flags → the single RTSP_URL config key is used.
        let cfg = Config::parse("RTSP_URL=rtsp://cam.local/s").unwrap();
        assert_eq!(
            resolve_rtsp_urls(&cli(), Some(&cfg)),
            vec!["rtsp://cam.local/s".to_string()]
        );
        // No flags and no config → empty.
        assert!(resolve_rtsp_urls(&cli(), None).is_empty());
    }

    #[test]
    fn resolve_sources_reads_alsa_card_from_config() {
        use birdnet_core::config::Config;
        let cfg = Config::parse("ALSA_CARD=plughw:2,0").unwrap();
        let sources = resolve_sources(&cli(), Some(&cfg));
        assert_eq!(sources.len(), 1);
        assert!(matches!(sources[0], CaptureSource::Microphone { .. }));
    }

    /// Extract the `stream_id` of every resolved microphone, panicking on any
    /// non-microphone source — keeps the multi-mic assertions terse.
    fn mic_stream_ids(sources: &[CaptureSource]) -> Vec<Option<String>> {
        sources
            .iter()
            .map(|s| match s {
                CaptureSource::Microphone { stream_id, .. } => stream_id.clone(),
                other => panic!("expected microphone, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn single_alsa_device_has_no_stream_id() {
        let mut c = cli();
        c.alsa_device = Some("plughw:1,0".to_string());
        let sources = resolve_sources(&c, None);
        // A lone mic keeps the historical id-less filename and `local` label.
        assert_eq!(mic_stream_ids(&sources), vec![None]);
    }

    #[test]
    fn multiple_alsa_devices_get_numbered_mic_ids() {
        let mut c = cli();
        c.alsa_devices = vec!["plughw:1,0".to_string(), "plughw:2,0".to_string()];
        let sources = resolve_sources(&c, None);
        assert_eq!(
            mic_stream_ids(&sources),
            vec![Some("MIC_1".to_string()), Some("MIC_2".to_string())]
        );
    }

    #[test]
    fn alsa_cards_config_splits_on_semicolon_preserving_commas() {
        use birdnet_core::config::Config;
        // ALSA names contain commas, so devices are separated by ';'. The
        // card,device commas inside each name must survive the split.
        let cfg = Config::parse("ALSA_CARDS=plughw:1,0;plughw:2,0").unwrap();
        let sources = resolve_sources(&cli(), Some(&cfg));
        let devices: Vec<_> = sources
            .iter()
            .map(|s| match s {
                CaptureSource::Microphone { device, .. } => device.clone(),
                other => panic!("expected microphone, got {other:?}"),
            })
            .collect();
        assert_eq!(
            devices,
            vec!["plughw:1,0".to_string(), "plughw:2,0".to_string()]
        );
        assert_eq!(
            mic_stream_ids(&sources),
            vec![Some("MIC_1".to_string()), Some("MIC_2".to_string())]
        );
    }

    #[test]
    fn alsa_devices_cli_overrides_single_and_config() {
        use birdnet_core::config::Config;
        let cfg = Config::parse("ALSA_CARD=plughw:9,0").unwrap();
        let mut c = cli();
        c.alsa_device = Some("plughw:8,0".to_string());
        c.alsa_devices = vec!["plughw:1,0".to_string(), "plughw:2,0".to_string()];
        // --alsa-devices wins over both --alsa-device and the config keys.
        let devices: Vec<_> = resolve_alsa_devices(&c, Some(&cfg));
        assert_eq!(
            devices,
            vec!["plughw:1,0".to_string(), "plughw:2,0".to_string()]
        );
    }

    #[test]
    fn sleep_with_stop_returns_promptly_when_already_stopped() {
        let stop = AtomicBool::new(true);
        let start = Instant::now();
        sleep_with_stop(Duration::from_secs(10), &stop);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "must wake immediately when the stop flag is already set"
        );
    }

    #[test]
    fn now_unix_secs_is_after_2020() {
        // 2020-01-01 UTC; a sanity floor that catches a broken clock helper.
        assert!(now_unix_secs() > 1_577_836_800);
    }

    // ------------------------------------------------------------------
    // O-13 — audio_sources → CaptureSource translator + DB resolver
    // ------------------------------------------------------------------

    use birdnet_db::audio_sources::{
        AudioSource, Channels, PipelineFlags, RtspTransport, SourceKind,
    };

    fn row(id: &str, kind: SourceKind, device_id: &str) -> AudioSource {
        AudioSource {
            id: id.to_string(),
            kind,
            device_id: device_id.to_string(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 16,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            disabled_at: None,
            created_at: "2026-05-01".to_string(),
            updated_at: "2026-05-01".to_string(),
        }
    }

    #[test]
    fn translator_usb_alsa_to_microphone_uses_row_id_as_stream_id() {
        // Stage 2 — every DB-driven source carries `stream_id = Some(row.id)`
        // so the `audio_source_up{<row.id>}` gauge matches the audio_sources
        // row the admin UI manipulates. (The lone-mic id-less filename
        // backward-compat carve-out from stage 1 is gone — operators who
        // opt into the DB path accept the new naming.)
        let r = row("src_usb_1", SourceKind::UsbAlsa, "plughw:1,0");
        let mut idx = 0;
        let cs = audio_source_to_capture_source(&r, false, &mut idx);
        match cs {
            CaptureSource::Microphone {
                device,
                sample_rate,
                channels,
                stream_id,
            } => {
                assert_eq!(device, "plughw:1,0");
                assert_eq!(sample_rate, 48_000);
                assert_eq!(channels, 1);
                assert_eq!(stream_id.as_deref(), Some("src_usb_1"));
            }
            other => panic!("expected Microphone, got {other:?}"),
        }
    }

    #[test]
    fn translator_usb_alsa_to_microphone_keeps_id_when_multi() {
        let r = row("src_usb_2", SourceKind::UsbAlsa, "plughw:2,0");
        let mut idx = 0;
        let cs = audio_source_to_capture_source(&r, true, &mut idx);
        match cs {
            CaptureSource::Microphone { stream_id, .. } => {
                assert_eq!(stream_id.as_deref(), Some("src_usb_2"));
            }
            other => panic!("expected Microphone, got {other:?}"),
        }
    }

    #[test]
    fn translator_pipewire_passes_through_device() {
        let r = row("src_pw_1", SourceKind::PipeWire, "alsa_input.usb-Edifier");
        let mut idx = 0;
        let cs = audio_source_to_capture_source(&r, false, &mut idx);
        match cs {
            CaptureSource::PipeWire { device, .. } => {
                assert_eq!(device, "alsa_input.usb-Edifier");
            }
            other => panic!("expected PipeWire, got {other:?}"),
        }
    }

    #[test]
    fn translator_rtsp_uses_row_id_as_stream_id() {
        // Stage 2 — RTSP rows also carry `stream_id = row.id` (not the
        // legacy `RTSP_N` numbering). Same probe-driven motivation as
        // local mics.
        let r1 = row("src_rtsp_a", SourceKind::Rtsp, "rtsp://a.lan/feed");
        let r2 = row("src_rtsp_b", SourceKind::Rtsp, "rtsp://b.lan/feed");
        let mut idx = 0;
        let cs1 = audio_source_to_capture_source(&r1, false, &mut idx);
        let cs2 = audio_source_to_capture_source(&r2, false, &mut idx);
        match (cs1, cs2) {
            (
                CaptureSource::Rtsp {
                    url: u1,
                    stream_id: s1,
                },
                CaptureSource::Rtsp {
                    url: u2,
                    stream_id: s2,
                },
            ) => {
                assert_eq!(u1, "rtsp://a.lan/feed");
                assert_eq!(s1, "src_rtsp_a");
                assert_eq!(u2, "rtsp://b.lan/feed");
                assert_eq!(s2, "src_rtsp_b");
            }
            other => panic!("expected two Rtsp variants, got {other:?}"),
        }
    }

    #[test]
    fn translator_honours_per_row_sample_rate() {
        let mut r = row("src_x", SourceKind::UsbAlsa, "plughw:0,0");
        r.sample_rate = 44_100;
        let mut idx = 0;
        let cs = audio_source_to_capture_source(&r, false, &mut idx);
        match cs {
            CaptureSource::Microphone { sample_rate, .. } => {
                assert_eq!(sample_rate, 44_100);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    // ---- resolve_sources_from_db against a real AppState ------------

    /// Build a fresh in-memory `AppState` with the schema migrated. The
    /// equivalent helper in `helpers::test_support` is `pub(super)` so
    /// it isn't reachable from this module — inlining the three lines
    /// here is cheaper than widening the visibility.
    fn fresh_state() -> birdnet_web::state::AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        birdnet_web::state::AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    fn insert_row(
        state: &birdnet_web::state::AppState,
        id: &str,
        kind: SourceKind,
        device_id: &str,
        disabled: bool,
    ) {
        use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource};
        let new = NewAudioSource::defaults(id.to_string(), kind, device_id.to_string());
        state
            .with_db(|conn| AudioSourceStore::insert(conn, &new))
            .expect("insert audio_source row");
        if disabled {
            state
                .with_db(|conn| {
                    conn.execute(
                        "UPDATE audio_sources SET disabled_at = datetime('now') WHERE id = ?1",
                        rusqlite::params![id],
                    )
                })
                .expect("disable row");
        }
    }

    #[test]
    fn resolve_from_db_returns_none_when_table_empty() {
        let state = fresh_state();
        // Migration 15 may not seed anything when settings.audio_source is absent
        // — confirm by inspecting the table is empty first, then assert the
        // resolver returns None.
        let count: i64 = state.with_db(|conn| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM audio_sources", [], |r| r.get(0))
                .unwrap_or(0)
        });
        assert_eq!(count, 0, "audio_sources should be empty on a fresh state");
        assert!(resolve_sources_from_db(&state).is_none());
    }

    #[test]
    fn resolve_from_db_translates_active_rows_only() {
        let state = fresh_state();
        insert_row(&state, "src_a", SourceKind::UsbAlsa, "plughw:1,0", false);
        insert_row(&state, "src_b", SourceKind::Rtsp, "rtsp://lan/feed", false);
        insert_row(&state, "src_dead", SourceKind::UsbAlsa, "plughw:9,9", true);

        let resolved = resolve_sources_from_db(&state).expect("non-empty DB result");
        assert_eq!(resolved.len(), 2);
        // The disabled row must not appear.
        for s in &resolved {
            match s {
                CaptureSource::Microphone { device, .. } => assert_ne!(device, "plughw:9,9"),
                CaptureSource::Rtsp { url, .. } => assert!(!url.contains("plughw")),
                CaptureSource::PipeWire { .. } => {}
            }
        }
    }

    // ---- seed_sources_from_config (O-13) ----------------------------

    #[test]
    fn seed_populates_empty_table_from_cli() {
        use birdnet_db::audio_sources::AudioSourceStore;
        let state = fresh_state();
        let mut c = cli();
        c.rtsp_url = Some("rtsp://lan/feed".to_string());
        let n = seed_sources_from_config(&state, &c, None);
        assert_eq!(n, 1);
        let rows = state.with_db(AudioSourceStore::list).expect("list");
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0].kind, SourceKind::Rtsp));
        assert_eq!(rows[0].device_id, "rtsp://lan/feed");
    }

    #[test]
    fn seed_skips_when_table_already_populated() {
        use birdnet_db::audio_sources::AudioSourceStore;
        let state = fresh_state();
        insert_row(
            &state,
            "src_existing",
            SourceKind::UsbAlsa,
            "plughw:1,0",
            false,
        );
        let mut c = cli();
        c.rtsp_url = Some("rtsp://lan/feed".to_string());
        let n = seed_sources_from_config(&state, &c, None);
        assert_eq!(n, 0, "must not seed when a row already exists");
        let rows = state.with_db(AudioSourceStore::list).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "src_existing");
    }

    #[test]
    fn seed_noop_when_no_cli_sources() {
        use birdnet_db::audio_sources::AudioSourceStore;
        let state = fresh_state();
        let n = seed_sources_from_config(&state, &cli(), None);
        assert_eq!(n, 0);
        let rows = state.with_db(AudioSourceStore::list).expect("list");
        assert!(rows.is_empty());
    }

    #[test]
    fn resolve_from_db_uses_row_id_even_for_lone_mic() {
        // Stage 2 — every DB-driven row gets `stream_id = row.id`,
        // including a lone mic. Stage 1's lone-mic id-less carve-out
        // is gone so the supervisor's per-source liveness gauge has a
        // stable, row-specific label.
        let state = fresh_state();
        insert_row(
            &state,
            "src_only_mic",
            SourceKind::UsbAlsa,
            "plughw:1,0",
            false,
        );
        insert_row(
            &state,
            "src_rtsp_1",
            SourceKind::Rtsp,
            "rtsp://lan/a",
            false,
        );

        let resolved = resolve_sources_from_db(&state).expect("non-empty");
        let mic = resolved
            .iter()
            .find_map(|s| match s {
                CaptureSource::Microphone { stream_id, .. } => Some(stream_id.clone()),
                _ => None,
            })
            .expect("mic row");
        assert_eq!(mic.as_deref(), Some("src_only_mic"));
    }

    #[test]
    fn resolve_from_db_assigns_ids_when_multiple_local_mics() {
        let state = fresh_state();
        insert_row(&state, "src_a", SourceKind::UsbAlsa, "plughw:1,0", false);
        insert_row(
            &state,
            "src_b",
            SourceKind::PipeWire,
            "alsa_input.foo",
            false,
        );

        let resolved = resolve_sources_from_db(&state).expect("non-empty");
        let ids: Vec<_> = resolved
            .iter()
            .filter_map(|s| match s {
                CaptureSource::Microphone { stream_id, .. }
                | CaptureSource::PipeWire { stream_id, .. } => stream_id.clone(),
                CaptureSource::Rtsp { .. } => None,
            })
            .collect();
        assert!(ids.contains(&"src_a".to_string()));
        assert!(ids.contains(&"src_b".to_string()));
    }
}
