//! Shared, read-mostly snapshot of the capture supervisor's per-source health.
//!
//! The capture supervisor runs on its own OS thread in the binary crate and
//! owns its source list privately; the web layer — a different subsystem in the
//! same process — needs to *show* that state on the Station Health page without
//! reaching into the supervisor. This module is the seam: the supervisor
//! publishes a [`CaptureStatus`] snapshot into a shared [`CaptureStatusHandle`]
//! once per reconcile tick, and the web layer reads it on demand.
//!
//! Both sides depend on `birdnet-core`, so defining the types here avoids a
//! dependency cycle (the binary's supervisor → `birdnet-core` ← `birdnet-web`).
//!
//! These are plain data types with no I/O; the rolling-uptime *accumulator*
//! that fills [`SourceStatus::uptime_24h`] lives with the supervisor.

use std::sync::{Arc, RwLock};

use serde::Serialize;

/// Number of half-hour segments in the rolling 24-hour uptime strip
/// (48 × 30 min = 24 h).
pub const UPTIME_SEGMENTS: usize = 48;

/// A shared handle to the capture supervisor's latest published status.
///
/// Cloned into both the web `AppState` (reader) and the supervisor thread
/// (writer). Reads and writes recover from lock poisoning rather than panic, so
/// a panic on one side can never wedge the other.
pub type CaptureStatusHandle = Arc<RwLock<CaptureStatus>>;

/// Create an empty [`CaptureStatusHandle`].
#[must_use]
pub fn new_capture_status() -> CaptureStatusHandle {
    Arc::new(RwLock::new(CaptureStatus::default()))
}

/// Read the current snapshot out of a handle (cloned), recovering from a
/// poisoned lock so a panicked writer can never wedge the reader.
#[must_use]
pub fn read_capture_status(handle: &CaptureStatusHandle) -> CaptureStatus {
    handle
        .read()
        .map_or_else(|poison| poison.into_inner().clone(), |guard| guard.clone())
}

/// Replace the snapshot in a handle, recovering from a poisoned lock.
pub fn publish_capture_status(handle: &CaptureStatusHandle, status: CaptureStatus) {
    match handle.write() {
        Ok(mut guard) => *guard = status,
        Err(poison) => *poison.into_inner() = status,
    }
}

/// A point-in-time snapshot of every supervised capture source.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CaptureStatus {
    /// Per-source health, in the supervisor's configured order.
    pub sources: Vec<SourceStatus>,
    /// Wall-clock seconds since the Unix epoch when this snapshot was published
    /// (`0` if never published).
    pub published_unix: u64,
}

/// One capture source's health at snapshot time.
#[derive(Debug, Clone, Serialize)]
pub struct SourceStatus {
    /// Source label, matching the `birdnet_audio_source_up{source}` gauge and
    /// the detection-side source tag (e.g. `local`, `RTSP_1`).
    pub label: String,
    /// Current lifecycle state.
    pub state: SourceState,
    /// Seconds the current process has been running, when connected.
    pub uptime_secs: Option<u64>,
    /// Seconds since this source last delivered fresh audio, if ever observed.
    pub last_audio_age_secs: Option<u64>,
    /// Consecutive (re)start attempts not yet healthy (`0` when connected).
    pub restart_attempts: u32,
    /// Seconds until the next restart attempt, while backing off.
    pub next_retry_in_secs: Option<u64>,
    /// Rolling 24-hour uptime, oldest → newest, one entry per half hour.
    /// Always [`UPTIME_SEGMENTS`] long.
    pub uptime_24h: Vec<UptimeSegment>,
}

/// A capture source's lifecycle state at snapshot time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    /// Process alive and delivering fresh audio.
    Connected,
    /// Process alive but writing no segments — a wedged source, being restarted.
    Stalled,
    /// Not running; retrying with capped exponential backoff.
    BackingOff,
    /// Intentionally stopped — outside the recording schedule or in a quiet
    /// window — not a fault.
    Paused,
}

impl SourceState {
    /// Whether this state represents a fault the operator should act on
    /// (stalled or backing off), as opposed to healthy or intentionally paused.
    #[must_use]
    pub const fn is_fault(self) -> bool {
        matches!(self, Self::Stalled | Self::BackingOff)
    }
}

/// One half-hour cell of the 24-hour uptime strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UptimeSegment {
    /// Connected for the majority of the half hour.
    Up,
    /// Down or stalled for the majority of the half hour.
    Down,
    /// No data — before the source was first observed, or intentionally paused.
    Out,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_then_read_round_trips() {
        let handle = new_capture_status();
        let status = CaptureStatus {
            sources: vec![SourceStatus {
                label: "local".into(),
                state: SourceState::Connected,
                uptime_secs: Some(120),
                last_audio_age_secs: Some(1),
                restart_attempts: 0,
                next_retry_in_secs: None,
                uptime_24h: vec![UptimeSegment::Up; UPTIME_SEGMENTS],
            }],
            published_unix: 1_700_000_000,
        };
        publish_capture_status(&handle, status);
        let read = read_capture_status(&handle);
        assert_eq!(read.sources.len(), 1);
        assert_eq!(read.sources[0].label, "local");
        assert_eq!(read.sources[0].state, SourceState::Connected);
        assert_eq!(read.published_unix, 1_700_000_000);
    }

    #[test]
    fn read_recovers_from_poisoned_lock() {
        let handle = new_capture_status();
        publish_capture_status(
            &handle,
            CaptureStatus {
                sources: vec![],
                published_unix: 42,
            },
        );
        // Poison the lock by panicking while holding the write guard.
        let h = Arc::clone(&handle);
        let _ = std::thread::spawn(move || {
            let _guard = h.write().unwrap();
            panic!("poison the lock");
        })
        .join();
        // The reader must still recover the last-published value, not panic.
        let read = read_capture_status(&handle);
        assert_eq!(read.published_unix, 42);
    }

    #[test]
    fn is_fault_classifies_states() {
        assert!(SourceState::Stalled.is_fault());
        assert!(SourceState::BackingOff.is_fault());
        assert!(!SourceState::Connected.is_fault());
        assert!(!SourceState::Paused.is_fault());
    }

    #[test]
    fn empty_status_is_default() {
        let handle = new_capture_status();
        let read = read_capture_status(&handle);
        assert!(read.sources.is_empty());
        assert_eq!(read.published_unix, 0);
    }
}
