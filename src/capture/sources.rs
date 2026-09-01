//! Capture-source resolution.
//!
//! Turns CLI flags, the config file, and the `audio_sources` SQLite table into
//! the [`CaptureSource`]s the supervisor drives, plus the per-source runtime
//! knobs (software gain, quiet windows). The parent module owns only the
//! thread orchestration that consumes these.

use birdnet_core::audio::capture::{CaptureSource, ChannelPick, RtspTransport};

use crate::cli::Cli;

use super::{schedule, supervisor};

/// Sample rate a station captures at unless the device says otherwise.
///
/// 48 kHz: what the V2.4 model wants, what most USB capture devices do, and a
/// clean integer ratio to the 32 kHz the V3.0 models want. Was written inline
/// at each autodetected source, so the two could drift apart.
const DEFAULT_CAPTURE_RATE: u32 = 48_000;

/// Resolve all RTSP URLs from CLI flags and config.
///
/// Priority, first match wins: `--rtsp-urls`, then config `RTSP_URLS`
/// (comma-separated, multi), then `--rtsp-url`, then config `RTSP_URL` (single).
/// The comma-separated config key lets a config-file / installer station drive
/// **several** RTSP streams without the CLI — RTSP URLs never contain commas
/// (unlike ALSA device names), so the comma split is unambiguous. Blank entries
/// are dropped.
fn resolve_rtsp_urls(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> Vec<String> {
    if !cli.rtsp_urls.is_empty() {
        return cli.rtsp_urls.clone();
    }
    if let Some(config) = config {
        let multi: Vec<String> = config
            .get("RTSP_URLS")
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if !multi.is_empty() {
            return multi;
        }
    }
    let single = cli
        .rtsp_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .or_else(|| {
            config?
                .get("RTSP_URL")
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(String::from)
        });
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

/// A capture source resolved from the DB (or CLI/config), bundled with the
/// per-source runtime knobs the supervisor and capture command apply:
/// `gain_db` (software gain, applied in the capture command) and `quiet`
/// (a pause window enforced by the supervisor). The CLI/config path carries
/// unity gain and no quiet window — those are admin-UI / `audio_sources`
/// concepts that the bare BirdNET-Pi-style flags don't express.
pub(super) struct ResolvedSource {
    pub(super) source: CaptureSource,
    pub(super) gain_db: f32,
    /// Signal conditioning for this source, mapped from the `audio_sources`
    /// row. Only the DB-driven path can carry it: a source configured purely
    /// from CLI flags has no row to read toggles from, so it gets the
    /// conditioning defaults.
    pub(super) pipeline: birdnet_core::audio::capture::AudioPipeline,
    pub(super) quiet: Option<supervisor::QuietWindow>,
}

/// Map the stored per-source toggles onto the core type the capture path
/// consumes.
///
/// `birdnet-core` must not depend on `birdnet-db`, so this is the one seam
/// where the storage shape and the audio shape meet. `rtsp_keepalive` maps to
/// `rtsp_stall_timeout`, which is renamed rather than copied: ffmpeg sends RTSP
/// keepalives on its own and has no switch for them, so the stored flag's
/// stated behaviour was unimplementable. See
/// [`birdnet_core::audio::capture::AudioPipeline::rtsp_stall_timeout`].
pub(super) const fn map_pipeline(
    flags: birdnet_db::audio_sources::PipelineFlags,
) -> birdnet_core::audio::capture::AudioPipeline {
    birdnet_core::audio::capture::AudioPipeline {
        high_pass: flags.high_pass,
        dc_removal: flags.dc_removal,
        agc: flags.agc,
        rtsp_stall_timeout: flags.rtsp_keepalive,
    }
}

/// Parse a DB `schedule_quiet` (`HH:MM`, `HH:MM`) pair into the supervisor's
/// minute-based [`supervisor::QuietWindow`].
///
/// Returns `None` when there is no window, or when either endpoint is malformed
/// — the `audio_sources` insert/update path validates `HH:MM`, so a malformed
/// value here is unexpected, but we degrade to "no quiet window" rather than
/// fail capture for the source.
fn parse_quiet_window(quiet: Option<&(String, String)>) -> Option<supervisor::QuietWindow> {
    let (start, end) = quiet?;
    Some(supervisor::QuietWindow::from_endpoints(
        parse_quiet_endpoint(start)?,
        parse_quiet_endpoint(end)?,
    ))
}

/// Parse one end of a quiet window: `HH:MM`, or `sunrise`/`sunset` with an
/// optional signed minute offset (`sunset+30`, `sunrise-15`, `sunset`).
///
/// The two forms share one stored column, which is what let solar windows ship
/// without a schema migration. They are unambiguous: a clock time contains a
/// colon and no letters.
pub(super) fn parse_quiet_endpoint(s: &str) -> Option<supervisor::QuietEndpoint> {
    let t = s.trim();
    if let Some(min) = schedule::parse_hhmm(t) {
        return Some(supervisor::QuietEndpoint::Fixed(min));
    }

    let lower = t.to_ascii_lowercase();
    let (event, rest) = lower.strip_prefix("sunrise").map_or_else(
        || {
            lower
                .strip_prefix("sunset")
                .map(|r| (SolarEvent::Sunset, r))
        },
        |r| Some((SolarEvent::Sunrise, r)),
    )?;

    let offset = match rest.trim() {
        "" => 0,
        // `+`/`-` is required: a bare `sunset30` is far more likely to be a
        // typo than an intent, and guessing at it would move a station's
        // recording window by half an hour without saying so.
        r if r.starts_with('+') || r.starts_with('-') => r.parse::<i32>().ok()?,
        _ => return None,
    };
    // Offsets beyond half a day stop meaning "around sunset" and start meaning
    // "some other time entirely", which an operator is better told about than
    // silently given.
    if offset.abs() > 12 * 60 {
        return None;
    }

    Some(match event {
        SolarEvent::Sunrise => supervisor::QuietEndpoint::Sunrise(offset),
        SolarEvent::Sunset => supervisor::QuietEndpoint::Sunset(offset),
    })
}

/// Which solar event a quiet endpoint anchors to.
#[derive(Debug, Clone, Copy)]
enum SolarEvent {
    Sunrise,
    Sunset,
}

/// Resolve capture sources from the `audio_sources` SQLite table.
///
/// Returns `None` when the DB read errors (the caller treats that as
/// "fall back to CLI/config"); returns `Some(empty)` when the table is
/// present but every row is disabled (the caller also falls back); and
/// `Some(non-empty)` only when at least one row is active. Each resolved
/// source carries the row's `gain_db` and (parsed) `schedule_quiet`, so the
/// admin-UI per-source gain and quiet window actually take effect.
///
/// O-13 stage 1 — additive layer over the existing CLI/config path.
/// Subsequent stages will retire the legacy path once every deployed
/// station has migrated its sources into the table.
pub(super) fn resolve_sources_from_db(
    state: &birdnet_web::state::AppState,
) -> Option<Vec<ResolvedSource>> {
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
        let source = audio_source_to_capture_source(&row, local_count > 1, &mut rtsp_index);
        out.push(ResolvedSource {
            source,
            gain_db: row.gain_db,
            pipeline: map_pipeline(row.pipeline),
            quiet: parse_quiet_window(row.schedule_quiet.as_ref()),
        });
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
    // Left and Right used to collapse into Mono here and were never
    // distinguished again, so the Audio page offered three options that all
    // did the same thing. They now select the channel they name: the device is
    // opened with both, and the tee keeps the requested half, so the segments
    // written to disk are the mono stream that half produced.
    //
    // That is worth having rather than cosmetic. `Stereo` keeps both channels
    // and the decoder averages them, which for a *spaced* pair is a comb
    // filter: measured through this project's decode path, one wavefront
    // reaching the capsules half a period apart loses about 66 dB to
    // cancellation, a quarter period costs 3 dB, and the notches move with the
    // bird's direction. Picking one channel is how an operator avoids that.
    let channel_pick = match row.channels {
        Channels::Left => Some(ChannelPick::Left),
        Channels::Right => Some(ChannelPick::Right),
        Channels::Mono | Channels::Stereo => None,
    };
    let channels: u16 = match row.channels {
        Channels::Mono => 1,
        // A pick needs both channels off the device; the tee reduces to one.
        Channels::Left | Channels::Right | Channels::Stereo => 2,
    };
    if matches!(row.channels, Channels::Stereo) {
        // Once per source at start-up, in the journal the operator is already
        // reading when detections look thin. Not an error: a coincident pair
        // averages harmlessly, and we cannot tell from here which kind is
        // plugged in.
        tracing::warn!(
            source = %row.id,
            device = %row.device_id,
            "source is configured Stereo: both channels are averaged to mono for analysis, \
             which on a spaced microphone pair cancels signal at frequencies where the two \
             capsules disagree. Set the source to Left or Right to analyse one channel \
             instead, unless the capsules are coincident"
        );
    }
    match row.kind {
        SourceKind::UsbAlsa => CaptureSource::Microphone {
            device: row.device_id.clone(),
            sample_rate: row.sample_rate,
            channels,
            channel_pick,
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
            // Honour the per-source transport the admin UI exposes — previously
            // dropped here, so ffmpeg was always forced to TCP and a UDP-only
            // camera could never be captured.
            transport: map_rtsp_transport(row.rtsp_transport),
        },
    }
}

/// Map a `birdnet-db` RTSP transport (the admin-UI/storage enum) to the
/// `birdnet-core` capture enum the ffmpeg command consumes. Kept here, in the
/// crate that depends on both, so `birdnet-core` need not know about `birdnet-db`.
const fn map_rtsp_transport(transport: birdnet_db::audio_sources::RtspTransport) -> RtspTransport {
    use birdnet_db::audio_sources::RtspTransport as DbTransport;
    match transport {
        DbTransport::Auto => RtspTransport::Auto,
        DbTransport::Tcp => RtspTransport::Tcp,
        DbTransport::Udp => RtspTransport::Udp,
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
pub(super) fn resolve_sources(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Vec<CaptureSource> {
    let pipewire_device = cli.pipewire_device.clone();
    let alsa_devices = resolve_alsa_devices(cli, config);
    let rtsp_urls = resolve_rtsp_urls(cli, config);

    let rtsp_sources = |urls: Vec<String>, mixed: bool| -> Vec<CaptureSource> {
        // A lone RTSP stream (one URL, no local mic) keeps the plain "rtsp" id
        // for filename backward-compatibility; the moment there is more than one
        // stream — whether from `--rtsp-urls`, config `RTSP_URLS`, or mixed with
        // a local mic — every stream is numbered so filenames and metrics stay
        // distinct. (Previously this keyed off `cli.rtsp_urls.is_empty()`, so a
        // multi-stream config left the first stream mislabeled "rtsp".)
        let single = urls.len() == 1;
        urls.into_iter()
            .enumerate()
            .map(|(i, url)| {
                let stream_id = if i == 0 && !mixed && single {
                    "rtsp".to_string()
                } else {
                    format!("RTSP_{}", i + 1)
                };
                CaptureSource::Rtsp {
                    url,
                    stream_id,
                    // A bare CLI/config URL carries no transport preference; the
                    // admin UI is where TCP/UDP is chosen per source.
                    transport: RtspTransport::Auto,
                }
            })
            .collect()
    };

    if let Some(device) = pipewire_device {
        let mut srcs = vec![CaptureSource::PipeWire {
            // PipeWire resamples internally, so it accepts any rate and there
            // is nothing to probe; the ALSA path below is where a device can
            // refuse.
            sample_rate: DEFAULT_CAPTURE_RATE,
            device,
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
                channel_pick: None,
                // Ask the device rather than assuming. A 44.1 kHz-only
                // interface handed `-r 48000` either fails to start — so the
                // supervisor restarts it forever behind an ALSA error nobody
                // reads — or is silently plug-converted, which is worse:
                // capture works and every spectrogram has been resampled from
                // something narrower than the station believes.
                sample_rate: capture_rate_for(&device, DEFAULT_CAPTURE_RATE),
                device,
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
pub(super) fn seed_sources_from_config(
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

/// The rate a microphone should capture at, given what it says it supports.
///
/// `preferred` is the model's rate. Falls back to it whenever the device says
/// nothing usable, so a probe that fails costs a station nothing — the config
/// is exactly what it would have been.
///
/// Separated from the autodetection below so the log line and the fallback are
/// testable without a sound card; the probe itself is
/// [`birdnet_core::audio::capture::probe::probe_alsa_rates`].
fn capture_rate_for(device: &str, preferred: u32) -> u32 {
    use birdnet_core::audio::capture::probe::{pick_rate, probe_alsa_rates};

    let support = probe_alsa_rates(device);
    pick_rate(&support, preferred).map_or(preferred, |rate| {
        tracing::info!(
            device,
            preferred,
            using = rate,
            ?support,
            "capture device does not support the preferred sample rate; using its nearest"
        );
        rate
    })
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
    fn resolve_rtsp_urls_reads_multiple_from_config() {
        use birdnet_core::config::Config;
        // The comma-separated RTSP_URLS config key drives several streams from a
        // config file / installer, without needing the --rtsp-urls CLI flag.
        let cfg =
            Config::parse("RTSP_URLS=rtsp://a.lan/s , rtsp://b.lan/s,rtsp://c.lan/s").unwrap();
        assert_eq!(
            resolve_rtsp_urls(&cli(), Some(&cfg)),
            vec![
                "rtsp://a.lan/s".to_string(),
                "rtsp://b.lan/s".to_string(),
                "rtsp://c.lan/s".to_string(),
            ],
            "RTSP_URLS must split on commas, trim, and drop blanks"
        );
        // RTSP_URLS (multi) takes precedence over RTSP_URL (single).
        let cfg2 =
            Config::parse("RTSP_URLS=rtsp://a.lan/s,rtsp://b.lan/s\nRTSP_URL=rtsp://x.lan/s")
                .unwrap();
        assert_eq!(resolve_rtsp_urls(&cli(), Some(&cfg2)).len(), 2);
    }

    #[test]
    fn resolve_sources_builds_multiple_rtsp_streams_from_config() {
        use birdnet_core::config::Config;
        let cfg = Config::parse("RTSP_URLS=rtsp://a.lan/s,rtsp://b.lan/s").unwrap();
        let sources = resolve_sources(&cli(), Some(&cfg));
        assert_eq!(sources.len(), 2, "two RTSP streams must both be captured");
        let ids: Vec<_> = sources
            .iter()
            .map(|s| match s {
                CaptureSource::Rtsp { stream_id, .. } => stream_id.clone(),
                other => panic!("expected RTSP, got {other:?}"),
            })
            .collect();
        // RTSP-only (no local mic) → numbered RTSP_n ids so filenames/metrics stay distinct.
        assert_eq!(ids, vec!["RTSP_1".to_string(), "RTSP_2".to_string()]);
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
    fn map_rtsp_transport_covers_all_variants() {
        // Bare `RtspTransport` in this test module is the birdnet-db enum
        // (explicitly imported); spell out the birdnet-core target type.
        use birdnet_core::audio::capture::RtspTransport as Core;
        use birdnet_db::audio_sources::RtspTransport as Db;
        assert_eq!(map_rtsp_transport(Db::Auto), Core::Auto);
        assert_eq!(map_rtsp_transport(Db::Tcp), Core::Tcp);
        assert_eq!(map_rtsp_transport(Db::Udp), Core::Udp);
    }

    #[test]
    fn translator_rtsp_preserves_per_row_transport() {
        // The admin UI's per-source TCP/UDP choice must reach the capture
        // command — previously it was dropped (always TCP).
        let mut r = row("src_rtsp_udp", SourceKind::Rtsp, "rtsp://cam/feed");
        r.rtsp_transport = RtspTransport::Udp;
        let mut idx = 0;
        match audio_source_to_capture_source(&r, false, &mut idx) {
            CaptureSource::Rtsp { transport, .. } => {
                assert_eq!(transport, birdnet_core::audio::capture::RtspTransport::Udp);
            }
            other => panic!("expected Rtsp, got {other:?}"),
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
                channel_pick: _,
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

    /// Left and Right must resolve to something Mono does not.
    ///
    /// They used to collapse into the same `channels: 1` microphone with
    /// nothing carried forward, so the Audio page offered three settings that
    /// produced byte-identical captures. A pick now opens the device with both
    /// channels — you cannot select one the driver was never asked for — and
    /// names which half the tee keeps.
    #[test]
    fn translator_maps_left_and_right_to_distinct_channel_picks() {
        use birdnet_db::audio_sources::Channels;

        let resolve = |channels: Channels| {
            let mut r = row("src_usb_1", SourceKind::UsbAlsa, "plughw:1,0");
            r.channels = channels;
            let mut idx = 0;
            match audio_source_to_capture_source(&r, false, &mut idx) {
                CaptureSource::Microphone {
                    channels,
                    channel_pick,
                    ..
                } => (channels, channel_pick),
                other => panic!("expected Microphone, got {other:?}"),
            }
        };

        assert_eq!(resolve(Channels::Mono), (1, None), "mono opens one channel");
        assert_eq!(
            resolve(Channels::Left),
            (2, Some(ChannelPick::Left)),
            "Left must open both channels and keep the first"
        );
        assert_eq!(
            resolve(Channels::Right),
            (2, Some(ChannelPick::Right)),
            "Right must open both channels and keep the second"
        );
        assert_eq!(
            resolve(Channels::Stereo),
            (2, None),
            "Stereo keeps both and lets the decoder mix them"
        );

        assert_ne!(
            resolve(Channels::Left),
            resolve(Channels::Right),
            "Left and Right must not resolve to the same capture"
        );
        assert_ne!(
            resolve(Channels::Mono),
            resolve(Channels::Right),
            "Right must not be a synonym for Mono"
        );
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
                    ..
                },
                CaptureSource::Rtsp {
                    url: u2,
                    stream_id: s2,
                    ..
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
            match &s.source {
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
            .find_map(|s| match &s.source {
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
            .filter_map(|s| match &s.source {
                CaptureSource::Microphone { stream_id, .. }
                | CaptureSource::PipeWire { stream_id, .. } => stream_id.clone(),
                CaptureSource::Rtsp { .. } => None,
            })
            .collect();
        assert!(ids.contains(&"src_a".to_string()));
        assert!(ids.contains(&"src_b".to_string()));
    }

    // ---- gain_db + schedule_quiet threading (Workstream D) ----------------

    #[test]
    fn resolve_from_db_carries_gain_and_quiet_window() {
        use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource};
        let state = fresh_state();
        let mut new = NewAudioSource::defaults("src_gain", SourceKind::UsbAlsa, "plughw:1,0");
        new.gain_db = 12.0;
        new.schedule_quiet = Some(("22:00".to_string(), "06:00".to_string()));
        state
            .with_db(|conn| AudioSourceStore::insert(conn, &new))
            .expect("insert row with gain + quiet");

        let resolved = resolve_sources_from_db(&state).expect("non-empty");
        assert_eq!(resolved.len(), 1);
        // The per-source gain reaches the resolved source (and thence the
        // RecordingConfig / capture command).
        assert!((resolved[0].gain_db - 12.0).abs() < 1e-4);
        // 22:00–06:00 parses to a wraparound quiet window the supervisor enforces.
        assert_eq!(
            resolved[0].quiet,
            Some(supervisor::QuietWindow::new(22 * 60, 6 * 60))
        );
    }

    #[test]
    fn resolve_from_db_defaults_to_unity_gain_and_no_quiet() {
        let state = fresh_state();
        insert_row(
            &state,
            "src_plain",
            SourceKind::UsbAlsa,
            "plughw:1,0",
            false,
        );
        let resolved = resolve_sources_from_db(&state).expect("non-empty");
        assert!((resolved[0].gain_db - 0.0).abs() < 1e-6);
        assert_eq!(resolved[0].quiet, None);
    }

    // ---- parse_quiet_window -----------------------------------------------

    #[test]
    fn parse_quiet_window_parses_valid_pair() {
        let q = parse_quiet_window(Some(&("22:00".to_string(), "06:00".to_string())));
        assert_eq!(q, Some(supervisor::QuietWindow::new(1320, 360)));
    }

    #[test]
    fn parse_quiet_window_none_for_absent_or_malformed() {
        assert_eq!(parse_quiet_window(None), None);
        // A malformed endpoint degrades to "no quiet window" rather than panic.
        assert_eq!(
            parse_quiet_window(Some(&("25:00".to_string(), "06:00".to_string()))),
            None
        );
        assert_eq!(
            parse_quiet_window(Some(&("22:00".to_string(), "nope".to_string()))),
            None
        );
    }
}

// ── parsing the two forms one column holds ──────────────────────────────
#[cfg(test)]
mod quiet_endpoint_tests {
    use super::*;

    #[test]
    fn clock_times_still_parse() {
        assert_eq!(
            parse_quiet_endpoint("22:00"),
            Some(supervisor::QuietEndpoint::Fixed(22 * 60))
        );
    }

    #[test]
    fn solar_anchors_parse_with_and_without_an_offset() {
        assert_eq!(
            parse_quiet_endpoint("sunset"),
            Some(supervisor::QuietEndpoint::Sunset(0))
        );
        assert_eq!(
            parse_quiet_endpoint("sunset+30"),
            Some(supervisor::QuietEndpoint::Sunset(30))
        );
        assert_eq!(
            parse_quiet_endpoint("SunRise-15"),
            Some(supervisor::QuietEndpoint::Sunrise(-15)),
            "case must not matter — an operator types what reads naturally"
        );
    }

    /// A bare number after the anchor is far more likely a typo than an
    /// intent, and guessing would move the window by half an hour silently.
    #[test]
    fn an_unsigned_offset_is_rejected() {
        assert_eq!(parse_quiet_endpoint("sunset30"), None);
    }

    /// Beyond half a day an "offset from sunset" is some other time entirely.
    #[test]
    fn an_absurd_offset_is_rejected() {
        assert_eq!(parse_quiet_endpoint("sunrise+800"), None);
        assert!(
            parse_quiet_endpoint("sunrise+720").is_some(),
            "12 h is the limit, inclusive"
        );
    }

    #[test]
    fn nonsense_is_rejected() {
        for s in ["", "moonrise", "25:00", "noon", "sun"] {
            assert_eq!(parse_quiet_endpoint(s), None, "{s} must not parse");
        }
    }

    // ── capture-rate probing ────────────────────────────────────────────

    #[test]
    fn a_device_that_cannot_be_probed_keeps_the_preferred_rate() {
        // The failure path, and the one that runs in this project's CI: there
        // is no sound card and no `arecord`, so the probe learns nothing. A
        // station must then be configured exactly as it was before probing
        // existed — the feature can improve a configuration, never break one.
        assert_eq!(
            capture_rate_for("no-such-device-anywhere", DEFAULT_CAPTURE_RATE),
            DEFAULT_CAPTURE_RATE
        );
        // And the model's rate is honoured, not a constant baked in here.
        assert_eq!(capture_rate_for("no-such-device-anywhere", 32_000), 32_000);
    }

    #[test]
    fn the_default_capture_rate_is_the_one_the_v24_model_wants() {
        // Named once rather than written at each autodetected source, which is
        // how the two could previously drift apart.
        assert_eq!(DEFAULT_CAPTURE_RATE, 48_000);
    }
}
