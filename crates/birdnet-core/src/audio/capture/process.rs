//! Audio capture subprocess management.
//!
//! Wraps `arecord` (local microphone) and `ffmpeg` (RTSP) child processes.

use std::path::Path;
use std::process::{Child, Command, Stdio};

use super::types::{CaptureError, CaptureSource, RecordingConfig, recording_filename};

/// A running audio capture process.
#[derive(Debug)]
pub struct CaptureProcess {
    pub(super) child: Child,
    source: CaptureSource,
}

impl CaptureProcess {
    /// Check if the capture process is still running.
    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }

    /// Stop the capture process gracefully.
    ///
    /// # Errors
    ///
    /// Returns `CaptureError` if the process cannot be terminated.
    pub fn stop(&mut self) -> Result<(), CaptureError> {
        self.child.kill().map_err(CaptureError::Spawn)?;
        self.child.wait().map_err(CaptureError::Spawn)?;
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
/// is treated as "off": local microphone capture stays on the lighter-weight
/// `arecord` path and the ffmpeg sources omit the `volume` filter entirely.
/// `0.05` dB is well below audible and matches the threshold the admin UI uses
/// to decide whether to *show* a gain badge, so "looks like no gain in the UI"
/// and "no gain applied at capture" always agree.
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

/// Start an audio capture process for a microphone source.
///
/// Uses `arecord` (ALSA) at unity gain — the historical, lightest path. When a
/// non-zero `gain_db` is configured, capture is routed through `ffmpeg -f alsa`
/// instead, because `arecord` has no software-gain control; the gain is applied
/// with ffmpeg's `volume` filter. macOS always uses `ffmpeg`'s avfoundation
/// input (ALSA is Linux-only), with the same optional gain filter.
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

    let filename_pattern = recording_filename(stream_id.as_deref(), config.format);
    let output_path = config.output_dir.join(&filename_pattern);
    let segment = config.segment_duration_secs.to_string();
    let gain = config.gain_db;

    // macOS captures the system microphone through ffmpeg's avfoundation input
    // (ALSA's `arecord` is Linux-only); on Linux we use `arecord` unless a gain
    // is configured, in which case we use `ffmpeg -f alsa` so the `volume`
    // filter can apply it (arecord has no software gain). Both ffmpeg branches
    // are compiled on every platform via `cfg!` (not `#[cfg]`), so the macOS
    // path is type-checked and linted even when the build runs on Linux.
    let mut cmd = if cfg!(target_os = "macos") {
        // The avfoundation input spec is "[video]:[audio]"; ":<n>" selects audio
        // device <n> with no video capture. ALSA-style device names (e.g.
        // "plughw:1,0") don't apply, so fall back to the default input (0).
        let audio_device = if device.is_empty() || device.contains(':') || device.starts_with("hw")
        {
            "0"
        } else {
            device.as_str()
        };
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-f")
            .arg("avfoundation")
            .arg("-i")
            .arg(format!(":{audio_device}"))
            .arg("-ar")
            .arg(sample_rate.to_string())
            .arg("-ac")
            .arg(channels.to_string());
        apply_gain_filter(&mut cmd, gain);
        cmd.arg("-f")
            .arg("segment")
            .arg("-segment_time")
            .arg(&segment)
            .arg("-strftime")
            .arg("1")
            .arg(output_path.to_string_lossy().as_ref());
        cmd
    } else if gain_is_active(gain) {
        // Linux microphone WITH gain: ffmpeg's ALSA input + `volume` filter.
        // `-f alsa -i <device>` takes the same device name `arecord -D` would.
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-f")
            .arg("alsa")
            .arg("-i")
            .arg(device)
            .arg("-ar")
            .arg(sample_rate.to_string())
            .arg("-ac")
            .arg(channels.to_string());
        apply_gain_filter(&mut cmd, gain);
        cmd.arg("-f")
            .arg("segment")
            .arg("-segment_time")
            .arg(&segment)
            .arg("-strftime")
            .arg("1")
            .arg(output_path.to_string_lossy().as_ref());
        cmd
    } else {
        // Linux microphone at unity gain: the historical, lightest `arecord`
        // path, byte-for-byte unchanged.
        let mut cmd = Command::new("arecord");
        cmd.arg("-D")
            .arg(device)
            .arg("-f")
            .arg("S16_LE")
            .arg("-r")
            .arg(sample_rate.to_string())
            .arg("-c")
            .arg(channels.to_string())
            .arg("--max-file-time")
            .arg(&segment)
            .arg("--use-strftime")
            .arg(output_path.to_string_lossy().as_ref());
        cmd
    };
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    drain_capture_stderr(&mut child, device);
    tracing::info!(device = %device, gain_db = gain, "started microphone capture");
    Ok(CaptureProcess {
        child,
        source: config.source.clone(),
    })
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
    use super::super::types::{AudioFormat, RtspTransport};
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
        };
        assert!(start_microphone_capture(&config).is_err());
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
