//! Service control: restart, status, systemd integration.

use axum::response::Html;

/// Restart the birdnet-behavior service.
///
/// Strategy (in order of preference):
/// 1. If running as a systemd service (`INVOCATION_ID` set), attempt `systemctl restart`
/// 2. Otherwise, send SIGTERM to self (systemd with `Restart=on-failure` will restart it)
pub(super) async fn service_restart() -> Html<String> {
    let result = tokio::task::spawn_blocking(|| {
        let under_systemd = std::env::var("INVOCATION_ID").is_ok()
            || std::env::var("JOURNAL_STREAM").is_ok();

        if under_systemd {
            let status = std::process::Command::new("systemctl")
                .args(["restart", "birdnet-behavior"])
                .status();
            match status {
                Ok(s) if s.success() => {
                    return Ok::<String, String>(
                        "Service restart initiated via systemctl.".to_string(),
                    )
                }
                Ok(s) => {
                    tracing::warn!(status = %s, "systemctl restart returned non-zero, falling back to SIGTERM");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "systemctl not available, falling back to SIGTERM");
                }
            }
        }

        let pid = std::process::id().to_string();
        tracing::info!(%pid, "sending SIGTERM to self for graceful restart");
        let pid_clone = pid;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &pid_clone])
                .status();
        });
        Ok("Restart signal sent. Service will restart momentarily.".to_string())
    })
    .await;

    match result {
        Ok(Ok(msg)) => Html(format!(
            r#"<p style="color:#4ade80;">{msg} Reconnect in a few seconds.</p>"#
        )),
        Ok(Err(e)) => Html(format!(
            r#"<p style="color:#f87171;">Restart failed: {e}</p>"#
        )),
        Err(e) => Html(format!(
            r#"<p style="color:#f87171;">Internal error: {e}</p>"#
        )),
    }
}

/// Return HTML with current process status (PID, uptime, memory, version).
pub(super) async fn service_status() -> Html<String> {
    let pid = std::process::id();
    let uptime_secs = get_process_uptime_secs(pid);
    let memory_mb = get_process_memory_mb(pid);
    let service_active = check_systemd_service_active("birdnet-behavior");
    let version = env!("CARGO_PKG_VERSION");

    let uptime_str = if uptime_secs >= 3600 {
        format!("{}h {}m", uptime_secs / 3600, (uptime_secs % 3600) / 60)
    } else if uptime_secs >= 60 {
        format!("{}m {}s", uptime_secs / 60, uptime_secs % 60)
    } else {
        format!("{uptime_secs}s")
    };

    let systemd_badge = if service_active {
        r#"<span style="color:#4ade80;font-weight:600;">● active</span>"#
    } else {
        r#"<span style="color:#94a3b8;">○ not managed by systemd</span>"#
    };

    Html(format!(
        r#"<table style="width:100%;border-collapse:collapse;font-size:.875rem;">
          <tr><td style="color:#64748b;padding:.25rem 0;">Version</td><td style="font-weight:600;">v{version}</td></tr>
          <tr><td style="color:#64748b;padding:.25rem 0;">PID</td><td>{pid}</td></tr>
          <tr><td style="color:#64748b;padding:.25rem 0;">Uptime</td><td>{uptime_str}</td></tr>
          <tr><td style="color:#64748b;padding:.25rem 0;">Memory (RSS)</td><td>{memory_mb:.1} MB</td></tr>
          <tr><td style="color:#64748b;padding:.25rem 0;">systemd service</td><td>{systemd_badge}</td></tr>
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
