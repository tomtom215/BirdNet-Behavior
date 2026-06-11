//! Capture supervisor: keeps audio-capture subprocesses alive for the
//! lifetime of an unattended field deployment.
//!
//! # Why this exists
//!
//! `CaptureManager` knows how to *start* and *stop* a recording
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

use birdnet_core::audio::capture::{CaptureError, CaptureSource};
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

/// The slice of `CaptureManager` behaviour the supervisor depends on.
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
    /// Propagates whatever `CaptureManager::start` would return (tool
    /// missing, spawn failure, …).
    fn start(&mut self) -> Result<(), CaptureError>;

    /// Stop the subprocess.
    fn stop(&mut self);

    /// Age of the newest recording segment this source has written, or
    /// `None` when none is visible (never produced, or all purged).
    ///
    /// Drives silent-stall detection: an RTSP camera (or wedged `arecord`)
    /// whose process is alive but which has stopped delivering audio writes
    /// no new segments, and `is_running` alone can never notice that.
    fn latest_output_age(&mut self) -> Option<Duration>;
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
        // A lone local mic has no id and collapses to `local`; when several
        // local mics are configured each carries its own id (`MIC_1`, …) so the
        // per-source health gauge can tell them apart — matching the label
        // `derive_source_label` recovers from the recording filename.
        CaptureSource::Microphone { stream_id, .. } | CaptureSource::PipeWire { stream_id, .. } => {
            stream_id.clone().unwrap_or_else(|| "local".to_owned())
        }
    }
}

/// Backoff delay before the next restart attempt, given the number of
/// consecutive start attempts that have not yet produced a healthy process.
///
/// `0` attempts → no delay (the first attempt fires immediately); then
/// `2s, 4s, 8s, …` doubling up to [`BACKOFF_CAP`].
#[must_use]
fn backoff_delay(attempts_since_healthy: u32) -> Duration {
    if attempts_since_healthy == 0 {
        return Duration::ZERO;
    }
    // Cap the shift (so the doubling can't overflow) and then cap the delay.
    // `.min` is used rather than `if a > b { .. }` so each bound is observable
    // on its own: with the comparison form the shift-clamp boundary (21) and
    // the cap boundary coincide with their neighbours, making `>`→`>=` an
    // equivalent (unkillable) mutation.
    let shift = (attempts_since_healthy - 1).min(20);
    let secs = (BACKOFF_BASE.as_secs() << shift).min(BACKOFF_CAP.as_secs());
    Duration::from_secs(secs)
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

/// A per-source "quiet" window during which capture is paused, expressed in
/// minutes since midnight on the **same clock basis as the recording schedule**
/// (`schedule::civil_from_unix_secs`, i.e. UTC). The admin UI stores it as an
/// `HH:MM`–`HH:MM` pair; `super` parses that to minutes once, at construction,
/// so the supervisor only ever deals with already-validated integers.
///
/// A window where `start == end` is treated as empty (never quiet), matching
/// how clearing both fields reads to an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct QuietWindow {
    start_min: u32,
    end_min: u32,
}

impl QuietWindow {
    /// Build a window from start/end minutes-since-midnight (each `0..=1439`).
    pub(super) const fn new(start_min: u32, end_min: u32) -> Self {
        Self { start_min, end_min }
    }
}

/// Whether `now_min` (minutes since midnight) falls inside the quiet window
/// `[start, end)`, handling windows that wrap past midnight (e.g. 22:00–06:00).
///
/// The window is half-open: the start minute is quiet, the end minute is not,
/// so a 22:00–06:00 window pauses at 22:00 and resumes exactly at 06:00. An
/// empty window (`start == end`) is never quiet.
#[must_use]
fn in_quiet_window(now_min: u32, start_min: u32, end_min: u32) -> bool {
    use std::cmp::Ordering;
    match start_min.cmp(&end_min) {
        // Empty window — recording is never suppressed by it.
        Ordering::Equal => false,
        // Same-day window: quiet on [start, end).
        Ordering::Less => now_min >= start_min && now_min < end_min,
        // Wraps midnight: quiet on [start, 24:00) ∪ [00:00, end).
        Ordering::Greater => now_min >= start_min || now_min < end_min,
    }
}

/// One supervised capture source plus its restart bookkeeping.
struct SupervisedSource<S: Source> {
    source: S,
    label: String,
    /// Per-source quiet window, if the operator configured one. `None` means
    /// "no quiet window" — this source follows only the global schedule gate.
    quiet: Option<QuietWindow>,
    /// A running process that has written no segment for this long is
    /// declared silently stalled and restarted. Sized by the caller from the
    /// segment duration (several segments must be overdue before we act).
    /// `Duration::MAX` disables stall detection.
    stall_after: Duration,
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
    /// When the most recent (re)start attempt was issued. The from-birth
    /// stall reference: a source that NEVER writes a segment after starting
    /// has no output mtime to age-check, but is just as stalled.
    started_at: Option<Instant>,
    /// Monotonic watermark of "fresh output was observed at this instant".
    /// Folded from `latest_output_age` probes so a segment that is consumed
    /// or purged between ticks cannot retroactively un-prove liveness.
    last_fresh_output: Option<Instant>,
}

impl<S: Source> SupervisedSource<S> {
    const fn new(
        source: S,
        label: String,
        quiet: Option<QuietWindow>,
        stall_after: Duration,
    ) -> Self {
        Self {
            source,
            label,
            quiet,
            stall_after,
            attempts_since_healthy: 0,
            next_attempt_at: None,
            down_since: None,
            last_down_warn: None,
            started_at: None,
            last_fresh_output: None,
        }
    }

    /// Whether this source is currently inside its quiet window.
    ///
    /// `now_min` is `None` when the wall clock is untrusted (unsynced) — we do
    /// not enforce quiet windows then, mirroring the schedule's fail-open
    /// behaviour so a bogus boot-time date can't silence a source. A source
    /// with no configured window is never quiet.
    fn in_quiet(&self, now_min: Option<u32>) -> bool {
        match (self.quiet, now_min) {
            (Some(q), Some(min)) => in_quiet_window(min, q.start_min, q.end_min),
            _ => false,
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

    /// Whether the running process counts as silently stalled: alive, the
    /// schedule wants it recording, but no fresh segment for `stall_after`.
    ///
    /// Fails open (never stalled) when:
    /// * the wall clock is untrusted (`clock_trusted == false`) — segment
    ///   mtimes are wall-clock stamps, so ages computed before NTP sync are
    ///   meaningless and could condemn a healthy source;
    /// * there is no reference point at all (the supervisor has not started
    ///   this process and has never seen output — nothing to age against).
    fn stalled(&mut self, now: Instant, clock_trusted: bool) -> bool {
        if !clock_trusted {
            return false;
        }
        let probe_fresh = self
            .source
            .latest_output_age()
            .is_some_and(|age| age < self.stall_after);
        if probe_fresh {
            self.last_fresh_output = Some(now);
            return false;
        }
        let reference = match (self.last_fresh_output, self.started_at) {
            (Some(output), Some(started)) => Some(output.max(started)),
            (output, started) => output.or(started),
        };
        reference.is_some_and(|r| now.saturating_duration_since(r) >= self.stall_after)
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
    ///
    /// `recording_allowed` is the global schedule gate (solar / fixed window);
    /// `now_min` is the current minute-of-day (UTC) used to evaluate this
    /// source's own quiet window, or `None` when the clock is untrusted. The
    /// source should record only when the global schedule allows it **and** it
    /// is not inside its quiet window.
    fn reconcile(
        &mut self,
        now: Instant,
        recording_allowed: bool,
        now_min: Option<u32>,
        metrics: &SharedMetrics,
    ) {
        let allowed = recording_allowed && !self.in_quiet(now_min);

        // Outside the schedule (or inside this source's quiet window): ensure
        // stopped, clear fault state, gauge 0.
        if !allowed {
            if self.source.is_running() {
                self.source.stop();
                tracing::info!(
                    source = %self.label,
                    "recording paused (outside schedule or in quiet window)"
                );
            }
            self.clear_fault();
            metrics.set_source_up(&self.label, false);
            return;
        }

        // Desired ON and the process is alive — but only count it healthy if
        // it is actually producing output. A silently stalled process (RTSP
        // session wedged, `arecord` hung on a vanished USB device) is stopped
        // here and falls through to the shared down/backoff/restart path, so
        // stall recovery degrades gracefully per source exactly like death.
        if self.source.is_running() {
            if self.stalled(now, now_min.is_some()) {
                tracing::warn!(
                    source = %self.label,
                    no_output_for_secs = self.stall_after.as_secs(),
                    "audio source STALLED — process alive but writing no segments; restarting it"
                );
                self.source.stop();
            } else {
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
        // The stall clock measures from the newest of {fresh output, this
        // attempt}: a process that starts and then never writes a segment is
        // stalled `stall_after` from HERE, not from a stale output watermark.
        self.started_at = Some(now);
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
    /// Build a supervisor over `(source, gauge_label, quiet_window,
    /// stall_after)` tuples. `stall_after` is the per-source silent-stall
    /// threshold (`Duration::MAX` disables stall detection).
    pub(super) fn new(sources: Vec<(S, String, Option<QuietWindow>, Duration)>) -> Self {
        Self {
            sources: sources
                .into_iter()
                .map(|(source, label, quiet, stall_after)| {
                    SupervisedSource::new(source, label, quiet, stall_after)
                })
                .collect(),
        }
    }

    /// Run one reconciliation pass over every source.
    ///
    /// `now` is the monotonic clock used for backoff/down timing;
    /// `recording_allowed` is the global schedule gate, and `now_min` is the
    /// current minute-of-day (UTC) used per source for its quiet window — both
    /// evaluated against the wall clock by the caller. Splitting the monotonic
    /// and wall clocks keeps this method deterministically testable.
    pub(super) fn tick(
        &mut self,
        now: Instant,
        recording_allowed: bool,
        now_min: Option<u32>,
        metrics: &SharedMetrics,
    ) {
        for source in &mut self.sources {
            source.reconcile(now, recording_allowed, now_min, metrics);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_core::audio::capture::RtspTransport;
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
        /// What `latest_output_age` reports; tests steer stall detection.
        output_age: Option<Duration>,
    }

    impl FakeSource {
        fn healthy() -> Self {
            Self {
                running: true,
                fail_starts: 0,
                start_calls: 0,
                stop_calls: 0,
                output_age: None,
            }
        }

        fn dead() -> Self {
            Self {
                running: false,
                fail_starts: 0,
                start_calls: 0,
                stop_calls: 0,
                output_age: None,
            }
        }

        /// Dead, and every `start` fails forever (simulates a broken source).
        fn always_failing() -> Self {
            Self {
                running: false,
                fail_starts: u32::MAX,
                start_calls: 0,
                stop_calls: 0,
                output_age: None,
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

        fn latest_output_age(&mut self) -> Option<Duration> {
            self.output_age
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
        Supervisor::new(vec![(source, "local".to_owned(), None, Duration::MAX)])
    }

    /// A supervisor over one source that carries a quiet window.
    fn one_with_quiet(source: FakeSource, quiet: QuietWindow) -> Supervisor<FakeSource> {
        Supervisor::new(vec![(
            source,
            "local".to_owned(),
            Some(quiet),
            Duration::MAX,
        )])
    }

    // ---- source_gauge_label ------------------------------------------------

    #[test]
    fn label_microphone_is_local() {
        let src = CaptureSource::Microphone {
            device: "plughw:1,0".into(),
            sample_rate: 48_000,
            channels: 1,
            stream_id: None,
        };
        assert_eq!(source_gauge_label(&src), "local");
    }

    #[test]
    fn label_pipewire_is_local() {
        let src = CaptureSource::PipeWire {
            device: String::new(),
            sample_rate: 48_000,
            channels: 1,
            stream_id: None,
        };
        assert_eq!(source_gauge_label(&src), "local");
    }

    #[test]
    fn label_microphone_with_id_uses_id() {
        // With several local mics each gets its own id so the health gauge can
        // distinguish them (round-trips with `derive_source_label`).
        let src = CaptureSource::Microphone {
            device: "plughw:2,0".into(),
            sample_rate: 48_000,
            channels: 1,
            stream_id: Some("MIC_2".into()),
        };
        assert_eq!(source_gauge_label(&src), "MIC_2");
    }

    #[test]
    fn label_rtsp_is_stream_id() {
        let src = CaptureSource::Rtsp {
            url: "rtsp://cam.local/s".into(),
            stream_id: "RTSP_2".into(),
            transport: RtspTransport::Auto,
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
        sup.tick(Instant::now(), false, None, &m);
        assert_eq!(sup.sources[0].source.stop_calls, 1);
        assert!(!sup.sources[0].source.running);
        assert_eq!(gauge(&m, "local"), Some(0));
    }

    #[test]
    fn schedule_closed_leaves_stopped_source_alone() {
        let m = metrics();
        let mut sup = one(FakeSource::dead());
        sup.tick(Instant::now(), false, None, &m);
        assert_eq!(sup.sources[0].source.start_calls, 0);
        assert_eq!(sup.sources[0].source.stop_calls, 0);
        assert_eq!(gauge(&m, "local"), Some(0));
    }

    // ---- reconcile: healthy path ------------------------------------------

    #[test]
    fn healthy_source_not_touched_and_gauge_up() {
        let m = metrics();
        let mut sup = one(FakeSource::healthy());
        sup.tick(Instant::now(), true, None, &m);
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
        sup.tick(t0, true, None, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        assert!(sup.sources[0].source.running);
        assert_eq!(gauge(&m, "local"), Some(0));
        assert!(sup.sources[0].down_since.is_some());

        // Tick 2: observed running → gauge up, fault state cleared.
        sup.tick(t0 + Duration::from_secs(1), true, None, &m);
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

        sup.tick(t0, true, None, &m);
        assert_eq!(gauge(&m, "local"), Some(1));

        // Fault injection: the subprocess dies.
        sup.sources[0].source.running = false;

        // Next tick notices and issues a restart.
        sup.tick(t0 + Duration::from_secs(1), true, None, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        assert!(sup.sources[0].source.running);

        // And it is confirmed back up.
        sup.tick(t0 + Duration::from_secs(2), true, None, &m);
        assert_eq!(gauge(&m, "local"), Some(1));
    }

    // ---- reconcile: exponential backoff on a broken source ----------------

    #[test]
    fn broken_source_backs_off_exponentially() {
        let m = metrics();
        let mut sup = one(FakeSource::always_failing());
        let t0 = Instant::now();

        // First tick: immediate attempt (attempt #1), next allowed at +2s.
        sup.tick(t0, true, None, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        assert_eq!(gauge(&m, "local"), Some(0));

        // Just before the 2s backoff elapses: no new attempt.
        sup.tick(t0 + Duration::from_secs(1), true, None, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);

        // Exactly at the 2s boundary: attempt #2, next allowed at +4s.
        sup.tick(t0 + Duration::from_secs(2), true, None, &m);
        assert_eq!(sup.sources[0].source.start_calls, 2);

        // Within the new 4s window: no attempt.
        sup.tick(t0 + Duration::from_secs(5), true, None, &m);
        assert_eq!(sup.sources[0].source.start_calls, 2);

        // Past 2+4 = 6s: attempt #3.
        sup.tick(t0 + Duration::from_secs(6), true, None, &m);
        assert_eq!(sup.sources[0].source.start_calls, 3);
    }

    #[test]
    fn backoff_boundary_is_inclusive() {
        // A retry must fire at exactly `next_attempt_at` (the `now < at`
        // guard, not `<=`). Pin the boundary so the comparison can't drift.
        let m = metrics();
        let mut sup = one(FakeSource::always_failing());
        let t0 = Instant::now();
        sup.tick(t0, true, None, &m);
        let next = sup.sources[0].next_attempt_at.expect("attempt armed");
        // One nanosecond before: still backing off.
        sup.tick(earlier(next, Duration::from_nanos(1)), true, None, &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        // Exactly at the boundary: fire.
        sup.tick(next, true, None, &m);
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
            output_age: None,
        };
        let mut sup = one(src);
        let t0 = Instant::now();

        sup.tick(t0, true, None, &m); // attempt #1 fails
        assert_eq!(sup.sources[0].attempts_since_healthy, 1);
        sup.tick(t0 + Duration::from_secs(2), true, None, &m); // #2 fails
        assert_eq!(sup.sources[0].attempts_since_healthy, 2);
        sup.tick(t0 + Duration::from_secs(6), true, None, &m); // #3 succeeds
        assert!(sup.sources[0].source.running);

        // Confirmed running → counters reset.
        sup.tick(t0 + Duration::from_secs(7), true, None, &m);
        assert_eq!(sup.sources[0].attempts_since_healthy, 0);
        assert!(sup.sources[0].next_attempt_at.is_none());
        assert_eq!(gauge(&m, "local"), Some(1));
    }

    // ---- multiple sources are independent ---------------------------------

    #[test]
    fn sources_are_supervised_independently() {
        let m = metrics();
        let mut sup = Supervisor::new(vec![
            (
                FakeSource::healthy(),
                "local".to_owned(),
                None,
                Duration::MAX,
            ),
            (FakeSource::dead(), "RTSP_1".to_owned(), None, Duration::MAX),
        ]);
        sup.tick(Instant::now(), true, None, &m);
        // Healthy one untouched, dead one restarted.
        assert_eq!(sup.sources[0].source.start_calls, 0);
        assert_eq!(sup.sources[1].source.start_calls, 1);
        assert_eq!(gauge(&m, "local"), Some(1));
        assert_eq!(gauge(&m, "RTSP_1"), Some(0));
    }

    // ---- "source down for N min" warning ----------------------------------

    #[test]
    fn long_down_source_records_warning() {
        // Exercises `maybe_warn_down`: a source that stays down past the warn
        // threshold must record `last_down_warn` (the observable effect of the
        // otherwise log-only warning).
        let m = metrics();
        let mut sup = one(FakeSource::always_failing());
        let t0 = Instant::now();

        sup.tick(t0, true, None, &m);
        assert!(
            sup.sources[0].last_down_warn.is_none(),
            "no warning before the threshold elapses"
        );

        // Still down well past the warn threshold.
        sup.tick(
            t0 + DOWN_WARN_AFTER + Duration::from_secs(1),
            true,
            None,
            &m,
        );
        assert!(
            sup.sources[0].last_down_warn.is_some(),
            "a source down past the threshold must record a warning"
        );
    }

    // ---- in_quiet_window (pure) -------------------------------------------

    #[test]
    fn empty_quiet_window_is_never_quiet() {
        // start == end → empty; no minute is inside it.
        assert!(!in_quiet_window(0, 360, 360));
        assert!(!in_quiet_window(360, 360, 360));
        assert!(!in_quiet_window(1439, 360, 360));
    }

    #[test]
    fn same_day_quiet_window_is_half_open() {
        // 06:00–12:00 → quiet on [360, 720).
        assert!(!in_quiet_window(359, 360, 720)); // 05:59 — before start
        assert!(in_quiet_window(360, 360, 720)); // 06:00 — start is inclusive
        assert!(in_quiet_window(719, 360, 720)); // 11:59 — inside
        assert!(!in_quiet_window(720, 360, 720)); // 12:00 — end is exclusive
        assert!(!in_quiet_window(721, 360, 720)); // 12:01 — after end
    }

    #[test]
    fn wraparound_quiet_window_spans_midnight() {
        // 22:00–06:00 → quiet on [1320, 24:00) ∪ [00:00, 360).
        assert!(!in_quiet_window(1319, 1320, 360)); // 21:59 — before start
        assert!(in_quiet_window(1320, 1320, 360)); // 22:00 — start inclusive
        assert!(in_quiet_window(0, 1320, 360)); // 00:00 — across midnight
        assert!(in_quiet_window(359, 1320, 360)); // 05:59 — inside
        assert!(!in_quiet_window(360, 1320, 360)); // 06:00 — end exclusive
        assert!(!in_quiet_window(720, 1320, 360)); // 12:00 — outside both legs
    }

    // ---- SupervisedSource::in_quiet (method) ------------------------------

    #[test]
    fn in_quiet_requires_both_a_window_and_a_trusted_clock() {
        let inside =
            SupervisedSource::new(FakeSource::healthy(), "s".to_owned(), None, Duration::MAX);
        // No window configured → never quiet, regardless of the clock.
        assert!(!inside.in_quiet(Some(400)));

        let win = SupervisedSource::new(
            FakeSource::healthy(),
            "s".to_owned(),
            Some(QuietWindow::new(360, 720)),
            Duration::MAX,
        );
        // Window set + clock untrusted (None) → not enforced (fail open).
        assert!(!win.in_quiet(None));
        // Window set + clock inside the window → quiet.
        assert!(win.in_quiet(Some(400)));
        // Window set + clock outside the window → not quiet.
        assert!(!win.in_quiet(Some(800)));
    }

    // ---- reconcile: per-source quiet window -------------------------------

    #[test]
    fn quiet_window_pauses_a_running_source_then_resumes_it() {
        let m = metrics();
        // 06:00–12:00 quiet window.
        let mut sup = one_with_quiet(FakeSource::healthy(), QuietWindow::new(360, 720));
        let t0 = Instant::now();

        // Inside the window, even though the global schedule allows recording:
        // the source must be stopped and the gauge driven to 0.
        sup.tick(t0, true, Some(400), &m);
        assert_eq!(sup.sources[0].source.stop_calls, 1);
        assert!(!sup.sources[0].source.running);
        assert_eq!(gauge(&m, "local"), Some(0));

        // Outside the window: the source is desired ON again, so a (re)start
        // attempt fires.
        sup.tick(t0 + Duration::from_secs(1), true, Some(800), &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        assert!(sup.sources[0].source.running);
    }

    #[test]
    fn quiet_window_not_enforced_when_clock_untrusted() {
        let m = metrics();
        // The window covers "now", but the clock is unsynced (now_min = None),
        // so we fail open and keep recording rather than trust a bogus time.
        let mut sup = one_with_quiet(FakeSource::healthy(), QuietWindow::new(360, 720));
        sup.tick(Instant::now(), true, None, &m);
        assert_eq!(sup.sources[0].source.stop_calls, 0);
        assert!(sup.sources[0].source.running);
        assert_eq!(gauge(&m, "local"), Some(1));
    }

    // ---- silent-stall detection --------------------------------------------

    /// A process that is alive but writes no segments must be stopped and
    /// fall into the normal restart path (the field failure `is_running`
    /// can never see: wedged RTSP session, hung `arecord`).
    #[test]
    fn stalled_source_is_stopped_and_restarted() {
        let m = metrics();
        let stall_after = Duration::from_secs(60);
        let mut sup = Supervisor::new(vec![(
            FakeSource::dead(),
            "RTSP_1".to_owned(),
            None,
            stall_after,
        )]);
        let t0 = Instant::now();

        // Tick 1: dead → restart issued (arms the from-birth stall clock).
        sup.tick(t0, true, Some(400), &m);
        assert_eq!(sup.sources[0].source.start_calls, 1);
        assert!(sup.sources[0].source.running);

        // Healthy-looking but mute: no output ever appears. Before the stall
        // threshold the source is left alone…
        sup.tick(t0 + Duration::from_secs(30), true, Some(400), &m);
        assert_eq!(sup.sources[0].source.stop_calls, 0);
        assert_eq!(gauge(&m, "RTSP_1"), Some(1));

        // …at the threshold it is declared stalled: stopped, marked down,
        // and (the backoff window having long expired) restarted in the same
        // tick. Repeat stalls therefore cost one respawn per `stall_after`,
        // never a hot loop.
        sup.tick(t0 + stall_after, true, Some(401), &m);
        assert_eq!(sup.sources[0].source.stop_calls, 1, "stall resets process");
        assert_eq!(sup.sources[0].source.start_calls, 2, "immediate respawn");
        assert_eq!(gauge(&m, "RTSP_1"), Some(0), "down until output reappears");

        // The respawn re-arms the from-birth stall clock: within the new
        // window the source counts healthy again (gauge recovers)…
        sup.tick(
            t0 + stall_after + Duration::from_secs(2),
            true,
            Some(401),
            &m,
        );
        assert_eq!(gauge(&m, "RTSP_1"), Some(1));
        assert_eq!(sup.sources[0].source.stop_calls, 1);

        // …and a still-mute process is reset again exactly one stall window
        // after the respawn.
        sup.tick(t0 + stall_after * 2, true, Some(402), &m);
        assert_eq!(sup.sources[0].source.stop_calls, 2);
        assert_eq!(sup.sources[0].source.start_calls, 3);
    }

    /// The probe boundary is exclusive: a newest segment EXACTLY
    /// `stall_after` old is already stale — "no fresh segment for
    /// `stall_after`" includes the boundary — so the source is reset on
    /// that tick. Pins `age < stall_after` against the `<=` mutation,
    /// which would count boundary-age output as fresh and skip the reset.
    #[test]
    fn stall_probe_boundary_age_is_stale() {
        let m = metrics();
        let stall_after = Duration::from_secs(60);
        let mut src = FakeSource::dead();
        src.output_age = Some(stall_after); // exactly at the boundary
        let mut sup = Supervisor::new(vec![(src, "RTSP_1".to_owned(), None, stall_after)]);
        let t0 = Instant::now();

        sup.tick(t0, true, Some(400), &m); // start issued; stall clock armed
        sup.tick(t0 + stall_after, true, Some(401), &m);
        assert_eq!(
            sup.sources[0].source.stop_calls, 1,
            "boundary-age output must not count as fresh"
        );
    }

    /// Fresh segments keep arriving → never stalled, however much wall time
    /// passes between ticks.
    #[test]
    fn fresh_output_prevents_stall() {
        let m = metrics();
        let stall_after = Duration::from_secs(60);
        let mut src = FakeSource::dead();
        src.output_age = Some(Duration::from_secs(5));
        let mut sup = Supervisor::new(vec![(src, "local".to_owned(), None, stall_after)]);
        let t0 = Instant::now();

        sup.tick(t0, true, Some(400), &m);
        for minutes in 1..=10_u64 {
            sup.tick(t0 + Duration::from_secs(minutes * 60), true, Some(400), &m);
        }
        assert_eq!(sup.sources[0].source.stop_calls, 0, "never stall-stopped");
        assert_eq!(gauge(&m, "local"), Some(1));
    }

    /// THE multi-stream guarantee: one stream silently stalling is recovered
    /// in isolation — the other streams are not touched, their gauges stay
    /// up, and processing continues.
    #[test]
    fn stalled_stream_does_not_disturb_healthy_streams() {
        let m = metrics();
        let stall_after = Duration::from_secs(60);
        let mut mute = FakeSource::dead(); // produces no output after start
        mute.output_age = None;
        let mut chatty = FakeSource::healthy();
        chatty.output_age = Some(Duration::from_secs(3));
        let mut sup = Supervisor::new(vec![
            (mute, "RTSP_1".to_owned(), None, stall_after),
            (chatty, "RTSP_2".to_owned(), None, stall_after),
        ]);
        let t0 = Instant::now();

        sup.tick(t0, true, Some(400), &m); // RTSP_1 started; RTSP_2 healthy
        sup.tick(t0 + stall_after, true, Some(401), &m); // RTSP_1 stalls

        assert_eq!(sup.sources[0].source.stop_calls, 1, "stalled stream reset");
        assert_eq!(
            sup.sources[1].source.stop_calls, 0,
            "healthy stream untouched by its neighbour's stall"
        );
        assert_eq!(sup.sources[1].source.start_calls, 0);
        assert_eq!(gauge(&m, "RTSP_2"), Some(1));
        assert_eq!(gauge(&m, "RTSP_1"), Some(0));
    }

    /// Stall detection fails open while the wall clock is untrusted: mtime
    /// ages are meaningless before NTP sync, so a mute-looking source is
    /// left recording rather than restart-looped.
    #[test]
    fn stall_detection_fails_open_when_clock_untrusted() {
        let m = metrics();
        let stall_after = Duration::from_secs(60);
        let mut sup = Supervisor::new(vec![(
            FakeSource::dead(),
            "local".to_owned(),
            None,
            stall_after,
        )]);
        let t0 = Instant::now();

        sup.tick(t0, true, None, &m); // started; clock untrusted
        sup.tick(t0 + stall_after * 3, true, None, &m);
        assert_eq!(
            sup.sources[0].source.stop_calls, 0,
            "no stall verdict without a trusted clock"
        );
        assert_eq!(gauge(&m, "local"), Some(1));
    }

    /// `Duration::MAX` disables stall detection entirely (the configuration
    /// used by sources whose cadence the caller cannot bound).
    #[test]
    fn stall_detection_disabled_with_max_threshold() {
        let m = metrics();
        let mut sup = one(FakeSource::dead());
        let t0 = Instant::now();
        sup.tick(t0, true, Some(400), &m);
        sup.tick(t0 + Duration::from_secs(86_400), true, Some(400), &m);
        assert_eq!(sup.sources[0].source.stop_calls, 0);
        assert_eq!(gauge(&m, "local"), Some(1));
    }

    #[test]
    fn quiet_window_outside_does_not_pause() {
        let m = metrics();
        // now_min is outside the 06:00–12:00 window, so a healthy source is
        // left running — this pins the `!in_quiet` term (a mutant dropping it
        // would still pass, but the pause test above would then fail).
        let mut sup = one_with_quiet(FakeSource::healthy(), QuietWindow::new(360, 720));
        sup.tick(Instant::now(), true, Some(720), &m);
        assert_eq!(sup.sources[0].source.stop_calls, 0);
        assert!(sup.sources[0].source.running);
        assert_eq!(gauge(&m, "local"), Some(1));
    }
}
