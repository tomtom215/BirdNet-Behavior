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

/// Failures one *vouching hop* may accumulate inside one [`WINDOW`], summed
/// over every client it has named.
///
/// A trusted hop can name any client it likes, so a LAN host that sends a
/// fresh `X-Forwarded-For` per attempt — directly, or through a proxy that
/// appends — gets a fresh five each time. This bounds what such a hop can
/// buy. It is wider than [`MAX_FAILURES`] because an honest proxy on another
/// box also vouches for every visitor behind it, and a loopback hop is not
/// counted at all: a same-host proxy records what it saw, and counting it
/// would let any few strangers lock the owner out of the whole station.
pub const VOUCHER_MAX_FAILURES: usize = 30;

/// How long a failure counts against its address.
pub const WINDOW: Duration = Duration::from_secs(15 * 60);

/// Addresses tracked before the table is pruned of expired ones on the next
/// touch. One entry is an address and at most [`MAX_FAILURES`] instants.
const PRUNE_ABOVE: usize = 1024;

type Table = HashMap<IpAddr, VecDeque<Instant>>;

/// Per-address record of recent failed sign-ins.
#[derive(Debug)]
pub struct LoginThrottle {
    /// Clients, then vouching hops. One lock, so an attempt is checked and
    /// counted against both in one step.
    tables: Mutex<(Table, Table)>,
    max_failures: usize,
    voucher_max_failures: usize,
    window: Duration,
}

/// A sign-in attempt [`LoginThrottle::begin`] let through.
///
/// It is counted as a failure *before* the password is checked. Counting it
/// afterwards let a parallel burst through: every request passed the check
/// while none had yet recorded, so the global limiter's burst of sixty was
/// sixty guesses, not five. Dropping an `Attempt` leaves the failure counted;
/// [`LoginThrottle::succeeded`] withdraws it.
#[derive(Debug)]
#[must_use = "drop it for a failure, or pass it to `succeeded`"]
pub struct Attempt {
    client: IpAddr,
    voucher: Option<IpAddr>,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new(MAX_FAILURES, WINDOW)
    }
}

impl LoginThrottle {
    /// A throttle refusing an address after `max_failures` failures inside
    /// `window` (and a vouching hop after [`VOUCHER_MAX_FAILURES`]).
    #[must_use]
    pub fn new(max_failures: usize, window: Duration) -> Self {
        Self {
            tables: Mutex::new((HashMap::new(), HashMap::new())),
            max_failures: max_failures.max(1),
            voucher_max_failures: VOUCHER_MAX_FAILURES,
            window,
        }
    }

    /// Admit a sign-in attempt from `client`, named by `voucher` (`None` when
    /// nothing but the connection itself vouched, or the hop was loopback),
    /// counting it as a failure now. `Err` carries how long until the budget
    /// that refused it frees up — the `Retry-After`.
    ///
    /// # Errors
    ///
    /// The client, or the hop that vouched for it, has used its budget inside
    /// the window.
    pub fn begin(&self, client: IpAddr, voucher: Option<IpAddr>) -> Result<Attempt, Duration> {
        self.begin_at(client, voucher, Instant::now())
    }

    /// The attempt signed in: forget the client's failures, and withdraw the
    /// one this attempt charged its voucher. Other clients' failures on that
    /// voucher stand.
    // By value on purpose: an attempt is withdrawn once, and taking it makes a
    // second withdrawal a compile error rather than a second `pop_back`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn succeeded(&self, attempt: Attempt) {
        let Attempt { client, voucher } = attempt;
        if let Ok(mut tables) = self.tables.lock() {
            tables.0.remove(&client);
            if let Some(v) = voucher
                && let Some(recent) = tables.1.get_mut(&v)
            {
                recent.pop_back();
                if recent.is_empty() {
                    tables.1.remove(&v);
                }
            }
        }
    }

    fn begin_at(
        &self,
        client: IpAddr,
        voucher: Option<IpAddr>,
        now: Instant,
    ) -> Result<Attempt, Duration> {
        // A poisoned lock (a panic while holding it) fails open, as the global
        // limiter does: refusing every sign-in for the life of the process is
        // the worse outcome for a station nobody can reach otherwise.
        let Ok(mut guard) = self.tables.lock() else {
            return Ok(Attempt { client, voucher });
        };
        let (clients, vouchers) = &mut *guard;
        let window = self.window;
        if let Some(wait) = refusal(clients, client, now, window, self.max_failures) {
            return Err(wait);
        }
        if let Some(v) = voucher
            && let Some(wait) = refusal(vouchers, v, now, window, self.voucher_max_failures)
        {
            return Err(wait);
        }
        charge(clients, client, now, window, self.max_failures);
        if let Some(v) = voucher {
            charge(vouchers, v, now, window, self.voucher_max_failures);
        }
        Ok(Attempt { client, voucher })
    }

    /// Addresses currently tracked (diagnostics).
    #[must_use]
    pub fn tracked_addresses(&self) -> usize {
        self.tables.lock().map_or(0, |t| t.0.len())
    }

    #[cfg(test)]
    fn check_at(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let mut t = self.tables.lock().unwrap();
        refusal(&mut t.0, ip, now, self.window, self.max_failures).map_or(Ok(()), Err)
    }

    #[cfg(test)]
    fn record_failure_at(&self, ip: IpAddr, now: Instant) {
        let mut t = self.tables.lock().unwrap();
        charge(&mut t.0, ip, now, self.window, self.max_failures);
    }

    #[cfg(test)]
    fn clear(&self, ip: IpAddr) {
        self.tables.lock().unwrap().0.remove(&ip);
    }
}

/// How long `ip` must wait, if its failures inside `window` have reached `max`.
fn refusal(
    table: &mut Table,
    ip: IpAddr,
    now: Instant,
    window: Duration,
    max: usize,
) -> Option<Duration> {
    let recent = table.get_mut(&ip)?;
    expire(recent, now, window);
    if recent.len() < max {
        return None;
    }
    let oldest = recent.front().copied().unwrap_or(now);
    Some(
        window
            .saturating_sub(now.saturating_duration_since(oldest))
            .max(Duration::from_secs(1)),
    )
}

/// Count one failure against `ip`.
fn charge(table: &mut Table, ip: IpAddr, now: Instant, window: Duration, max: usize) {
    if table.len() > PRUNE_ABOVE {
        table.retain(|_, recent| {
            expire(recent, now, window);
            !recent.is_empty()
        });
    }
    let recent = table.entry(ip).or_default();
    expire(recent, now, window);
    recent.push_back(now);
    while recent.len() > max {
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

    /// Five attempts in flight at once, none finished: the sixth is refused.
    /// The old check-then-record order admitted all of them, because none had
    /// recorded when the next was checked.
    #[test]
    fn attempts_in_flight_count_before_they_finish() {
        let t = LoginThrottle::default();
        let now = Instant::now();
        let held: Vec<Attempt> = (0..MAX_FAILURES)
            .map(|i| {
                t.begin_at(A, None, now)
                    .unwrap_or_else(|_| panic!("attempt {i}"))
            })
            .collect();
        assert!(t.begin_at(A, None, now).is_err(), "the sixth in flight");
        drop(held);
    }

    /// A hop that names a new client each time runs out at its own budget;
    /// a different hop's clients are untouched.
    #[test]
    fn a_hop_naming_a_new_client_each_time_runs_out() {
        let t = LoginThrottle::default();
        let now = Instant::now();
        let hop = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 50));
        let other = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 51));
        for i in 0..VOUCHER_MAX_FAILURES {
            let minted = IpAddr::V4(std::net::Ipv4Addr::from(
                0x0909_0000 + u32::try_from(i).unwrap(),
            ));
            let _ = t
                .begin_at(minted, Some(hop), now)
                .expect("inside the hop's budget");
        }
        assert!(
            t.begin_at(
                IpAddr::V4(std::net::Ipv4Addr::new(9, 9, 99, 99)),
                Some(hop),
                now
            )
            .is_err()
        );
        assert!(
            t.begin_at(B, Some(other), now).is_ok(),
            "another hop is not affected"
        );
    }

    /// A success withdraws the charge it made on its hop, and only that.
    #[test]
    fn a_success_withdraws_only_its_own_charge_on_the_hop() {
        let t = LoginThrottle::default();
        let now = Instant::now();
        let hop = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 50));
        let _ = t.begin_at(A, Some(hop), now).unwrap();
        let ok = t.begin_at(B, Some(hop), now).unwrap();
        t.succeeded(ok);
        assert_eq!(
            t.tables.lock().unwrap().1[&hop].len(),
            1,
            "A's failure stands"
        );
        assert_eq!(t.tracked_addresses(), 1);
    }
}
