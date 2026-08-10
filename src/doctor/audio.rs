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

    // What capture will ACTUALLY use, which is not necessarily what the config
    // says. `capture.rs` seeds the `audio_sources` table from CLI/config only
    // while that table is EMPTY, and from then on the table is the source of
    // truth ("so the table is the single source of truth for capture and the
    // web surface"). An operator who edits ALSA_CARD on an established station
    // changes nothing, and a diagnostic that reads only the config cheerfully
    // validates a device the daemon will never open.
    //
    // Measured on a Raspberry Pi 4: config edited to `plughw:CARD=PRO,DEV=0`,
    // service restarted, and the journal kept reporting
    // `started microphone capture device=plughw:1,0` from the table — with the
    // gauge at 0 and no recording, for an hour, while every configuration file
    // on the box said the right thing.
    //
    // This is the same shape as the CADDY_PWD defect: two readers of one
    // setting, disagreeing, with the diagnostic reading the wrong one. It is
    // resolved the same way — by consulting what the runtime consults.
    let db_sources = effective_alsa_devices(config);
    match db_sources.as_deref() {
        Some(devices) if !devices.is_empty() => {
            for dev in devices {
                out.push(probe_alsa_device(dev));
            }
            if let Some(cfg_dev) = alsa.as_deref()
                && !devices.iter().any(|d| d == cfg_dev)
            {
                out.push(Check::warn(
                    "Audio source (config vs database)",
                    format!(
                        "the config says `{cfg_dev}` but capture uses `{}` from the audio_sources table",
                        devices.join("`, `")
                    ),
                    "the table wins once it has rows — editing the config will not change capture. \
                     Change the device on the /admin/audio page (or clear the table to re-seed \
                     from the config on next start)",
                ));
            }
        }
        // No rows: the config really is what capture will use, because the
        // table gets seeded from it on the next start.
        _ => {
            if let Some(dev) = alsa.as_deref() {
                out.push(probe_alsa_device(dev));
            }
        }
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

/// ALSA devices the capture path will use, read from the same `audio_sources`
/// table it reads, via the same [`AudioSourceStore::list`] query.
///
/// Deliberately shares the query rather than restating the rule: two copies of
/// "where does the device come from" is exactly how the config and the runtime
/// drifted apart in the first place.
///
/// Returns `None` when the table cannot be consulted at all — no database yet,
/// unreadable, or corrupt. That is not a finding here: `check_database` owns
/// the database's health, and a doctor that failed because the DB was corrupt
/// would block the startup that repairs it, which is the trap this release
/// spent its time removing.
fn effective_alsa_devices(config: Option<&Config>) -> Option<Vec<String>> {
    use birdnet_db::audio_sources::{AudioSourceStore, SourceKind};

    let db_path = crate::helpers::db_path_from_config(config);
    if !db_path.exists() {
        return None;
    }
    // Read-only: the diagnostic must never take a write lock on a station's
    // live database, and ExecStartPre runs this while nothing else holds it.
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    let rows = conn.list().ok()?;
    Some(
        rows.into_iter()
            .filter(|s| s.kind == SourceKind::UsbAlsa)
            .map(|s| s.device_id)
            .collect(),
    )
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
            match parse_card_ref(device) {
                Some(card) if listing_has_card(&listing, &card) => Check::pass(
                    "ALSA device probe",
                    format!("{device} matches an entry in `arecord -l`"),
                ),
                // An index that is absent is the failure this release exists to
                // stop being silent, so the remedy names the stable form and
                // the card actually present rather than "check the number".
                Some(CardRef::Index(idx)) => Check::warn(
                    "ALSA device probe",
                    format!("{device} refers to card {idx}, which is not in `arecord -l`"),
                    stable_form_hint(&listing).unwrap_or_else(|| {
                        "run `arecord -l`: no capture card is present at all".to_string()
                    }),
                ),
                Some(CardRef::Id(id)) => Check::warn(
                    "ALSA device probe",
                    format!("{device} names card id `{id}`, which is not in `arecord -l`"),
                    "run `arecord -l` and compare the id (the word after `card N:`); \
                     if you pinned it with usb-audio-mapper, check the udev rule applied"
                        .to_string(),
                ),
                None => Check::skip(
                    "ALSA device probe",
                    format!("cannot tell which card `{device}` refers to; not verifying it"),
                ),
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

/// How a configured ALSA device string identifies its card.
///
/// Two forms reach us, and they are checked against `arecord -l` differently:
///
/// * an **index** (`plughw:1,0`) — assigned in detection order, and not stable:
///   the same microphone was `card 1` before a cold reboot on a Raspberry Pi 4
///   and `card 3` after it;
/// * an **id** (`plughw:CARD=PRO,DEV=0`) — the name ALSA gives the card, which
///   `usb-audio-mapper` pins via a udev rule (`ATTR{id}="<name>"`).
///
/// The id form used to be unrecognised here, so it fell through to "not found
/// in `arecord -l`" and warned on every startup — the diagnostic telling
/// operators that the robust configuration was the broken one.
#[derive(Debug, PartialEq, Eq)]
enum CardRef {
    Index(String),
    Id(String),
}

/// Parse the card out of an ALSA device string.
///
/// Accepts `plughw:1,0`, `hw:1,0`, `1,0`, `1`, and the id form
/// `plughw:CARD=PRO,DEV=0` / `hw:CARD=PRO`. Returns `None` for anything else
/// (`default`, a PipeWire node name, empty), which the caller reports as
/// unverifiable rather than as broken.
fn parse_card_ref(device: &str) -> Option<CardRef> {
    // Strip a leading PCM plugin name ("plughw:", "hw:") if present. Split on
    // the FIRST colon: an id could in principle contain one, and everything
    // after it belongs to the argument list.
    let args = match device.split_once(':') {
        Some((_plugin, rest)) => rest,
        None => device,
    };

    // Named-argument form: CARD=<id>[,DEV=<n>][,SUBDEV=<n>]. alsa-lib declares
    // CARD as `type string` in its own alsa.conf, so the value is a name.
    for field in args.split(',') {
        if let Some(id) = field.trim().strip_prefix("CARD=") {
            let id = id.trim();
            if !id.is_empty() {
                return Some(CardRef::Id(id.to_string()));
            }
            return None;
        }
    }

    // Positional form: the card is the first argument.
    let first = args.split(',').next().unwrap_or(args).trim();
    if !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) {
        return Some(CardRef::Index(first.to_string()));
    }
    None
}

/// First capture card in an `arecord -l` listing, as `(index, id, device)`.
///
/// A line reads `card 3: PRO [Comica_Traxshot PRO], device 0: USB Audio [...]`.
fn first_card(listing: &str) -> Option<(String, String, String)> {
    listing.lines().map(str::trim_start).find_map(|line| {
        let rest = line.strip_prefix("card ")?;
        let (index, tail) = rest.split_once(':')?;
        let id = tail.split_whitespace().next()?;
        // `device N:` appears after the card's description on the same line.
        let device = tail
            .split_once("device ")
            .and_then(|(_, d)| d.split(':').next())
            .map(str::trim)
            .filter(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or("0");
        Some((index.trim().to_string(), id.to_string(), device.to_string()))
    })
}

/// Suggest a device string for a card that IS present, so a wrong `ALSA_CARD`
/// is answered with a line to paste rather than an instruction to go and work
/// it out. Returns `None` when the listing holds no capture card at all.
fn stable_form_hint(listing: &str) -> Option<String> {
    let (index, id, device) = first_card(listing)?;
    let portable = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if portable {
        Some(format!(
            "card {index} is present with id `{id}` — set ALSA_CARD=plughw:CARD={id},DEV={device} \
             and run `sudo bash install.sh repair`. The id survives the re-enumeration that moved \
             the index; see docs/book/admin/audio.md"
        ))
    } else {
        Some(format!(
            "card {index} is present — set ALSA_CARD=plughw:{index},{device} and run \
             `sudo bash install.sh repair`. Its id is not usable as a name, so consider \
             usb-audio-mapper to pin one; see docs/book/admin/audio.md"
        ))
    }
}

/// Does `arecord -l` output list the card this device string refers to?
///
/// Matching is line-anchored on purpose. The previous check asked whether the
/// listing merely *contained* `"card 1"`, which is also true of `card 12:` — a
/// station configured for a card that is absent could be told it was present.
fn listing_has_card(listing: &str, card: &CardRef) -> bool {
    listing.lines().map(str::trim_start).any(|line| {
        let Some(rest) = line.strip_prefix("card ") else {
            return false;
        };
        // "card 3: PRO [Comica_Traxshot PRO], device 0: ..."
        let Some((index, tail)) = rest.split_once(':') else {
            return false;
        };
        match card {
            CardRef::Index(want) => index.trim() == want,
            CardRef::Id(want) => tail.split_whitespace().next() == Some(want.as_str()),
        }
    })
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
    use super::{
        CardRef, check_audio_source, effective_alsa_devices, first_card, listing_has_card,
        parse_card_ref, parse_host_port, probe_rtsp_url, stable_form_hint,
    };
    use crate::cli::Cli;
    use crate::doctor::Status;
    use clap::Parser as _;

    #[test]
    fn check_audio_source_none_configured_warns() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let checks = check_audio_source(&cli, None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "Audio source");
        assert_eq!(checks[0].status, Status::Warn);
    }

    #[test]
    fn check_audio_source_single_source_passes_summary() {
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.rtsp_url = Some("rtsp://nonexistent.invalid:554/stream".into());
        let checks = check_audio_source(&cli, None);
        // First check is the source summary (exactly one source → pass); the
        // RTSP probe follows. We don't assert the probe verdict (it depends on
        // the host's resolvability), only that the summary is a pass.
        assert_eq!(checks[0].name, "Audio source");
        assert_eq!(checks[0].status, Status::Pass);
        assert!(checks.iter().any(|c| c.name == "RTSP URL probe"));
    }

    #[test]
    fn check_audio_source_multiple_sources_warn() {
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.alsa_device = Some("plughw:1,0".into());
        cli.rtsp_url = Some("rtsp://nonexistent.invalid:554/s".into());
        let checks = check_audio_source(&cli, None);
        assert_eq!(checks[0].name, "Audio source");
        assert_eq!(checks[0].status, Status::Warn);
        assert!(checks[0].message.contains('2'));
    }

    #[test]
    fn probe_rtsp_url_rejects_non_rtsp_scheme() {
        assert_eq!(probe_rtsp_url("http://example.com/s").status, Status::Fail);
    }

    #[test]
    fn probe_rtsp_url_warns_on_unresolvable_host() {
        // `.invalid` is reserved and never resolves (RFC 6761), so the probe
        // takes the resolution-failure branch deterministically.
        let c = probe_rtsp_url("rtsp://nonexistent.invalid:554/stream");
        assert_eq!(c.status, Status::Warn);
    }

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

    /// Captured verbatim from a Raspberry Pi 4 BEFORE a cold reboot.
    const PI_BEFORE: &str = "\
**** List of CAPTURE Hardware Devices ****
card 1: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
  Subdevices: 1/1
  Subdevice #0: subdevice #0
";

    /// The SAME microphone on the SAME board AFTER the reboot. The index moved
    /// from 1 to 3; the id did not move. This pair is the entire argument for
    /// addressing a card by id, and it is real rather than constructed.
    const PI_AFTER: &str = "\
**** List of CAPTURE Hardware Devices ****
card 3: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
  Subdevices: 1/1
  Subdevice #0: subdevice #0
";

    #[test]
    fn parse_card_ref_recognises_common_forms() {
        assert_eq!(
            parse_card_ref("plughw:1,0"),
            Some(CardRef::Index("1".into()))
        );
        assert_eq!(parse_card_ref("hw:2,0"), Some(CardRef::Index("2".into())));
        assert_eq!(parse_card_ref("1,0"), Some(CardRef::Index("1".into())));
        assert_eq!(parse_card_ref("1"), Some(CardRef::Index("1".into())));
        assert_eq!(parse_card_ref("default"), None);
        assert_eq!(parse_card_ref("USB Audio"), None);
        assert_eq!(parse_card_ref(""), None);
    }

    #[test]
    fn parse_card_ref_recognises_the_id_form() {
        assert_eq!(
            parse_card_ref("plughw:CARD=PRO,DEV=0"),
            Some(CardRef::Id("PRO".into()))
        );
        assert_eq!(
            parse_card_ref("hw:CARD=PRO"),
            Some(CardRef::Id("PRO".into()))
        );
        assert_eq!(
            parse_card_ref("plughw:CARD=Scarlett,DEV=2,SUBDEV=0"),
            Some(CardRef::Id("Scarlett".into()))
        );
        // A blank name identifies nothing; better unverifiable than wrong.
        assert_eq!(parse_card_ref("plughw:CARD=,DEV=0"), None);
    }

    #[test]
    fn id_form_matches_the_real_listing_at_either_index() {
        let card = parse_card_ref("plughw:CARD=PRO,DEV=0").expect("id form parses");
        // The regression this fixes: before, the id form was unparseable, so it
        // warned "not found in arecord -l" on every startup — against a card
        // that was sitting right there, at whichever index.
        assert!(listing_has_card(PI_BEFORE, &card));
        assert!(listing_has_card(PI_AFTER, &card));
    }

    #[test]
    fn index_form_is_exactly_as_fragile_as_the_hardware_showed() {
        let card = parse_card_ref("plughw:1,0").expect("index form parses");
        assert!(listing_has_card(PI_BEFORE, &card));
        // Same mic, same board, one reboot later — and the configured card is
        // gone. This is the field failure, reproduced.
        assert!(!listing_has_card(PI_AFTER, &card));
    }

    #[test]
    fn index_match_is_line_anchored_not_substring() {
        let listing = "**** List of CAPTURE Hardware Devices ****\n\
                       card 12: PRO [Some Device], device 0: USB Audio [USB Audio]\n";
        let one = parse_card_ref("plughw:1,0").expect("parses");
        // `listing.contains("card 1")` is true here, and was the old test.
        assert!(listing.contains("card 1"));
        assert!(!listing_has_card(listing, &one));
        let twelve = parse_card_ref("plughw:12,0").expect("parses");
        assert!(listing_has_card(listing, &twelve));
    }

    #[test]
    fn id_match_does_not_fire_on_a_description_word() {
        // "PRO" also appears inside the bracketed description; only the id
        // token immediately after `card N:` may satisfy the check.
        let listing = "**** List of CAPTURE Hardware Devices ****\n\
                       card 0: Headset [PRO Gaming Headset], device 0: USB Audio [USB Audio]\n";
        let pro = parse_card_ref("plughw:CARD=PRO,DEV=0").expect("parses");
        assert!(listing.contains("PRO"));
        assert!(!listing_has_card(listing, &pro));
    }

    #[test]
    fn hint_names_the_card_that_is_actually_present() {
        let hint = stable_form_hint(PI_AFTER).expect("a card is present");
        assert!(
            hint.contains("plughw:CARD=PRO,DEV=0"),
            "hint should be pasteable, got: {hint}"
        );
        assert!(hint.contains("card 3"), "hint should name the index found");
        assert_eq!(
            stable_form_hint("**** List of CAPTURE Hardware Devices ****\n"),
            None
        );
    }

    #[test]
    fn hint_falls_back_when_the_id_is_not_a_usable_name() {
        let listing = "**** List of CAPTURE Hardware Devices ****\n\
                       card 1: My.Odd$Card [Weird], device 2: USB Audio [USB Audio]\n";
        let hint = stable_form_hint(listing).expect("a card is present");
        assert!(hint.contains("plughw:1,2"), "got: {hint}");
        // Must not suggest the id form for an id that cannot be used as one.
        // Checked against `plughw:CARD=` rather than the bare substring
        // `CARD=`, which the config key `ALSA_CARD=` also contains.
        assert!(!hint.contains("plughw:CARD="), "got: {hint}");
        assert!(hint.contains("usb-audio-mapper"), "got: {hint}");
    }

    /// Build a real on-disk database carrying one ALSA source, and a config
    /// pointing `DB_PATH` at it. Nothing is mocked: the check opens this file
    /// and runs the same `list()` query the capture path runs.
    fn db_with_alsa_source(dir: &std::path::Path, device: &str) -> birdnet_core::config::Config {
        use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
        let db_path = dir.join("birds.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        conn.insert(&NewAudioSource::defaults(
            "src_seed_1".to_string(),
            SourceKind::UsbAlsa,
            device.to_string(),
        ))
        .unwrap();
        crate::helpers::test_support::config_with(&[("DB_PATH", db_path.to_str().unwrap())])
    }

    #[test]
    fn effective_devices_come_from_the_table_not_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = db_with_alsa_source(dir.path(), "plughw:1,0");
        assert_eq!(
            effective_alsa_devices(Some(&config)),
            Some(vec!["plughw:1,0".to_string()])
        );
    }

    #[test]
    fn no_database_yet_is_not_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::helpers::test_support::config_with(&[(
            "DB_PATH",
            dir.path().join("absent.db").to_str().unwrap(),
        )]);
        // The config is what capture will use, because the table gets seeded
        // from it on first start. Must not be reported as a divergence.
        assert_eq!(effective_alsa_devices(Some(&config)), None);
    }

    #[test]
    fn divergence_between_config_and_table_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        // The stale index the Pi actually carried while its config said
        // CARD=PRO — the exact state that produced an hour of silence.
        let db_path = dir.path().join("birds.db");
        db_with_alsa_source(dir.path(), "plughw:1,0");
        let config = crate::helpers::test_support::config_with(&[
            ("DB_PATH", db_path.to_str().unwrap()),
            ("ALSA_CARD", "plughw:CARD=PRO,DEV=0"),
        ]);

        let cli = Cli::parse_from(["birdnet-behavior"]);
        let checks = check_audio_source(&cli, Some(&config));
        let divergence = checks
            .iter()
            .find(|c| c.name == "Audio source (config vs database)")
            .expect("the disagreement must be reported");
        assert_eq!(divergence.status, Status::Warn);
        assert!(
            divergence.message.contains("plughw:CARD=PRO,DEV=0")
                && divergence.message.contains("plughw:1,0"),
            "both values must be named, got: {}",
            divergence.message
        );
    }

    #[test]
    fn agreement_between_config_and_table_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("birds.db");
        {
            use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            conn.insert(&NewAudioSource::defaults(
                "src_seed_1".to_string(),
                SourceKind::UsbAlsa,
                "plughw:CARD=PRO,DEV=0".to_string(),
            ))
            .unwrap();
        }
        let config = crate::helpers::test_support::config_with(&[
            ("DB_PATH", db_path.to_str().unwrap()),
            ("ALSA_CARD", "plughw:CARD=PRO,DEV=0"),
        ]);
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let checks = check_audio_source(&cli, Some(&config));
        assert!(
            !checks
                .iter()
                .any(|c| c.name == "Audio source (config vs database)"),
            "no disagreement to report when the two agree"
        );
    }

    #[test]
    fn first_card_reads_index_id_and_device() {
        assert_eq!(
            first_card(PI_AFTER),
            Some(("3".into(), "PRO".into(), "0".into()))
        );
        let nonzero = "card 2: Scarlett [Focusrite Scarlett 2i2], device 2: USB Audio [x]\n";
        assert_eq!(
            first_card(nonzero),
            Some(("2".into(), "Scarlett".into(), "2".into()))
        );
    }
}
