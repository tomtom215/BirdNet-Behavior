//! System monitoring: CPU, memory, load average, and temperature.
//!
//! Uses the `sysinfo` crate to query live system metrics without any unsafe
//! code or direct `/proc` reads.  All reads are synchronous and should be
//! called from a `spawn_blocking` context.

use serde::Serialize;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

/// Snapshot of current system resource usage.
#[derive(Debug, Clone, Serialize)]
pub struct SystemSnapshot {
    /// CPU usage averaged across all cores (0.0–100.0).
    pub cpu_usage_pct: f32,
    /// Per-core CPU usage percentages.
    pub cpu_cores: Vec<f32>,
    /// Total physical memory in bytes.
    pub total_memory_bytes: u64,
    /// Used physical memory in bytes.
    pub used_memory_bytes: u64,
    /// Available (free + reclaimable) memory in bytes.
    pub available_memory_bytes: u64,
    /// Memory usage percentage (0.0–100.0).
    pub memory_usage_pct: f32,
    /// Number of logical CPU cores.
    pub cpu_count: usize,
    /// System uptime in seconds.
    pub uptime_secs: u64,
    /// CPU temperature in degrees Celsius, if available.
    pub cpu_temp_celsius: Option<f32>,
}

impl SystemSnapshot {
    /// Whether memory usage is critically high (> 90 %).
    pub fn is_memory_critical(&self) -> bool {
        self.memory_usage_pct > 90.0
    }

    /// Whether CPU is heavily loaded (> 80 %).
    pub fn is_cpu_high(&self) -> bool {
        self.cpu_usage_pct > 80.0
    }

    /// Human-readable memory usage string.
    pub fn memory_summary(&self) -> String {
        format!(
            "{} / {} ({:.0}%)",
            format_bytes(self.used_memory_bytes),
            format_bytes(self.total_memory_bytes),
            self.memory_usage_pct,
        )
    }
}

/// Sample current system metrics.
///
/// Note: CPU usage requires two samples to be meaningful (the first sample
/// returns 0 % on most platforms).  For a live dashboard, call this function
/// on a background task with a regular interval.
///
/// # Panics
///
/// Does not panic.  Returns a zeroed snapshot on any internal error.
pub fn sample() -> SystemSnapshot {
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );

    // Two-pass CPU measurement (sleep briefly for delta)
    sys.refresh_cpu_usage();
    std::thread::sleep(std::time::Duration::from_millis(200));
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    let cpus = sys.cpus();
    let cpu_cores: Vec<f32> = cpus.iter().map(|c| c.cpu_usage()).collect();
    let cpu_usage_pct = if cpu_cores.is_empty() {
        0.0
    } else {
        cpu_cores.iter().sum::<f32>() / cpu_cores.len() as f32
    };

    let total = sys.total_memory();
    let used = sys.used_memory();
    let available = sys.available_memory();

    #[allow(clippy::cast_precision_loss)]
    let memory_usage_pct = if total > 0 {
        used as f32 / total as f32 * 100.0
    } else {
        0.0
    };

    let uptime_secs = System::uptime();
    let cpu_count = sys.cpus().len();

    // Temperature (optional — not available on all platforms)
    let cpu_temp_celsius = sample_cpu_temperature();

    SystemSnapshot {
        cpu_usage_pct,
        cpu_cores,
        total_memory_bytes: total,
        used_memory_bytes: used,
        available_memory_bytes: available,
        memory_usage_pct,
        cpu_count,
        uptime_secs,
        cpu_temp_celsius,
    }
}

/// Try to read CPU temperature from sysinfo components.
///
/// Returns `None` if not available (many cloud VMs and containers don't expose this).
fn sample_cpu_temperature() -> Option<f32> {
    // sysinfo Components API requires a separate refresh
    use sysinfo::{Component, Components};
    let mut components = Components::new_with_refreshed_list();
    components.refresh();

    components
        .iter()
        .find(|c: &&Component| {
            let label = c.label().to_ascii_lowercase();
            label.contains("cpu") || label.contains("core") || label.contains("package")
        })
        .map(|c: &Component| c.temperature())
}

/// Format bytes as human-readable string.
pub fn format_bytes(bytes: u64) -> String {
    const GIB: u64 = 1_073_741_824;
    const MIB: u64 = 1_048_576;
    const KIB: u64 = 1_024;

    #[allow(clippy::cast_precision_loss)]
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format uptime in seconds as a human-readable duration string.
pub fn format_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_gib() {
        assert_eq!(format_bytes(2_147_483_648), "2.0 GiB");
    }

    #[test]
    fn format_bytes_mib() {
        assert_eq!(format_bytes(10_485_760), "10.0 MiB");
    }

    #[test]
    fn format_bytes_kib() {
        assert_eq!(format_bytes(2_048), "2.0 KiB");
    }

    #[test]
    fn format_bytes_b() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn format_uptime_days() {
        assert_eq!(format_uptime(90_061), "1d 1h 1m");
    }

    #[test]
    fn format_uptime_hours() {
        assert_eq!(format_uptime(3_660), "1h 1m");
    }

    #[test]
    fn format_uptime_minutes() {
        assert_eq!(format_uptime(125), "2m");
    }

    #[test]
    fn sample_snapshot_sanity() {
        let snap = sample();
        // CPU count should be positive on any real machine
        assert!(snap.cpu_count > 0);
        // Memory should be positive
        assert!(snap.total_memory_bytes > 0);
        // Uptime should be positive
        assert!(snap.uptime_secs > 0);
        // Usage percentages should be in range
        assert!(snap.cpu_usage_pct >= 0.0);
        assert!(snap.cpu_usage_pct <= 100.0);
        assert!(snap.memory_usage_pct >= 0.0);
        assert!(snap.memory_usage_pct <= 100.0);
    }

    #[test]
    fn snapshot_critical_thresholds() {
        // Fabricate a snapshot to test threshold methods
        let snap = SystemSnapshot {
            cpu_usage_pct: 95.0,
            cpu_cores: vec![95.0],
            total_memory_bytes: 1000,
            used_memory_bytes: 950,
            available_memory_bytes: 50,
            memory_usage_pct: 95.0,
            cpu_count: 1,
            uptime_secs: 3600,
            cpu_temp_celsius: Some(72.0),
        };
        assert!(snap.is_cpu_high());
        assert!(snap.is_memory_critical());
    }

    #[test]
    fn snapshot_memory_summary_format() {
        let snap = SystemSnapshot {
            cpu_usage_pct: 10.0,
            cpu_cores: vec![10.0],
            total_memory_bytes: 8_589_934_592, // 8 GiB
            used_memory_bytes: 4_294_967_296,  // 4 GiB
            available_memory_bytes: 4_294_967_296,
            memory_usage_pct: 50.0,
            cpu_count: 4,
            uptime_secs: 3600,
            cpu_temp_celsius: None,
        };
        let summary = snap.memory_summary();
        assert!(summary.contains("4.0 GiB"));
        assert!(summary.contains("8.0 GiB"));
        assert!(summary.contains("50%"));
    }
}
