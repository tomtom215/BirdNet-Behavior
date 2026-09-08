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
    //
    // `birdnet_detections_stored` counts every row, rejections included, and
    // deliberately does not switch to `detections_analytic`: it is a *pipeline
    // throughput* signal — "is the station still turning audio into rows?" — and
    // a detection a human later rejected still proves the chain ran. Exporting
    // the rejection count alongside it is what makes both questions answerable
    // from one scrape, so a dashboard can show either the raw rate or
    // `stored - rejected` to match what the web UI displays. Picking one and
    // hiding the other is what made the UI's own tiles disagree.
    //
    // None of these three wears a `_total` suffix, and that is not cosmetic.
    // `_total` is the Prometheus convention for a *counter*; all three are
    // gauges that fall when a row is deleted or a purge runs. The gauge here
    // used to be called `birdnet_detections_total` — the same name
    // `crate::metrics` gives its genuine per-species counter, which is appended
    // to this body a few lines below. One name, two `# TYPE` lines, two
    // meanings: `expfmt.TextParser` rejects the whole document on the second
    // `# HELP`, so an agent using it (promtool, Telegraf, the Python client)
    // scraped *nothing* from this station, and a Prometheus server took both
    // and `rate()`d a decreasing gauge as a counter.
    let (detection_count, species_count, rejected_count) = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            state.with_db(|conn| {
                let det: i64 = conn
                    .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
                    .unwrap_or(0);
                let sp: i64 = conn
                    .query_row(
                        "SELECT COUNT(DISTINCT Com_Name) FROM detections_analytic",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                let rej =
                    i64::try_from(birdnet_db::sqlite::rejected_detection_count(conn).unwrap_or(0))
                        .unwrap_or(0);
                (det, sp, rej)
            })
        }
    })
    .await
    .unwrap_or((0, 0, 0));

    // The station's own acoustic health, per source: the current mean noise
    // floor and how far it has moved from the same source's own 30-day
    // baseline. Exported because it is the only signal that separates "the
    // season has gone quiet" from "this microphone has gone deaf" — see
    // `birdnet_db::audio_levels`. A source with no baseline yet exports the
    // level and omits the drift, rather than exporting a drift of zero, which
    // would read as "measured, and unchanged".
    let acoustic = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            state.with_db(|conn| {
                birdnet_db::audio_levels::drift_by_source(conn, 7, 30).unwrap_or_default()
            })
        }
    })
    .await
    .unwrap_or_default();

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

    out.push_str(
        "# HELP birdnet_detections_stored Bird detections currently stored, rejections included.\n",
    );
    out.push_str("# TYPE birdnet_detections_stored gauge\n");
    writeln!(out, "birdnet_detections_stored {detection_count}").unwrap_or_default();

    out.push_str("# HELP birdnet_detections_rejected Detections a reviewer has marked rejected.\n");
    out.push_str("# TYPE birdnet_detections_rejected gauge\n");
    writeln!(out, "birdnet_detections_rejected {rejected_count}").unwrap_or_default();

    out.push_str(
        "# HELP birdnet_species_distinct Distinct species detected, excluding rejected detections.\n",
    );
    out.push_str("# TYPE birdnet_species_distinct gauge\n");
    writeln!(out, "birdnet_species_distinct {species_count}").unwrap_or_default();

    out.push_str("# HELP birdnet_process_resident_memory_bytes Resident memory size in bytes.\n");
    out.push_str("# TYPE birdnet_process_resident_memory_bytes gauge\n");
    writeln!(out, "birdnet_process_resident_memory_bytes {rss_bytes}").unwrap_or_default();

    out.push_str("# HELP birdnet_cpu_count Number of CPU cores available.\n");
    out.push_str("# TYPE birdnet_cpu_count gauge\n");
    writeln!(out, "birdnet_cpu_count {cpu_count}").unwrap_or_default();

    push_host_gauges(&mut out, &state);

    let maintenance = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            state.with_db(|conn| {
                [
                    birdnet_db::sqlite::JOB_BACKUP_VACUUM,
                    birdnet_db::sqlite::JOB_INTEGRITY_CHECK,
                    birdnet_db::sqlite::JOB_OFFSITE_BACKUP,
                ]
                .into_iter()
                .filter_map(|job| {
                    birdnet_db::sqlite::last_run_result(conn, job)
                        .ok()
                        .flatten()
                        .map(|(when, ok)| (job, when, ok))
                })
                .collect::<Vec<_>>()
            })
        }
    })
    .await
    .unwrap_or_default();
    push_maintenance_gauges(&mut out, &maintenance);

    let has_analytics: u8 = u8::from(state.has_analytics());
    out.push_str("# HELP birdnet_analytics_enabled Whether DuckDB analytics is enabled.\n");
    out.push_str("# TYPE birdnet_analytics_enabled gauge\n");
    writeln!(out, "birdnet_analytics_enabled {has_analytics}").unwrap_or_default();

    if !acoustic.is_empty() {
        out.push_str(
            "# HELP birdnet_noise_floor_dbfs Mean measured noise floor per capture source over the last 7 days, dBFS.\n",
        );
        out.push_str("# TYPE birdnet_noise_floor_dbfs gauge\n");
        for d in &acoustic {
            writeln!(
                out,
                "birdnet_noise_floor_dbfs{{source=\"{}\"}} {:.2}",
                crate::metrics::escape_label(&d.source),
                d.recent_dbfs
            )
            .unwrap_or_default();
        }
        out.push_str(
            "# HELP birdnet_noise_floor_drift_db Change in a source's mean noise floor against its own preceding 30-day baseline, dB. A large sustained negative value is a microphone going deaf.\n",
        );
        out.push_str("# TYPE birdnet_noise_floor_drift_db gauge\n");
        for d in &acoustic {
            if let Some(moved) = d.moved_db() {
                writeln!(
                    out,
                    "birdnet_noise_floor_drift_db{{source=\"{}\"}} {moved:.2}",
                    crate::metrics::escape_label(&d.source)
                )
                .unwrap_or_default();
            }
        }
    }

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

/// OP-3: disk, scratch, CPU temperature and the Pi's power mask were all
/// measured and none exported. `data` is the database's volume; `scratch`
/// the temporary directory when it is a different filesystem.
fn push_host_gauges(out: &mut String, state: &AppState) {
    let data_dir = state.db_path().parent().map_or_else(
        || std::path::PathBuf::from("."),
        std::path::Path::to_path_buf,
    );
    let data_disk = birdnet_core::audio::capture::disk_usage(&data_dir).ok();
    let scratch_disk = birdnet_core::audio::capture::disk_usage(&std::env::temp_dir())
        .ok()
        .filter(|s| {
            data_disk
                .as_ref()
                .is_none_or(|d| d.total_bytes != s.total_bytes)
        });
    if data_disk.is_some() || scratch_disk.is_some() {
        out.push_str("# HELP birdnet_disk_used_percent Space used on the volume, per cent of what this user can reach (used / (used + available)).\n");
        out.push_str("# TYPE birdnet_disk_used_percent gauge\n");
        for (volume, usage) in [("data", &data_disk), ("scratch", &scratch_disk)] {
            if let Some(u) = usage {
                writeln!(
                    out,
                    "birdnet_disk_used_percent{{volume=\"{volume}\"}} {:.2}",
                    u.used_percent()
                )
                .unwrap_or_default();
            }
        }
        out.push_str(
            "# HELP birdnet_disk_available_bytes Bytes this user can still write on the volume.\n",
        );
        out.push_str("# TYPE birdnet_disk_available_bytes gauge\n");
        for (volume, usage) in [("data", &data_disk), ("scratch", &scratch_disk)] {
            if let Some(u) = usage {
                writeln!(
                    out,
                    "birdnet_disk_available_bytes{{volume=\"{volume}\"}} {}",
                    u.available_bytes
                )
                .unwrap_or_default();
            }
        }
    }
    if let Some(temp) = crate::system_info::cpu_temperature() {
        out.push_str(
            "# HELP birdnet_cpu_temperature_celsius CPU temperature from the board's sensor.\n",
        );
        out.push_str("# TYPE birdnet_cpu_temperature_celsius gauge\n");
        writeln!(out, "birdnet_cpu_temperature_celsius {temp:.1}").unwrap_or_default();
    }
    if let Some(t) = crate::system_info::pi_throttled() {
        out.push_str("# HELP birdnet_pi_throttled_bits The Raspberry Pi firmware's get_throttled mask: bits 0-3 now (under-voltage, frequency capped, throttled, soft temperature limit), bits 16-19 since boot.\n");
        out.push_str("# TYPE birdnet_pi_throttled_bits gauge\n");
        writeln!(out, "birdnet_pi_throttled_bits {}", t.bits).unwrap_or_default();
    }
}

/// OP-3: when each scheduled job last completed and whether it succeeded,
/// for the jobs that stand between a corrupt database and a lost season.
fn push_maintenance_gauges(out: &mut String, maintenance: &[(&str, i64, Option<bool>)]) {
    if !maintenance.is_empty() {
        out.push_str("# HELP birdnet_maintenance_last_run_seconds When the scheduled job last completed, seconds since the Unix epoch.\n");
        out.push_str("# TYPE birdnet_maintenance_last_run_seconds gauge\n");
        for (job, when, _) in maintenance {
            writeln!(
                out,
                "birdnet_maintenance_last_run_seconds{{job=\"{job}\"}} {when}"
            )
            .unwrap_or_default();
        }
        if maintenance.iter().any(|(_, _, ok)| ok.is_some()) {
            out.push_str("# HELP birdnet_maintenance_last_ok Whether the scheduled job's last run succeeded (1) or failed (0); absent for a job that records no verdict.\n");
            out.push_str("# TYPE birdnet_maintenance_last_ok gauge\n");
            for (job, _, ok) in maintenance {
                if let Some(ok) = ok {
                    writeln!(
                        out,
                        "birdnet_maintenance_last_ok{{job=\"{job}\"}} {}",
                        u8::from(*ok)
                    )
                    .unwrap_or_default();
                }
            }
        }
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
