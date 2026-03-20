//! Audio capture from microphone and RTSP streams.
//!
//! Manages subprocess control for `arecord` (local microphone) and
//! `ffmpeg` (RTSP streams), replacing `birdnet_recording.sh`.
//!
//! # Submodules
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | `types` | `CaptureSource`, `RecordingConfig`, `AudioFormat`, `CaptureError` |
//! | `process` | `CaptureProcess`, spawn helpers, tool availability checks |
//! | `manager` | `CaptureManager` lifecycle (start/stop/restart) |
//! | `disk` | `DiskUsage`, `disk_usage`, `recording_stats`, `cleanup_old_recordings` |
//! | `tmpfs` | `TmpfsConfig`, `TmpfsError`, tmpfs mount/unmount helpers |

pub mod disk;
pub mod manager;
pub mod process;
pub mod tmpfs;
pub mod types;

// Re-export the public API so callers keep the same import path.
pub use disk::{
    DiskManager, DiskManagerConfig, DiskUsage, FullDiskAction, cleanup_old_recordings, disk_usage,
    recording_stats,
};
pub use manager::CaptureManager;
pub use process::{is_tool_available, start_microphone_capture, start_rtsp_capture};
pub use tmpfs::{
    TmpfsConfig, TmpfsError, generate_systemd_mount_unit, is_tmpfs_mounted, mount_tmpfs,
    unmount_tmpfs,
};
pub use types::{AudioFormat, CaptureError, CaptureSource, RecordingConfig};

// Internal re-export for detection pipeline modules (daemon.rs, pipeline.rs).
pub(crate) use process::is_audio_file;
