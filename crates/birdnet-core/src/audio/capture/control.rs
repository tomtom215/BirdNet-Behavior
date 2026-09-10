//! Operator-initiated control requests for the capture supervisor.
//!
//! The counterpart to [`super::status`]: that module carries state *out* of the
//! supervisor for the web layer to show, this one carries a request *in* for the
//! supervisor to act on. Both exist because the supervisor owns its source list
//! privately on its own OS thread in the binary crate, and the web layer — a
//! different subsystem in the same process — must neither block it nor reach
//! into it.
//!
//! The only request today is "restart this one source". It exists because the
//! alternative an operator has for a wedged RTSP camera is restarting the whole
//! service, which drops every *other* source and loses the audio in flight.
//!
//! ## Why a set of pending labels rather than a channel
//!
//! A request is idempotent and has no payload: asking twice before the
//! supervisor next ticks must mean the same as asking once, and a request that
//! arrives while the supervisor is mid-tick must be honoured on the next one
//! rather than lost. A set drained once per tick gives both properties without
//! an unbounded queue that a stuck supervisor could grow without limit.
//!
//! Locks recover from poisoning rather than panic, exactly as [`super::status`]
//! does, so a panic on one side cannot wedge the other.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

/// A shared handle to the set of capture sources with a restart pending.
///
/// Cloned into both the web `AppState` (writer) and the supervisor thread
/// (drainer). Keyed by the supervisor's source label — the same string the
/// `birdnet_audio_source_up{source}` gauge and
/// [`super::status::SourceStatus::label`] carry, which for a database-driven
/// station is the `audio_sources` row id.
pub type CaptureControlHandle = Arc<Mutex<BTreeSet<String>>>;

/// Create an empty [`CaptureControlHandle`].
#[must_use]
pub fn new_capture_control() -> CaptureControlHandle {
    Arc::new(Mutex::new(BTreeSet::new()))
}

/// Record a restart request for the source labelled `label`.
///
/// Returns `true` when this call recorded the request and `false` when one was
/// already pending for that source — the caller can use it to tell an operator
/// clicking twice that the first click is still in flight, but either way the
/// supervisor will restart the source exactly once.
pub fn request_source_restart(handle: &CaptureControlHandle, label: &str) -> bool {
    with_pending(handle, |pending| pending.insert(label.to_owned()))
}

/// Whether a restart is still pending for `label` (i.e. the supervisor has not
/// drained it yet).
#[must_use]
pub fn is_restart_pending(handle: &CaptureControlHandle, label: &str) -> bool {
    with_pending(handle, |pending| pending.contains(label))
}

/// Take every pending restart request, leaving the set empty.
///
/// Called once per supervisor tick, before reconciliation, so a request made
/// while the previous tick was running is picked up by the next one.
#[must_use]
pub fn take_source_restarts(handle: &CaptureControlHandle) -> BTreeSet<String> {
    with_pending(handle, std::mem::take)
}

/// Run `f` against the pending set, recovering from a poisoned lock.
fn with_pending<T>(handle: &CaptureControlHandle, f: impl FnOnce(&mut BTreeSet<String>) -> T) -> T {
    match handle.lock() {
        Ok(mut guard) => f(&mut guard),
        Err(poison) => f(&mut poison.into_inner()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_restart_pending, new_capture_control, request_source_restart, take_source_restarts,
    };

    #[test]
    fn a_request_is_pending_until_it_is_taken() {
        let control = new_capture_control();
        assert!(!is_restart_pending(&control, "cam"));
        assert!(request_source_restart(&control, "cam"));
        assert!(is_restart_pending(&control, "cam"));

        let taken = take_source_restarts(&control);
        assert_eq!(taken.len(), 1);
        assert!(taken.contains("cam"));
        assert!(
            !is_restart_pending(&control, "cam"),
            "taking the requests must leave the set empty, or the supervisor \
             restarts the source again on every subsequent tick"
        );
    }

    #[test]
    fn asking_twice_before_a_tick_restarts_once() {
        // Idempotence is the whole reason this is a set. An operator who clicks
        // Restart twice must not get two stop/start cycles out of one tick.
        let control = new_capture_control();
        assert!(request_source_restart(&control, "cam"));
        assert!(
            !request_source_restart(&control, "cam"),
            "the second request must report that one was already pending"
        );
        assert_eq!(take_source_restarts(&control).len(), 1);
    }

    #[test]
    fn requests_are_per_source() {
        let control = new_capture_control();
        request_source_restart(&control, "cam");
        request_source_restart(&control, "mic");
        assert!(is_restart_pending(&control, "cam"));
        assert!(is_restart_pending(&control, "mic"));
        assert!(!is_restart_pending(&control, "other"));

        let taken = take_source_restarts(&control);
        assert_eq!(
            taken.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["cam", "mic"]
        );
    }

    #[test]
    fn a_poisoned_lock_does_not_wedge_the_other_side() {
        // The supervisor and the web layer are different threads; a panic while
        // one holds the lock must not turn every later request into a panic.
        let control = new_capture_control();
        let poisoner = std::sync::Arc::clone(&control);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().expect("lock");
            panic!("poison the mutex");
        })
        .join();

        assert!(request_source_restart(&control, "cam"));
        assert!(take_source_restarts(&control).contains("cam"));
    }
}
