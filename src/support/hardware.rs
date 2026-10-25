//! The hardware inventory in the support bundle (`hardware.json`).
//!
//! "It doesn't hear anything" and "inference takes four seconds" are hardware
//! questions, and without this the bundle answered them with `uname -a`: no CPU
//! model, no memory, no word on whether the USB sound card the station is
//! supposed to record from is plugged in or what the kernel calls it.
//! tphakala/birdnet-go's collector reports the same things (`system_info.json`,
//! `internal/support/hardware_inventory.go`); this is read from `/proc` and
//! `/sys` directly, with no new dependency and no command run.
//!
//! # What is never read
//!
//! Serial numbers. Not redacted after reading: **never opened.** A USB serial
//! is a stable identifier for a physical device, and a Raspberry Pi's
//! `/proc/cpuinfo` carries the board's serial on its own line. The USB
//! `serial` attribute, DMI's `product_serial` and `product_uuid`, the
//! `Serial` line of `cpuinfo`, and the network interfaces' MAC addresses are
//! not read; a test plants a marker in each and fails if it reaches the
//! output. Free-text names (`manufacturer`, `product`) are control-character
//! stripped and capped, because they come from firmware on a device that may
//! not be the vendor's.
//!
//! # Reading a root, not `/`
//!
//! Every function takes the directory to read under, so the tests build a
//! fixture tree and the production call passes `/`. A section that cannot be
//! read says why (`not_visible` for a missing directory, which is what a
//! container with `/sys` masked looks like, `unreadable` otherwise) rather than
//! being empty: an empty list of sound cards and an unreadable `/sys/class/sound`
//! are different findings, and the second is the one that explains a station
//! that hears nothing.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Most USB devices listed.
const MAX_USB_DEVICES: usize = 128;
/// Most sound cards listed.
const MAX_SOUND_CARDS: usize = 32;
/// Longest free-text field kept, in characters.
const MAX_TEXT_CHARS: usize = 128;
/// Most CPU feature flags listed.
const MAX_CPU_FLAGS: usize = 256;

/// Why a section is empty.
fn reason_of(err: &std::io::Error) -> &'static str {
    match err.kind() {
        std::io::ErrorKind::NotFound => "not_visible",
        _ => "unreadable",
    }
}

/// One small text file under `root`, trimmed; `None` if absent or unreadable.
fn read_text(root: &Path, rel: &str) -> Option<String> {
    let text = std::fs::read(root.join(rel)).ok()?;
    let text = String::from_utf8_lossy(&text);
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Free text from firmware: control characters dropped, whitespace collapsed,
/// capped.
fn sanitize(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TEXT_CHARS)
        .collect()
}

/// `Some(s)` only when `s` is exactly `len` hex digits.
fn hex_of(raw: Option<String>, len: usize) -> Option<String> {
    raw.filter(|s| s.len() == len && s.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A kernel USB device name: `usb1`, `1-1`, `1-1.2`.
fn is_usb_bus_id(name: &str) -> bool {
    if let Some(n) = name.strip_prefix("usb") {
        return !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    }
    let Some((bus, ports)) = name.split_once('-') else {
        return false;
    };
    !bus.is_empty()
        && bus.bytes().all(|b| b.is_ascii_digit())
        && !ports.is_empty()
        && ports
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

// ── CPU, board, memory, kernel ────────────────────────────────────────────

/// The CPU facts from `/proc/cpuinfo`, **without** the `Serial` line.
fn cpu(root: &Path) -> Value {
    let Some(text) = read_text(root, "proc/cpuinfo") else {
        return json!({ "available": false, "reason": "not_visible" });
    };
    let mut model: Option<String> = None;
    let mut hardware: Option<String> = None;
    let mut flags: Vec<String> = Vec::new();
    let mut logical = 0_u32;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        // `Serial` is not matched by anything below, and is not kept.
        match key {
            "processor" => logical += 1,
            "model name" | "Model Name" if model.is_none() => model = Some(sanitize(value)),
            "Hardware" if hardware.is_none() => hardware = Some(sanitize(value)),
            "flags" | "Features" if flags.is_empty() => {
                flags = value
                    .split_whitespace()
                    .take(MAX_CPU_FLAGS)
                    .map(sanitize)
                    .collect();
            }
            _ => {}
        }
    }
    json!({
        "available": true,
        "model": model,
        "soc": hardware,
        "logical_cpus": logical,
        "flags": flags,
    })
}

/// The board: the device tree's model on ARM, DMI's vendor and product name on
/// x86. Never `product_serial` or `product_uuid`.
fn board(root: &Path) -> Value {
    json!({
        "device_tree_model": read_text(root, "proc/device-tree/model").map(|s| sanitize(&s)),
        "vendor": read_text(root, "sys/class/dmi/id/sys_vendor").map(|s| sanitize(&s)),
        "product": read_text(root, "sys/class/dmi/id/product_name").map(|s| sanitize(&s)),
    })
}

/// `MemTotal` / `MemAvailable` from `/proc/meminfo`, in MiB.
fn memory(root: &Path) -> Value {
    let Some(text) = read_text(root, "proc/meminfo") else {
        return json!({ "available": false, "reason": "not_visible" });
    };
    let mib = |key: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(key))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb / 1024)
    };
    json!({
        "available": true,
        "total_mib": mib("MemTotal:"),
        "available_mib": mib("MemAvailable:"),
        "swap_total_mib": mib("SwapTotal:"),
    })
}

/// The memory cgroup limit this process lives under, if one applies. A limit
/// below the machine's memory is what makes a container's "out of memory"
/// differ from the host's `MemTotal`.
fn cgroup_memory(root: &Path) -> Value {
    let v2 = read_text(root, "sys/fs/cgroup/memory.max");
    let (version, limit, usage) = if let Some(max) = v2 {
        ("v2", max, read_text(root, "sys/fs/cgroup/memory.current"))
    } else if let Some(max) = read_text(root, "sys/fs/cgroup/memory/memory.limit_in_bytes") {
        (
            "v1",
            max,
            read_text(root, "sys/fs/cgroup/memory/memory.usage_in_bytes"),
        )
    } else {
        return Value::Null;
    };
    // "max" (v2) and the huge sentinel (v1) both mean "no limit".
    let limit_bytes = limit.parse::<u64>().ok().filter(|n| *n < (1_u64 << 60));
    json!({
        "version": version,
        "limit_bytes": limit_bytes,
        "usage_bytes": usage.and_then(|u| u.parse::<u64>().ok()),
    })
}

// ── USB and sound ─────────────────────────────────────────────────────────

/// The USB devices under `sys/bus/usb/devices`.
fn usb(root: &Path) -> Value {
    let dir = root.join("sys/bus/usb/devices");
    let mut names: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| is_usb_bus_id(n))
            .collect(),
        Err(e) => return json!({ "available": false, "reason": reason_of(&e) }),
    };
    names.sort();
    let truncated = names.len() > MAX_USB_DEVICES;
    names.truncate(MAX_USB_DEVICES);
    let devices: Vec<Value> = names
        .iter()
        .map(|name| {
            let attr = |a: &str| read_text(root, &format!("sys/bus/usb/devices/{name}/{a}"));
            json!({
                "bus_id": name,
                "vendor_id": hex_of(attr("idVendor"), 4),
                "product_id": hex_of(attr("idProduct"), 4),
                "manufacturer": attr("manufacturer").map(|s| sanitize(&s)),
                "product": attr("product").map(|s| sanitize(&s)),
                "speed_mbps": attr("speed").filter(|s| s.len() <= 8
                    && s.chars().all(|c| c.is_ascii_digit() || c == '.')),
                "device_class": hex_of(attr("bDeviceClass"), 2),
            })
        })
        .collect();
    json!({ "available": true, "truncated": truncated, "devices": devices })
}

/// The USB bus id that backs `card`, found by walking the `device` symlink's
/// ancestors for a name the kernel gives a USB device.
fn card_usb_bus_id(root: &Path, card: &str) -> Option<String> {
    let link: PathBuf =
        std::fs::canonicalize(root.join("sys/class/sound").join(card).join("device")).ok()?;
    link.ancestors()
        .filter_map(|p| p.file_name()?.to_str())
        .find(|n| is_usb_bus_id(n))
        .map(str::to_owned)
}

/// The ALSA cards under `sys/class/sound`, with whether each can capture and
/// which USB device carries it.
fn sound_cards(root: &Path) -> Value {
    let dir = root.join("sys/class/sound");
    let mut cards: Vec<(u32, String)> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter_map(|n| {
                let idx = n.strip_prefix("card")?.parse::<u32>().ok()?;
                (n == format!("card{idx}")).then_some((idx, n))
            })
            .collect(),
        Err(e) => return json!({ "available": false, "reason": reason_of(&e) }),
    };
    cards.sort();
    let truncated = cards.len() > MAX_SOUND_CARDS;
    cards.truncate(MAX_SOUND_CARDS);
    let list: Vec<Value> = cards
        .iter()
        .map(|(idx, name)| {
            let has_capture = std::fs::read_dir(dir.join(name)).is_ok_and(|rd| {
                rd.filter_map(|e| e.ok()?.file_name().into_string().ok())
                    .any(|n| {
                        n.strip_prefix("pcmC")
                            .and_then(|r| r.strip_suffix('c'))
                            .is_some_and(|mid| {
                                mid.split_once('D').is_some_and(|(c, d)| {
                                    !c.is_empty()
                                        && !d.is_empty()
                                        && c.bytes().all(|b| b.is_ascii_digit())
                                        && d.bytes().all(|b| b.is_ascii_digit())
                                })
                            })
                    })
            });
            json!({
                "index": idx,
                "id": read_text(root, &format!("sys/class/sound/{name}/id")).map(|s| sanitize(&s)),
                "usb_bus_id": card_usb_bus_id(root, name),
                "has_capture": has_capture,
            })
        })
        .collect();
    json!({ "available": true, "truncated": truncated, "cards": list })
}

// ── the whole inventory ───────────────────────────────────────────────────

/// The inventory under `root` as pretty JSON.
#[must_use]
pub fn inventory_json(root: &Path) -> String {
    let doc = json!({
        "note": "Read from /proc and /sys. No serial number is read: not the USB \
                 devices', not the board's, not a network card's.",
        "kernel": read_text(root, "proc/sys/kernel/osrelease"),
        "architecture": std::env::consts::ARCH,
        "cpu": cpu(root),
        "board": board(root),
        "memory": memory(root),
        "cgroup_memory": cgroup_memory(root),
        "usb": usb(root),
        "sound_cards": sound_cards(root),
    });
    let mut out = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_owned());
    let _ = writeln!(out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Marker planted in every field that must never be read.
    const SECRET: &str = "SERIALNUMBER-9f3a77";

    fn put(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    /// A Raspberry Pi 5 with a USB microphone on `1-1.2`, its serial planted
    /// everywhere a serial could be read from.
    fn pi_with_usb_mic() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        put(
            r,
            "proc/cpuinfo",
            &format!(
                "processor\t: 0\nBogoMIPS\t: 108.00\nFeatures\t: fp asimd evtstrm crc32 cpuid asimdrdm\n\n\
                 processor\t: 1\nFeatures\t: fp asimd\n\n\
                 Hardware\t: BCM2712\nRevision\t: d04170\nSerial\t\t: {SECRET}\nModel\t\t: Raspberry Pi 5 Model B Rev 1.0\n"
            ),
        );
        put(
            r,
            "proc/device-tree/model",
            "Raspberry Pi 5 Model B Rev 1.0\0",
        );
        put(
            r,
            "proc/meminfo",
            "MemTotal:        8123456 kB\nMemFree: 1 kB\nMemAvailable:    6000000 kB\nSwapTotal:  2097148 kB\n",
        );
        put(r, "proc/sys/kernel/osrelease", "6.6.51+rpt-rpi-2712\n");
        put(r, "sys/fs/cgroup/memory.max", "max\n");
        put(r, "sys/fs/cgroup/memory.current", "123456789\n");
        for (id, vid, pid, mfr, prod) in [
            (
                "usb1",
                "1d6b",
                "0002",
                "Linux 6.6 xhci-hcd",
                "xHCI Host Controller",
            ),
            (
                "1-1.2",
                "0d8c",
                "0014",
                "C-Media Electronics Inc.",
                "USB Audio Device",
            ),
        ] {
            let base = format!("sys/bus/usb/devices/{id}");
            put(r, &format!("{base}/idVendor"), &format!("{vid}\n"));
            put(r, &format!("{base}/idProduct"), &format!("{pid}\n"));
            put(r, &format!("{base}/manufacturer"), &format!("{mfr}\n"));
            put(r, &format!("{base}/product"), &format!("{prod}\n"));
            put(r, &format!("{base}/speed"), "480\n");
            put(r, &format!("{base}/bDeviceClass"), "00\n");
            put(r, &format!("{base}/serial"), &format!("{SECRET}\n"));
        }
        // card1 is the USB microphone; card0 the Pi's HDMI audio (no capture).
        std::fs::create_dir_all(r.join("sys/devices/platform/usb/1-1/1-1.2/1-1.2:1.0")).unwrap();
        put(r, "sys/class/sound/card0/id", "vc4hdmi0\n");
        std::fs::create_dir_all(r.join("sys/class/sound/card0/pcmC0D0p")).unwrap();
        put(r, "sys/class/sound/card1/id", "Device\n");
        std::fs::create_dir_all(r.join("sys/class/sound/card1/pcmC1D0c")).unwrap();
        std::os::unix::fs::symlink(
            r.join("sys/devices/platform/usb/1-1/1-1.2/1-1.2:1.0"),
            r.join("sys/class/sound/card1/device"),
        )
        .unwrap();
        put(r, "sys/class/dmi/id/product_serial", &format!("{SECRET}\n"));
        put(r, "sys/class/dmi/id/product_uuid", &format!("{SECRET}\n"));
        put(r, "sys/class/net/eth0/address", &format!("{SECRET}\n"));
        dir
    }

    fn doc(root: &Path) -> Value {
        serde_json::from_str(&inventory_json(root)).expect("valid JSON")
    }

    #[test]
    fn a_pi_with_a_usb_microphone_is_described() {
        let dir = pi_with_usb_mic();
        let d = doc(dir.path());
        assert_eq!(d["kernel"], "6.6.51+rpt-rpi-2712");
        assert_eq!(d["cpu"]["logical_cpus"], 2);
        assert_eq!(d["cpu"]["soc"], "BCM2712");
        assert_eq!(d["cpu"]["flags"][0], "fp");
        assert_eq!(
            d["board"]["device_tree_model"],
            "Raspberry Pi 5 Model B Rev 1.0"
        );
        assert_eq!(d["memory"]["total_mib"], 7933);
        assert_eq!(d["memory"]["available_mib"], 5859);
        assert_eq!(d["cgroup_memory"]["version"], "v2");
        assert_eq!(
            d["cgroup_memory"]["limit_bytes"],
            Value::Null,
            "`max` is no limit"
        );
        assert_eq!(d["cgroup_memory"]["usage_bytes"], 123_456_789_u64);

        let usb = d["usb"]["devices"].as_array().unwrap();
        assert_eq!(usb.len(), 2);
        let mic = usb
            .iter()
            .find(|u| u["bus_id"] == "1-1.2")
            .expect("the microphone");
        assert_eq!(mic["vendor_id"], "0d8c");
        assert_eq!(mic["product_id"], "0014");
        assert_eq!(mic["product"], "USB Audio Device");
        assert_eq!(mic["speed_mbps"], "480");

        let cards = d["sound_cards"]["cards"].as_array().unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0]["id"], "vc4hdmi0");
        assert_eq!(cards[0]["has_capture"], false, "HDMI audio cannot record");
        assert_eq!(cards[1]["id"], "Device");
        assert_eq!(cards[1]["has_capture"], true);
        assert_eq!(
            cards[1]["usb_bus_id"], "1-1.2",
            "the card is tied to its USB device"
        );
        assert_eq!(cards[0]["usb_bus_id"], Value::Null);
    }

    /// The point of the module: no serial reaches the bundle. Each location a
    /// serial could be read from carries the same marker, and the marker must
    /// appear nowhere in the output.
    #[test]
    fn no_serial_number_is_read_from_anywhere() {
        let dir = pi_with_usb_mic();
        // Sanity: the fixture really does hold the marker in six places, so an
        // absent marker below means "not read", not "not planted".
        let planted = [
            "sys/bus/usb/devices/1-1.2/serial",
            "sys/bus/usb/devices/usb1/serial",
            "sys/class/dmi/id/product_serial",
            "sys/class/dmi/id/product_uuid",
            "sys/class/net/eth0/address",
        ]
        .iter()
        .filter(|p| std::fs::read_to_string(dir.path().join(p)).is_ok_and(|s| s.contains(SECRET)))
        .count();
        assert_eq!(planted, 5);
        assert!(
            std::fs::read_to_string(dir.path().join("proc/cpuinfo"))
                .unwrap()
                .contains(SECRET)
        );
        let out = inventory_json(dir.path());
        assert!(
            !out.contains(SECRET),
            "a serial number reached the bundle:\n{out}"
        );
        assert!(!out.to_lowercase().contains("serial\":"), "{out}");
    }

    #[test]
    fn firmware_text_is_stripped_of_control_characters_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        put(
            dir.path(),
            "sys/bus/usb/devices/1-1/product",
            &format!("Mic\u{1b}[31m\u{0}\u{7}  Pro\n{}", "x".repeat(500)),
        );
        let d = doc(dir.path());
        let product = d["usb"]["devices"][0]["product"].as_str().unwrap();
        assert!(!product.chars().any(char::is_control), "{product:?}");
        assert!(product.starts_with("Mic [31m Pro x"), "{product:?}");
        assert_eq!(product.chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn a_vendor_id_that_is_not_four_hex_digits_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        put(
            dir.path(),
            "sys/bus/usb/devices/1-1/idVendor",
            "ZZZZ; rm -rf\n",
        );
        put(dir.path(), "sys/bus/usb/devices/1-1/idProduct", "0014\n");
        let d = doc(dir.path());
        assert_eq!(d["usb"]["devices"][0]["vendor_id"], Value::Null);
        assert_eq!(d["usb"]["devices"][0]["product_id"], "0014");
    }

    #[test]
    fn a_directory_name_that_is_not_a_usb_device_is_ignored() {
        for ok in ["usb1", "1-1", "1-1.2", "2-3.4.5"] {
            assert!(is_usb_bus_id(ok), "{ok}");
        }
        for bad in [
            "1-1:1.0", "usb", "usbX", "-1", "1-", "1-1.", "../x", "1-1/2", "",
        ] {
            assert!(!is_usb_bus_id(bad), "{bad}");
        }
    }

    #[test]
    fn a_masked_sys_says_why_it_is_empty() {
        // The container shape: /proc present, /sys/bus and /sys/class absent.
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "proc/meminfo", "MemTotal: 2048 kB\n");
        let d = doc(dir.path());
        assert_eq!(d["usb"]["available"], false);
        assert_eq!(d["usb"]["reason"], "not_visible");
        assert_eq!(d["sound_cards"]["reason"], "not_visible");
        assert_eq!(d["cpu"]["available"], false);
        // And the section that *was* readable still is.
        assert_eq!(d["memory"]["total_mib"], 2);
    }

    #[test]
    fn a_present_but_empty_sound_directory_is_not_the_same_as_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sys/class/sound")).unwrap();
        let d = doc(dir.path());
        assert_eq!(d["sound_cards"]["available"], true);
        assert_eq!(d["sound_cards"]["cards"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_cgroup_limit_is_reported_when_there_is_one() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "sys/fs/cgroup/memory.max", "1073741824\n");
        assert_eq!(
            doc(dir.path())["cgroup_memory"]["limit_bytes"],
            1_073_741_824_u64
        );
    }

    /// The real machine this runs on: not a fixture, so a parse that works on
    /// hand-written files and fails on a kernel's own is caught. It asserts only
    /// what any Linux host has.
    #[test]
    #[cfg(target_os = "linux")]
    fn the_real_proc_parses() {
        let d = doc(Path::new("/"));
        assert!(d["kernel"].is_string(), "{d}");
        assert!(d["memory"]["total_mib"].as_u64().unwrap_or(0) > 0, "{d}");
        assert!(d["cpu"]["logical_cpus"].as_u64().unwrap_or(0) > 0, "{d}");
    }
}
