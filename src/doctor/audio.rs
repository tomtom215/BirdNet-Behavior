//! Audio-source checks: which source is configured, plus best-effort probes
//! of ALSA / PulseAudio devices and RTSP camera reachability.

use std::process::Command;
use std::time::Duration;

use birdnet_core::config::Config;

use super::{Check, tool_exists};
use crate::cli::Cli;

pub(super) fn check_audio_source(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let alsa = cli
        .alsa_device
        .clone()
        .or_else(|| config?.get("ALSA_CARD").map(String::from));
    let pulse = cli
        .pipewire_device
        .clone()
        .or_else(|| config?.get("PIPEWIRE_DEVICE").map(String::from));
    let rtsp_single = cli
        .rtsp_url
        .clone()
        .or_else(|| config?.get("RTSP_URL").map(String::from));
    let rtsp_multi = if cli.rtsp_urls.is_empty() {
        None
    } else {
        Some(cli.rtsp_urls.clone())
    };

    let configured: Vec<&str> = [
        alsa.as_deref().map(|_| "ALSA"),
        pulse.as_deref().map(|_| "PulseAudio"),
        rtsp_single.as_deref().map(|_| "RTSP"),
        rtsp_multi.as_ref().map(|_| "RTSP (multi)"),
    ]
    .into_iter()
    .flatten()
    .collect();

    let mut out = Vec::new();

    match configured.len() {
        0 => out.push(Check::warn(
            "Audio source",
            "no audio source configured (no live detections will be produced)",
            "set one of: --alsa-device, --pipewire-device, --rtsp-url, --rtsp-urls, \
             or the equivalent ALSA_CARD / RTSP_URL / PIPEWIRE_DEVICE config keys",
        )),
        1 => out.push(Check::pass(
            "Audio source",
            format!("{} source configured", configured[0]),
        )),
        n => out.push(Check::warn(
            "Audio source",
            format!("{n} audio sources configured ({}); only one will be used", configured.join(", ")),
            "remove all but one of --alsa-device / --pipewire-device / --rtsp-url to avoid surprises",
        )),
    }

    if let Some(dev) = alsa.as_deref() {
        out.push(probe_alsa_device(dev));
    }
    if let Some(url) = rtsp_single.as_deref() {
        out.push(probe_rtsp_url(url));
    }
    for url in rtsp_multi.unwrap_or_default() {
        out.push(probe_rtsp_url(&url));
    }
    if let Some(dev) = pulse.as_deref() {
        out.push(probe_pulse_source(dev));
    }

    out
}

fn probe_alsa_device(device: &str) -> Check {
    if !tool_exists("arecord") {
        return Check::skip(
            "ALSA device probe",
            "arecord not installed; cannot verify --alsa-device exists",
        );
    }
    match Command::new("arecord").arg("-l").output() {
        Ok(out) if out.status.success() => {
            let listing = String::from_utf8_lossy(&out.stdout);
            let card_part = extract_card_number(device);
            let found = card_part
                .as_deref()
                .is_some_and(|c| listing.contains(&format!("card {c}")));
            if found {
                Check::pass(
                    "ALSA device probe",
                    format!("{device} matches an entry in `arecord -l`"),
                )
            } else {
                Check::warn(
                    "ALSA device probe",
                    format!("{device} was not found in `arecord -l` output"),
                    "run `arecord -l` and check the card number; \
                     a typical USB mic shows up as `plughw:1,0`",
                )
            }
        }
        Ok(_) => Check::warn(
            "ALSA device probe",
            "`arecord -l` returned a non-zero exit code",
            "verify the running user has access to /dev/snd (member of the `audio` group)",
        ),
        Err(e) => Check::warn(
            "ALSA device probe",
            format!("could not invoke arecord: {e}"),
            "install alsa-utils (Debian/Ubuntu: `apt install alsa-utils`)",
        ),
    }
}

fn extract_card_number(device: &str) -> Option<String> {
    // Accepts forms like "plughw:1,0", "hw:1,0", "1,0", "1".
    let s = device.split(':').next_back().unwrap_or(device);
    let s = s.split(',').next().unwrap_or(s);
    if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() {
        Some(s.to_string())
    } else {
        None
    }
}

fn probe_pulse_source(source: &str) -> Check {
    if !tool_exists("pactl") {
        return Check::skip(
            "PulseAudio source probe",
            "pactl not installed; cannot verify --pipewire-device exists",
        );
    }
    match Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
    {
        Ok(out) if out.status.success() => {
            let listing = String::from_utf8_lossy(&out.stdout);
            if source == "default" || listing.contains(source) {
                Check::pass("PulseAudio source probe", format!("{source} is reachable"))
            } else {
                Check::warn(
                    "PulseAudio source probe",
                    format!("{source} not present in `pactl list short sources`"),
                    "run `pactl list short sources` and copy a name from the second column",
                )
            }
        }
        Ok(_) => Check::warn(
            "PulseAudio source probe",
            "`pactl` returned a non-zero exit code",
            "check that PulseAudio or pipewire-pulse is running for the current user",
        ),
        Err(e) => Check::warn(
            "PulseAudio source probe",
            format!("could not invoke pactl: {e}"),
            "install pulseaudio-utils (Debian/Ubuntu: `apt install pulseaudio-utils`)",
        ),
    }
}

fn probe_rtsp_url(url: &str) -> Check {
    // Lightweight TCP-port probe: parse host[:port] and try a short connect.
    // Avoids a full RTSP handshake (would need ffmpeg) but catches the most
    // common failure modes (typo, wrong port, host unreachable).
    let Some(stripped) = url
        .strip_prefix("rtsp://")
        .or_else(|| url.strip_prefix("rtsps://"))
    else {
        return Check::fail(
            "RTSP URL probe",
            format!("{url:?} does not start with rtsp:// or rtsps://"),
            "RTSP URLs must look like rtsp://camera.local:554/stream",
        );
    };

    // Drop credentials and path.
    let after_auth = stripped.rsplit_once('@').map_or(stripped, |(_, h)| h);
    let hostport = after_auth.split('/').next().unwrap_or(after_auth);
    let (host, port) = parse_host_port(hostport, 554);

    match std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port)) {
        Err(e) => Check::warn(
            "RTSP URL probe",
            format!("could not resolve {host}: {e}"),
            "check the hostname/IP and your DNS resolver",
        ),
        Ok(mut addrs) => {
            let Some(addr) = addrs.next() else {
                return Check::warn(
                    "RTSP URL probe",
                    format!("{host} resolved to no addresses"),
                    "check the hostname",
                );
            };
            match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
                Ok(_) => Check::pass(
                    "RTSP URL probe",
                    format!("TCP connect to {host}:{port} succeeded"),
                ),
                Err(e) => Check::warn(
                    "RTSP URL probe",
                    format!("TCP connect to {host}:{port} failed: {e}"),
                    "verify the camera is powered on, on the same network, and the port is correct",
                ),
            }
        }
    }
}

fn parse_host_port(hp: &str, default: u16) -> (String, u16) {
    // Handle IPv6 literals: [::1]:554
    if let Some(rest) = hp.strip_prefix('[')
        && let Some((host, after)) = rest.split_once(']')
    {
        let port = after
            .strip_prefix(':')
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(default);
        return (host.to_string(), port);
    }
    if let Some((h, p)) = hp.rsplit_once(':')
        && let Ok(port) = p.parse::<u16>()
    {
        return (h.to_string(), port);
    }
    (hp.to_string(), default)
}

#[cfg(test)]
mod tests {
    use super::{extract_card_number, parse_host_port};

    #[test]
    fn parse_host_port_basic() {
        assert_eq!(
            parse_host_port("camera.local", 554),
            ("camera.local".into(), 554)
        );
        assert_eq!(
            parse_host_port("camera.local:8554", 554),
            ("camera.local".into(), 8554)
        );
        assert_eq!(
            parse_host_port("10.0.0.1:1234", 554),
            ("10.0.0.1".into(), 1234)
        );
    }

    #[test]
    fn parse_host_port_ipv6() {
        let (h, p) = parse_host_port("[::1]:554", 554);
        assert_eq!(h, "::1");
        assert_eq!(p, 554);
        let (h2, p2) = parse_host_port("[2001:db8::1]", 554);
        assert_eq!(h2, "2001:db8::1");
        assert_eq!(p2, 554);
    }

    #[test]
    fn extract_card_number_recognises_common_forms() {
        assert_eq!(extract_card_number("plughw:1,0").as_deref(), Some("1"));
        assert_eq!(extract_card_number("hw:2,0").as_deref(), Some("2"));
        assert_eq!(extract_card_number("default"), None);
        assert_eq!(extract_card_number("USB Audio"), None);
    }
}
