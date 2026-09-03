//! Service control: restart, status, systemd integration.

use axum::response::Html;

/// What asking for a restart actually did.
///
/// Two outcomes, and the difference matters enough to be a type rather than a
/// rendered string: on a station not under systemd nothing brings the process
/// back, so signalling it would leave the operator staring at a dead server
/// behind a cheerful "restart sent". The HTMX page and
/// `POST /api/v2/control/restart` render this differently; neither decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartOutcome {
    /// SIGTERM is on its way; `Restart=always` will bring a fresh instance up.
    Signalled,
    /// Not under systemd — refused, because nothing would restart us.
    NotUnderSystemd,
}

/// Whether this process is supervised by systemd, read from the environment.
///
/// Called **once**, by the application wiring its [`crate::state::AppState`],
/// and the answer is carried on the state from then on. It is a property of
/// how the process was started and cannot change while it runs, so re-reading
/// it per request bought nothing — and cost a great deal: the handlers below
/// then behaved differently depending on the environment the test binary
/// inherited. A GitHub Actions runner sets `INVOCATION_ID`, so
/// `crates/birdnet-web/tests/the_api_can_change_the_station.rs` reached the
/// signalling branch in CI and the test process took its own SIGTERM.
#[must_use]
pub fn supervised_by_systemd() -> bool {
    std::env::var_os("INVOCATION_ID").is_some() || std::env::var_os("JOURNAL_STREAM").is_some()
}

/// What a restart request would do, given whether systemd is supervising us.
///
/// Pure, and separate from [`request_restart`] so the discrimination can be
/// asserted without a test process signalling itself.
#[must_use]
pub const fn restart_outcome(under_systemd: bool) -> RestartOutcome {
    if under_systemd {
        RestartOutcome::Signalled
    } else {
        RestartOutcome::NotUnderSystemd
    }
}

/// Ask the process to restart itself, and say what happened.
///
/// The unit runs as a non-root, sandboxed `Type=notify` service with
/// `Restart=always`. The robust, privilege-free way for it to restart itself is
/// to exit on SIGTERM and let systemd start a fresh instance — so that is what
/// we do. We deliberately do NOT shell out to `systemctl restart`: a non-root
/// service is polkit-denied (the call fails silently), and restarting our own
/// unit from inside its cgroup races the `KillMode=mixed` teardown that kills
/// the `systemctl` child mid-job. When not running under systemd there is
/// nothing to bring us back, so we say so rather than kill the process and leave
/// the operator staring at a dead server behind a misleading "restart sent".
///
/// `under_systemd` is a parameter rather than an environment read so that the
/// only thing separating a test from a self-inflicted SIGTERM is an explicit
/// argument. See [`supervised_by_systemd`].
///
/// Does **not** audit: the caller does, because the two callers have different
/// identities (a logged-in person, or a bearer token that is nobody) and the
/// audit-log vocabulary gate reads the action literal at the call site.
pub fn request_restart(under_systemd: bool) -> RestartOutcome {
    let outcome = restart_outcome(under_systemd);
    if outcome == RestartOutcome::NotUnderSystemd {
        return outcome;
    }

    // Respond first, then signal: send SIGTERM from a detached thread after a
    // short delay so the HTTP response reaches the caller before the process
    // exits. The graceful-shutdown path then runs and `Restart=always` starts a
    // fresh instance. `kill` of our own PID needs no privilege (same uid), so it
    // works even with every capability dropped.
    let pid = std::process::id().to_string();
    tracing::info!(%pid, "restart requested; SIGTERM self, systemd Restart=always brings us back");
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status();
    });
    outcome
}

/// The HTML fragment the dashboard's restart button swaps in.
///
/// Split from the handler so both arms can be asserted without reaching the
/// one that signals.
fn restart_fragment(outcome: RestartOutcome) -> Html<String> {
    match outcome {
        RestartOutcome::NotUnderSystemd => Html(
            "<p class=\"ctl-warn\">Not running under systemd, so the service can't restart itself \
from here. Restart it from a shell: <code>sudo systemctl restart birdnet-behavior</code> \
(or stop and re-run the binary).</p>"
                .to_string(),
        ),
        RestartOutcome::Signalled => Html(
            "<p class=\"ctl-ok\">Restarting now — the dashboard will reconnect in a few seconds.</p>"
                .to_string(),
        ),
    }
}

/// `POST /admin/system/restart` — the dashboard's restart button.
///
/// The decision is [`request_restart`]'s; this renders it as the HTML fragment
/// HTMX swaps in, and records *who* asked. `POST /api/v2/control/restart` is
/// the same decision rendered as JSON.
#[allow(clippy::unused_async)] // async required by axum's Handler trait
pub(super) async fn service_restart(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
    request_user: crate::auth_middleware::RequestUser,
) -> Html<String> {
    // Before the SIGTERM, and before the systemd check, so the record survives
    // the restart and exists even on a station where the restart is refused —
    // "who kept pressing this?" is a question either outcome raises.
    crate::audit::audit(&state, Some(&request_user), "system.restart", None, None);

    restart_fragment(request_restart(state.supervised_by_systemd()))
}

/// Return HTML with current process status (PID, uptime, memory, version).
pub(super) async fn service_status() -> Html<String> {
    let pid = std::process::id();
    // Gathering status reads `/proc` and spawns `getconf` / `systemctl`
    // subprocesses — all blocking. Run it off the async runtime so a slow
    // `/proc` (e.g. NFS-mounted) or a hung `systemctl` can't stall the request
    // executor and wedge unrelated requests.
    let (uptime_secs, memory_mb, service_active) = tokio::task::spawn_blocking(move || {
        (
            get_process_uptime_secs(pid),
            get_process_memory_mb(pid),
            check_systemd_service_active("birdnet-behavior"),
        )
    })
    .await
    .unwrap_or((0, 0.0, false));
    let version = env!("CARGO_PKG_VERSION");

    let uptime_str = if uptime_secs >= 3600 {
        format!("{}h {}m", uptime_secs / 3600, (uptime_secs % 3600) / 60)
    } else if uptime_secs >= 60 {
        format!("{}m {}s", uptime_secs / 60, uptime_secs % 60)
    } else {
        format!("{uptime_secs}s")
    };

    let systemd_badge = if service_active {
        r#"<span class="ctl-ok-strong">● active</span>"#
    } else {
        r#"<span class="ctl-muted">○ not managed by systemd</span>"#
    };

    Html(format!(
        r#"<table class="ctl-table">
          <tr><td class="ctl-k">Version</td><td class="ctl-v-strong">v{version}</td></tr>
          <tr><td class="ctl-k">PID</td><td>{pid}</td></tr>
          <tr><td class="ctl-k">Uptime</td><td>{uptime_str}</td></tr>
          <tr><td class="ctl-k">Memory (RSS)</td><td>{memory_mb:.1} MB</td></tr>
          <tr><td class="ctl-k">systemd service</td><td>{systemd_badge}</td></tr>
        </table>"#
    ))
}

fn get_process_uptime_secs(_pid: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let (Ok(stat), Ok(uptime_str)) = (
            std::fs::read_to_string("/proc/self/stat"),
            std::fs::read_to_string("/proc/uptime"),
        ) {
            let hz: u64 = std::process::Command::new("getconf")
                .arg("CLK_TCK")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(100);

            if let (Some(start_field), Some(uptime_field)) = (
                stat.split_whitespace().nth(21),
                uptime_str.split_whitespace().next(),
            ) && let (Ok(start_jiffies), Ok(sys_uptime)) =
                (start_field.parse::<u64>(), uptime_field.parse::<f64>())
                && hz > 0
            {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    clippy::cast_possible_wrap,
                    clippy::cast_lossless
                )]
                let proc_uptime = sys_uptime - (start_jiffies / hz) as f64;
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    clippy::cast_possible_wrap,
                    clippy::cast_lossless
                )]
                return proc_uptime.max(0.0) as u64;
            }
        }
    }
    0
}

fn get_process_memory_mb(pid: u32) -> f64 {
    #[cfg(target_os = "linux")]
    {
        let status_path = format!("/proc/{pid}/status");
        if let Ok(content) = std::fs::read_to_string(&status_path) {
            for line in content.lines() {
                if line.starts_with("VmRSS:")
                    && let Some(kb_str) = line.split_whitespace().nth(1)
                    && let Ok(kb) = kb_str.parse::<f64>()
                {
                    return kb / 1024.0;
                }
            }
        }
    }
    let _ = pid;
    0.0
}

fn check_systemd_service_active(service: &str) -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", service])
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::{RestartOutcome, restart_fragment, restart_outcome};

    /// The decision, both ways, without anything signalling itself.
    ///
    /// `request_restart` was a single function that read the environment and
    /// then sent the SIGTERM, so neither half could be asserted: reaching the
    /// signalling branch meant killing the test process, and which branch you
    /// got depended on whether the runner's environment happened to carry
    /// `INVOCATION_ID`. Splitting the decision out is what makes this a test.
    #[test]
    fn a_restart_is_refused_when_nothing_would_bring_us_back() {
        assert_eq!(restart_outcome(true), RestartOutcome::Signalled);
        assert_eq!(restart_outcome(false), RestartOutcome::NotUnderSystemd);
    }

    /// The dashboard tells the operator which of the two happened.
    ///
    /// The counterpart to the decision test: a fragment that rendered the same
    /// text either way would satisfy it and leave an operator reading
    /// "Restarting now" at a station that is not going to restart.
    #[test]
    fn the_two_outcomes_do_not_render_the_same_thing() {
        let signalled = restart_fragment(RestartOutcome::Signalled).0;
        let refused = restart_fragment(RestartOutcome::NotUnderSystemd).0;

        assert!(signalled.contains("ctl-ok"), "{signalled}");
        assert!(signalled.contains("Restarting now"), "{signalled}");

        assert!(refused.contains("ctl-warn"), "{refused}");
        assert!(
            refused.contains("systemctl restart birdnet-behavior"),
            "the refusal should say how to restart it by hand: {refused}"
        );
        assert!(
            !refused.contains("Restarting now"),
            "a station that is not restarting must not say it is: {refused}"
        );
        assert_ne!(signalled, refused);
    }
}
