//! Shared retry backoff with jitter for the network integration clients.
//!
//! The HTTP clients (`birdweather`, `apprise`) retry transient failures with
//! exponential backoff. Plain `2^attempt` backoff is *synchronised*: if the
//! upstream blips, every in-flight retry — and every station in a fleet that
//! posts on the same cadence — wakes at the same instants and hammers the
//! endpoint in lockstep (a "thundering herd"). Adding jitter spreads those
//! wakeups out, which is the difference between a recovering service and one
//! that's kept down by its own clients.
//!
//! The backoff is also **capped** and **overflow-safe**, so a long outage
//! settles at a steady retry cadence instead of growing the delay without
//! bound (or panicking on `2^attempt` overflow).

use std::time::Duration;

/// Upper bound on the backoff. A persistently failing endpoint is retried at
/// least this often rather than drifting to minutes-apart waits.
const BACKOFF_CAP_SECS: u64 = 32;

/// Largest shift we ever apply, so `1 << shift` can never overflow `u64`
/// regardless of how high the caller's attempt counter climbs.
const MAX_SHIFT: u32 = 20;

/// Delay before retry `attempt` (1-based) using **equal jitter**.
///
/// The deterministic component grows as `2^attempt` seconds, capped at
/// `BACKOFF_CAP_SECS`; half of it is fixed and the other half is randomised
/// by `jitter_frac` (clamped to `[0.0, 1.0]`). So retry 1 lands in `[1s, 2s]`,
/// retry 2 in `[2s, 4s]`, retry 3 in `[4s, 8s]`, … each spread across a window
/// rather than firing at one instant. `attempt == 0` is the first try and
/// returns [`Duration::ZERO`].
///
/// Pure — the randomness is injected via `jitter_frac` — so the schedule is
/// unit-testable without a clock or RNG.
#[must_use]
pub fn backoff_delay(attempt: u32, jitter_frac: f64) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    // Cap the shift first (overflow safety), then the seconds (bounded cadence).
    let shift = attempt.min(MAX_SHIFT);
    let full = (1_u64 << shift).min(BACKOFF_CAP_SECS); // 2^attempt secs, capped
    let half = full / 2;
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let jitter = (jitter_frac.clamp(0.0, 1.0) * half as f64) as u64;
    Duration::from_secs(half + jitter)
}

/// A pseudo-random fraction in `[0.0, 1.0)` drawn from the clock's sub-second
/// bits. Not cryptographic — just enough entropy to de-synchronise retries
/// across attempts and across stations, which is all jitter needs.
#[must_use]
pub fn jitter_frac() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    f64::from(nanos) / f64::from(1_000_000_000_u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_attempt_has_no_delay() {
        assert_eq!(backoff_delay(0, 0.5), Duration::ZERO);
    }

    #[test]
    fn delay_lands_in_the_equal_jitter_window() {
        // Retry N's deterministic full backoff is 2^N (capped); the returned
        // delay must lie in [full/2, full]. Lower bound at jitter 0, upper at 1.
        for (attempt, full) in [(1_u32, 2_u64), (2, 4), (3, 8), (4, 16), (5, 32)] {
            let half = full / 2;
            assert_eq!(
                backoff_delay(attempt, 0.0),
                Duration::from_secs(half),
                "attempt {attempt} floor"
            );
            assert_eq!(
                backoff_delay(attempt, 1.0),
                Duration::from_secs(full),
                "attempt {attempt} ceiling"
            );
            // A mid jitter is strictly inside the window.
            let mid = backoff_delay(attempt, 0.5);
            assert!(
                mid >= Duration::from_secs(half) && mid <= Duration::from_secs(full),
                "attempt {attempt} mid {mid:?} outside [{half}, {full}]"
            );
        }
    }

    #[test]
    fn backoff_is_capped() {
        // Past the point where 2^attempt exceeds the cap, the window is fixed
        // at [cap/2, cap] — a steady cadence, never minutes apart.
        let lo = backoff_delay(10, 0.0);
        let hi = backoff_delay(10, 1.0);
        assert_eq!(lo, Duration::from_secs(BACKOFF_CAP_SECS / 2));
        assert_eq!(hi, Duration::from_secs(BACKOFF_CAP_SECS));
    }

    #[test]
    fn never_overflows_or_panics_at_extreme_attempts() {
        // A runaway attempt counter must stay capped, not shift-overflow.
        assert_eq!(
            backoff_delay(u32::MAX, 1.0),
            Duration::from_secs(BACKOFF_CAP_SECS)
        );
        assert_eq!(
            backoff_delay(1000, 0.0),
            Duration::from_secs(BACKOFF_CAP_SECS / 2)
        );
    }

    #[test]
    fn jitter_frac_out_of_range_is_clamped() {
        // Defensive: a caller passing a bogus fraction can't widen the window.
        assert_eq!(backoff_delay(3, -5.0), Duration::from_secs(4)); // clamps to 0.0
        assert_eq!(backoff_delay(3, 9.0), Duration::from_secs(8)); // clamps to 1.0
    }

    #[test]
    fn jitter_source_is_a_unit_fraction() {
        let f = jitter_frac();
        assert!((0.0..1.0).contains(&f), "jitter_frac {f} not in [0,1)");
    }
}
