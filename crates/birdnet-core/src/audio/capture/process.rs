//! Audio capture subprocess management.
//!
//! Wraps `arecord` (local microphone) and `ffmpeg` (`PipeWire` / RTSP) child
//! processes.
//!
//! # Two segmentation mechanisms, and why
//!
//! `PipeWire` and RTSP sources are segmented by **ffmpeg's** `segment` muxer,
//! as they always have been. A local **ALSA microphone** is not: `arecord`
//! streams headerless PCM to a pipe and this process splits it, because an ALSA
//! `plughw:` device is exclusive and something other than the recorder — the
//! live `/stream` endpoint — needs the same audio. See the `tee` module.
//!
//! The two mechanisms share the one thing that must not diverge: the filenames,
//! which come from `recording_filename`'s `strftime` pattern in the ffmpeg case
//! and from [`super::types::recording_filename_at`]'s expansion of that same
//! pattern in the tee case, pinned to each other by a test.

use std::path::Path;
use std::process::{Child, Command, Stdio};

use super::live::PcmSpec;
use super::segment::{SegmentClock, SegmentWriter};
use super::tee::{self, Tee};
use super::types::{AudioFormat, CaptureError, CaptureSource, RecordingConfig, recording_filename};

/// A running audio capture process.
#[derive(Debug)]
pub struct CaptureProcess {
    pub(super) child: Child,
    source: CaptureSource,
    /// The in-process splitter, for sources this crate segments itself. `None`
    /// for the ffmpeg-segmented sources, which write their own files.
    tee: Option<Tee>,
}

impl CaptureProcess {
    /// Check if the capture process is still running.
    ///
    /// For a teed source that means **both** halves: the producer subprocess
    /// and the reader thread that turns its output into files. A reader that
    /// died — a full disk, a vanished output mount — leaves `arecord` alive and
    /// recording into a pipe nobody drains, which is not a running capture by
    /// any definition the operator cares about.
    pub fn is_running(&mut self) -> bool {
        if self.child.try_wait().ok().flatten().is_some() {
            return false;
        }
        self.tee.as_ref().is_none_or(Tee::is_alive)
    }

    /// Stop the capture process gracefully.
    ///
    /// # Errors
    ///
    /// Returns `CaptureError` if the process cannot be terminated.
    pub fn stop(&mut self) -> Result<(), CaptureError> {
        // Kill first, wait second, then tear down the reader: the reader spends
        // its life blocked reading the producer's stdout, and it is the EOF
        // that killing the producer causes which lets it finish the segment in
        // progress and exit. Stopping it first would just block on the join.
        let killed = self.child.kill();
        let waited = self.child.wait();
        self.tee = None;
        killed.map_err(CaptureError::Spawn)?;
        waited.map_err(CaptureError::Spawn)?;
        Ok(())
    }

    /// Get the capture source configuration.
    pub const fn source(&self) -> &CaptureSource {
        &self.source
    }
}

impl Drop for CaptureProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.tee = None;
    }
}

/// Check if a path has a supported audio extension (.wav / .flac / .mp3).
pub(crate) fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ext.eq_ignore_ascii_case("wav")
                || ext.eq_ignore_ascii_case("flac")
                || ext.eq_ignore_ascii_case("mp3")
        })
}

/// Check if a required capture tool is available on the system.
pub fn is_tool_available(tool: &str) -> bool {
    Command::new("which")
        .arg(tool)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Whether a capture-subprocess stderr line reports a failure rather than
/// routine chatter.
///
/// Deliberately substring-based over the vocabulary `arecord`, `ffmpeg` and the
/// ALSA libraries actually use: matching precisely would mean tracking three
/// tools' message catalogues, and the cost of a false positive (one warn line)
/// is far below the cost of a false negative (a silent, unexplained restart
/// loop — the failure mode this exists to prevent).
fn is_capture_failure(line: &str) -> bool {
    const MARKERS: &[&str] = &[
        "audio open error",
        "open error",
        "cannot open",
        "cannot get",
        // Covers ALSA's "No such file or directory", "No such device" and
        // "No such card" without enumerating each noun.
        "no such",
        "permission denied",
        "device or resource busy",
        // ALSA reports a bad card/device index as "Invalid value for card",
        // and a rejected format as "Invalid argument".
        "invalid value",
        "invalid argument",
        // arecord's set_params failures read "Sample format non available",
        // "Channels count non available", "Rate non available".
        "non available",
        "not available",
        "unable to",
        "failed",
        "error:",
    ];
    let lower = line.to_ascii_lowercase();
    MARKERS.iter().any(|m| lower.contains(m))
}

/// Drain a capture subprocess's stderr into the log, line by line.
///
/// `arecord` / `ffmpeg` write diagnostics (ALSA xruns, RTSP reconnects, fatal
/// errors) to stderr. If that piped stream is never read, the OS pipe buffer
/// (~64 KB) eventually fills and the subprocess blocks on `write(2)`: it stays
/// alive — so the supervisor's [`CaptureProcess::is_running`] still reports it
/// healthy — but stops producing audio, and the station goes silently deaf
/// with no recovery. Draining the pipe in a detached reader thread both
/// prevents that stall and surfaces the subprocess's own error messages for
/// field debugging. The thread ends at EOF — i.e. when the child exits or is
/// killed on stop/drop — so it needs no explicit join.
///
/// Lines matching [`is_capture_failure`] are logged at `warn`; everything else
/// stays at `debug`.
fn drain_capture_stderr(child: &mut Child, source: &str) {
    let Some(stderr) = child.stderr.take() else {
        return;
    };
    let source = source.to_owned();
    if let Err(e) = std::thread::Builder::new()
        .name("capture-stderr".to_owned())
        .spawn(move || {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(stderr).lines() {
                match line {
                    Ok(line) if !line.trim().is_empty() => {
                        // Routine chatter (ALSA xruns, RTSP reconnect notices)
                        // stays at debug so a busy station does not spam the
                        // journal. A line that reports a *failure* is promoted
                        // to warn, because it is the only place the reason a
                        // source will not start is ever written down.
                        //
                        // This used to be debug unconditionally, and the
                        // default filter is `info,birdnet_behavior=debug` —
                        // this crate is `birdnet_core`, so it sat below the
                        // threshold. The supervisor's "capture (re)start
                        // issued" (in `birdnet_behavior`) was visible while the
                        // arecord error explaining it was not, leaving an
                        // operator with an infinite restart loop and no cause.
                        if is_capture_failure(&line) {
                            tracing::warn!(source = %source, "capture subprocess: {line}");
                        } else {
                            tracing::debug!(source = %source, "capture subprocess: {line}");
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        })
    {
        tracing::warn!(error = %e, "could not start capture stderr drainer thread");
    }
}

/// The smallest absolute gain (in dB) we bother applying. Below this the gain
/// is treated as "off": the capture tee forwards samples untouched — which is
/// what keeps the unity-gain path byte-exact — and the ffmpeg sources omit the
/// `volume` filter entirely. `0.05` dB is well below audible and matches the
/// threshold the admin UI uses to decide whether to *show* a gain badge, so
/// "looks like no gain in the UI" and "no gain applied at capture" always
/// agree.
const GAIN_EPSILON_DB: f32 = 0.05;

/// Whether a configured `gain_db` is large enough to apply.
///
/// Pure so the boundary is unit-testable. The `abs()` makes a cut (negative dB)
/// active just like a boost; only a value within `GAIN_EPSILON_DB` of unity
/// is treated as "no gain".
#[must_use]
pub fn gain_is_active(gain_db: f32) -> bool {
    gain_db.abs() >= GAIN_EPSILON_DB
}

/// The ffmpeg `volume` filter expression that applies `gain_db`.
///
/// `None` when the gain is effectively unity. ffmpeg's `volume` filter accepts a
/// dB suffix, e.g. `volume=12.00dB` (boost) or `volume=-6.00dB` (cut). Pure so it
/// can be unit-tested without spawning ffmpeg.
#[must_use]
pub fn gain_volume_filter(gain_db: f32) -> Option<String> {
    gain_is_active(gain_db).then(|| format!("volume={gain_db:.2}dB"))
}

/// Append `-af volume=<gain>dB` to `cmd` when the gain is active; a no-op at
/// unity gain so the ffmpeg command line is unchanged for sources without gain.
fn apply_gain_filter(cmd: &mut Command, gain_db: f32) {
    if let Some(filter) = gain_volume_filter(gain_db) {
        cmd.arg("-af").arg(filter);
    }
}

/// Build the `arecord` command that streams headerless PCM to stdout.
///
/// Deliberately **not** given `--max-file-time` / `--use-strftime` or an output
/// path: segmentation and naming move into this process (see [`super::tee`]),
/// and `arecord` writes to standard output when it is handed no filename.
/// `-t raw` suppresses the WAV header it would otherwise prepend to that
/// stream — the header belongs to each segment, not to the stream.
///
/// Pure, so the exact argv is unit-testable without an ALSA device present.
fn arecord_raw_command(device: &str, spec: PcmSpec) -> Command {
    let mut cmd = Command::new("arecord");
    cmd.arg("-D")
        .arg(device)
        .arg("-f")
        .arg("S16_LE")
        .arg("-r")
        .arg(spec.sample_rate.to_string())
        .arg("-c")
        .arg(spec.channels.to_string())
        .arg("-t")
        .arg("raw");
    cmd
}

/// Start an audio capture process for a microphone source.
///
/// # Linux (the deployment target)
///
/// `arecord` streams raw PCM into this process, which writes the rotating WAV
/// segments itself and publishes the same audio to a live tap. That is what
/// makes live audio possible at all on a single-microphone station: the ALSA
/// device is exclusive, so the previous design — capture holding the device
/// while `/stream` tried to open it again — could only ever produce `Device or
/// resource busy`.
///
/// It also retires the second capture backend. `arecord` has no software gain,
/// so a gain-configured microphone used to be captured by `ffmpeg -f alsa`
/// instead; now the gain is applied to the samples in-process, so there is one
/// microphone backend regardless of gain. [`required_tool`] said `arecord` for
/// both cases and was simply wrong about the second — a station with gain
/// configured and no ffmpeg installed passed the availability check and then
/// failed to spawn.
///
/// # macOS
///
/// Unchanged: `ffmpeg`'s avfoundation input with its own `segment` muxer.
/// avfoundation cannot be exercised here — there is no macOS runner in CI and
/// no macOS hardware behind this change — and macOS is a development platform
/// for this project rather than a deployment target, so it keeps the path that
/// is known to work. Live audio on macOS therefore still opens the device a
/// second time, as before.
///
/// # Errors
///
/// Returns `CaptureError` if the capture tool cannot be started.
pub fn start_microphone_capture(config: &RecordingConfig) -> Result<CaptureProcess, CaptureError> {
    let CaptureSource::Microphone {
        ref device,
        sample_rate,
        channels,
        ref stream_id,
    } = config.source
    else {
        return Err(CaptureError::Config("expected microphone source".into()));
    };
    let spec = PcmSpec {
        sample_rate,
        channels,
    };

    // Both branches are compiled on every platform (`cfg!`, not `#[cfg]`), so
    // the macOS path stays type-checked and linted on a Linux build.
    if cfg!(target_os = "macos") {
        return start_avfoundation_microphone(config, device, spec, stream_id.as_deref());
    }
    start_teed_alsa_microphone(config, device, spec, stream_id.as_deref())
}

/// Linux microphone: `arecord` → in-process tee → {segment writer, live tap}.
fn start_teed_alsa_microphone(
    config: &RecordingConfig,
    device: &str,
    spec: PcmSpec,
    stream_id: Option<&str>,
) -> Result<CaptureProcess, CaptureError> {
    if !matches!(config.format, AudioFormat::Wav) {
        // Pre-existing, now visible: `arecord` only ever wrote WAV, so a
        // FLAC-configured microphone always produced WAV bytes in a `.flac`
        // file. Say so instead of continuing to lie silently.
        tracing::warn!(
            device,
            "microphone capture writes WAV; the configured format is ignored"
        );
    }

    let mut cmd = arecord_raw_command(device, spec);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    drain_capture_stderr(&mut child, device);

    let Some(stdout) = child.stdout.take() else {
        // Unreachable with `Stdio::piped()`, but the alternative to handling it
        // is an unwrap in the one place that must never panic.
        kill_child(&mut child);
        return Err(CaptureError::Process(
            "arecord was started without a stdout pipe".into(),
        ));
    };

    let label = config.source.label();
    let tap = config.live_audio.as_ref().map(|hub| hub.tap(&label, spec));
    let live = tap.is_some();
    let writer = SegmentWriter::new(
        config.output_dir.clone(),
        stream_id.map(ToOwned::to_owned),
        config.format,
        spec,
        config.segment_duration_secs,
        config.local_offset.clone(),
        SegmentClock::System,
    );

    let tee = match tee::spawn(label, stdout, writer, tap, config.gain_db) {
        Ok(tee) => tee,
        Err(e) => {
            // `std::process::Child` does **not** kill on drop, so bailing out
            // without this would leave `arecord` holding the device forever.
            kill_child(&mut child);
            return Err(CaptureError::Spawn(e));
        }
    };

    tracing::info!(
        device,
        gain_db = config.gain_db,
        segment_secs = config.segment_duration_secs,
        live_audio = live,
        "started microphone capture (in-process tee)"
    );
    Ok(CaptureProcess {
        child,
        source: config.source.clone(),
        tee: Some(tee),
    })
}

/// macOS microphone: ffmpeg's avfoundation input, segmenting for itself.
fn start_avfoundation_microphone(
    config: &RecordingConfig,
    device: &str,
    spec: PcmSpec,
    stream_id: Option<&str>,
) -> Result<CaptureProcess, CaptureError> {
    let output_path = config
        .output_dir
        .join(recording_filename(stream_id, config.format));
    // The avfoundation input spec is "[video]:[audio]"; ":<n>" selects audio
    // device <n> with no video capture. ALSA-style device names (e.g.
    // "plughw:1,0") don't apply, so fall back to the default input (0).
    let audio_device = if device.is_empty() || device.contains(':') || device.starts_with("hw") {
        "0"
    } else {
        device
    };
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-f")
        .arg("avfoundation")
        .arg("-i")
        .arg(format!(":{audio_device}"))
        .arg("-ar")
        .arg(spec.sample_rate.to_string())
        .arg("-ac")
        .arg(spec.channels.to_string());
    apply_gain_filter(&mut cmd, config.gain_db);
    cmd.arg("-f")
        .arg("segment")
        .arg("-segment_time")
        .arg(config.segment_duration_secs.to_string())
        .arg("-strftime")
        .arg("1")
        .arg(output_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    drain_capture_stderr(&mut child, device);
    tracing::info!(
        device,
        gain_db = config.gain_db,
        "started microphone capture (avfoundation)"
    );
    Ok(CaptureProcess {
        child,
        source: config.source.clone(),
        tee: None,
    })
}

/// Terminate and reap a child we are abandoning during startup.
fn kill_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Detect whether a `PipeWire` or `PulseAudio` server is running.
///
/// Returns `true` if `pw-cli` (`PipeWire`) or `pactl` (`PulseAudio` / `pipewire-pulse` compat)
/// is available on the system.
pub fn detect_pipewire_or_pulseaudio() -> bool {
    is_tool_available("pw-cli") || is_tool_available("pactl")
}

/// Start an audio capture process for a `PipeWire`/`PulseAudio` source via `ffmpeg -f pulse`.
///
/// Works with both native `PulseAudio` and `PipeWire` (via `pipewire-pulse` compatibility layer).
///
/// # Errors
///
/// Returns `CaptureError` if `ffmpeg` cannot be started.
pub fn start_pipewire_capture(config: &RecordingConfig) -> Result<CaptureProcess, CaptureError> {
    let CaptureSource::PipeWire {
        ref device,
        sample_rate,
        channels,
        ref stream_id,
    } = config.source
    else {
        return Err(CaptureError::Config("expected PipeWire source".into()));
    };

    // PipeWire's pipewire-pulse layer exposes PulseAudio compatibility.
    // An empty device string means "use the system default source".
    let pulse_device = if device.is_empty() {
        "default"
    } else {
        device.as_str()
    };

    let filename_pattern = recording_filename(stream_id.as_deref(), config.format);
    let output_path = config.output_dir.join(&filename_pattern);

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-f")
        .arg("pulse")
        .arg("-i")
        .arg(pulse_device)
        .arg("-ar")
        .arg(sample_rate.to_string())
        .arg("-ac")
        .arg(channels.to_string());
    apply_gain_filter(&mut cmd, config.gain_db);
    let mut child = cmd
        .arg("-f")
        .arg("segment")
        .arg("-segment_time")
        .arg(config.segment_duration_secs.to_string())
        .arg("-strftime")
        .arg("1")
        .arg(output_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    drain_capture_stderr(&mut child, pulse_device);

    tracing::info!(
        device = pulse_device,
        gain_db = config.gain_db,
        "started PipeWire/PulseAudio capture via ffmpeg pulse"
    );
    Ok(CaptureProcess {
        child,
        source: config.source.clone(),
        tee: None,
    })
}

/// Start an audio capture process for an RTSP stream via `ffmpeg`.
///
/// # Errors
///
/// Returns `CaptureError` if `ffmpeg` cannot be started.
pub fn start_rtsp_capture(config: &RecordingConfig) -> Result<CaptureProcess, CaptureError> {
    let CaptureSource::Rtsp {
        ref url,
        ref stream_id,
        transport,
    } = config.source
    else {
        return Err(CaptureError::Config("expected RTSP source".into()));
    };

    let filename_pattern = recording_filename(Some(stream_id), config.format);
    let output_path = config.output_dir.join(&filename_pattern);

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-rtsp_transport")
        .arg(transport.ffmpeg_arg())
        .arg("-i")
        .arg(url)
        .arg("-vn")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg("-ar")
        .arg("48000")
        .arg("-ac")
        .arg("1");
    apply_gain_filter(&mut cmd, config.gain_db);
    let mut child = cmd
        .arg("-f")
        .arg("segment")
        .arg("-segment_time")
        .arg(config.segment_duration_secs.to_string())
        .arg("-strftime")
        .arg("1")
        .arg(output_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    drain_capture_stderr(&mut child, stream_id);

    tracing::info!(
        stream_id = stream_id,
        url = url,
        gain_db = config.gain_db,
        "started RTSP capture via ffmpeg"
    );
    Ok(CaptureProcess {
        child,
        source: config.source.clone(),
        tee: None,
    })
}

/// Spawn the appropriate capture process based on the source type.
///
/// # Errors
///
/// Returns `CaptureError` if the process cannot be started.
pub fn spawn_capture(config: &RecordingConfig) -> Result<CaptureProcess, CaptureError> {
    match &config.source {
        CaptureSource::Microphone { .. } => start_microphone_capture(config),
        CaptureSource::PipeWire { .. } => start_pipewire_capture(config),
        CaptureSource::Rtsp { .. } => start_rtsp_capture(config),
    }
}

/// Return the system tool name required for the given source.
pub const fn required_tool(source: &CaptureSource) -> &'static str {
    match source {
        // macOS records the microphone via ffmpeg/avfoundation; Linux via arecord.
        CaptureSource::Microphone { .. } => {
            if cfg!(target_os = "macos") {
                "ffmpeg"
            } else {
                "arecord"
            }
        }
        CaptureSource::PipeWire { .. } | CaptureSource::Rtsp { .. } => "ffmpeg",
    }
}

#[cfg(test)]
mod capture_failure_classification {
    use super::is_capture_failure;

    /// The exact stderr a Raspberry Pi 4 emitted while the unit's
    /// `DeviceAllow=/dev/snd` denied every ALSA node. These two lines were
    /// written on every restart for the entire life of the station and never
    /// reached the journal, because they were logged below the default filter.
    #[test]
    fn the_lines_that_were_invisible_on_real_hardware_are_warnings() {
        assert!(is_capture_failure(
            "ALSA lib confmisc.c:165:(snd_config_get_card) Cannot get card index for 1"
        ));
        assert!(is_capture_failure(
            "arecord: main:850: audio open error: No such file or directory"
        ));
    }

    #[test]
    fn other_common_failures_are_warnings() {
        for line in [
            "arecord: main:830: audio open error: Device or resource busy",
            "ALSA lib pcm_hw.c:1829:(_snd_pcm_hw_open) Invalid value for card",
            "arecord: set_params:1416: Sample format non available",
            "cannot open device /dev/snd/pcmC1D0c: Permission denied",
            "[rtsp @ 0x1] Failed to resolve hostname camera.local",
            "ffmpeg: Unable to open input",
        ] {
            assert!(is_capture_failure(line), "should be a failure: {line}");
        }
    }

    /// Routine chatter must stay at debug — promoting an xrun storm to warn
    /// would bury the one line that matters.
    #[test]
    fn routine_chatter_stays_at_debug() {
        for line in [
            "overrun!!! (at least 12.345 ms long)",
            "Recording WAVE '/tmp/birdnet-stream/x.wav' : Signed 16 bit Little Endian, Rate 48000 Hz, Mono",
            // What `arecord -t raw` (the teed microphone producer) announces on
            // every start. Promoting this to `warn` would put a line in the
            // journal every time a source restarts, for no reason.
            "Recording raw data 'stdout' : Signed 16 bit Little Endian, Rate 48000 Hz, Mono",
            "frame=  120 fps= 25 q=-1.0 size=     512kB time=00:00:04.80",
            "Warning: Some sources (like microphones) may produce inaudible results",
        ] {
            assert!(!is_capture_failure(line), "should be chatter: {line}");
        }
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(is_capture_failure(
            "ARECORD: AUDIO OPEN ERROR: No Such File"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{AudioFormat, LocalOffset, RtspTransport};
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn is_audio_file_wav() {
        assert!(is_audio_file(Path::new("test.wav")));
        assert!(is_audio_file(Path::new("test.WAV")));
    }

    #[test]
    fn is_audio_file_flac() {
        assert!(is_audio_file(Path::new("test.flac")));
    }

    #[test]
    fn is_audio_file_mp3() {
        assert!(is_audio_file(Path::new("test.mp3")));
    }

    #[test]
    fn is_audio_file_rejects_txt() {
        assert!(!is_audio_file(Path::new("BirdDB.txt")));
    }

    #[test]
    fn required_tool_microphone() {
        let src = CaptureSource::Microphone {
            device: "plughw:1,0".into(),
            sample_rate: 48_000,
            channels: 1,
            stream_id: None,
        };
        let expected = if cfg!(target_os = "macos") {
            "ffmpeg"
        } else {
            "arecord"
        };
        assert_eq!(required_tool(&src), expected);
    }

    #[test]
    fn required_tool_rtsp() {
        let src = CaptureSource::Rtsp {
            url: "rtsp://cam.local/stream".into(),
            stream_id: "cam1".into(),
            transport: RtspTransport::Auto,
        };
        assert_eq!(required_tool(&src), "ffmpeg");
    }

    #[test]
    fn start_microphone_missing_tool() {
        // This test verifies graceful failure when arecord is absent.
        // On CI arecord may not exist — that's fine.
        if is_tool_available("arecord") {
            return; // skip if arecord is present (would actually start recording)
        }
        let config = RecordingConfig {
            source: CaptureSource::Microphone {
                device: "plughw:1,0".into(),
                sample_rate: 48_000,
                channels: 1,
                stream_id: None,
            },
            output_dir: PathBuf::from("/tmp"),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
            gain_db: 0.0,
            local_offset: LocalOffset::utc(),
            live_audio: None,
        };
        assert!(start_microphone_capture(&config).is_err());
    }

    // ---- liveness spans both halves of a teed source -----------------------

    /// Build a `CaptureProcess` around a long-lived stand-in child and a tee
    /// reading `source`. Returns `None` if `sleep(1)` is unavailable, so this
    /// degrades to a skip rather than a spurious failure.
    fn teed_process<R: std::io::Read + Send + 'static>(
        source: R,
        dir: &std::path::Path,
    ) -> Option<CaptureProcess> {
        let child = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let spec = PcmSpec {
            sample_rate: 8_000,
            channels: 1,
        };
        let writer = SegmentWriter::new(
            dir.to_path_buf(),
            None,
            AudioFormat::Wav,
            spec,
            3,
            LocalOffset::utc(),
            SegmentClock::System,
        );
        let tee = tee::spawn("test".to_owned(), source, writer, None, 0.0).ok()?;
        Some(CaptureProcess {
            child,
            source: CaptureSource::Microphone {
                device: "plughw:1,0".into(),
                sample_rate: 8_000,
                channels: 1,
                stream_id: None,
            },
            tee: Some(tee),
        })
    }

    /// A producer that is alive but whose reader has stopped is **not** a
    /// running capture: nothing is draining the pipe, so nothing is being
    /// recorded. Reporting it healthy is how a full disk would look like a
    /// working station forever, instead of a source the supervisor restarts.
    #[test]
    fn a_dead_reader_makes_a_live_producer_read_as_down() {
        let dir = tempfile::tempdir().expect("tempdir");
        // An already-exhausted source: the reader hits EOF and exits at once,
        // while the stand-in producer keeps running.
        let Some(mut process) = teed_process(std::io::Cursor::new(Vec::new()), dir.path()) else {
            return; // no `sleep` on this runner
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while process.is_running() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !process.is_running(),
            "the producer is alive but its reader has exited — that is not a \
             running capture"
        );
        // The producer itself is still alive, which is precisely why checking
        // it alone (as `is_running` used to) would have said "healthy".
        assert!(
            process.child.try_wait().ok().flatten().is_none(),
            "the stand-in producer should still be running"
        );
    }

    /// The counter-case: both halves alive reads as running.
    #[test]
    fn a_live_producer_and_reader_read_as_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (reader, writer_end) = std::io::pipe().expect("pipe");
        let Some(mut process) = teed_process(reader, dir.path()) else {
            return;
        };
        assert!(
            process.is_running(),
            "an open pipe keeps the reader blocked and alive"
        );
        drop(writer_end);
        drop(process);
    }

    // ---- the arecord producer command -------------------------------------

    /// The producer's argv, pinned. There is no ALSA device on CI, so this is
    /// the only place the flags that make the tee work can be checked at all —
    /// and every one of them is load-bearing:
    ///
    /// * no output path, so `arecord` writes to **stdout** for the tee to read;
    /// * `-t raw`, so it emits headerless PCM rather than one endless WAV whose
    ///   header would land in the middle of the first segment;
    /// * **no** `--max-file-time` / `--use-strftime`, because segmentation and
    ///   naming moved into this process.
    #[test]
    fn arecord_streams_raw_pcm_to_stdout() {
        let cmd = arecord_raw_command(
            "plughw:CARD=PRO,DEV=0",
            PcmSpec {
                sample_rate: 48_000,
                channels: 1,
            },
        );
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            argv,
            vec![
                "-D",
                "plughw:CARD=PRO,DEV=0",
                "-f",
                "S16_LE",
                "-r",
                "48000",
                "-c",
                "1",
                "-t",
                "raw",
            ]
        );
        assert_eq!(cmd.get_program(), "arecord");
        // The three flags whose presence would mean arecord is still doing the
        // segmentation — and therefore still owning the only copy of the audio.
        for gone in ["--max-file-time", "--use-strftime", "-t wav"] {
            assert!(
                !argv.iter().any(|a| a == gone),
                "{gone} must not be passed any more"
            );
        }
        // Nothing that looks like an output file: a stray path here would send
        // the stream to disk and leave the tee reading an empty pipe forever.
        assert!(
            !argv.iter().any(|a| a.contains(".wav") || a.contains('/')),
            "arecord must be given no output path: {argv:?}"
        );
    }

    #[test]
    fn arecord_honours_the_source_format() {
        let cmd = arecord_raw_command(
            "plughw:1,0",
            PcmSpec {
                sample_rate: 44_100,
                channels: 2,
            },
        );
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        // A rate or channel count that disagreed with the tap's `PcmSpec` would
        // be decoded at the wrong pitch by `/stream` and written into a WAV
        // header that misdescribes its own samples.
        assert!(argv.contains(&"44100".to_string()));
        assert!(argv.contains(&"2".to_string()));
    }

    // ---- gain decision helpers (pure) -------------------------------------

    #[test]
    fn gain_inactive_at_and_below_epsilon() {
        // Unity gain and anything within the epsilon band is "no gain".
        assert!(!gain_is_active(0.0));
        assert!(!gain_is_active(0.04));
        assert!(!gain_is_active(-0.04));
        // Just under the 0.05 boundary stays inactive.
        assert!(!gain_is_active(0.049));
    }

    #[test]
    fn gain_active_at_or_above_epsilon_either_sign() {
        // Exactly at the boundary is active (>=, not >), and a cut counts via abs().
        assert!(gain_is_active(0.05));
        assert!(gain_is_active(-0.05));
        assert!(gain_is_active(12.0));
        assert!(gain_is_active(-6.0));
    }

    #[test]
    fn volume_filter_none_at_unity() {
        assert_eq!(gain_volume_filter(0.0), None);
        assert_eq!(gain_volume_filter(0.04), None);
    }

    #[test]
    fn volume_filter_formats_db_with_two_decimals() {
        // ffmpeg's `volume` filter wants a dB suffix; we always emit 2 decimals
        // so a boost and a cut format identically (and the sign is preserved).
        assert_eq!(gain_volume_filter(12.0).as_deref(), Some("volume=12.00dB"));
        assert_eq!(gain_volume_filter(-6.5).as_deref(), Some("volume=-6.50dB"));
        assert_eq!(gain_volume_filter(3.25).as_deref(), Some("volume=3.25dB"));
    }

    #[test]
    fn apply_gain_filter_only_adds_args_when_active() {
        // At unity gain the ffmpeg command line is unchanged; with gain it gains
        // exactly the `-af volume=...dB` pair. We assert on the rendered argv.
        let argv = |gain: f32| -> Vec<String> {
            let mut cmd = Command::new("ffmpeg");
            apply_gain_filter(&mut cmd, gain);
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        assert!(argv(0.0).is_empty(), "unity gain adds no args");
        assert_eq!(
            argv(9.0),
            vec!["-af".to_string(), "volume=9.00dB".to_string()]
        );
    }
}
