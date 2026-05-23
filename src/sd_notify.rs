//! Minimal systemd `sd_notify` client.
//!
//! Implements just enough of the systemd notification protocol so the
//! BirdNet-Behavior daemon can:
//!   * Tell systemd it has finished starting up (`READY=1`)
//!   * Keep the systemd watchdog timer satisfied (`WATCHDOG=1`)
//!   * Announce a clean shutdown (`STOPPING=1`)
//!
//! Reference: <https://www.freedesktop.org/software/systemd/man/sd_notify.html>
//!
//! Why not pull in the `sd-notify` crate? The protocol is small (write a
//! string to the unix datagram socket whose path lives in `$NOTIFY_SOCKET`)
//! and we already deny `unsafe_code` workspace-wide. Owning the four lines
//! here keeps the supply-chain surface tighter and avoids one more
//! dependency to audit / pin / track for advisories.
//!
//! All functions are no-ops outside Unix or when the daemon is not running
//! under systemd (i.e. `NOTIFY_SOCKET` is unset). They never panic; on
//! failure they log at `debug` level so a misconfigured manual run does
//! not flood the journal.

#[cfg(unix)]
use std::os::unix::net::UnixDatagram;
#[cfg(unix)]
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Send a single notification message to the systemd notify socket.
///
/// Returns `true` if the message was delivered, `false` if the daemon is
/// not running under systemd or the send failed. The latter is logged at
/// `debug` so a developer running the binary by hand does not see noise.
#[cfg(unix)]
pub fn notify(message: &str) -> bool {
    let Ok(socket_path) = std::env::var("NOTIFY_SOCKET") else {
        return false;
    };
    if socket_path.is_empty() {
        return false;
    }
    // systemd uses "@" as the prefix for abstract sockets on Linux; replace
    // with the leading NUL byte the kernel expects. Regular paths pass
    // through unchanged.
    let path_for_connect = socket_path
        .strip_prefix('@')
        .map_or_else(|| socket_path.clone(), |rest| format!("\0{rest}"));
    match UnixDatagram::unbound().and_then(|sock| {
        sock.connect(Path::new(&path_for_connect))
            .and_then(|()| sock.send(message.as_bytes()).map(|_| ()))
    }) {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!(error = %e, message, "sd_notify send failed (non-fatal)");
            false
        }
    }
}

#[cfg(not(unix))]
pub fn notify(_message: &str) -> bool {
    false
}

/// Tell systemd the daemon has finished initialising and is serving requests.
pub fn ready() {
    if notify("READY=1") {
        tracing::info!("systemd notified READY=1");
    }
}

/// Tell systemd the daemon is shutting down cleanly.
pub fn stopping() {
    if notify("STOPPING=1") {
        tracing::info!("systemd notified STOPPING=1");
    }
}

/// Ping the systemd watchdog so the unit's `WatchdogSec=` timer does not
/// elapse and trigger a restart.
///
/// Returns `true` if systemd accepted the ping (or appeared to — we send
/// a datagram, so this is a best-effort signal that the socket exists
/// and the kernel didn't reject the send), `false` otherwise.
///
/// Call from a background task whose interval is roughly **half** the
/// configured `WatchdogSec`, per the systemd documentation: a single missed
/// ping inside the window should not trigger a kill.
pub fn watchdog_ping() -> bool {
    notify("WATCHDOG=1")
}

/// Spawn a background task that pings the systemd watchdog on a fixed
/// interval. Returns immediately if `NOTIFY_SOCKET` or `WATCHDOG_USEC` is
/// unset (i.e. running outside systemd, or the unit did not configure a
/// watchdog). The interval is taken from `WATCHDOG_USEC` and halved.
///
/// When `heartbeat` is `Some`, the ping is **gated on detection-loop
/// progress**: each interval we ping only if the counter advanced since the
/// last ping. A stalled counter means the pipeline hung, so we deliberately
/// withhold the ping and let systemd's `WatchdogSec` fire — a frozen daemon
/// gets restarted instead of looking healthy forever. With `None` (e.g.
/// `--web-only`, no detection loop) we ping unconditionally: this task running
/// is itself proof the runtime is live.
///
/// When `metrics` is `Some`, every delivered ping bumps the
/// `birdnet_watchdog_pings_total` counter, giving operators a Prometheus-
/// readable signal that the supervisor link is alive.
pub fn spawn_watchdog_pinger(
    metrics: Option<birdnet_web::metrics::SharedMetrics>,
    heartbeat: Option<Arc<AtomicU64>>,
) {
    if std::env::var_os("NOTIFY_SOCKET").is_none() {
        return;
    }
    let Some(interval) = read_watchdog_interval() else {
        tracing::debug!("WATCHDOG_USEC not set; watchdog pinger not started");
        return;
    };
    let ping_every = interval / 2;
    tracing::info!(
        watchdog_interval_secs = interval.as_secs(),
        ping_every_secs = ping_every.as_secs(),
        gated_on_detection_loop = heartbeat.is_some(),
        "starting systemd watchdog pinger"
    );
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(ping_every);
        // Skip the immediate first tick — systemd considers the unit healthy
        // until WatchdogSec elapses, so an early ping is unnecessary.
        ticker.tick().await;
        // The heartbeat value at the previous ping; used to detect progress.
        let mut last_seen = heartbeat.as_ref().map_or(0, |h| h.load(Ordering::Relaxed));
        loop {
            ticker.tick().await;
            let (do_ping, seen) = watchdog_should_ping(heartbeat.as_deref(), last_seen);
            last_seen = seen;
            if do_ping {
                if watchdog_ping()
                    && let Some(m) = metrics.as_ref()
                {
                    m.inc_watchdog_pings();
                }
            } else {
                tracing::error!(
                    "detection loop heartbeat stalled; withholding watchdog ping so systemd restarts the hung daemon"
                );
            }
        }
    });
}

/// Decide whether to send a watchdog ping this interval, given the optional
/// detection-loop heartbeat and the value seen at the previous ping.
///
/// Returns `(should_ping, heartbeat_to_remember)`. With no heartbeat we always
/// ping; with one we ping only if it advanced (a stalled counter means the
/// pipeline hung and the watchdog should be allowed to fire).
#[must_use]
fn watchdog_should_ping(heartbeat: Option<&AtomicU64>, last_seen: u64) -> (bool, u64) {
    heartbeat.map_or((true, last_seen), |hb| {
        let current = hb.load(Ordering::Relaxed);
        (current != last_seen, current)
    })
}

/// Parse the systemd-supplied `WATCHDOG_USEC` env var (microseconds) into
/// a `Duration`. Returns `None` if unset or unparseable.
fn read_watchdog_interval() -> Option<Duration> {
    parse_watchdog_usec(std::env::var("WATCHDOG_USEC").ok().as_deref())
}

/// Pure parsing helper, factored out for tests so they do not have to mutate
/// process-global env vars (which is `unsafe` in Rust 2024 and prohibited
/// by the workspace lint).
fn parse_watchdog_usec(raw: Option<&str>) -> Option<Duration> {
    let usec: u64 = raw?.trim().parse().ok()?;
    if usec == 0 {
        return None;
    }
    Some(Duration::from_micros(usec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_watchdog_usec_handles_unset() {
        assert!(parse_watchdog_usec(None).is_none());
    }

    #[test]
    fn parse_watchdog_usec_parses_microseconds() {
        assert_eq!(
            parse_watchdog_usec(Some("30000000")),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn parse_watchdog_usec_rejects_zero() {
        assert!(parse_watchdog_usec(Some("0")).is_none());
    }

    #[test]
    fn parse_watchdog_usec_rejects_garbage() {
        assert!(parse_watchdog_usec(Some("not-a-number")).is_none());
    }

    #[test]
    fn parse_watchdog_usec_handles_whitespace() {
        assert_eq!(
            parse_watchdog_usec(Some("  60000000  ")),
            Some(Duration::from_secs(60))
        );
    }

    #[test]
    fn parse_watchdog_usec_rejects_negative() {
        // u64 parse fails on negatives — this just pins the behaviour.
        assert!(parse_watchdog_usec(Some("-1")).is_none());
    }

    #[test]
    fn notify_returns_false_when_notify_socket_unset() {
        // We do not mutate the env (forbidden by the no-unsafe rule);
        // instead we rely on the inherited test environment where
        // NOTIFY_SOCKET is overwhelmingly likely to be absent.
        if std::env::var_os("NOTIFY_SOCKET").is_some() {
            // Running under a systemd-style supervisor (rare in cargo test)
            // — skip rather than mutate the env.
            return;
        }
        assert!(!notify("READY=1"));
    }

    #[test]
    fn watchdog_pings_unconditionally_without_heartbeat() {
        // No detection loop to gate on (e.g. --web-only): always ping, and
        // carry the previous `last_seen` through unchanged.
        assert_eq!(watchdog_should_ping(None, 0), (true, 0));
        assert_eq!(watchdog_should_ping(None, 99), (true, 99));
    }

    #[test]
    fn watchdog_pings_when_heartbeat_advances() {
        // Loop made progress since the last ping → ping, remember the new value.
        let hb = AtomicU64::new(10);
        assert_eq!(watchdog_should_ping(Some(&hb), 5), (true, 10));
    }

    #[test]
    fn watchdog_withholds_ping_when_heartbeat_stalled() {
        // Counter unchanged since the last ping → pipeline hung → withhold the
        // ping so systemd's WatchdogSec fires.
        let hb = AtomicU64::new(42);
        assert_eq!(watchdog_should_ping(Some(&hb), 42), (false, 42));
    }

    #[test]
    fn notify_wrappers_are_safe_no_ops_without_systemd() {
        // Documented behaviour: every entry point is a graceful no-op when not
        // running under systemd. (Skip if a real NOTIFY_SOCKET is inherited,
        // since we must not mutate the env under the no-unsafe rule.)
        if std::env::var_os("NOTIFY_SOCKET").is_some() {
            return;
        }
        ready();
        stopping();
        assert!(!watchdog_ping());
        // No NOTIFY_SOCKET → returns immediately without spawning anything, so
        // this is safe to call outside a tokio runtime.
        spawn_watchdog_pinger(None, None);
    }
}
