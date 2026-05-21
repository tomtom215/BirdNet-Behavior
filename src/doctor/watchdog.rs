//! systemd watchdog check: verify the supervisor is honouring the
//! `sd_notify` watchdog the daemon advertises.

use super::Check;

/// Verify the systemd watchdog plumbing the daemon advertises is actually
/// being honoured by the supervisor.
///
/// Three sub-questions, asked in order:
///
/// 1. **Is `NOTIFY_SOCKET` set?** If not we're running interactively (or
///    under an init system that doesn't speak `sd_notify`); skip the rest.
/// 2. **Is `WATCHDOG_USEC` set?** The unit shipped by `install.sh`
///    declares `WatchdogSec=120`, which makes systemd export this var. If
///    we see a `NOTIFY_SOCKET` but no `WATCHDOG_USEC`, the operator has
///    `Type=notify` without a watchdog timer — a degraded config; the
///    daemon won't be killed if it locks up.
/// 3. **Does a synthetic ping reach the socket?** Open a `UnixDatagram`,
///    `connect()` to the path, `send(b"WATCHDOG=1")`. A successful send
///    proves the pipe is intact end-to-end. A failure (`ENOENT`,
///    `ECONNREFUSED`) means the unit declares `NotifyAccess` correctly
///    but the supervisor has stopped reading the socket — operator
///    should restart the service.
///
/// Skipped on non-Unix because the protocol is Linux-specific.
pub(super) fn check_systemd_watchdog() -> Check {
    #[cfg(unix)]
    {
        watchdog_state().describe()
    }
    #[cfg(not(unix))]
    {
        Check::skip(
            "systemd watchdog",
            "sd_notify is Linux-specific; nothing to verify here",
        )
    }
}

/// Result of probing the systemd notify channel.
///
/// Pure value type so `check_systemd_watchdog` is unit-testable: the
/// probe lives in `probe_watchdog_socket`, the describe step is pure
/// formatting, and the test cases pin every cell of the decision matrix
/// without actually mutating `NOTIFY_SOCKET` (forbidden in this workspace
/// because `std::env::set_var` is `unsafe` in Rust 2024).
#[derive(Debug, Clone, PartialEq, Eq)]
enum WatchdogState {
    /// Daemon is not running under systemd — `NOTIFY_SOCKET` unset.
    NotUnderSystemd,
    /// `Type=notify` but no `WatchdogSec=` configured.
    NotifyButNoWatchdog,
    /// Both `NOTIFY_SOCKET` and `WATCHDOG_USEC` present; synthetic ping
    /// reached the supervisor.
    Working { interval_secs: u64 },
    /// Both `NOTIFY_SOCKET` and `WATCHDOG_USEC` present but the ping
    /// could not be delivered — supervisor has gone away.
    PingFailed { interval_secs: u64, reason: String },
}

impl WatchdogState {
    fn describe(&self) -> Check {
        match self {
            Self::NotUnderSystemd => Check::skip(
                "systemd watchdog",
                "NOTIFY_SOCKET is not set — running outside systemd (interactive run, docker, etc.)",
            ),
            Self::NotifyButNoWatchdog => Check::warn(
                "systemd watchdog",
                "NOTIFY_SOCKET is set but WATCHDOG_USEC is not — no watchdog timer is configured",
                "add `WatchdogSec=120` to the service's [Service] section and reload \
                 (`systemctl daemon-reload && systemctl restart birdnet-behavior`) \
                 so the supervisor can detect hangs",
            ),
            Self::Working { interval_secs } => Check::pass(
                "systemd watchdog",
                format!(
                    "ping delivered to systemd; supervisor expects pings every {interval_secs} s"
                ),
            ),
            Self::PingFailed {
                interval_secs,
                reason,
            } => Check::fail(
                "systemd watchdog",
                format!(
                    "WATCHDOG_USEC={interval_secs}s is set but the notify socket is not accepting pings: {reason}"
                ),
                "this means systemd has stopped reading the socket — \
                 `systemctl restart birdnet-behavior` to re-establish the supervisor link",
            ),
        }
    }
}

#[cfg(unix)]
fn watchdog_state() -> WatchdogState {
    let Some(socket_path) = std::env::var("NOTIFY_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        return WatchdogState::NotUnderSystemd;
    };
    let Some(interval_secs) = std::env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|usec| usec / 1_000_000)
        .filter(|&s| s > 0)
    else {
        return WatchdogState::NotifyButNoWatchdog;
    };

    match probe_watchdog_socket(&socket_path) {
        Ok(()) => WatchdogState::Working { interval_secs },
        Err(reason) => WatchdogState::PingFailed {
            interval_secs,
            reason,
        },
    }
}

/// Send one `WATCHDOG=1` datagram to the supervisor.
///
/// Mirrors the production `sd_notify::watchdog_ping` send path but
/// returns the error rather than swallowing it, so the diagnostic can
/// surface it on the operator's screen.
#[cfg(unix)]
fn probe_watchdog_socket(socket_path: &str) -> Result<(), String> {
    use std::os::unix::net::UnixDatagram;
    use std::path::Path;
    let path_for_connect = socket_path
        .strip_prefix('@')
        .map_or_else(|| socket_path.to_owned(), |rest| format!("\0{rest}"));
    let sock = UnixDatagram::unbound().map_err(|e| format!("UnixDatagram::unbound: {e}"))?;
    sock.connect(Path::new(&path_for_connect))
        .map_err(|e| format!("connect({socket_path}): {e}"))?;
    sock.send(b"WATCHDOG=1")
        .map_err(|e| format!("send WATCHDOG=1: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::WatchdogState;
    use crate::doctor::Status;

    #[test]
    fn watchdog_describe_not_under_systemd_is_skip() {
        let c = WatchdogState::NotUnderSystemd.describe();
        assert_eq!(c.status, Status::Skip);
        assert!(c.message.contains("NOTIFY_SOCKET"));
    }

    #[test]
    fn watchdog_describe_no_watchdog_warns_with_remediation() {
        let c = WatchdogState::NotifyButNoWatchdog.describe();
        assert_eq!(c.status, Status::Warn);
        assert!(
            c.remediation
                .as_ref()
                .is_some_and(|r| r.contains("WatchdogSec"))
        );
    }

    #[test]
    fn watchdog_describe_working_includes_interval() {
        let c = WatchdogState::Working { interval_secs: 120 }.describe();
        assert_eq!(c.status, Status::Pass);
        assert!(c.message.contains("120"));
    }

    #[test]
    fn watchdog_describe_ping_failed_is_fail_with_reason() {
        let c = WatchdogState::PingFailed {
            interval_secs: 60,
            reason: "connect(/run/foo): No such file or directory".into(),
        }
        .describe();
        assert_eq!(c.status, Status::Fail);
        assert!(c.message.contains("60"));
        assert!(c.message.contains("No such file"));
    }

    #[cfg(unix)]
    #[test]
    fn probe_watchdog_socket_succeeds_against_a_real_receiver() {
        use super::probe_watchdog_socket;
        use std::os::unix::net::UnixDatagram;

        let tmp =
            std::env::temp_dir().join(format!("birdnet-doctor-probe-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let server = UnixDatagram::bind(&tmp).expect("bind notify socket");

        let result = probe_watchdog_socket(tmp.to_str().unwrap());
        let _ = std::fs::remove_file(&tmp);
        drop(server);

        assert!(result.is_ok(), "probe failed: {result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn probe_watchdog_socket_fails_on_missing_path() {
        use super::probe_watchdog_socket;
        let err = probe_watchdog_socket("/tmp/this/path/does/not/exist.sock")
            .expect_err("should not be able to send to a missing socket");
        assert!(err.contains("connect") || err.contains("No such file"));
    }
}
