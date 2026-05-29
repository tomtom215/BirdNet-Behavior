//! Prometheus metrics endpoint.
//!
//! Provides production-grade observability:
//! - `GET /api/v2/metrics` — Prometheus-compatible metrics export
//!
//! The health check endpoint lives in `system.rs` (`GET /api/v2/health`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Router, routing::get};
use std::fmt::Write as _;

use crate::metrics::render_runtime_metrics;
use crate::state::AppState;

/// Mount metrics routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/metrics", get(prometheus_metrics))
}

/// Prometheus-compatible metrics endpoint.
///
/// Exports key metrics in Prometheus text exposition format for scraping
/// by monitoring systems (Prometheus, Grafana Agent, Victoria Metrics, etc.).
async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let version = env!("CARGO_PKG_VERSION");
    let uptime_secs = get_process_uptime();

    // Gather database metrics.
    let (detection_count, species_count) = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            state.with_db(|conn| {
                let det: i64 = conn
                    .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
                    .unwrap_or(0);
                let sp: i64 = conn
                    .query_row("SELECT COUNT(DISTINCT Com_Name) FROM detections", [], |r| {
                        r.get(0)
                    })
                    .unwrap_or(0);
                (det, sp)
            })
        }
    })
    .await
    .unwrap_or((0, 0));

    // Gather process metrics.
    let (rss_bytes, cpu_count) = process_metrics();

    let mut out = String::with_capacity(2048);

    // Standard Prometheus format.
    out.push_str("# HELP birdnet_info Build information.\n");
    out.push_str("# TYPE birdnet_info gauge\n");
    writeln!(out, "birdnet_info{{version=\"{version}\"}} 1").unwrap_or_default();

    out.push_str("# HELP birdnet_uptime_seconds Process uptime in seconds.\n");
    out.push_str("# TYPE birdnet_uptime_seconds gauge\n");
    writeln!(out, "birdnet_uptime_seconds {uptime_secs}").unwrap_or_default();

    out.push_str("# HELP birdnet_detections_total Total number of bird detections in database.\n");
    out.push_str("# TYPE birdnet_detections_total gauge\n");
    writeln!(out, "birdnet_detections_total {detection_count}").unwrap_or_default();

    out.push_str("# HELP birdnet_species_total Total number of distinct species detected.\n");
    out.push_str("# TYPE birdnet_species_total gauge\n");
    writeln!(out, "birdnet_species_total {species_count}").unwrap_or_default();

    out.push_str("# HELP birdnet_process_resident_memory_bytes Resident memory size in bytes.\n");
    out.push_str("# TYPE birdnet_process_resident_memory_bytes gauge\n");
    writeln!(out, "birdnet_process_resident_memory_bytes {rss_bytes}").unwrap_or_default();

    out.push_str("# HELP birdnet_cpu_count Number of CPU cores available.\n");
    out.push_str("# TYPE birdnet_cpu_count gauge\n");
    writeln!(out, "birdnet_cpu_count {cpu_count}").unwrap_or_default();

    let has_analytics: u8 = u8::from(state.has_analytics());
    out.push_str("# HELP birdnet_analytics_enabled Whether DuckDB analytics is enabled.\n");
    out.push_str("# TYPE birdnet_analytics_enabled gauge\n");
    writeln!(out, "birdnet_analytics_enabled {has_analytics}").unwrap_or_default();

    // Append the runtime counters/histograms maintained by the detection
    // daemon. Snapshot is computed under the registry's read locks so the
    // hot insert path is never blocked.
    let runtime = render_runtime_metrics(&state.metrics().snapshot());
    out.push_str(&runtime);

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
}

/// Get process uptime in seconds (0 when it can't be determined).
fn get_process_uptime() -> u64 {
    crate::system_info::process_uptime_secs().unwrap_or(0)
}

/// Get process RSS and CPU count.
fn process_metrics() -> (u64, u32) {
    // `/proc` is Linux-only; other platforms (macOS, etc.) report unknown (0)
    // RSS and a single core, which the System page renders as "—". Keeping the
    // mutable accumulators inside the Linux arm avoids an `unused_mut` warning
    // on non-Linux targets.
    #[cfg(not(target_os = "linux"))]
    {
        (0, 1)
    }
    #[cfg(target_os = "linux")]
    {
        let mut rss_bytes: u64 = 0;
        let mut cpu_count: u32 = 1;

        // RSS from /proc/self/status
        if let Ok(content) = std::fs::read_to_string("/proc/self/status") {
            for line in content.lines() {
                if line.starts_with("VmRSS:")
                    && let Some(kb_str) = line.split_whitespace().nth(1)
                    && let Ok(kb) = kb_str.parse::<u64>()
                {
                    rss_bytes = kb * 1024;
                }
            }
        }
        // CPU count from /proc/cpuinfo
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            let count = content
                .lines()
                .filter(|l| l.starts_with("processor"))
                .count();
            if count > 0 {
                cpu_count = u32::try_from(count).unwrap_or(1);
            }
        }

        (rss_bytes, cpu_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_is_non_negative() {
        let uptime = get_process_uptime();
        // Just verify it doesn't panic and returns a reasonable value.
        assert!(
            uptime < 365 * 24 * 3600 * 100,
            "uptime seems unreasonably large"
        );
    }

    #[test]
    fn process_metrics_returns_values() {
        let (rss, cpus) = process_metrics();
        // On Linux, rss should be > 0; on other platforms, 0 is acceptable.
        let _ = rss;
        assert!(cpus >= 1);
    }
}
