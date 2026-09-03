//! Service control: restart, status, systemd integration.

use axum::response::Html;

/// Restart the birdnet-behavior service.
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
#[allow(clippy::unused_async)] // async required by axum's Handler trait
pub(super) async fn service_restart(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
    request_user: crate::auth_middleware::RequestUser,
) -> Html<String> {
    // Before the SIGTERM, and before the systemd check, so the record survives
    // the restart and exists even on a station where the restart is refused —
    // "who kept pressing this?" is a question either outcome raises.
    crate::audit::audit(&state, Some(&request_user), "system.restart", None, None);
    let under_systemd =
        std::env::var("INVOCATION_ID").is_ok() || std::env::var("JOURNAL_STREAM").is_ok();

    if !under_systemd {
        return Html(
            "<p class=\"ctl-warn\">Not running under systemd, so the service can't restart itself \
from here. Restart it from a shell: <code>sudo systemctl restart birdnet-behavior</code> \
(or stop and re-run the binary).</p>"
                .to_string(),
        );
    }

    // Respond first, then signal: send SIGTERM from a detached thread after a
    // short delay so this HTTP response reaches the browser before the process
    // exits. The graceful-shutdown path then runs and `Restart=always` starts a
    // fresh instance. `kill` of our own PID needs no privilege (same uid), so it
    // works even with every capability dropped.
    let pid = std::process::id().to_string();
    tracing::info!(%pid, "admin UI requested restart; SIGTERM self, systemd Restart=always brings us back");
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status();
    });

    Html(
        "<p class=\"ctl-ok\">Restarting now — the dashboard will reconnect in a few seconds.</p>"
            .to_string(),
    )
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
