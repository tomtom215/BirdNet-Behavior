//! Input-level checks: what the operating system says a microphone's capture
//! gain is, graded against the guide's cap.
//!
//! Read-only. The station never changes a mixer: a level set at run time does
//! not survive a replug or a power cut unless someone with root also stores it,
//! and the right level is whatever keeps the loudest bird below full scale,
//! which only the recorded signal can say. The remedy is therefore a command
//! for the operator to read and run, printed beside the finding. Every tool runs
//! under a deadline, and a finding here is at most a warning: it makes
//! `--doctor` exit 1, and it cannot stop the service starting, because the start
//! gate (`--doctor-gate`) is blocked only by failures of the checks it names and
//! this is not one. A mixer setting is no reason to stop recording. The page
//! `docs/book/field/input-level.md` gives the reasons.

use std::fmt::Write as _;
use std::process::Command;
use std::time::Duration;

use birdnet_core::audio::capture::alsa::{CardRef, parse_card_ref};
use birdnet_core::audio::input_level::{
    Advice, Backend, CAP_HARDWARE_DB, CAP_PERCENT, InputLevel, assess, command_to_reach_cap,
    default_source, parse_amixer_scontents, parse_pactl_source,
};
use birdnet_core::process::run_with_timeout;

use super::{Check, tool_exists};

const NAME: &str = "Input level";

/// How long `amixer` or `pactl` may take. Both answer in milliseconds when they
/// can; `pactl` with no server it can reach retries its connection, which is a
/// finding, not a reason to hang.
const TOOL_TIMEOUT: Duration = Duration::from_secs(5);

const GUIDE: &str = "docs/book/field/input-level.md (Input Level in the manual)";

/// The capture gain of an ALSA device (`plughw:1,0`, `hw:CARD=PRO`).
pub(super) fn alsa_level(device: &str) -> Check {
    let Some(CardRef::Index(card) | CardRef::Id(card)) = parse_card_ref(device) else {
        return Check::skip(
            NAME,
            format!("{device} is not a card the doctor can name, so its mixer was not read"),
        );
    };
    // The id goes to `amixer` as one argument and never through a shell, so this is not a
    // defence against injection; it keeps to the alphabet ALSA ids use, so an odd string
    // from the configuration is reported as what it is rather than handed on.
    if !card
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    {
        return Check::skip(
            NAME,
            format!(
                "{device}: the card name has characters other than letters, digits and `_-.:`, so its mixer was not read"
            ),
        );
    }
    if !tool_exists("amixer") {
        return Check::skip(
            NAME,
            "amixer is not installed, so the capture gain was not read (alsa-utils provides it)",
        );
    }
    match run_with_timeout(
        Command::new("amixer")
            .env("LC_ALL", "C")
            .args(["-c", &card, "scontents"]),
        TOOL_TIMEOUT,
    ) {
        Ok(out) if out.status.success() => {
            grade_alsa(device, &card, &String::from_utf8_lossy(&out.stdout))
        }
        Ok(out) => Check::skip(
            NAME,
            format!(
                "amixer could not read card {card}: {}",
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .last()
                    .unwrap_or("no message")
                    .trim()
            ),
        ),
        Err(e) => Check::skip(NAME, format!("could not run amixer: {e}")),
    }
}

/// The capture gain of a PulseAudio or PipeWire source (`PIPEWIRE_DEVICE`).
pub(super) fn pulse_level(source: &str) -> Check {
    if !tool_exists("pactl") {
        return Check::skip(
            NAME,
            "pactl is not installed, so the source's volume was not read",
        );
    }
    let run = |args: &[&str]| {
        run_with_timeout(
            Command::new("pactl").env("LC_ALL", "C").args(args),
            TOOL_TIMEOUT,
        )
    };
    let name = if source == "default" {
        match run(&["info"]) {
            Ok(out) if out.status.success() => {
                match default_source(&String::from_utf8_lossy(&out.stdout)) {
                    Some(name) => name,
                    None => return Check::skip(NAME, "pactl names no default source"),
                }
            }
            Ok(_) | Err(_) => return Check::skip(NAME, "pactl could not reach the sound server"),
        }
    } else {
        source.to_owned()
    };
    match run(&["--format=json", "list", "sources"]) {
        Ok(out) if out.status.success() => {
            grade_pulse(&name, &String::from_utf8_lossy(&out.stdout))
        }
        Ok(_) => Check::skip(
            NAME,
            "pactl could not list sources (is the sound server running for this user?)",
        ),
        Err(e) => Check::skip(NAME, format!("could not run pactl: {e}")),
    }
}

/// `'Mic' 50% (+11.9 dB)`.
fn describe(level: &InputLevel) -> String {
    // A PulseAudio source is named in the finding already; an ALSA control is not.
    let mut text = match level.backend {
        Backend::Alsa => format!("'{}' at {}%", level.control, level.percent),
        Backend::Pulse => format!("at {}%", level.percent),
    };
    if let Some(db) = level.db {
        let _ = write!(text, " ({db:+.1} dB)");
    }
    if level.muted == Some(true) {
        text.push_str(", capture switched off");
    }
    text
}

const fn severity(advice: Advice) -> u8 {
    match advice {
        Advice::Ok => 0,
        Advice::AboveCap => 1,
        Advice::AtMaximum => 2,
        Advice::Muted => 3,
    }
}

/// The verdict on the controls `amixer -c <card> scontents` printed.
fn grade_alsa(device: &str, card: &str, scontents: &str) -> Check {
    let levels = parse_amixer_scontents(scontents);
    let Some(worst) = levels.iter().max_by_key(|l| severity(assess(l))) else {
        return Check::skip(
            NAME,
            format!(
                "{device}: card {card} has no capture volume control (a fixed-gain device, or one the driver does not expose)"
            ),
        );
    };
    let list = levels.iter().map(describe).collect::<Vec<_>>().join("; ");
    let both = if levels.len() > 1 {
        " (their gains add: the signal passes through each)"
    } else {
        ""
    };
    verdict(
        device,
        &list,
        both,
        worst,
        command_to_reach_cap(worst, card),
        &format!("alsamixer -c {card}"),
    )
}

/// The verdict on one source in `pactl --format=json list sources`.
fn grade_pulse(source: &str, json: &str) -> Check {
    let level = match parse_pactl_source(json, source) {
        Ok(level) => level,
        Err(e) => return Check::skip(NAME, format!("{source}: {e}")),
    };
    if source.ends_with(".monitor") {
        return Check::warn(
            NAME,
            format!("{source} is a monitor of an output, not a microphone"),
            "its level is the speakers' and says nothing about a recording; choose the \
             microphone's source from `pactl list short sources`",
        );
    }
    let mut list = describe(&level);
    if let Some(hw) = level.hardware_db.filter(|_| level.hardware_controlled) {
        let _ = write!(list, ", {hw:+.1} dB on the hardware's own 0 dB");
    }
    let command = command_to_reach_cap(&level, &level.control);
    verdict(source, &list, "", &level, command, "pavucontrol")
}

fn verdict(
    device: &str,
    list: &str,
    note: &str,
    worst: &InputLevel,
    command: Option<String>,
    mixer: &str,
) -> Check {
    match assess(worst) {
        Advice::Ok => Check::pass(
            NAME,
            format!(
                "{device}: {list}{note}; not above the guide's cap of {CAP_PERCENT}% or +{CAP_HARDWARE_DB:.0} dB \
                 (the cap is a guard rail, not proof the level is right: check a recording's peak, see {GUIDE})"
            ),
        ),
        Advice::Muted => Check::warn(
            NAME,
            format!(
                "{device}: {list}{note}: nothing reaches the station while capture is switched off"
            ),
            format!(
                "switch capture on in `{mixer}` (or `amixer sset '<control>' cap`), then see {GUIDE}"
            ),
        ),
        advice => {
            let how = match advice {
                Advice::AtMaximum => "at the control's maximum",
                _ => "above the guide's cap",
            };
            let fix = command.map_or_else(
                || {
                    format!(
                        "a 16-bit recording clips for good at a level this high. Lower it in `{mixer}`, \
                         then check a recording's peak, see {GUIDE}"
                    )
                },
                |cmd| {
                    format!(
                        "a 16-bit recording clips for good at a level this high. To reach the cap, run `{cmd}` \
                         (it lasts until the card is unplugged or the machine loses power; storing it needs root, \
                         see {GUIDE}), then check a recording's peak"
                    )
                },
            );
            Check::warn(NAME, format!("{device}: {list}{note}: {how}"), fix)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{grade_alsa, grade_pulse};
    use crate::doctor::Status;

    /// The same fixtures `crates/birdnet-core/tests/input_level_readings.rs` parses; see the README beside them
    /// for which are captured and which are published.
    macro_rules! fixture {
        ($name:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/crates/birdnet-core/tests/data/input_level/",
                $name
            ))
        };
    }
    const USB: &str = fixture!("amixer_scontents_published_usb_codec.txt");
    const PA16: &str = fixture!("pactl_list_sources_pulseaudio_16.1.json");
    const PIPEWIRE: &str = fixture!("pactl_list_sources_pipewire_pulse_1.0.5.json");

    #[test]
    fn a_level_under_the_cap_passes_and_says_the_cap_proves_nothing() {
        let check = grade_alsa("plughw:1,0", "1", USB);
        assert_eq!(check.status, Status::Pass, "{check:?}");
        assert!(
            check.message.contains("'Mic' at 50% (+11.9 dB)"),
            "{}",
            check.message
        );
        assert!(
            check.message.contains("not proof the level is right"),
            "{}",
            check.message
        );
        assert!(check.remediation.is_none());
    }

    #[test]
    fn a_control_at_its_maximum_is_a_warning_with_a_command_to_read_and_run() {
        let top = USB.replace("Capture 8 [50%] [11.90dB]", "Capture 16 [100%] [23.81dB]");
        let check = grade_alsa("plughw:1,0", "1", &top);
        assert_eq!(
            check.status,
            Status::Warn,
            "a mixer setting must never stop the unit: {check:?}"
        );
        assert!(
            check.message.contains("at the control's maximum"),
            "{}",
            check.message
        );
        let fix = check.remediation.expect("a remedy");
        assert!(
            fix.contains("amixer -c 1 -- sset 'Mic' capture 12"),
            "{fix}"
        );
        assert!(fix.contains("clips for good"), "{fix}");
    }

    #[test]
    fn with_several_capture_controls_the_worst_one_decides() {
        // The published USB `Mic` is fine at 50%; a second control, `Capture`, at its maximum is not.
        let two = format!(
            "{USB}Simple mixer control 'Capture',0\n  Capabilities: cvolume cswitch\n  Limits: Capture 0 - 31\n  Front Left: Capture 31 [100%] [30.00dB] [on]\n"
        );
        let check = grade_alsa("plughw:1,0", "1", &two);
        assert_eq!(check.status, Status::Warn, "{check:?}");
        assert!(
            check.message.contains("'Capture' at 100%"),
            "{}",
            check.message
        );
        assert!(
            check.message.contains("'Mic' at 50%"),
            "all controls are listed: {}",
            check.message
        );
        assert!(
            check.message.contains("their gains add"),
            "{}",
            check.message
        );
    }

    #[test]
    fn a_virtual_source_is_not_described_as_hardware() {
        let check = grade_pulse("virt_mic", PA16);
        assert!(!check.message.contains("hardware"), "{}", check.message);
    }

    #[test]
    fn a_muted_capture_is_a_warning_that_names_the_cause() {
        let off = USB.replace(
            "Capture 8 [50%] [11.90dB] [on]",
            "Capture 8 [50%] [11.90dB] [off]",
        );
        let check = grade_alsa("plughw:1,0", "1", &off);
        assert_eq!(check.status, Status::Warn);
        assert!(
            check.message.contains("capture is switched off"),
            "{}",
            check.message
        );
    }

    #[test]
    fn a_card_name_outside_the_alsa_alphabet_is_reported_and_not_handed_to_amixer() {
        for device in [
            "plughw:CARD=a b,DEV=0",
            "plughw:CARD=$(id),DEV=0",
            "hw:CARD=x;y",
        ] {
            let check = super::alsa_level(device);
            assert_eq!(check.status, Status::Skip, "{device}: {check:?}");
            assert!(
                check.message.contains("characters other than"),
                "{device}: {}",
                check.message
            );
        }
        // A plain id still gets as far as the tool (which is absent or finds no such card here).
        let plain = super::alsa_level("plughw:CARD=PRO,DEV=0");
        assert!(
            !plain.message.contains("characters other than"),
            "{}",
            plain.message
        );
    }

    #[test]
    fn a_card_without_a_capture_volume_is_skipped_not_graded() {
        let speaker_only = USB
            .split("Simple mixer control 'Mic',0")
            .next()
            .unwrap_or_default();
        let check = grade_alsa("plughw:1,0", "1", speaker_only);
        assert_eq!(check.status, Status::Skip, "{check:?}");
        assert!(
            check.message.contains("no capture volume control"),
            "{}",
            check.message
        );
    }

    #[test]
    fn a_pulse_source_is_graded_and_a_monitor_is_called_what_it_is() {
        let check = grade_pulse("virt_mic", PA16);
        assert_eq!(check.status, Status::Pass, "{check:?}");
        let monitor = grade_pulse("dummy_sink.monitor", PA16);
        assert_eq!(monitor.status, Status::Warn, "{monitor:?}");
        assert!(
            monitor.message.contains("not a microphone"),
            "{}",
            monitor.message
        );
    }

    #[test]
    fn a_muted_pipewire_source_is_a_warning() {
        let check = grade_pulse("virt_mic", PIPEWIRE);
        assert_eq!(check.status, Status::Warn, "{check:?}");
        assert!(
            check.message.contains("capture switched off"),
            "{}",
            check.message
        );
    }

    #[test]
    fn an_unreadable_listing_is_skipped_with_the_reason() {
        for (source, listing, expect) in [
            ("virt_mic", "not json", "did not print JSON"),
            ("nope", PA16, "no source named nope"),
        ] {
            let check = grade_pulse(source, listing);
            assert_eq!(check.status, Status::Skip, "{check:?}");
            assert!(check.message.contains(expect), "{}", check.message);
        }
    }
}
