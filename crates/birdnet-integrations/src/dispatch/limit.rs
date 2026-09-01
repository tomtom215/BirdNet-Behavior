//! Two guards on outbound notifications: a circuit breaker and a rate limit.
//!
//! # Why
//!
//! A station notifies per detection. During a dawn chorus that is a lot of
//! detections, and the two failure modes are opposite:
//!
//! * **A destination that is down.** A retired Discord webhook answers 404 to
//!   every request forever. Without a breaker the station spends three attempts
//!   with backoff on each one, every detection, all day — tens of thousands of
//!   pointless requests, and the retries are what get an IP rate-limited or
//!   banned rather than the sends.
//! * **A destination that is up.** Discord allows 5 requests/second per
//!   webhook, Telegram ~30/second, and Pushover 10 000 messages a *month*. A
//!   station with fifty species active can exceed the last one in a fortnight.
//!
//! # Testability
//!
//! Both take `now: Instant` as a parameter rather than reading the clock, so
//! every transition is exercised in a unit test without sleeping. The same
//! reason [`crate::retry::backoff_delay`] takes its jitter as an argument.

use std::time::{Duration, Instant};

/// Consecutive failures before a destination is considered down.
///
/// Two rather than one: a single timeout is ordinary on a domestic uplink, and
/// opening on it would suppress the next notification for no reason.
const TRIP_AFTER: u32 = 3;

/// How long the circuit stays open on the first trip.
const OPEN_BASE: Duration = Duration::from_secs(60);

/// Ceiling on the open period, so a destination that comes back is retried
/// within a bounded time rather than hours later.
const OPEN_CAP: Duration = Duration::from_secs(30 * 60);

/// What the breaker says about a send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Send normally.
    Send,
    /// The circuit is open but the cooldown has elapsed: send one probe. A
    /// failure re-opens it for longer; a success closes it.
    Probe,
    /// The circuit is open. Skip, and try again after this long.
    Skip(Duration),
}

/// Per-destination circuit breaker.
///
/// Closed until [`TRIP_AFTER`] consecutive failures, then open for a period
/// that doubles with each further trip up to [`OPEN_CAP`]. One probe is
/// admitted each time the open period elapses.
#[derive(Debug, Clone)]
pub struct Breaker {
    /// Consecutive failures since the last success.
    failures: u32,
    /// How many times the circuit has opened without an intervening success.
    trips: u32,
    /// When the current open period ends, if the circuit is open.
    open_until: Option<Instant>,
    /// Whether a probe has been handed out for the current open period.
    probe_issued: bool,
}

impl Default for Breaker {
    fn default() -> Self {
        Self::new()
    }
}

impl Breaker {
    /// A closed breaker.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            failures: 0,
            trips: 0,
            open_until: None,
            probe_issued: false,
        }
    }

    /// Whether a send should be attempted now.
    ///
    /// Takes `&mut self` because handing out a probe is a state change: only
    /// one is admitted per open period, so a burst of detections while a
    /// destination is down produces one request, not a burst of them.
    pub fn check(&mut self, now: Instant) -> Verdict {
        let Some(until) = self.open_until else {
            return Verdict::Send;
        };
        if now < until {
            return Verdict::Skip(until.saturating_duration_since(now));
        }
        if self.probe_issued {
            // The probe is still outstanding. Hold everything else back until
            // it reports, rather than letting the queue through behind it.
            return Verdict::Skip(Duration::ZERO);
        }
        self.probe_issued = true;
        Verdict::Probe
    }

    /// Record a delivery. Closes the circuit and forgets the trip history.
    pub const fn on_success(&mut self) {
        self.failures = 0;
        self.trips = 0;
        self.open_until = None;
        self.probe_issued = false;
    }

    /// Record a failure. Opens or re-opens the circuit once the threshold is
    /// reached, for a period that doubles per trip up to [`OPEN_CAP`].
    pub fn on_failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        // A failed probe counts as a fresh trip: the destination is still down.
        if self.probe_issued || self.failures >= TRIP_AFTER {
            self.trips = self.trips.saturating_add(1);
            self.failures = 0;
            self.probe_issued = false;
            self.open_until = Some(now + self.open_period());
        }
    }

    /// How long the circuit stays open for the current trip count.
    fn open_period(&self) -> Duration {
        // `trips` is 1 on the first trip, so shift by `trips - 1`.
        let shift = self.trips.saturating_sub(1).min(16);
        OPEN_BASE.saturating_mul(1_u32 << shift).min(OPEN_CAP)
    }

    /// Whether the attempt just admitted was the open period's probe.
    #[must_use]
    pub const fn is_probing(&self) -> bool {
        self.probe_issued
    }

    /// Whether the circuit is currently open, for reporting.
    #[must_use]
    pub fn is_open(&self, now: Instant) -> bool {
        self.open_until.is_some_and(|until| now < until)
    }
}

/// Token bucket bounding how fast one destination is written to.
///
/// Capacity is the burst a quiet station may spend at once — a dawn chorus
/// arriving all at the same minute should not be throttled — and the refill
/// rate is the sustained ceiling.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    /// Maximum tokens held.
    capacity: f64,
    /// Tokens added per second.
    per_second: f64,
    /// Tokens available.
    tokens: f64,
    /// When `tokens` was last brought up to date.
    last: Instant,
}

impl RateLimiter {
    /// A limiter allowing `per_minute` sustained sends, bursting to `per_minute`.
    ///
    /// `per_minute == 0` disables the limit; [`Self::try_take`] then always
    /// succeeds.
    #[must_use]
    pub fn per_minute(per_minute: u32, now: Instant) -> Self {
        let capacity = f64::from(per_minute);
        Self {
            capacity,
            per_second: capacity / 60.0,
            tokens: capacity,
            last: now,
        }
    }

    /// Whether this send is within the limit, spending a token if so.
    pub fn try_take(&mut self, now: Instant) -> bool {
        if self.capacity <= 0.0 {
            return true; // disabled
        }
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = elapsed
            .mul_add(self.per_second, self.tokens)
            .min(self.capacity);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Why a send was or was not admitted to a destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Send normally.
    Send,
    /// The destination is thought to be down; this is the one probe admitted
    /// for the current open period. It is exempt from the rate limit.
    Probe,
    /// The circuit is open. Try again after this long.
    CircuitOpen(Duration),
    /// The destination is up but over its rate limit.
    RateLimited,
}

/// The two guards for one destination, and the rules for how they interact.
///
/// Kept together, and given `now` explicitly, because the interesting cases
/// are in the *interaction* and only reachable minutes apart in real time:
/// a probe must not be spent on the rate limit, and a rate-limited send must
/// not count as a destination failure. Neither is testable through a client
/// that reads the clock itself.
#[derive(Debug)]
pub struct Gate {
    /// Suppresses a destination that is down.
    breaker: Breaker,
    /// Bounds how fast a destination that is up is written to.
    limiter: RateLimiter,
}

impl Gate {
    /// A gate allowing `rate_per_minute` sustained sends; `0` disables the
    /// rate limit but keeps the breaker.
    #[must_use]
    pub fn new(rate_per_minute: u32, now: Instant) -> Self {
        Self {
            breaker: Breaker::new(),
            limiter: RateLimiter::per_minute(rate_per_minute, now),
        }
    }

    /// Decide whether to send now, spending a rate-limit token if one is used.
    ///
    /// The probe is *not* exempt from the rate limit, and does not need to be:
    /// the shortest open period ([`OPEN_BASE`]) is at least as long as the time
    /// the bucket takes to refill one token at the slowest configurable rate,
    /// so a probe always finds a token waiting. An earlier version had an
    /// exemption here; it was unreachable, and no mutation of it could be made
    /// to fail a test. `the_bucket_always_refills_before_a_probe_is_due` pins
    /// the relationship the exemption's absence depends on.
    pub fn admit(&mut self, now: Instant) -> Admission {
        match self.breaker.check(now) {
            Verdict::Skip(wait) => Admission::CircuitOpen(wait),
            Verdict::Probe | Verdict::Send => {
                if self.limiter.try_take(now) {
                    if self.breaker.is_probing() {
                        Admission::Probe
                    } else {
                        Admission::Send
                    }
                } else {
                    Admission::RateLimited
                }
            }
        }
    }

    /// Record a delivery.
    pub const fn on_success(&mut self) {
        self.breaker.on_success();
    }

    /// Record a failed delivery. Only call this for a send that was actually
    /// attempted: a rate-limited send is not evidence about the destination.
    pub fn on_failure(&mut self, now: Instant) {
        self.breaker.on_failure(now);
    }
}

#[cfg(test)]
mod tests {
    use super::{Admission, Breaker, Gate, OPEN_BASE, OPEN_CAP, RateLimiter, TRIP_AFTER, Verdict};
    use std::time::{Duration, Instant};

    /// A fixed origin, so every test is arithmetic rather than wall-clock.
    fn t0() -> Instant {
        Instant::now()
    }

    // ── circuit breaker ─────────────────────────────────────────────────

    #[test]
    fn a_healthy_destination_is_never_held_back() {
        let mut b = Breaker::new();
        let now = t0();
        for i in 0..1_000 {
            assert_eq!(b.check(now + Duration::from_secs(i)), Verdict::Send);
            b.on_success();
        }
    }

    #[test]
    fn two_consecutive_failures_do_not_open_the_circuit() {
        // Counterpart to the gate below, and the reason the threshold is not
        // one: a single timeout on a domestic uplink is ordinary, and
        // suppressing the next notification for a minute because of it would
        // be worse than the retry.
        //
        // The counts here are literal, not `TRIP_AFTER`. Written in terms of
        // the constant, both halves of this pair survive any value of it: the
        // loop bounds move with the mutation and the assertions still hold.
        assert_eq!(TRIP_AFTER, 3, "the counts below assume this threshold");

        let mut b = Breaker::new();
        let now = t0();
        b.on_failure(now);
        b.on_failure(now);
        assert_eq!(b.check(now), Verdict::Send);
        assert!(!b.is_open(now));
    }

    #[test]
    fn the_third_consecutive_failure_opens_the_circuit() {
        let mut b = Breaker::new();
        let now = t0();
        b.on_failure(now);
        b.on_failure(now);
        b.on_failure(now);
        assert!(b.is_open(now));
        assert!(matches!(b.check(now), Verdict::Skip(_)));
    }

    #[test]
    fn a_success_before_the_threshold_forgets_the_failures() {
        // Intermittent failures must not accumulate into a trip over hours.
        let mut b = Breaker::new();
        let now = t0();
        for _ in 0..10 {
            b.on_failure(now);
            b.on_failure(now);
            b.on_success();
        }
        assert_eq!(b.check(now), Verdict::Send);
    }

    #[test]
    fn only_one_probe_is_admitted_per_open_period() {
        // The point of the breaker: a burst of detections while a destination
        // is down produces one request, not one per detection.
        let mut b = Breaker::new();
        let now = t0();
        for _ in 0..TRIP_AFTER {
            b.on_failure(now);
        }
        let later = now + Duration::from_secs(61);
        assert_eq!(b.check(later), Verdict::Probe);
        for i in 0..100 {
            assert_eq!(
                b.check(later + Duration::from_millis(i)),
                Verdict::Skip(Duration::ZERO),
                "a second probe was admitted"
            );
        }
    }

    #[test]
    fn a_successful_probe_closes_the_circuit() {
        let mut b = Breaker::new();
        let now = t0();
        for _ in 0..TRIP_AFTER {
            b.on_failure(now);
        }
        let later = now + Duration::from_secs(61);
        assert_eq!(b.check(later), Verdict::Probe);
        b.on_success();
        assert_eq!(b.check(later), Verdict::Send);
        assert!(!b.is_open(later));
    }

    #[test]
    fn a_failed_probe_reopens_the_circuit_for_longer() {
        // A retired webhook answers 404 forever. Each failed probe must cost
        // more than the last, or the station settles into a fixed poll of a
        // dead endpoint.
        let mut b = Breaker::new();
        let mut now = t0();
        for _ in 0..TRIP_AFTER {
            b.on_failure(now);
        }

        let mut previous = Duration::ZERO;
        for _ in 0..4 {
            now += Duration::from_secs(60 * 60); // well past any open period
            assert_eq!(b.check(now), Verdict::Probe);
            b.on_failure(now);
            let Verdict::Skip(wait) = b.check(now) else {
                panic!("a failed probe must re-open the circuit");
            };
            assert!(
                wait > previous,
                "open period did not grow: {wait:?} after {previous:?}"
            );
            previous = wait;
        }
    }

    #[test]
    fn the_open_period_is_capped() {
        // Counterpart to the growth gate: unbounded doubling would leave a
        // destination that came back untried for days.
        let mut b = Breaker::new();
        let mut now = t0();
        for _ in 0..TRIP_AFTER {
            b.on_failure(now);
        }
        for _ in 0..40 {
            now += OPEN_CAP + Duration::from_secs(1);
            assert_eq!(b.check(now), Verdict::Probe);
            b.on_failure(now);
        }
        let Verdict::Skip(wait) = b.check(now) else {
            panic!("still expected to be open");
        };
        assert!(wait <= OPEN_CAP, "{wait:?} exceeds the cap");
    }

    // ── rate limiter ────────────────────────────────────────────────────

    #[test]
    fn a_burst_within_the_budget_is_allowed_in_full() {
        // A dawn chorus arriving in one minute must not be throttled.
        let now = t0();
        let mut r = RateLimiter::per_minute(12, now);
        for i in 0..12 {
            assert!(r.try_take(now), "token {i} refused inside the burst");
        }
    }

    #[test]
    fn the_burst_after_the_budget_is_refused() {
        // Counterpart: without this, a limiter that always returned true would
        // satisfy the gate above.
        let now = t0();
        let mut r = RateLimiter::per_minute(12, now);
        for _ in 0..12 {
            assert!(r.try_take(now));
        }
        assert!(!r.try_take(now), "the 13th send in one instant was allowed");
    }

    #[test]
    fn tokens_come_back_at_the_configured_rate() {
        let now = t0();
        let mut r = RateLimiter::per_minute(12, now);
        for _ in 0..12 {
            assert!(r.try_take(now));
        }
        // 12/minute is one every five seconds.
        assert!(!r.try_take(now + Duration::from_secs(4)));
        assert!(r.try_take(now + Duration::from_secs(5)));
        assert!(!r.try_take(now + Duration::from_secs(5)));
    }

    #[test]
    fn refill_never_exceeds_the_burst_size() {
        // An idle week must not bank a week's worth of sends and release them
        // all at the next detection — which is exactly the burst a service's
        // own rate limiter would ban the station for.
        let now = t0();
        let mut r = RateLimiter::per_minute(12, now);
        let after_a_week = now + Duration::from_secs(7 * 24 * 60 * 60);
        for _ in 0..12 {
            assert!(r.try_take(after_a_week));
        }
        assert!(!r.try_take(after_a_week));
    }

    // ── the two together ────────────────────────────────────────────────

    #[test]
    fn the_bucket_always_refills_before_a_probe_is_due() {
        // `admit` deliberately does *not* exempt the probe from the rate
        // limit, because it never needs to: the shortest open period is at
        // least the time one token takes to come back at the slowest rate an
        // operator can configure. If that stops holding — someone shortens
        // OPEN_BASE, or `per_minute` gains a sub-1 setting — a probe can be
        // eaten by an empty bucket, the breaker marks it issued anyway, and a
        // destination that came back stays shut out for the *next* open
        // period, which is twice as long.
        let slowest_refill = Duration::from_secs(60); // 1/minute, capacity 1
        assert!(
            OPEN_BASE >= slowest_refill,
            "the probe exemption removed from `admit` is needed again: \
             OPEN_BASE {OPEN_BASE:?} < slowest refill {slowest_refill:?}"
        );
    }

    #[test]
    fn a_probe_that_finds_a_token_is_reported_as_a_probe() {
        // The distinction matters to the caller's counters: a probe is not an
        // ordinary send, and must not be counted as one.
        let now = t0();
        let mut g = Gate::new(1, now);
        for _ in 0..3 {
            g.on_failure(now);
        }
        assert!(matches!(g.admit(now), Admission::CircuitOpen(_)));
        assert_eq!(g.admit(now + Duration::from_secs(61)), Admission::Probe);
    }

    #[test]
    fn an_ordinary_send_is_still_rate_limited() {
        // Counterpart: exempting the probe must not exempt everything. A gate
        // that returned `Probe` (or `Send`) unconditionally would satisfy the
        // gate above.
        let now = t0();
        let mut g = Gate::new(1, now);
        assert_eq!(g.admit(now), Admission::Send);
        assert_eq!(g.admit(now), Admission::RateLimited);
    }

    #[test]
    fn rate_limited_sends_alone_never_open_the_circuit() {
        // A busy morning is not an outage. `on_failure` is only called for a
        // send that was actually attempted, so a run of `RateLimited` verdicts
        // leaves the breaker closed.
        let now = t0();
        let mut g = Gate::new(1, now);
        assert_eq!(g.admit(now), Admission::Send);
        for _ in 0..100 {
            assert_eq!(g.admit(now), Admission::RateLimited);
        }
        // A minute later the bucket has refilled and the circuit is still shut.
        assert_eq!(g.admit(now + Duration::from_secs(60)), Admission::Send);
    }

    #[test]
    fn a_zero_rate_disables_the_limit() {
        let now = t0();
        let mut r = RateLimiter::per_minute(0, now);
        for _ in 0..10_000 {
            assert!(r.try_take(now));
        }
    }
}
