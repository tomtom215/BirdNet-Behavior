//! Capture supervisor: keeps audio-capture subprocesses alive for the
//! lifetime of an unattended field deployment.
//!
//! # Why this exists
//!
//! [`CaptureManager`] knows how to *start* and *stop* a recording
//! subprocess (`arecord` / `ffmpeg`), but on its own it never notices when
//! that subprocess dies. A bumped USB microphone, an `ffmpeg` crash, or an
//! RTSP camera reboot leaves a dead process that is never respawned — so on
//! a station meant to run untouched for months, one transient glitch
//! silences the sensor until the next manual restart. Losing a night (or a
//! season) of irreplaceable detections is the single worst field failure.
//!
//! This module supervises one or more capture sources. On a fixed cadence
//! it reconciles each source toward its *desired* state:
//!
//! * **inside** the recording schedule → the subprocess must be running;
//!   if it died, restart it with capped exponential backoff so a genuinely
//!   broken source backs off instead of spinning the CPU — but we **never
//!   permanently give up**, because a camera that is down for an hour may
//!   come back on hour two and we must be recording when it does;
//! * **outside** the recording schedule → the subprocess must be stopped,
//!   so a solar / fixed window that closes at night actually pauses capture
//!   instead of only logging that it "should".
//!
//! It also drives the `birdnet_audio_source_up{source}` gauge from real
//! process health, so a silent night (no detections) is no longer
//! indistinguishable from a dead microphone, and surfaces a loud log when a
//! source has been unexpectedly down for more than a couple of minutes.
//!
//! # Testability
//!
//! Every restart / backoff / schedule decision lives in [`Supervisor::tick`]
//! and the pure helpers below; the only real I/O is hidden behind the
//! [`Source`] trait. The infinite loop, the OS thread, and the wall-clock
//! sleeps live in the (inherently untestable) caller in [`super`]. Unit
//! tests drive `tick` with a fake source and a hand-controlled clock to
//! exercise death → backoff → recovery and the schedule gate without ever
//! spawning a real subprocess.

use std::time::{Duration, Instant};

use birdnet_core::audio::capture::{CaptureError, CaptureManager, CaptureSource};
use birdnet_web::metrics::SharedMetrics;

/// First retry delay after a source is found dead.
const BACKOFF_BASE: Duration = Duration::from_secs(2);

/// Upper bound on the retry delay. A persistently broken source is retried
/// at least this often, forever — we never stop trying to bring a field
/// sensor back.
const BACKOFF_CAP: Duration = Duration::from_secs(60);

/// How long a source may be unexpectedly down before we log a loud warning.
const DOWN_WARN_AFTER: Duration = Duration::from_secs(120);

/// Cadence for repeating the "still down" warning after the first one.
const DOWN_WARN_EVERY: Duration = Duration::from_secs(300);

/// The slice of [`CaptureManager`] behaviour the supervisor depends on.
///
/// Abstracted into a trait so unit tests can inject a fake process that
/// "dies" on command and exercise the restart path without a real
/// `arecord` / `ffmpeg` on the test runner.
pub(super) trait Source {
    /// Whether the underlying subprocess is currently running.
    fn is_running(&mut self) -> bool;

    /// (Re)start the subprocess.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`CaptureManager::start`] would return (tool
    /// missing, spawn failure, …).
    fn start(&mut self) -> Result<(), CaptureError>;

    /// Stop the subprocess.
    fn stop(&mut self);
}

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

/// Gauge label for a capture source.
///
/// Matches the detection-side `derive_source_label` (in
/// `crate::daemon::disposition`) so the `birdnet_audio_source_up{source}`
/// series produced by process-health here coincides with the one produced
/// per-detection there: RTSP streams keep their `stream_id`
/// (`rtsp` / `RTSP_1` / …), and every local microphone collapses to
/// `local`.
#[must_use]
pub(super) fn source_gauge_label(source: &CaptureSource) -> String {
    match source {
        CaptureSource::Rtsp { stream_id, .. } => stream_id.clone(),
        CaptureSource::Microphone { .. } | CaptureSource::PipeWire { .. } => "local".to_owned(),
    }
}

/// Backoff delay before the next restart attempt, given the number of
/// consecutive start attempts that have not yet produced a healthy process.
///
/// `0` attempts → no delay (the first attempt fires immediately); then
/// `2s, 4s, 8s, …` doubling up to [`BACKOFF_CAP`].
#[must_use]
const fn backoff_delay(attempts_since_healthy: u32) -> Duration {
    if attempts_since_healthy == 0 {
        return Duration::ZERO;
    }
    // Clamp the shift so the doubling can never overflow the `u64`.
    let shift = if attempts_since_healthy > 21 {
        20
    } else {
        attempts_since_healthy - 1
    };
    let secs = BACKOFF_BASE.as_secs() << shift;
    let cap = BACKOFF_CAP.as_secs();
    Duration::from_secs(if secs > cap { cap } else { secs })
}

/// Whether a "still down" warning is due right now.
///
/// True only once a source has been continuously down for at least
/// [`DOWN_WARN_AFTER`], and then no more often than [`DOWN_WARN_EVERY`].
#[must_use]
fn should_warn_down(down_since: Option<Instant>, last_warn: Option<Instant>, now: Instant) -> bool {
    let Some(since) = down_since else {
        return false;
    };
    if now.saturating_duration_since(since) < DOWN_WARN_AFTER {
        return false;
    }
    last_warn.is_none_or(|last| now.saturating_duration_since(last) >= DOWN_WARN_EVERY)
}

/// One supervised capture source plus its restart bookkeeping.
struct SupervisedSource<S: Source> {
    source: S,
    label: String,
    /// Consecutive (re)start attempts that have not yet produced a process
    /// observed alive on a later tick. Drives the backoff delay; reset to
    /// `0` the moment the process is seen running.
    attempts_since_healthy: u32,
    /// Earliest instant at which the next start attempt may be made.
    /// `None` means "may attempt immediately".
    next_attempt_at: Option<Instant>,
    /// When the source first became unexpectedly down (desired-on but not
    /// running). `None` while up or while intentionally paused.
    down_since: Option<Instant>,
    /// Last instant a "still down" warning was emitted, to rate-limit it.
    last_down_warn: Option<Instant>,
}

impl<S: Source> SupervisedSource<S> {
    const fn new(source: S, label: String) -> Self {
        Self {
            source,
            label,
            attempts_since_healthy: 0,
            next_attempt_at: None,
            down_since: None,
            last_down_warn: None,
        }
    }

    /// Forget all fault state — used when the source is healthy or
    /// intentionally paused.
    const fn clear_fault(&mut self) {
        self.attempts_since_healthy = 0;
        self.next_attempt_at = None;
        self.down_since = None;
        self.last_down_warn = None;
    }

    /// Emit (and rate-limit) the loud "source still down" warning.
    fn maybe_warn_down(&mut self, now: Instant) {
        if should_warn_down(self.down_since, self.last_down_warn, now) {
            let down_for = self
                .down_since
                .map_or(Duration::ZERO, |s| now.saturating_duration_since(s));
            tracing::warn!(
                source = %self.label,
                down_for_secs = down_for.as_secs(),
                "audio source DOWN — no recording from this source; still trying to restart"
            );
            self.last_down_warn = Some(now);
        }
    }

    /// Reconcile this one source toward its desired state for this tick.
    fn reconcile(&mut self, now: Instant, recording_allowed: bool, metrics: &SharedMetrics) {
        // Outside the schedule: ensure stopped, clear fault state, gauge 0.
        if !recording_allowed {
            if self.source.is_running() {
                self.source.stop();
                tracing::info!(source = %self.label, "recording schedule closed — capture paused");
            }
            self.clear_fault();
            metrics.set_source_up(&self.label, false);
            return;
        }

        // Desired ON and healthy: clear fault state, gauge 1.
        if self.source.is_running() {
            if let Some(since) = self.down_since {
                tracing::info!(
                    source = %self.label,
                    downtime_secs = now.saturating_duration_since(since).as_secs(),
                    "audio source up"
                );
            }
            self.clear_fault();
            metrics.set_source_up(&self.label, true);
            return;
        }

        // Desired ON but not running: it died, never started, or must
        // resume after a scheduled pause.
        metrics.set_source_up(&self.label, false);
        if self.down_since.is_none() {
            self.down_since = Some(now);
        }

        // Honour the backoff window before trying again.
        if self.next_attempt_at.is_some_and(|at| now < at) {
            self.maybe_warn_down(now);
            return;
        }

        // Make exactly one attempt this tick and schedule the next one.
        // We count the attempt and arm the backoff *regardless* of whether
        // `start` returns `Ok`: a process that launches and then dies before
        // the next tick must still back off, or a flapping source would hot
        // loop. The attempt counter is only refunded once the process is
        // actually observed running (the healthy branch above).
        self.attempts_since_healthy = self.attempts_since_healthy.saturating_add(1);
        let delay = backoff_delay(self.attempts_since_healthy);
        self.next_attempt_at = Some(now + delay);
        match self.source.start() {
            Ok(()) => tracing::info!(
                source = %self.label,
                attempt = self.attempts_since_healthy,
                "capture (re)start issued"
            ),
            Err(e) => tracing::warn!(
                source = %self.label,
                error = %e,
                attempt = self.attempts_since_healthy,
                retry_in_secs = delay.as_secs(),
                "capture (re)start failed; will retry"
            ),
        }
        self.maybe_warn_down(now);
    }
}

/// Supervises a set of capture sources, restarting them on death and
/// pausing/resuming them with the recording schedule.
pub(super) struct Supervisor<S: Source> {
    sources: Vec<SupervisedSource<S>>,
}

impl<S: Source> Supervisor<S> {
    /// Build a supervisor over `(source, gauge_label)` pairs.
    pub(super) fn new(sources: Vec<(S, String)>) -> Self {
        Self {
            sources: sources
                .into_iter()
                .map(|(source, label)| SupervisedSource::new(source, label))
                .collect(),
        }
    }

    /// Run one reconciliation pass over every source.
    ///
    /// `now` is the monotonic clock used for backoff/down timing;
    /// `recording_allowed` is the schedule gate evaluated against the wall
    /// clock by the caller. Splitting the two clocks keeps this method
    /// deterministically testable.
    pub(super) fn tick(&mut self, now: Instant, recording_allowed: bool, metrics: &SharedMetrics) {
        for source in &mut self.sources {
            source.reconcile(now, recording_allowed, metrics);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use birdnet_web::metrics::MetricsRegistry;

    /// A fake capture source whose liveness the test controls directly.
    struct FakeSource {
        running: bool,
        /// Number of upcoming `start` calls that should fail before one
        /// succeeds. Saturates at 0.
        fail_starts: u32,
        start_calls: u32,
        stop_calls: u32,
    }

    impl FakeSource {
        fn healthy() -> Self {
            Self {
                running: true,
                fail_starts: 0,
                start_calls: 0,
                stop_calls: 0,
            }
        }

        fn dead() -> Self {
            Self {
                running: false,
                fail_starts: 0,
                start_calls: 0,
                stop_calls: 0,
            }
        }

        /// Dead, and every `start` fails forever (simulates a broken source).
        fn always_failing() -> Self {
            Self {
                running: false,
                fail_starts: u32::MAX,
                start_calls: 0,
                stop_calls: 0,
            }
        }
    }

    impl Source for FakeSource {
        fn is_running(&mut self) -> bool {
            self.running
        }

        fn start(&mut self) -> Result<(), CaptureError> {
            self.start_calls += 1;
            if self.fail_starts > 0 {
                self.fail_starts -= 1;
                return Err(CaptureError::Config("fake start failure".into()));
            }
            self.running = true;
            Ok(())
        }

        fn stop(&mut self) {
            self.stop_calls += 1;
            self.running = false;
        }
    }

    fn metrics() -> SharedMetrics {
        Arc::new(MetricsRegistry::new())
    }

    fn gauge(m: &SharedMetrics, label: &str) -> Option<u64> {
        m.snapshot()
            .source_up
            .into_iter()
            .find(|(k, _)| k == label)
            .map(|(_, v)| v)
    }

    fn one(source: FakeSource) -> Supervisor<FakeSource> {
        Supervisor::new(vec![(source, "local".to_owned())])
    }

    // ---- source_gauge_label ------------------------------------------------

    #[test]
    fn label_microphone_is_local() {
        let src = CaptureSource::Microphone {
            device: "plughw:1,0".into(),
            sample_rate: 48_000,
            channels: 1,
        };
        assert_eq!(source_gauge_label(&src), "local");
    }

    #[test]
    fn label_pipewire_is_local() {
        let src = CaptureSource::PipeWire {
            device: String::new(),
            sample_rate: 48_000,
            channels: 1,
        };
        assert_eq!(source_gauge_label(&src), "local");
    }

    #[test]
    fn label_rtsp_is_stream_id() {
        let src = CaptureSource::Rtsp {
            url: "rtsp://cam.local/s".into(),
            stream_id: "RTSP_2".into(),
        };
        assert_eq!(source_gauge_label(&src), "RTSP_2");
    }

    // ---- backoff_delay -----------------------------------------------------

    #[test]
    fn backoff_zero_attempts_is_immediate() {
        assert_eq!(backoff_delay(0), Duration::ZERO);
    }

    #[test]
    fn backoff_doubles_then_caps() {
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(2), Duration::from_secs(4));
        assert_eq!(backoff_delay(3), Duration::from_secs(8));
        assert_eq!(backoff_delay(4), Duration::from_secs(16));
        assert_eq!(backoff_delay(5), Duration::from_secs(32));
        // 64s would exceed the 60s cap.
        assert_eq!(backoff_delay(6), Duration::from_secs(60));
        assert_eq!(backoff_delay(7), Duration::from_secs(60));
    }

    #[test]
    fn backoff_never_overflows_at_extreme_attempts() {
        // Must stay capped (and not panic) for absurd attempt counts.
        assert_eq!(backoff_delay(u32::MAX), Duration::from_secs(60));
        assert_eq!(backoff_delay(1000), Duration::from_secs(60));
    }

    // ---- should_warn_down --------------------------------------------------

    /// `now` minus `ago`, for building past instants in tests without
    /// tripping the `unchecked_time_subtraction` lint.
    fn earlier(now: Instant, ago: Duration) -> Instant {
        now.checked_sub(ago).expect("instant within test range")
    }

    #[test]
    fn no_warn_when_not_down() {
        assert!(!should_warn_down(None, None, Instant::now()));
    }

    #[test]
    fn no_warn_before_threshold() {
        let now = Instant::now();
        let since = earlier(now, Duration::from_secs(DOWN_WARN_AFTER.as_secs() - 1));
        assert!(!should_warn_down(Some(since), None, now));
    }

    #[test]
    fn warn_at_threshold_when_never_warned() {
        let now = Instant::now();
        let since = earlier(now, DOWN_WARN_AFTER);
        assert!(should_warn_down(Some(since), None, now));
    }

    #[test]
    fn no_repeat_warn_within_interval() {
        let now = Instant::now();
        let since = earlier(now, DOWN_WARN_AFTER + DOWN_WARN_EVERY);
        let last = earlier(now, Duration::from_secs(DOWN_WARN_EVERY.as_secs() - 1));
        assert!(!should_warn_down(Some(since), Some(last), now));
    }

    #[test]
    fn repeat_warn_after_interval() {
        let now = Instant::now();
        let since = earlier(now, DOWN_WARN_AFTER + DOWN_WARN_EVERY);
        let last = earlier(now, DOWN_WARN_EVERY);
        assert!(should_warn_down(Some(since), Some(last), now));
    }

    // ---- reconcile: schedule gate -----------------------------------------

    #[test]
    fn schedule_closed_stops_running_source() {
        let m = metrics();
        let mut sup = one(FakeSource::healthy());
        sup.tick(Instant::now(), false, &m);
        assert_eq!(sup.sources[0].source.stop_calls, 1);
        assert!(!sup.sources[0].source.running);
        assert_eq!(gauge(&m, "local"), Some(0));
    }

    #[test]
    fn schedule_closed_leaves_stopped_source_alone() {
        let m = metrics();
        let mut sup = one(FakeSource::dead());
        sup.tick(Instant::now(), false, &m);
        assert_eq!(sup.sources[0].source.start_calls, 0);
        assert_eq!(sup.sources[0].source.stop_calls, 0);
        assert_eq!(gauge(&m, "local"), Some(0));
    }

    // ---- reconcile: healthy path ------------------------------------------

    #[test]
    fn healthy_source_not_touched_and_gauge_up() {
        let m = metrics();
        let mut sup = one(FakeSource::healthy());
        sup.tick(Instant::now(), true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 0);
        assert_eq!(sup.sources[0].source.stop_calls, 0);
        assert_eq!(gauge(&m, "local"), Some(1));
    }

    // ---- reconcile: restart-on-death (the core fault-injection test) ------

    #[test]
    fn dead_source_is_restarted_and_recovers() {
        let m = metrics();
        let mut sup = one(FakeSource::dead());
        let t0 = Instant::now();

        // Tick 1: observed down → one start attempt fires, source comes
        // back to life, but the gauge is only confirmed up next tick.
        sup.tick(t0, true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        assert!(sup.sources[0].source.running);
        assert_eq!(gauge(&m, "local"), Some(0));
        assert!(sup.sources[0].down_since.is_some());

        // Tick 2: observed running → gauge up, fault state cleared.
        sup.tick(t0 + Duration::from_secs(1), true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1, "no spurious restart");
        assert_eq!(gauge(&m, "local"), Some(1));
        assert_eq!(sup.sources[0].attempts_since_healthy, 0);
        assert!(sup.sources[0].down_since.is_none());
    }

    /// Inject a fault mid-run: a healthy source dies, and the supervisor
    /// must bring it back.
    #[test]
    fn source_that_dies_mid_run_is_revived() {
        let m = metrics();
        let mut sup = one(FakeSource::healthy());
        let t0 = Instant::now();

        sup.tick(t0, true, &m);
        assert_eq!(gauge(&m, "local"), Some(1));

        // Fault injection: the subprocess dies.
        sup.sources[0].source.running = false;

        // Next tick notices and issues a restart.
        sup.tick(t0 + Duration::from_secs(1), true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        assert!(sup.sources[0].source.running);

        // And it is confirmed back up.
        sup.tick(t0 + Duration::from_secs(2), true, &m);
        assert_eq!(gauge(&m, "local"), Some(1));
    }

    // ---- reconcile: exponential backoff on a broken source ----------------

    #[test]
    fn broken_source_backs_off_exponentially() {
        let m = metrics();
        let mut sup = one(FakeSource::always_failing());
        let t0 = Instant::now();

        // First tick: immediate attempt (attempt #1), next allowed at +2s.
        sup.tick(t0, true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        assert_eq!(gauge(&m, "local"), Some(0));

        // Just before the 2s backoff elapses: no new attempt.
        sup.tick(t0 + Duration::from_secs(1), true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);

        // Exactly at the 2s boundary: attempt #2, next allowed at +4s.
        sup.tick(t0 + Duration::from_secs(2), true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 2);

        // Within the new 4s window: no attempt.
        sup.tick(t0 + Duration::from_secs(5), true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 2);

        // Past 2+4 = 6s: attempt #3.
        sup.tick(t0 + Duration::from_secs(6), true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 3);
    }

    #[test]
    fn backoff_boundary_is_inclusive() {
        // A retry must fire at exactly `next_attempt_at` (the `now < at`
        // guard, not `<=`). Pin the boundary so the comparison can't drift.
        let m = metrics();
        let mut sup = one(FakeSource::always_failing());
        let t0 = Instant::now();
        sup.tick(t0, true, &m);
        let next = sup.sources[0].next_attempt_at.expect("attempt armed");
        // One nanosecond before: still backing off.
        sup.tick(earlier(next, Duration::from_nanos(1)), true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        // Exactly at the boundary: fire.
        sup.tick(next, true, &m);
        assert_eq!(sup.sources[0].source.start_calls, 2);
    }

    #[test]
    fn recovery_after_failures_resets_backoff() {
        let m = metrics();
        // Fail twice, then succeed.
        let src = FakeSource {
            running: false,
            fail_starts: 2,
            start_calls: 0,
            stop_calls: 0,
        };
        let mut sup = one(src);
        let t0 = Instant::now();

        sup.tick(t0, true, &m); // attempt #1 fails
        assert_eq!(sup.sources[0].attempts_since_healthy, 1);
        sup.tick(t0 + Duration::from_secs(2), true, &m); // #2 fails
        assert_eq!(sup.sources[0].attempts_since_healthy, 2);
        sup.tick(t0 + Duration::from_secs(6), true, &m); // #3 succeeds
        assert!(sup.sources[0].source.running);

        // Confirmed running → counters reset.
        sup.tick(t0 + Duration::from_secs(7), true, &m);
        assert_eq!(sup.sources[0].attempts_since_healthy, 0);
        assert!(sup.sources[0].next_attempt_at.is_none());
        assert_eq!(gauge(&m, "local"), Some(1));
    }

    // ---- multiple sources are independent ---------------------------------

    #[test]
    fn sources_are_supervised_independently() {
        let m = metrics();
        let mut sup = Supervisor::new(vec![
            (FakeSource::healthy(), "local".to_owned()),
            (FakeSource::dead(), "RTSP_1".to_owned()),
        ]);
        sup.tick(Instant::now(), true, &m);
        // Healthy one untouched, dead one restarted.
        assert_eq!(sup.sources[0].source.start_calls, 0);
        assert_eq!(sup.sources[1].source.start_calls, 1);
        assert_eq!(gauge(&m, "local"), Some(1));
        assert_eq!(gauge(&m, "RTSP_1"), Some(0));
    }
}
