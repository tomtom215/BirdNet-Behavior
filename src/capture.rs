//! Audio capture manager startup.
//!
//! Resolves capture source from CLI flags or config, then starts the
//! `CaptureManager` subprocess lifecycle.

use std::path::PathBuf;

use birdnet_core::audio::capture::{AudioFormat, CaptureManager, CaptureSource, RecordingConfig};

use crate::cli::Cli;

/// Start a managed audio capture process from CLI/config settings.
///
/// Returns the `CaptureManager` handle (keeps recording alive until dropped),
/// or `None` if no capture source is configured or start fails.
pub fn start_capture_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<CaptureManager> {
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

    match manager.start() {
        Ok(()) => {
            tracing::info!("audio capture started");
            Some(manager)
        }
        Err(e) => {
            tracing::warn!(error = %e, "audio capture not started (non-fatal)");
            None
        }
    }
}
