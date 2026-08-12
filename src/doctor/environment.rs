//! Runtime-environment checks: CPU, temp directory, and optional CLI tools.

use birdnet_core::config::Config;

use super::{Check, tool_exists, writable};
use crate::cli::Cli;

pub(super) fn check_runtime_environment() -> Vec<Check> {
    let mut out = Vec::new();

    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    if cores >= 2 {
        out.push(Check::pass(
            "CPU cores",
            format!("{cores} cores available for audio + inference"),
        ));
    } else {
        out.push(Check::warn(
            "CPU cores",
            format!("only {cores} CPU core detected"),
            "BirdNet-Behavior runs on a single core but real-time inference \
             benefits from at least 2 cores. Consider an upgrade if detections lag.",
        ));
    }

    out.push(check_temp_directory());
    out
}

fn check_temp_directory() -> Check {
    let tmp = std::env::temp_dir();
    if tmp.exists() && writable(&tmp) {
        Check::pass("Temp directory", format!("{} is writable", tmp.display()))
    } else {
        Check::fail(
            "Temp directory",
            format!("{} is not writable", tmp.display()),
            "set TMPDIR to a writable location, or check filesystem permissions",
        )
    }
}

pub(super) fn check_optional_tools(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let mut out = Vec::new();

    let fmt = cli.audio_format.to_ascii_lowercase();
    if fmt != "wav" {
        let has_ff = tool_exists("ffmpeg");
        let has_sox = tool_exists("sox");
        if has_ff || has_sox {
            out.push(Check::pass(
                "Audio encoder",
                format!(
                    "found {} for {fmt} encoding",
                    if has_ff { "ffmpeg" } else { "sox" }
                ),
            ));
        } else {
            out.push(Check::fail(
                "Audio encoder",
                format!("--audio-format {fmt} requires ffmpeg or sox but neither is installed"),
                "install ffmpeg (`apt install ffmpeg`) or fall back to --audio-format wav",
            ));
        }
    }

    // Live capture needs ffmpeg for RTSP on every platform, and for the
    // microphone on macOS (which captures through ffmpeg's avfoundation input
    // rather than ALSA's arecord). A configured source with no ffmpeg means no
    // detections, so this is a hard error rather than a warning.
    //
    // A Linux microphone never needs ffmpeg for *capture*, whatever its gain:
    // the gain used to route capture through `ffmpeg -f alsa`, which this
    // predicate did not account for (so a gain-configured station with no
    // ffmpeg was reported healthy and then failed to spawn). The gain is now
    // applied in-process, so the predicate and the runtime agree again.
    let rtsp = cli.rtsp_url.is_some()
        || !cli.rtsp_urls.is_empty()
        || config.is_some_and(|c| c.get("RTSP_URL").is_some_and(|v| !v.is_empty()));
    let mic = cli.alsa_device.is_some()
        || cli.pipewire_device.is_some()
        || config.is_some_and(|c| {
            c.get("ALSA_CARD").is_some_and(|v| !v.is_empty())
                || c.get("PIPEWIRE_DEVICE").is_some_and(|v| !v.is_empty())
        });
    let needs_ffmpeg_capture = rtsp || (cfg!(target_os = "macos") && mic);

    // Live audio (`GET /stream`) shells out to ffmpeg for *every* source kind,
    // including the plain ALSA path — but the installer only ensures ffmpeg for
    // RTSP capture, and this check only ran under the same condition. The
    // result on the commonest station of all (Linux + USB microphone): the
    // Listen → Live tab returns 500 on every request, and `--doctor` reports
    // the station entirely healthy. Capture and detection are genuinely fine
    // there, so this is a warning about a broken feature rather than a failed
    // station — the hard error below still covers the cases where capture
    // itself cannot run without ffmpeg.
    if !needs_ffmpeg_capture && mic && !tool_exists("ffmpeg") {
        out.push(Check::warn(
            "Live audio (ffmpeg)",
            "live audio streaming needs ffmpeg, which is not installed — capture \
             and detection are unaffected, but Listen → Live will not play",
            "install ffmpeg (`sudo apt install ffmpeg`), then reload the dashboard",
        ));
    }

    if needs_ffmpeg_capture {
        if tool_exists("ffmpeg") {
            out.push(Check::pass(
                "Capture backend (ffmpeg)",
                "ffmpeg is available for live capture",
            ));
        } else {
            let why = if rtsp {
                "RTSP capture"
            } else {
                "macOS microphone capture (avfoundation)"
            };
            out.push(Check::fail(
                "Capture backend (ffmpeg)",
                format!("{why} requires ffmpeg but it is not installed"),
                "install ffmpeg (macOS: `brew install ffmpeg`; Debian/Ubuntu: `apt install ffmpeg`)",
            ));
        }
    }

    if cli.freq_shift_hz != 0 {
        if tool_exists("ffmpeg") || tool_exists("sox") {
            out.push(Check::pass(
                "Frequency-shift backend",
                "ffmpeg/sox available for --freq-shift-hz",
            ));
        } else {
            out.push(Check::warn(
                "Frequency-shift backend",
                "--freq-shift-hz is set but no ffmpeg/sox installed",
                "install ffmpeg or remove --freq-shift-hz",
            ));
        }
    }

    // Apprise CLI is only needed when apprise-config (file mode) is used.
    if cli.apprise_config.is_some()
        || config.is_some_and(|c| c.get("APPRISE_CONFIG_FILE").is_some_and(|v| !v.is_empty()))
    {
        if tool_exists("apprise") {
            out.push(Check::pass("Apprise CLI", "apprise is on PATH"));
        } else {
            out.push(Check::warn(
                "Apprise CLI",
                "Apprise config is set but the `apprise` binary is missing",
                "install apprise (`pipx install apprise` or `apt install apprise`)",
            ));
        }
    }

    out
}

/// Report what the station will contact on its own initiative.
///
/// Answers "does this phone home?" from the diagnostic rather than from a
/// reading of the source. Only counts connections the station makes *by itself*
/// — an operator who configured Apprise or BirdWeather already knows about
/// those, and listing them here would bury the two that are on by default.
pub(super) fn check_egress(cli: &Cli) -> Vec<Check> {
    const NAME: &str = "Outbound connections";

    let mut on: Vec<&str> = Vec::new();
    if crate::helpers::egress::update_check_allowed(cli) {
        on.push("api.github.com (daily release check)");
    }
    if crate::helpers::egress::image_downloads_allowed(cli) {
        on.push("en.wikipedia.org / upload.wikimedia.org (species images, on demand)");
    }

    vec![if on.is_empty() {
        Check::pass(
            NAME,
            "none — this station makes no unsolicited outbound connections. \
             Integrations you configured explicitly are unaffected.",
        )
    } else {
        Check::pass(NAME, format!("on by default: {}", on.join("; ")))
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use clap::Parser;

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    #[test]
    fn runtime_environment_reports_cpu_and_temp() {
        let checks = check_runtime_environment();
        assert_eq!(checks.len(), 2);
        assert!(checks[0].name.contains("CPU cores"));
        // Pass on a multi-core host, Warn on a single core — both are valid.
        assert!(matches!(checks[0].status, Status::Pass | Status::Warn));
        assert!(checks[1].name.contains("Temp directory"));
    }

    #[test]
    fn optional_tools_empty_for_wav_defaults() {
        // Default CLI: WAV output, no freq-shift, no Apprise config.
        let checks = check_optional_tools(&cli(), None);
        assert!(checks.is_empty());
    }

    #[test]
    fn optional_tools_checks_encoder_for_nonwav_format() {
        let mut cli = cli();
        cli.audio_format = "mp3".to_string();
        let checks = check_optional_tools(&cli, None);
        // Presence is deterministic; status depends on whether ffmpeg/sox exists.
        assert!(checks.iter().any(|c| c.name.contains("Audio encoder")));
    }

    #[test]
    fn optional_tools_checks_freq_shift_backend() {
        let mut cli = cli();
        cli.freq_shift_hz = 2000;
        let checks = check_optional_tools(&cli, None);
        assert!(checks.iter().any(|c| c.name.contains("Frequency-shift")));
    }

    #[test]
    fn optional_tools_checks_apprise_when_configured() {
        let mut cli = cli();
        cli.apprise_config = Some(std::path::PathBuf::from("/tmp/apprise.conf"));
        let checks = check_optional_tools(&cli, None);
        assert!(checks.iter().any(|c| c.name.contains("Apprise")));
    }

    #[test]
    fn optional_tools_requires_ffmpeg_for_rtsp() {
        // RTSP capture shells out to ffmpeg on every platform, so the capture
        // backend check must appear (verdict depends on whether ffmpeg exists).
        let mut cli = cli();
        cli.rtsp_url = Some("rtsp://camera.invalid:554/stream".into());
        let checks = check_optional_tools(&cli, None);
        assert!(checks.iter().any(|c| c.name.contains("Capture backend")));
    }

    #[test]
    fn egress_check_lists_the_two_default_on_connections() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let checks = check_egress(&cli);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, Status::Pass);
        assert!(checks[0].message.contains("api.github.com"));
        assert!(checks[0].message.contains("wikipedia.org"));
    }

    #[test]
    fn egress_check_reports_silence_in_offline_mode() {
        let cli = Cli::parse_from(["birdnet-behavior", "--offline"]);
        let checks = check_egress(&cli);
        assert_eq!(checks[0].status, Status::Pass);
        assert!(
            checks[0].message.contains("none"),
            "offline mode must report no unsolicited egress: {}",
            checks[0].message
        );
        assert!(!checks[0].message.contains("api.github.com"));
    }

    #[test]
    fn egress_check_narrows_when_only_the_update_check_is_off() {
        let cli = Cli::parse_from(["birdnet-behavior", "--no-update-check"]);
        let checks = check_egress(&cli);
        assert!(!checks[0].message.contains("api.github.com"));
        assert!(checks[0].message.contains("wikipedia.org"));
    }
}
