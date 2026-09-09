//! A throttle on the sign-in form, per client address (O-6).
//!
//! Argon2id is deliberately slow, and that cuts both ways: the global limiter
//! permits tens of `/login` posts a second per address, each one a hash the
//! Pi has to compute while it is supposed to be running inference. Five
//! guesses a quarter-hour is the reference project's policy and is what this
//! enforces — consulted *before* a hash is computed, so a locked-out address
//! costs the station a map lookup and nothing else.
//!
//! Keyed on the resolved client address ([`crate::client_ip::ClientIp`], the
//! same resolution the global limiter and the session fingerprint use), so a
//! station behind a trusted proxy throttles visitors and not the proxy. A
//! sixth attempt from a different address is not affected — the gate for that
//! discrimination lives in `tests/login_is_throttled.rs`.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Failures an address may accumulate inside one [`WINDOW`] before it is
/// refused — the reference project's number.
pub const MAX_FAILURES: usize = 5;

/// How long a failure counts against its address.
pub const WINDOW: Duration = Duration::from_secs(15 * 60);

/// Addresses tracked before the table is pruned of expired ones on the next
/// touch. One entry is an address and at most [`MAX_FAILURES`] instants.
const PRUNE_ABOVE: usize = 1024;

/// Per-address record of recent failed sign-ins.
#[derive(Debug)]
pub struct LoginThrottle {
    failures: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
    max_failures: usize,
    window: Duration,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new(MAX_FAILURES, WINDOW)
    }
}

impl LoginThrottle {
    /// A throttle refusing an address after `max_failures` failures inside
    /// `window`.
    #[must_use]
    pub fn new(max_failures: usize, window: Duration) -> Self {
        Self {
            failures: Mutex::new(HashMap::new()),
            max_failures: max_failures.max(1),
            window,
        }
    }

    /// Whether `ip` may attempt to sign in now. `Err` carries how long until
    /// its oldest counted failure expires — the `Retry-After`.
    ///
    /// # Errors
    ///
    /// The address has [`MAX_FAILURES`] failures inside the window.
    pub fn check(&self, ip: IpAddr) -> Result<(), Duration> {
        self.check_at(ip, Instant::now())
    }

    /// Count a failed attempt from `ip`.
    pub fn record_failure(&self, ip: IpAddr) {
        self.record_failure_at(ip, Instant::now());
    }

    /// Forget `ip`'s failures — a successful sign-in.
    pub fn clear(&self, ip: IpAddr) {
        if let Ok(mut map) = self.failures.lock() {
            map.remove(&ip);
        }
    }

    fn check_at(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        // A poisoned lock (a panic while holding it) fails open, as the global
        // limiter does: refusing every sign-in for the life of the process is
        // the worse outcome for a station nobody can reach otherwise.
        let Ok(mut map) = self.failures.lock() else {
            return Ok(());
        };
        let Some(recent) = map.get_mut(&ip) else {
            return Ok(());
        };
        Self::expire(recent, now, self.window);
        if recent.len() < self.max_failures {
            return Ok(());
        }
        let oldest = recent.front().copied().unwrap_or(now);
        Err(self
            .window
            .saturating_sub(now.saturating_duration_since(oldest))
            .max(Duration::from_secs(1)))
    }

    fn record_failure_at(&self, ip: IpAddr, now: Instant) {
        let Ok(mut map) = self.failures.lock() else {
            return;
        };
        if map.len() > PRUNE_ABOVE {
            let window = self.window;
            map.retain(|_, recent| {
                Self::expire(recent, now, window);
                !recent.is_empty()
            });
        }
        let recent = map.entry(ip).or_default();
        Self::expire(recent, now, self.window);
        recent.push_back(now);
        while recent.len() > self.max_failures {
            recent.pop_front();
        }
    }

    fn expire(recent: &mut VecDeque<Instant>, now: Instant, window: Duration) {
        while recent
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= window)
        {
            recent.pop_front();
        }
    }

    /// Addresses currently tracked (diagnostics).
    #[must_use]
    pub fn tracked_addresses(&self) -> usize {
        self.failures.lock().map_or(0, |m| m.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 7));
    const B: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 8));

    #[test]
    fn the_sixth_failure_inside_the_window_is_refused_and_the_fifth_is_not() {
        let t = LoginThrottle::default();
        let now = Instant::now();
        for _ in 0..4 {
            t.record_failure_at(A, now);
        }
        assert!(t.check_at(A, now).is_ok(), "four failures: still allowed");
        t.record_failure_at(A, now);
        let retry = t.check_at(A, now).expect_err("five failures: refused");
        assert!(
            retry <= WINDOW && retry > Duration::from_secs(14 * 60),
            "{retry:?}"
        );
        assert!(
            t.check_at(B, now).is_ok(),
            "another address is not affected"
        );
    }

    #[test]
    fn a_failure_stops_counting_when_the_window_passes() {
        let t = LoginThrottle::default();
        let start = Instant::now();
        for _ in 0..MAX_FAILURES {
            t.record_failure_at(A, start);
        }
        assert!(
            t.check_at(
                A,
                start + WINDOW.checked_sub(Duration::from_secs(1)).unwrap()
            )
            .is_err()
        );
        assert!(t.check_at(A, start + WINDOW).is_ok());
    }

    #[test]
    fn the_retry_after_is_the_time_until_the_oldest_failure_expires() {
        let t = LoginThrottle::new(2, Duration::from_secs(600));
        let start = Instant::now();
        t.record_failure_at(A, start);
        t.record_failure_at(A, start + Duration::from_secs(100));
        let retry = t
            .check_at(A, start + Duration::from_secs(200))
            .expect_err("locked");
        assert_eq!(retry, Duration::from_secs(400));
    }

    #[test]
    fn a_successful_sign_in_clears_the_address() {
        let t = LoginThrottle::default();
        let now = Instant::now();
        for _ in 0..MAX_FAILURES {
            t.record_failure_at(A, now);
        }
        t.clear(A);
        assert!(t.check_at(A, now).is_ok());
        assert_eq!(t.tracked_addresses(), 0);
    }

    #[test]
    fn expired_addresses_are_pruned_once_the_table_is_large() {
        let t = LoginThrottle::default();
        let start = Instant::now();
        for i in 0..=PRUNE_ABOVE {
            let ip = IpAddr::V4(std::net::Ipv4Addr::from(u32::try_from(i).unwrap() + 1));
            t.record_failure_at(ip, start);
        }
        assert_eq!(t.tracked_addresses(), PRUNE_ABOVE + 1);
        t.record_failure_at(A, start + WINDOW);
        assert_eq!(
            t.tracked_addresses(),
            1,
            "only the fresh failure survives the prune"
        );
    }
}
