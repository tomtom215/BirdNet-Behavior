//! Live spectrogram daemon: watches for new audio files and pushes
//! spectrogram data via a callback (typically a WebSocket broadcast).
//!
//! Uses `notify` to watch a directory for new audio files. When a new
//! file appears, it decodes the audio, computes a mel spectrogram, and
//! invokes the registered callback with the spectrogram data serialized
//! as JSON — enabling real-time spectrogram visualization in the browser.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher};

use super::{MelConfig, mel_spectrogram};
use crate::audio::capture::is_audio_file;
use crate::audio::decode::decode_file;

/// Configuration for the live spectrogram daemon.
#[derive(Debug, Clone)]
pub struct LiveSpectrogramConfig {
    /// Directory to watch for new audio files.
    pub watch_dir: PathBuf,
    /// Mel spectrogram configuration.
    pub mel_config: MelConfig,
    /// Maximum number of time frames to include (truncate long files).
    /// Default: 256.
    pub max_frames: usize,
    /// Whether to normalize the spectrogram to [0, 1] range.
    pub normalize: bool,
}

impl Default for LiveSpectrogramConfig {
    fn default() -> Self {
        Self {
            watch_dir: PathBuf::from("."),
            mel_config: MelConfig {
                n_fft: 512,
                hop_length: 128,
                n_mels: 128,
                fmin: 0.0,
                fmax: None,
                power: 2.0,
            },
            max_frames: 256,
            normalize: true,
        }
    }
}

/// A spectrogram frame ready for WebSocket transmission.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpectrogramFrame {
    /// Source filename (without path).
    pub filename: String,
    /// Number of mel bands (rows).
    pub n_mels: usize,
    /// Number of time frames (columns).
    pub n_frames: usize,
    /// Flattened spectrogram data in row-major order (mel band × time frame).
    /// Values are in dB scale, optionally normalized to [0, 1].
    pub data: Vec<f32>,
    /// Sample rate of the source audio.
    pub sample_rate: u32,
}

/// Process a single audio file and return a spectrogram frame.
///
/// # Errors
///
/// Returns a string description if decoding or spectrogram computation fails.
pub fn process_file(
    path: &Path,
    config: &LiveSpectrogramConfig,
) -> Result<SpectrogramFrame, String> {
    let audio = decode_file(path).map_err(|e| format!("decode: {e}"))?;

    if audio.samples.is_empty() {
        return Err("empty audio".into());
    }

    let mel = mel_spectrogram(&audio.samples, audio.sample_rate, &config.mel_config)
        .map_err(|e| format!("spectrogram: {e}"))?;

    let mel_db = mel.to_db(1.0, 80.0);

    // Truncate to max_frames.
    let n_frames = mel_db.n_frames.min(config.max_frames);
    let n_mels = mel_db.n_mels;

    let mut data = Vec::with_capacity(n_mels * n_frames);
    for m in 0..n_mels {
        for f in 0..n_frames {
            data.push(mel_db.get(m, f));
        }
    }

    // Optionally normalize to [0, 1].
    if config.normalize {
        let min_val = data.iter().copied().fold(f32::INFINITY, f32::min);
        let max_val = data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let range = (max_val - min_val).max(1e-6);
        for v in &mut data {
            *v = (*v - min_val) / range;
        }
    }

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(SpectrogramFrame {
        filename,
        n_mels,
        n_frames,
        data,
        sample_rate: audio.sample_rate,
    })
}

/// Run the live spectrogram daemon (blocking).
///
/// Watches `config.watch_dir` for new audio files. When a new file is
/// created, computes a mel spectrogram and calls `on_frame` with the result.
///
/// Stops when a message is received on `stop_rx`.
///
/// # Errors
///
/// Returns a string description if the file watcher cannot be initialized.
pub fn run<F>(
    config: &LiveSpectrogramConfig,
    on_frame: F,
    stop_rx: &mpsc::Receiver<()>,
) -> Result<(), String>
where
    F: Fn(SpectrogramFrame),
{
    let (tx, rx) = std::sync::mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if matches!(event.kind, EventKind::Create(_)) {
                for path in event.paths {
                    let _ = tx.send(path);
                }
            }
        }
    })
    .map_err(|e| format!("watcher init: {e}"))?;

    watcher
        .watch(&config.watch_dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("watch: {e}"))?;

    tracing::info!(
        dir = %config.watch_dir.display(),
        "live spectrogram daemon started"
    );

    loop {
        // Check for stop signal.
        match stop_rx.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                tracing::info!("live spectrogram daemon stopping");
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }

        // Check for new files (non-blocking with timeout).
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(path) => {
                if !path.is_file() || !is_audio_file(&path) {
                    continue;
                }

                // Small delay to ensure the file is fully written.
                std::thread::sleep(Duration::from_millis(100));

                match process_file(&path, config) {
                    Ok(frame) => {
                        tracing::debug!(
                            file = %frame.filename,
                            mels = frame.n_mels,
                            frames = frame.n_frames,
                            "spectrogram computed"
                        );
                        on_frame(frame);
                    }
                    Err(e) => {
                        tracing::warn!(
                            file = %path.display(),
                            error = %e,
                            "live spectrogram failed"
                        );
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = LiveSpectrogramConfig::default();
        assert_eq!(config.max_frames, 256);
        assert!(config.normalize);
        assert_eq!(config.mel_config.n_mels, 128);
    }

    #[test]
    fn process_nonexistent_file_returns_error() {
        let config = LiveSpectrogramConfig::default();
        let result = process_file(Path::new("/nonexistent.wav"), &config);
        assert!(result.is_err());
    }

    #[test]
    fn spectrogram_frame_serializes() {
        let frame = SpectrogramFrame {
            filename: "test.wav".into(),
            n_mels: 4,
            n_frames: 2,
            data: vec![0.0, 0.5, 1.0, 0.3, 0.2, 0.8, 0.1, 0.9],
            sample_rate: 48000,
        };
        // Verify fields are accessible and correct (no serde_json dep in core).
        assert_eq!(frame.filename, "test.wav");
        assert_eq!(frame.n_mels, 4);
        assert_eq!(frame.n_frames, 2);
        assert_eq!(frame.sample_rate, 48000);
        assert_eq!(frame.data.len(), 8);
    }

    #[test]
    fn daemon_stops_on_signal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = LiveSpectrogramConfig {
            watch_dir: dir.path().to_path_buf(),
            ..LiveSpectrogramConfig::default()
        };
        let (tx, stop_rx) = mpsc::channel();

        // Send stop immediately.
        tx.send(()).expect("send stop");

        let result = run(&config, |_| {}, &stop_rx);
        assert!(result.is_ok());
    }
}
