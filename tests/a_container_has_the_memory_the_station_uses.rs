//! A container must be allowed the memory the station actually uses.
//!
//! # The defect
//!
//! `docker-compose.yml` capped the container at 512 MB, and the ALSA and
//! Pulse overlays at 768 MB, on the stated belief that the model is
//! memory-mapped and its pages reclaimable. It is not: ONNX Runtime loads the
//! weights into anonymous memory, which a cgroup cannot reclaim. Measured with
//! the real BirdNET+ V3.0 FP32 model and analytics on (debug build, x86_64):
//! 573 MB anonymous at idle, 651 MB while analysing a 30 s recording, 831 MB
//! peak RSS. A 512 MB limit is below the anonymous memory alone, so the
//! container is killed as the model loads, restarts, and is killed again.
//!
//! # What is guarded
//!
//! Every shipped compose file's memory limit is at least the systemd unit's
//! `MemoryMax`, the ceiling the bare-metal station is sized to and runs under.
//! The unit's figure is read, not retyped, so the two cannot drift apart.

use std::path::Path;

/// `512M`, `1G`, `768m` — as Compose and systemd write sizes — in bytes.
fn bytes(size: &str) -> u64 {
    let size = size.trim().trim_matches('"');
    let (number, unit) = size.split_at(size.len() - 1);
    let n: u64 = number.parse().unwrap_or_else(|_| panic!("not a size: {size}"));
    n * match unit.to_ascii_uppercase().as_str() {
        "K" => 1 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        other => panic!("unknown size unit {other} in {size}"),
    }
}

#[test]
fn every_container_may_use_what_the_station_is_sized_to() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let unit = std::fs::read_to_string(root.join("installer/lib/65-service.sh")).unwrap();
    let memory_max = unit
        .lines()
        .find_map(|l| l.strip_prefix("MemoryMax="))
        .map(bytes)
        .expect("the unit sets MemoryMax");

    let mut limits = Vec::new();
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("docker-compose") && name.ends_with(".yml")) {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).unwrap();
        let mut in_limits = false;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('#') {
                continue;
            }
            if t == "limits:" {
                in_limits = true;
            } else if in_limits && let Some(v) = t.strip_prefix("memory:") {
                limits.push((name.clone(), v.trim().to_owned()));
                in_limits = false;
            } else if t.ends_with(':') {
                in_limits = false;
            }
        }
    }
    assert!(
        limits.len() >= 3,
        "precondition: the base file and both overlays set a limit: {limits:?}"
    );
    let short: Vec<_> = limits
        .iter()
        .filter(|(_, v)| bytes(v) < memory_max)
        .collect();
    assert!(
        short.is_empty(),
        "below the unit's MemoryMax ({memory_max} bytes), where the model alone does not fit: {short:?}"
    );
}
