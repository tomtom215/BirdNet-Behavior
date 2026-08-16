//! Audio capture from microphone and RTSP streams.
//!
//! Manages subprocess control for `arecord` (local microphone) and
//! `ffmpeg` (RTSP streams), replacing `birdnet_recording.sh`.
//!
//! # Submodules
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | `types` | `CaptureSource`, `RecordingConfig`, `AudioFormat`, `CaptureError`, `LocalOffset` |
//! | `process` | `CaptureProcess`, spawn helpers, tool availability checks |
//! | `manager` | `CaptureManager` lifecycle (start/stop/liveness) |
//! | `tee` | The in-process splitter: one device open, two consumers |
//! | `segment` | Rotating WAV segment writer used by the tee |
//! | `live` | `LiveTap` / `LiveAudioHub` — the capture→web live-audio seam |
//! | `disk` | `DiskUsage`, `disk_usage`, `recording_stats`, `cleanup_old_recordings` |
//! | `tmpfs` | `TmpfsConfig`, `TmpfsError`, tmpfs mount/unmount helpers |
//! | `status` | `CaptureStatus` — the supervisor→web per-source health seam |

pub mod disk;
pub mod live;
pub mod manager;
pub mod process;
pub mod status;
pub mod tmpfs;
pub mod types;

mod segment;
mod tee;

// Re-export the public API so callers keep the same import path.
pub use disk::{
    DiskManager, DiskManagerConfig, DiskUsage, FullDiskAction, LockedFilesProvider,
    cleanup_old_recordings, disk_usage, recording_stats,
};
pub use live::{
    LiveAudioHub, LiveAudioHubHandle, LiveSubscription, LiveTap, PcmSpec, new_live_audio_hub,
};
pub use manager::CaptureManager;
pub use process::{is_tool_available, start_microphone_capture, start_rtsp_capture};
pub use status::{
    CaptureStatus, CaptureStatusHandle, SourceState, SourceStatus, UPTIME_SEGMENTS, UptimeSegment,
    new_capture_status, publish_capture_status, read_capture_status,
};
/// Which half of a stereo capture to keep; see [`tee::ChannelPick`].
pub use tee::ChannelPick;
pub use tmpfs::{
    TmpfsConfig, TmpfsError, generate_systemd_mount_unit, is_tmpfs_mounted, mount_tmpfs,
    unmount_tmpfs,
};
pub use types::{
    AudioFormat, CaptureError, CaptureSource, LocalOffset, RecordingConfig, RtspTransport,
    recording_filename_at,
};

// Internal re-export for detection pipeline modules (daemon.rs, pipeline.rs).
pub(crate) use process::is_audio_file;
