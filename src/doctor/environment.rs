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
}
