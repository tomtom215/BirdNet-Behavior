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

/// The minimum gap sysinfo requires between two CPU refreshes for the delta
/// between them to mean anything.
const MIN_CPU_DELTA: std::time::Duration = std::time::Duration::from_millis(200);

/// How long a sampled snapshot is reused before the `System` is refreshed again.
///
/// Small enough that the System page is live, large enough that a page with
/// several system-reading fragments refreshes once rather than once per
/// fragment.
const SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_secs(2);

/// The process-wide sampler: one live `System` handle plus its last snapshot.
///
/// Keeping the handle alive is the whole point — see [`sample`].
struct Sampler {
    sys: System,
    taken_at: std::time::Instant,
    snapshot: SystemSnapshot,
}

static SAMPLER: std::sync::LazyLock<std::sync::Mutex<Option<Sampler>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

/// Sample current system metrics.
///
/// # Why this does not sleep any more
///
/// It used to. The body was:
///
/// ```text
/// sys.refresh_cpu_usage();
/// std::thread::sleep(Duration::from_millis(200));   // <-- in the request
/// sys.refresh_cpu_usage();
/// ```
///
/// because sysinfo reports CPU usage as the delta between two refreshes *of the
/// same `System` value*, and a freshly constructed one has no previous refresh
/// to subtract. Constructing a new `System` per call therefore forced a sleep
/// per call. Measured against the seeded fixture, that made `/station` take
/// **238 ms** serially where `/patterns` took 4 ms — 60× the next slowest page,
/// all of it sleep. `/pages/station-health-line`, which the Today page polls
/// `every 60s`, paid it too: on a kiosk left on that page, 200 ms of a blocking
/// thread once a minute, forever, to render one temperature and one percentage.
///
/// The function's own doc comment already said what to do — "for a live
/// dashboard, call this function on a background task with a regular interval"
/// — and six call sites did the opposite.
///
/// Keeping one `System` alive fixes it without a background task: the delta is
/// then "since the previous caller", which for a page polled once a minute is a
/// *better* window than a 200 ms instantaneous slice. Only the very first call
/// in a process has no predecessor, and only that call waits.
///
/// # Panics
///
/// Does not panic. Returns a zeroed snapshot on any internal error, and
/// recovers from a poisoned lock rather than propagating it — a wedged system
/// readout must not take the health page down.
pub fn sample() -> SystemSnapshot {
    let mut guard = SAMPLER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Some(sampler) = guard.as_mut() {
        if sampler.taken_at.elapsed() < SNAPSHOT_TTL {
            return sampler.snapshot.clone();
        }
        sampler.snapshot = refresh_into_snapshot(&mut sampler.sys);
        sampler.taken_at = std::time::Instant::now();
        return sampler.snapshot.clone();
    }

    // First call in this process. sysinfo 0.39 renamed `RefreshKind::new()` to
    // `RefreshKind::nothing()` because the previous name was misleading (the
    // value was actually empty, not "default everything").
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    sys.refresh_cpu_usage();
    // The one remaining sleep, once per process, so the first reader gets a real
    // number instead of 0 %.
    std::thread::sleep(MIN_CPU_DELTA);
    let snapshot = refresh_into_snapshot(&mut sys);
    *guard = Some(Sampler {
        sys,
        taken_at: std::time::Instant::now(),
        snapshot: snapshot.clone(),
    });
    snapshot
}

/// Refresh `sys` and read a snapshot out of it.
///
/// Split out so the first-call and steady-state paths in [`sample`] cannot
/// drift in what they read.
fn refresh_into_snapshot(sys: &mut System) -> SystemSnapshot {
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    let cpus = sys.cpus();
    let cpu_cores: Vec<f32> = cpus.iter().map(sysinfo::Cpu::cpu_usage).collect();
    #[allow(clippy::cast_precision_loss)]
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

/// The CPU temperature in degrees Celsius, on its own.
///
/// Two callers want only this — the Today rail's station line
/// (`routes::pages::health`) and the station-health integration — and both used
/// to reach it through [`sample`], paying that function's 200 ms CPU-delta
/// sleep and a full CPU + memory refresh for what is, on the deployment target,
/// a single `read_to_string` of a sysfs file. Nothing about a temperature needs
/// two samples.
///
/// Returns `None` where no sensor is exposed, which is normal in containers and
/// on several cloud VMs.
#[must_use]
pub fn cpu_temperature() -> Option<f32> {
    sample_cpu_temperature()
}

/// Try to read CPU temperature from sysinfo components.
///
/// Returns `None` if not available (many cloud VMs and containers don't expose this).
fn sample_cpu_temperature() -> Option<f32> {
    // sysinfo Components API requires a separate refresh.
    //
    // sysinfo 0.39 changes two contracts here vs. the 0.32 we used to be on:
    //   1. `refresh()` now takes a `bool` (`true` to remove components that
    //      vanished between refreshes). We pass `true` so a hotplugged
    //      sensor that disappears doesn't leave a stale reading.
    //   2. `Component::temperature()` returns `Option<f32>` instead of
    //      `f32` — many cloud VMs and containers expose a component with
    //      no thermal reading, and the old API conflated "no sensor" with
    //      "0 °C". We propagate the inner None with `.flatten()`.
    use sysinfo::{Component, Components};
    let mut components = Components::new_with_refreshed_list();
    components.refresh(true);

    let from_sysinfo = components
        .iter()
        .find(|c: &&Component| {
            let label = c.label().to_ascii_lowercase();
            label.contains("cpu") || label.contains("core") || label.contains("package")
        })
        .and_then(|c: &Component| c.temperature());

    // sysinfo's component sensors are routinely empty on a Raspberry Pi (and
    // several other ARM SBCs), where the CPU temperature is exposed only through
    // the thermal-zone sysfs. Fall back to that so the System page shows a real
    // reading on a Pi instead of a blank.
    #[cfg(target_os = "linux")]
    {
        from_sysinfo.or_else(read_thermal_zone_temp)
    }
    #[cfg(not(target_os = "linux"))]
    {
        from_sysinfo
    }
}

/// Read the CPU temperature from the Linux thermal-zone sysfs (°C).
///
/// Prefers a zone whose `type` names the CPU/SoC (e.g. `cpu-thermal` on a
/// Raspberry Pi, `x86_pkg_temp` on x86); otherwise takes the first zone, since
/// `thermal_zone0` is the package/CPU on most boards. Zone temperatures are in
/// millidegrees Celsius; implausible values are rejected so a bogus sensor never
/// renders a wild reading.
#[cfg(target_os = "linux")]
fn read_thermal_zone_temp() -> Option<f32> {
    let mut zones: Vec<std::path::PathBuf> = std::fs::read_dir("/sys/class/thermal")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("thermal_zone"))
        })
        .collect();
    zones.sort();

    let cpu_zone = zones.iter().find(|z| {
        std::fs::read_to_string(z.join("type")).is_ok_and(|t| {
            let t = t.to_ascii_lowercase();
            t.contains("cpu") || t.contains("soc") || t.contains("x86_pkg")
        })
    });
    let zone = cpu_zone.or_else(|| zones.first())?;

    let milli: f32 = std::fs::read_to_string(zone.join("temp"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let celsius = milli / 1000.0;
    (celsius > -40.0 && celsius < 150.0).then_some(celsius)
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

/// Process uptime in seconds, or `None` when it can't be determined.
///
/// Returns `None` on non-Linux targets or when `/proc` is unreadable. Derived
/// from the process start time in `/proc/self/stat` (field 22, in jiffies)
/// against `/proc/uptime`.
#[must_use]
pub fn process_uptime_secs() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
        let hz: u64 = 100; // typical USER_HZ on Linux
        let start_jiffies: u64 = stat.split_whitespace().nth(21)?.parse().ok()?;
        let sys_uptime: f64 = uptime.split_whitespace().next()?.parse().ok()?;
        #[allow(clippy::cast_precision_loss)] // hz division is small
        let proc_uptime = sys_uptime - (start_jiffies / hz) as f64;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some(proc_uptime.max(0.0) as u64)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_temperature_is_plausible_or_absent() {
        // Whichever path is taken (a sysinfo component sensor or the Linux
        // thermal-zone fallback), the reading must be a sane Celsius value or
        // None — never a panic and never a wild number from a misread sensor.
        if let Some(t) = sample_cpu_temperature() {
            assert!(t > -40.0 && t < 150.0, "implausible CPU temperature: {t}");
        }
    }

    /// Sampling must not sleep in the request path.
    ///
    /// `sample()` used to construct a fresh `System` per call and
    /// `thread::sleep(200ms)` between its two CPU refreshes, so every reader of
    /// this module — `/station`, `/admin/system`, `/admin/overview`, and the
    /// Today rail's 60-second poll — paid 200 ms of a blocking thread. Measured
    /// on the seeded fixture, `/station` served in 238 ms against 4 ms for
    /// `/patterns`.
    ///
    /// The bound is deliberately generous. Five calls at the old cost is 1 s;
    /// 300 ms leaves room for the one first-call sleep (which the warm-up below
    /// absorbs) and for a slow CI box, while still being an order of magnitude
    /// under a regression to per-call sleeping.
    #[test]
    fn repeated_sampling_does_not_sleep() {
        let _warm = sample(); // absorbs the once-per-process delta wait

        let started = std::time::Instant::now();
        for _ in 0..5 {
            let _ = sample();
        }
        let elapsed = started.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(300),
            "five samples took {elapsed:?} — that is a sleep per call, not a \
             live System handle"
        );
    }

    /// The counterpart. "Fast" is trivially satisfiable by returning zeros, so
    /// the snapshot has to still describe the machine.
    #[test]
    fn a_fast_sample_is_still_a_real_one() {
        let snap = sample();
        assert!(snap.cpu_count > 0, "no CPUs reported");
        assert!(snap.total_memory_bytes > 0, "no memory reported");
        assert!(!snap.cpu_cores.is_empty(), "no per-core readings");
        assert!(
            (0.0..=100.0).contains(&snap.memory_usage_pct),
            "memory {}% is not a percentage",
            snap.memory_usage_pct
        );
        assert!(
            (0.0..=100.0).contains(&snap.cpu_usage_pct),
            "cpu {}% is not a percentage",
            snap.cpu_usage_pct
        );
    }

    /// Reading only the temperature must not go through the CPU sampler at all.
    ///
    /// Two call sites want exactly this, and both used to reach it through
    /// `sample()`. This runs *before* any warm-up in its own process would
    /// help — a `cargo test` process may schedule it first — so a regression to
    /// `sample().cpu_temp_celsius` shows up here as the full first-call wait.
    #[test]
    fn cpu_temperature_alone_never_waits() {
        let started = std::time::Instant::now();
        let _ = cpu_temperature();
        let elapsed = started.elapsed();
        assert!(
            elapsed < MIN_CPU_DELTA,
            "reading a temperature took {elapsed:?} — it went through the CPU \
             delta sampler"
        );
    }

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

    #[cfg(target_os = "linux")]
    #[test]
    fn process_uptime_is_available_on_linux() {
        // The test process has been running, so `/proc` yields a value. We only
        // assert it resolves (the exact seconds are timing-dependent).
        assert!(
            process_uptime_secs().is_some(),
            "expected /proc-derived process uptime on Linux"
        );
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
        // Per-core readings must exist, not just the average — the CPU vital
        // and the Pi-health page both render them.
        assert_eq!(
            snap.cpu_cores.len(),
            snap.cpu_count,
            "expected one usage reading per core"
        );
    }

    /// The range assertions above are all satisfied by a sampler that always
    /// returns `0.0`, so on their own they cannot tell a working CPU monitor
    /// from a dead one. Only putting the machine under load and demanding the
    /// number move can. Added after a report that the CPU monitor looked
    /// broken on a Pi: it was not — measured against `/proc/stat` the reading
    /// agrees exactly (2 % idle, 100 % saturated) — but nothing in the suite
    /// could have told us either way.
    ///
    /// # It polls, and the first version did not
    ///
    /// This used to sleep 150 ms and take a single reading, with a comment
    /// asserting it was "not flaky by construction". That was wrong, and the
    /// construction was mine: `sample()` serves a cached snapshot for
    /// [`SNAPSHOT_TTL`], and every other test in this binary shares that cache.
    /// `cargo test` runs them in parallel in one process, so a single call can
    /// hand back a reading taken up to two seconds *before* these threads
    /// existed. CI caught it on `main`:
    ///
    /// ```text
    /// CPU sampled at 24.9% while every core was spinning
    /// ```
    ///
    /// Reproduced deterministically by warming the cache first, which is the
    /// same thing a neighbouring test does by accident:
    ///
    /// ```text
    /// let _warm = sample();   // then the old body
    /// reproduced: CPU sampled at 0.0% while every core was spinning
    /// ```
    ///
    /// So it polls until a refresh lands whose whole delta window sits inside
    /// the load — which needs two refreshes at least `SNAPSHOT_TTL` apart, not
    /// one call. The TTL is not the bug; it is the point of the shared handle,
    /// and a test that ignores it is testing a sampler this crate does not
    /// ship.
    ///
    /// A contended runner still only makes the reading *higher*, and the 25 %
    /// floor stays far below the ~100 % these threads produce.
    #[test]
    fn cpu_usage_rises_under_load() {
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let workers: Vec<_> = (0..cores)
            .map(|_| {
                let stop = std::sync::Arc::clone(&stop);
                std::thread::spawn(move || {
                    let mut x: u64 = 0;
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        x = x.wrapping_mul(31).wrapping_add(7);
                    }
                    x
                })
            })
            .collect();

        // Poll rather than sample once: see the note above. Half a TTL between
        // reads guarantees a refresh every other iteration, so a window wholly
        // inside the load arrives within a few seconds. The deadline is far
        // longer than that — it exists so a genuinely broken sampler fails the
        // test rather than hanging it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut peak = 0.0_f32;
        while std::time::Instant::now() < deadline {
            peak = peak.max(sample().cpu_usage_pct);
            if peak > 25.0 {
                break;
            }
            std::thread::sleep(SNAPSHOT_TTL / 2);
        }

        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for w in workers {
            let _ = w.join();
        }

        assert!(
            peak > 25.0,
            "peak CPU was {peak:.1}% across 30s with every core spinning — the \
             sampler is not measuring anything"
        );
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
