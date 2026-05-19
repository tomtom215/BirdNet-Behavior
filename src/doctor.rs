//! End-user diagnostic subcommand.
//!
//! Runs a suite of preflight checks that answer the question
//! "is my BirdNet-Behavior install in a state where it can actually
//! detect birds?" and prints a one-screen report that a non-technical
//! operator can act on without having to read a stack trace.
//!
//! Each check is independent: a failure in one does not skip the others,
//! so the operator sees every issue in a single pass.
//!
//! The exit code summarises the worst severity observed:
//!   * `0` — all checks passed (some may be skipped/informational)
//!   * `1` — at least one warning, no errors
//!   * `2` — at least one error
//!
//! This makes the command useful both interactively and from monitoring
//! scripts (`birdnet-behavior --doctor; echo $?`).

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use birdnet_core::config::Config;
use birdnet_core::config::validate::{self as cfg_validate, Severity as ConfigSeverity};

use crate::cli::Cli;
use crate::helpers::db_path_from_config;

/// Verdict of a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Everything looks healthy.
    Pass,
    /// Check did not apply in this configuration (informational only).
    Skip,
    /// Functionality is degraded but the system will still start.
    Warn,
    /// The system will not work correctly until this is fixed.
    Fail,
}

impl Status {
    const fn tag(self) -> &'static str {
        match self {
            Self::Pass => "[ PASS ]",
            Self::Skip => "[ SKIP ]",
            Self::Warn => "[ WARN ]",
            Self::Fail => "[ FAIL ]",
        }
    }
}

/// Outcome of a single diagnostic check.
#[derive(Debug, Clone)]
pub struct Check {
    /// Short, human-readable name of the check.
    pub name: String,
    /// Verdict.
    pub status: Status,
    /// Short message shown next to the status tag.
    pub message: String,
    /// Optional remediation hint (printed on the next line if present).
    pub remediation: Option<String>,
}

impl Check {
    fn pass(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Pass,
            message: message.into(),
            remediation: None,
        }
    }
    fn skip(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Skip,
            message: message.into(),
            remediation: None,
        }
    }
    fn warn(name: impl Into<String>, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Warn,
            message: message.into(),
            remediation: Some(fix.into()),
        }
    }
    fn fail(name: impl Into<String>, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Fail,
            message: message.into(),
            remediation: Some(fix.into()),
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {} — {}", self.status.tag(), self.name, self.message)?;
        if let Some(fix) = &self.remediation {
            writeln!(f, "         → {fix}")?;
        }
        Ok(())
    }
}

/// Output format for the diagnostic report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Human-readable text with one check per line and a trailing summary.
    Text,
    /// Machine-readable single-line JSON object suitable for monitoring
    /// scripts. Schema:
    /// `{"summary":{"passed":N,"warnings":N,"errors":N,"skipped":N,"exit_code":N},`
    /// ` "checks":[{"status":"pass|warn|fail|skip","name":"...","message":"...","remediation":"..."|null}, ...]}`
    Json,
}

/// Run every preflight check and print a report in the given format.
///
/// Returns the process exit code that should be used (`0`/`1`/`2`).
pub fn run_with_format(cli: &Cli, config: Option<&Config>, format: Format) -> i32 {
    let mut checks: Vec<Check> = Vec::new();

    checks.extend(check_runtime_environment());
    checks.push(check_config_file(cli, config));
    if let Some(cfg) = config {
        checks.extend(check_config_values(cfg));
    }
    checks.push(check_listen_address(cli));
    checks.extend(check_database(cli, config));
    checks.extend(check_paths(cli, config));
    checks.extend(check_audio_source(cli, config));
    checks.extend(check_model(cli, config));
    checks.extend(check_optional_tools(cli, config));
    checks.extend(check_disk_space(cli, config));
    checks.push(check_systemd_watchdog());

    let exit_code = summarise(&checks);
    match format {
        Format::Text => print!("{}", render_text(&checks)),
        Format::Json => println!("{}", render_json(&checks, exit_code)),
    }
    exit_code
}

/// Render checks + summary as a single-line JSON object.
///
/// Hand-rolled rather than `serde_json::to_string` derive because the
/// shape is small, fixed, and we want to keep the diagnostic surface
/// free of macro magic that would obscure the contract. Strings are
/// escaped per RFC 8259 §7 (handles `\`, `"`, control chars, surrogate
/// pairs are not produced).
#[must_use]
pub fn render_json(checks: &[Check], exit_code: i32) -> String {
    let (passed, warnings, errors, skipped) = tally(checks);
    let mut out = String::with_capacity(512 + checks.len() * 96);
    out.push_str("{\"summary\":{");
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!(
            "\"passed\":{passed},\"warnings\":{warnings},\"errors\":{errors},\"skipped\":{skipped},\"exit_code\":{exit_code}"
        ),
    );
    out.push_str("},\"checks\":[");
    for (i, c) in checks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let status = match c.status {
            Status::Pass => "pass",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Skip => "skip",
        };
        out.push_str("{\"status\":\"");
        out.push_str(status);
        out.push_str("\",\"name\":");
        push_json_str(&mut out, &c.name);
        out.push_str(",\"message\":");
        push_json_str(&mut out, &c.message);
        out.push_str(",\"remediation\":");
        if let Some(r) = &c.remediation {
            push_json_str(&mut out, r);
        } else {
            out.push_str("null");
        }
        out.push('}');
    }
    out.push_str("]}");
    out
}

fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Summarise check results into a single exit code.
#[must_use]
pub fn summarise(checks: &[Check]) -> i32 {
    let mut worst = Status::Pass;
    for c in checks {
        if c.status > worst {
            worst = c.status;
        }
    }
    match worst {
        Status::Pass | Status::Skip => 0,
        Status::Warn => 1,
        Status::Fail => 2,
    }
}

/// Render the full diagnostic report as a single string of text.
///
/// Pure function with no I/O — every byte of the human-readable
/// `--doctor` output goes through here. Split out from the I/O wrapper
/// so it can be snapshot-tested against a golden file: a drift in the
/// user-facing format requires updating the snapshot, which has to be
/// reviewed in a PR.
#[must_use]
pub fn render_text(checks: &[Check]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(checks.len() * 128 + 256);
    let _ = writeln!(out);
    let _ = writeln!(out, "BirdNet-Behavior preflight report");
    let _ = writeln!(out, "=================================");
    let _ = writeln!(out);
    for c in checks {
        let _ = write!(out, "{c}");
    }

    let (passes, warns, fails, skips) = tally(checks);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Summary: {passes} passed, {warns} warning(s), {fails} error(s), {skips} skipped."
    );
    if fails > 0 {
        let _ = writeln!(
            out,
            "Status:  NOT READY — fix the errors above before starting the detection daemon."
        );
    } else if warns > 0 {
        let _ = writeln!(
            out,
            "Status:  READY WITH WARNINGS — the daemon will start but some features are degraded."
        );
    } else {
        let _ = writeln!(
            out,
            "Status:  READY — start the daemon with `birdnet-behavior`."
        );
    }
    let _ = writeln!(out);
    out
}

fn tally(checks: &[Check]) -> (usize, usize, usize, usize) {
    let mut p = 0;
    let mut w = 0;
    let mut f = 0;
    let mut s = 0;
    for c in checks {
        match c.status {
            Status::Pass => p += 1,
            Status::Warn => w += 1,
            Status::Fail => f += 1,
            Status::Skip => s += 1,
        }
    }
    (p, w, f, s)
}

// ── Individual checks ───────────────────────────────────────────────────────

fn check_runtime_environment() -> Vec<Check> {
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

fn check_config_file(cli: &Cli, config: Option<&Config>) -> Check {
    if config.is_some() {
        return Check::pass(
            "Configuration file",
            format!("loaded from {}", cli.config.display()),
        );
    }
    if cli.config.exists() {
        Check::fail(
            "Configuration file",
            format!("{} exists but could not be parsed", cli.config.display()),
            "check the file for syntax errors (key=value, one per line; '#' for comments)",
        )
    } else {
        Check::warn(
            "Configuration file",
            format!(
                "{} not found — using built-in defaults",
                cli.config.display()
            ),
            "copy .env.example to /etc/birdnet/birdnet.conf and edit before going to production",
        )
    }
}

fn check_config_values(config: &Config) -> Vec<Check> {
    let findings = cfg_validate::validate(config);
    if findings.is_empty() {
        return vec![Check::pass(
            "Configuration values",
            "all settings are within valid ranges",
        )];
    }
    findings
        .into_iter()
        .map(|f| {
            let name = format!("Config: {}", f.key);
            match f.severity {
                ConfigSeverity::Error => Check::fail(name, f.message, f.remediation),
                ConfigSeverity::Warning => Check::warn(name, f.message, f.remediation),
            }
        })
        .collect()
}

fn check_listen_address(cli: &Cli) -> Check {
    match cli.listen.parse::<std::net::SocketAddr>() {
        Ok(addr) => Check::pass(
            "Web listen address",
            format!("{addr} parses as a valid socket address"),
        ),
        Err(e) => Check::fail(
            "Web listen address",
            format!("{:?} is not a valid socket address: {e}", cli.listen),
            "use the form HOST:PORT, e.g. 127.0.0.1:8502 or 0.0.0.0:8502",
        ),
    }
}

fn check_database(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let db_path = db_path_from_config(config);
    let mut out = Vec::new();

    let Some(parent) = db_path.parent() else {
        out.push(Check::fail(
            "Database directory",
            format!("{} has no parent directory", db_path.display()),
            "set DB_PATH in the config to an absolute path with a writable parent",
        ));
        return out;
    };

    if parent.exists() {
        if writable(parent) {
            out.push(Check::pass(
                "Database directory",
                format!("{} is writable", parent.display()),
            ));
        } else {
            out.push(Check::fail(
                "Database directory",
                format!("{} is not writable", parent.display()),
                "ensure the running user owns this directory (chown / chmod u+w)",
            ));
        }
    } else {
        out.push(Check::warn(
            "Database directory",
            format!(
                "{} does not exist yet — will be created on first run",
                parent.display()
            ),
            "no action needed unless you want to pre-create it with `mkdir -p`",
        ));
    }

    if db_path.exists() {
        let _ = cli; // cli unused beyond db_path; keep symmetric signature
        match birdnet_db::resilience::full_integrity_check(&db_path) {
            Ok(true) => out.push(Check::pass(
                "Database integrity",
                format!("{} passes integrity check", db_path.display()),
            )),
            Ok(false) => out.push(Check::fail(
                "Database integrity",
                format!("{} reports corruption", db_path.display()),
                "run `birdnet-behavior --backup-db` then restore from the most recent backup",
            )),
            Err(e) => out.push(Check::fail(
                "Database integrity",
                format!("{} could not be opened: {e}", db_path.display()),
                "verify the file is a valid SQLite database; restore from backup if not",
            )),
        }
    } else {
        out.push(Check::skip(
            "Database integrity",
            "no database file yet — will be created on first run",
        ));
    }

    out
}

fn check_paths(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let mut out = Vec::new();

    let watch_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from));
    if let Some(dir) = watch_dir {
        if !dir.exists() {
            out.push(Check::warn(
                "Recordings directory",
                format!("{} does not exist", dir.display()),
                "create it with `mkdir -p` or let the daemon create it on first capture",
            ));
        } else if !writable(&dir) {
            out.push(Check::fail(
                "Recordings directory",
                format!("{} is not writable", dir.display()),
                "fix ownership/permissions on this directory",
            ));
        } else {
            out.push(Check::pass(
                "Recordings directory",
                format!("{} is writable", dir.display()),
            ));
        }
    } else {
        out.push(Check::skip(
            "Recordings directory",
            "no --watch-dir or RECS_DIR configured (file-watcher mode disabled)",
        ));
    }

    if let Some(image_dir) = cli
        .image_cache_dir
        .clone()
        .or_else(|| config?.get("IMAGE_CACHE_DIR").map(PathBuf::from))
    {
        if image_dir.exists() && !writable(&image_dir) {
            out.push(Check::warn(
                "Image cache directory",
                format!(
                    "{} is not writable — species images will not be cached",
                    image_dir.display()
                ),
                "fix ownership/permissions on this directory",
            ));
        } else {
            out.push(Check::pass(
                "Image cache directory",
                format!("{} is OK", image_dir.display()),
            ));
        }
    }

    out
}

fn check_audio_source(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
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

fn check_model(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let model_path = cli
        .model
        .clone()
        .or_else(|| config?.get("MODEL").map(PathBuf::from));
    let mut out = Vec::new();

    if let Some(p) = model_path {
        if p.exists() {
            match std::fs::metadata(&p) {
                Ok(m) if m.len() > 1_000_000 => out.push(Check::pass(
                    "ONNX model file",
                    format!("{} ({} bytes)", p.display(), m.len()),
                )),
                Ok(m) => out.push(Check::warn(
                    "ONNX model file",
                    format!(
                        "{} is only {} bytes — likely truncated or empty",
                        p.display(),
                        m.len()
                    ),
                    "re-download the model (delete it; the entrypoint will fetch it again)",
                )),
                Err(e) => out.push(Check::fail(
                    "ONNX model file",
                    format!("{} could not be inspected: {e}", p.display()),
                    "check filesystem health and permissions",
                )),
            }
        } else {
            out.push(Check::fail(
                "ONNX model file",
                format!("{} does not exist", p.display()),
                "either let the entrypoint download it (Docker), or run `install.sh` again",
            ));
        }
    } else {
        out.push(Check::skip(
            "ONNX model file",
            "no --model / MODEL configured (will use the bundled default at startup)",
        ));
    }

    let labels_path = cli
        .labels
        .clone()
        .or_else(|| config?.get("LABELS").map(PathBuf::from));
    if let Some(p) = labels_path {
        if p.exists() {
            out.push(Check::pass(
                "Labels file",
                format!("{} exists", p.display()),
            ));
        } else {
            out.push(Check::fail(
                "Labels file",
                format!("{} does not exist", p.display()),
                "the labels file ships alongside the model; re-run `install.sh`",
            ));
        }
    }

    out
}

fn check_optional_tools(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
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

fn check_disk_space(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    // Best-effort disk space check: use the recordings dir if known, else /.
    let dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    disk_free_bytes(&dir).map_or_else(
        || {
            vec![Check::skip(
                "Disk space",
                "could not query filesystem usage",
            )]
        },
        |bytes| {
            let gib = bytes / (1024 * 1024 * 1024);
            if gib >= 5 {
                vec![Check::pass(
                    "Disk space",
                    format!("{gib} GiB free on the volume containing {}", dir.display()),
                )]
            } else if gib >= 1 {
                vec![Check::warn(
                    "Disk space",
                    format!("only {gib} GiB free on {}", dir.display()),
                    "recordings will accumulate quickly; \
                     consider --max-files-per-species or external storage",
                )]
            } else {
                vec![Check::fail(
                    "Disk space",
                    format!("less than 1 GiB free on {}", dir.display()),
                    "free up space immediately — the disk manager may not be able to keep up",
                )]
            }
        },
    )
}

/// Verify the systemd watchdog plumbing the daemon advertises is actually
/// being honoured by the supervisor.
///
/// Three sub-questions, asked in order:
///
/// 1. **Is `NOTIFY_SOCKET` set?** If not we're running interactively (or
///    under an init system that doesn't speak `sd_notify`); skip the rest.
/// 2. **Is `WATCHDOG_USEC` set?** The unit shipped by `install.sh`
///    declares `WatchdogSec=120`, which makes systemd export this var. If
///    we see a `NOTIFY_SOCKET` but no `WATCHDOG_USEC`, the operator has
///    `Type=notify` without a watchdog timer — a degraded config; the
///    daemon won't be killed if it locks up.
/// 3. **Does a synthetic ping reach the socket?** Open a `UnixDatagram`,
///    `connect()` to the path, `send(b"WATCHDOG=1")`. A successful send
///    proves the pipe is intact end-to-end. A failure (`ENOENT`,
///    `ECONNREFUSED`) means the unit declares `NotifyAccess` correctly
///    but the supervisor has stopped reading the socket — operator
///    should restart the service.
///
/// Skipped on non-Unix because the protocol is Linux-specific.
fn check_systemd_watchdog() -> Check {
    #[cfg(unix)]
    {
        watchdog_state().describe()
    }
    #[cfg(not(unix))]
    {
        Check::skip(
            "systemd watchdog",
            "sd_notify is Linux-specific; nothing to verify here",
        )
    }
}

/// Result of probing the systemd notify channel.
///
/// Pure value type so `check_systemd_watchdog` is unit-testable: the
/// probe lives in `probe_watchdog_socket`, the describe step is pure
/// formatting, and the test cases pin every cell of the decision matrix
/// without actually mutating `NOTIFY_SOCKET` (forbidden in this workspace
/// because `std::env::set_var` is `unsafe` in Rust 2024).
#[derive(Debug, Clone, PartialEq, Eq)]
enum WatchdogState {
    /// Daemon is not running under systemd — `NOTIFY_SOCKET` unset.
    NotUnderSystemd,
    /// `Type=notify` but no `WatchdogSec=` configured.
    NotifyButNoWatchdog,
    /// Both `NOTIFY_SOCKET` and `WATCHDOG_USEC` present; synthetic ping
    /// reached the supervisor.
    Working { interval_secs: u64 },
    /// Both `NOTIFY_SOCKET` and `WATCHDOG_USEC` present but the ping
    /// could not be delivered — supervisor has gone away.
    PingFailed { interval_secs: u64, reason: String },
}

impl WatchdogState {
    fn describe(&self) -> Check {
        match self {
            Self::NotUnderSystemd => Check::skip(
                "systemd watchdog",
                "NOTIFY_SOCKET is not set — running outside systemd (interactive run, docker, etc.)",
            ),
            Self::NotifyButNoWatchdog => Check::warn(
                "systemd watchdog",
                "NOTIFY_SOCKET is set but WATCHDOG_USEC is not — no watchdog timer is configured",
                "add `WatchdogSec=120` to the service's [Service] section and reload \
                 (`systemctl daemon-reload && systemctl restart birdnet-behavior`) \
                 so the supervisor can detect hangs",
            ),
            Self::Working { interval_secs } => Check::pass(
                "systemd watchdog",
                format!(
                    "ping delivered to systemd; supervisor expects pings every {interval_secs} s"
                ),
            ),
            Self::PingFailed {
                interval_secs,
                reason,
            } => Check::fail(
                "systemd watchdog",
                format!(
                    "WATCHDOG_USEC={interval_secs}s is set but the notify socket is not accepting pings: {reason}"
                ),
                "this means systemd has stopped reading the socket — \
                 `systemctl restart birdnet-behavior` to re-establish the supervisor link",
            ),
        }
    }
}

#[cfg(unix)]
fn watchdog_state() -> WatchdogState {
    let Some(socket_path) = std::env::var("NOTIFY_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        return WatchdogState::NotUnderSystemd;
    };
    let Some(interval_secs) = std::env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|usec| usec / 1_000_000)
        .filter(|&s| s > 0)
    else {
        return WatchdogState::NotifyButNoWatchdog;
    };

    match probe_watchdog_socket(&socket_path) {
        Ok(()) => WatchdogState::Working { interval_secs },
        Err(reason) => WatchdogState::PingFailed {
            interval_secs,
            reason,
        },
    }
}

/// Send one `WATCHDOG=1` datagram to the supervisor.
///
/// Mirrors the production `sd_notify::watchdog_ping` send path but
/// returns the error rather than swallowing it, so the diagnostic can
/// surface it on the operator's screen.
#[cfg(unix)]
fn probe_watchdog_socket(socket_path: &str) -> Result<(), String> {
    use std::os::unix::net::UnixDatagram;
    let path_for_connect = socket_path
        .strip_prefix('@')
        .map_or_else(|| socket_path.to_owned(), |rest| format!("\0{rest}"));
    let sock = UnixDatagram::unbound().map_err(|e| format!("UnixDatagram::unbound: {e}"))?;
    sock.connect(Path::new(&path_for_connect))
        .map_err(|e| format!("connect({socket_path}): {e}"))?;
    sock.send(b"WATCHDOG=1")
        .map_err(|e| format!("send WATCHDOG=1: {e}"))?;
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn writable(path: &Path) -> bool {
    let probe = path.join(".birdnet-doctor-write-probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn tool_exists(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|d| {
        let candidate = d.join(name);
        candidate.is_file()
    })
}

/// Best-effort free-bytes query that shells out to `df` so we don't have to
/// pull a libc crate or write unsafe FFI. Returns `None` if `df` is missing
/// or its output cannot be parsed.
fn disk_free_bytes(path: &Path) -> Option<u64> {
    let out = Command::new("df")
        .args(["-Pk", "--"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_df_available_kib(&text).map(|kib| kib * 1024)
}

fn parse_df_available_kib(df_output: &str) -> Option<u64> {
    // POSIX `df -Pk` prints exactly two lines: a header and one data row.
    // Columns: Filesystem  1024-blocks  Used  Available  Capacity  Mounted on
    // We want the 4th column of the data row. Handle wrapped lines defensively.
    let data = df_output.lines().nth(1)?;
    data.split_whitespace().nth(3)?.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_ordering() {
        assert!(Status::Pass < Status::Skip);
        assert!(Status::Skip < Status::Warn);
        assert!(Status::Warn < Status::Fail);
    }

    #[test]
    fn summarise_reports_worst_status() {
        let checks = vec![
            Check::pass("a", "ok"),
            Check::warn("b", "m", "fix"),
            Check::pass("c", "ok"),
        ];
        assert_eq!(summarise(&checks), 1);

        let mut with_fail = checks;
        with_fail.push(Check::fail("d", "broken", "fix"));
        assert_eq!(summarise(&with_fail), 2);

        let only_pass = vec![Check::pass("a", "ok"), Check::skip("b", "n/a")];
        assert_eq!(summarise(&only_pass), 0);
    }

    #[test]
    fn empty_checks_pass() {
        assert_eq!(summarise(&[]), 0);
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

    #[test]
    fn extract_card_number_recognises_common_forms() {
        assert_eq!(extract_card_number("plughw:1,0").as_deref(), Some("1"));
        assert_eq!(extract_card_number("hw:2,0").as_deref(), Some("2"));
        assert_eq!(extract_card_number("default"), None);
        assert_eq!(extract_card_number("USB Audio"), None);
    }

    #[test]
    fn tool_exists_finds_basic_unix_binaries() {
        if cfg!(unix) {
            assert!(tool_exists("ls"), "ls should exist on a POSIX system");
        }
        assert!(!tool_exists("definitely-not-a-real-binary-name-93kfh"));
    }

    #[test]
    fn writable_detects_writable_tempdir() {
        let tmp = std::env::temp_dir();
        assert!(writable(&tmp));
    }

    #[test]
    fn check_format_includes_status_tag() {
        let c = Check::warn("X", "m", "fix me");
        let s = format!("{c}");
        assert!(s.contains("[ WARN ]"));
        assert!(s.contains('X'));
        assert!(s.contains('m'));
        assert!(s.contains("fix me"));
    }

    // ── JSON rendering ─────────────────────────────────────────────────────

    #[test]
    fn json_summary_reflects_tally() {
        let checks = vec![
            Check::pass("a", "ok"),
            Check::warn("b", "m", "fix"),
            Check::fail("c", "broken", "fix"),
            Check::skip("d", "n/a"),
        ];
        let json = render_json(&checks, 2);
        assert!(json.contains("\"passed\":1"));
        assert!(json.contains("\"warnings\":1"));
        assert!(json.contains("\"errors\":1"));
        assert!(json.contains("\"skipped\":1"));
        assert!(json.contains("\"exit_code\":2"));
        assert!(json.contains("\"status\":\"pass\""));
        assert!(json.contains("\"status\":\"warn\""));
        assert!(json.contains("\"status\":\"fail\""));
        assert!(json.contains("\"status\":\"skip\""));
    }

    #[test]
    fn json_escapes_control_characters_and_quotes() {
        let c = Check::warn("name\"X", "line1\nline2\twith\\backslash", "fix\rme");
        let json = render_json(&[c], 1);
        // Must not contain unescaped specials in the string payload.
        assert!(json.contains("name\\\"X"), "{json}");
        assert!(json.contains("line1\\nline2\\twith\\\\backslash"), "{json}");
        assert!(json.contains("fix\\rme"), "{json}");
        // Must be parseable as a JSON object (last character is `}`).
        assert!(json.ends_with('}'));
    }

    #[test]
    fn json_omits_remediation_as_null() {
        let c = Check::pass("a", "ok");
        let json = render_json(&[c], 0);
        assert!(json.contains("\"remediation\":null"), "{json}");
    }

    #[test]
    fn json_empty_check_list_still_valid() {
        let json = render_json(&[], 0);
        // Empty arrays and zeroed summary.
        assert!(json.contains("\"checks\":[]"));
        assert!(json.contains("\"passed\":0"));
    }

    #[test]
    fn json_handles_low_codepoint_via_unicode_escape() {
        // U+0001 is below the 0x20 cut-off and must be \u0001-encoded.
        let c = Check::pass("a", "x\u{0001}y");
        let json = render_json(&[c], 0);
        assert!(json.contains("x\\u0001y"), "{json}");
    }

    // ── Systemd watchdog check ──────────────────────────────────────────
    //
    // The describe() arm is a pure value→Check translation; the probe is
    // testable against a temporary UnixDatagram receiver we control.

    #[test]
    fn watchdog_describe_not_under_systemd_is_skip() {
        let c = WatchdogState::NotUnderSystemd.describe();
        assert_eq!(c.status, Status::Skip);
        assert!(c.message.contains("NOTIFY_SOCKET"));
    }

    #[test]
    fn watchdog_describe_no_watchdog_warns_with_remediation() {
        let c = WatchdogState::NotifyButNoWatchdog.describe();
        assert_eq!(c.status, Status::Warn);
        assert!(
            c.remediation
                .as_ref()
                .is_some_and(|r| r.contains("WatchdogSec"))
        );
    }

    #[test]
    fn watchdog_describe_working_includes_interval() {
        let c = WatchdogState::Working { interval_secs: 120 }.describe();
        assert_eq!(c.status, Status::Pass);
        assert!(c.message.contains("120"));
    }

    #[test]
    fn watchdog_describe_ping_failed_is_fail_with_reason() {
        let c = WatchdogState::PingFailed {
            interval_secs: 60,
            reason: "connect(/run/foo): No such file or directory".into(),
        }
        .describe();
        assert_eq!(c.status, Status::Fail);
        assert!(c.message.contains("60"));
        assert!(c.message.contains("No such file"));
    }

    #[cfg(unix)]
    #[test]
    fn probe_watchdog_socket_succeeds_against_a_real_receiver() {
        use std::os::unix::net::UnixDatagram;

        let tmp =
            std::env::temp_dir().join(format!("birdnet-doctor-probe-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let server = UnixDatagram::bind(&tmp).expect("bind notify socket");

        let result = probe_watchdog_socket(tmp.to_str().unwrap());
        let _ = std::fs::remove_file(&tmp);
        drop(server);

        assert!(result.is_ok(), "probe failed: {result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn probe_watchdog_socket_fails_on_missing_path() {
        let err = probe_watchdog_socket("/tmp/this/path/does/not/exist.sock")
            .expect_err("should not be able to send to a missing socket");
        assert!(err.contains("connect") || err.contains("No such file"));
    }

    // ── Snapshot tests for the human-readable report ─────────────────────
    //
    // Pin the exact bytes of the `--doctor` output against a golden file so
    // accidental wording or formatting drifts have to come through a PR. The
    // input is a hand-curated fixture (no live filesystem / network access)
    // so the snapshot is deterministic across hosts.
    //
    // To update after an intentional UX change:
    //   UPDATE_DOCTOR_SNAPSHOTS=1 cargo test -p birdnet-behavior --bin birdnet-behavior \
    //       -- tests::snapshot
    // and review the resulting diff against
    // `src/testdata/doctor_snapshots/*.txt`.

    const SNAPSHOT_DIR: &str = "src/testdata/doctor_snapshots";

    fn sample_all_pass() -> Vec<Check> {
        vec![
            Check::pass("CPU cores", "4 cores available for audio + inference"),
            Check::pass("Temp directory", "/tmp is writable"),
            Check::pass(
                "Configuration file",
                "loaded from /etc/birdnet/birdnet.conf",
            ),
            Check::pass(
                "Configuration values",
                "all settings are within valid ranges",
            ),
            Check::pass(
                "Web listen address",
                "127.0.0.1:8502 parses as a valid socket address",
            ),
            Check::pass("Database directory", "/var/lib/birdnet is writable"),
            Check::pass(
                "Database integrity",
                "/var/lib/birdnet/birds.db passes integrity check",
            ),
            Check::pass(
                "Recordings directory",
                "/var/lib/birdnet/recordings is writable",
            ),
            Check::pass("Audio source", "ALSA source configured"),
            Check::pass(
                "ALSA device probe",
                "plughw:1,0 matches an entry in `arecord -l`",
            ),
            Check::pass(
                "ONNX model file",
                "/usr/share/birdnet/model.onnx (541000000 bytes)",
            ),
            Check::pass(
                "Disk space",
                "120 GiB free on the volume containing /var/lib/birdnet/recordings",
            ),
        ]
    }

    fn sample_mixed() -> Vec<Check> {
        vec![
            Check::pass("CPU cores", "4 cores available for audio + inference"),
            Check::pass("Temp directory", "/tmp is writable"),
            Check::warn(
                "Configuration file",
                "/etc/birdnet/birdnet.conf not found — using built-in defaults",
                "copy .env.example to /etc/birdnet/birdnet.conf and edit before going to production",
            ),
            Check::pass(
                "Web listen address",
                "127.0.0.1:8502 parses as a valid socket address",
            ),
            Check::warn(
                "Database directory",
                "/var/lib/birdnet does not exist yet — will be created on first run",
                "no action needed unless you want to pre-create it with `mkdir -p`",
            ),
            Check::skip(
                "Database integrity",
                "no database file yet — will be created on first run",
            ),
            Check::skip(
                "Recordings directory",
                "no --watch-dir or RECS_DIR configured (file-watcher mode disabled)",
            ),
            Check::warn(
                "Audio source",
                "no audio source configured (no live detections will be produced)",
                "set one of: --alsa-device, --pipewire-device, --rtsp-url, --rtsp-urls, \
                 or the equivalent ALSA_CARD / RTSP_URL / PIPEWIRE_DEVICE config keys",
            ),
            Check::skip(
                "ONNX model file",
                "no --model / MODEL configured (will use the bundled default at startup)",
            ),
            Check::pass("Disk space", "120 GiB free on the volume containing /"),
        ]
    }

    fn sample_with_errors() -> Vec<Check> {
        vec![
            Check::pass("CPU cores", "4 cores available for audio + inference"),
            Check::pass("Temp directory", "/tmp is writable"),
            Check::pass(
                "Configuration file",
                "loaded from /etc/birdnet/birdnet.conf",
            ),
            Check::fail(
                "Config: LATITUDE",
                "latitude 200 is outside the valid range -90.0 to 90.0",
                "use decimal degrees, e.g. LATITUDE=42.3601 for Boston, MA",
            ),
            Check::fail(
                "Config: AUDIO_FORMAT",
                "AUDIO_FORMAT=\"aiff\" is not supported",
                "use \"wav\", \"mp3\", \"flac\", or \"ogg\" (non-WAV formats need ffmpeg or sox)",
            ),
            Check::fail(
                "Web listen address",
                "\"not-an-address\" is not a valid socket address: invalid socket address syntax",
                "use the form HOST:PORT, e.g. 127.0.0.1:8502 or 0.0.0.0:8502",
            ),
            Check::pass("Database directory", "/var/lib/birdnet is writable"),
            Check::skip(
                "Database integrity",
                "no database file yet — will be created on first run",
            ),
            Check::warn(
                "Audio source",
                "no audio source configured (no live detections will be produced)",
                "set one of: --alsa-device, --pipewire-device, --rtsp-url, --rtsp-urls, \
                 or the equivalent ALSA_CARD / RTSP_URL / PIPEWIRE_DEVICE config keys",
            ),
            Check::pass("Disk space", "120 GiB free on the volume containing /"),
        ]
    }

    fn check_snapshot(name: &str, actual: &str) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(SNAPSHOT_DIR)
            .join(format!("{name}.txt"));
        let update = std::env::var("UPDATE_DOCTOR_SNAPSHOTS").is_ok();
        if update || !path.exists() {
            std::fs::create_dir_all(path.parent().expect("snapshot dir"))
                .expect("create snapshot dir");
            std::fs::write(&path, actual).expect("write snapshot");
            return;
        }
        let expected = std::fs::read_to_string(&path).expect("read snapshot");
        assert_eq!(
            actual,
            expected,
            "Snapshot {name} drifted.\nRun with UPDATE_DOCTOR_SNAPSHOTS=1 to refresh.\n\
             Expected (from {}):\n{expected}\n\nActual:\n{actual}",
            path.display()
        );
    }

    #[test]
    fn snapshot_all_pass() {
        check_snapshot("all_pass", &render_text(&sample_all_pass()));
    }

    #[test]
    fn snapshot_mixed_warnings_and_skips() {
        check_snapshot("mixed", &render_text(&sample_mixed()));
    }

    #[test]
    fn snapshot_with_errors() {
        check_snapshot("with_errors", &render_text(&sample_with_errors()));
    }

    #[test]
    fn snapshot_empty_report() {
        check_snapshot("empty", &render_text(&[]));
    }
}

#[cfg(test)]
mod proptests_json {
    use super::*;
    use proptest::prelude::*;

    fn arb_status() -> impl Strategy<Value = Status> {
        prop_oneof![
            Just(Status::Pass),
            Just(Status::Warn),
            Just(Status::Fail),
            Just(Status::Skip),
        ]
    }

    fn arb_check() -> impl Strategy<Value = Check> {
        (
            arb_status(),
            ".{0,40}",
            ".{0,80}",
            proptest::option::of(".{0,60}"),
        )
            .prop_map(|(status, name, message, remediation)| Check {
                name,
                status,
                message,
                remediation,
            })
    }

    proptest! {
        /// JSON output is always parseable, and the parsed object has the
        /// documented schema (top-level object with `summary` and `checks`,
        /// every check entry has all four required fields).
        #[test]
        fn json_is_always_parseable(
            checks in proptest::collection::vec(arb_check(), 0..16),
            exit in -10_i32..=10,
        ) {
            let s = render_json(&checks, exit);
            let v: serde_json::Value = serde_json::from_str(&s)
                .expect("render_json must produce valid JSON");
            prop_assert!(v.is_object());
            let obj = v.as_object().unwrap();
            prop_assert!(obj.contains_key("summary"));
            prop_assert!(obj.contains_key("checks"));
            let arr = obj["checks"].as_array().expect("checks must be an array");
            prop_assert_eq!(arr.len(), checks.len());
            for entry in arr {
                let m = entry.as_object().unwrap();
                prop_assert!(m.contains_key("status"));
                prop_assert!(m.contains_key("name"));
                prop_assert!(m.contains_key("message"));
                prop_assert!(m.contains_key("remediation"));
            }
        }

        /// Summary counts always sum to the total number of checks — catches
        /// off-by-one bugs in `tally` regardless of input ordering.
        #[test]
        fn json_summary_sums_to_check_count(
            checks in proptest::collection::vec(arb_check(), 0..32),
        ) {
            let s = render_json(&checks, 0);
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            let sum = v["summary"]["passed"].as_u64().unwrap()
                + v["summary"]["warnings"].as_u64().unwrap()
                + v["summary"]["errors"].as_u64().unwrap()
                + v["summary"]["skipped"].as_u64().unwrap();
            prop_assert_eq!(sum, checks.len() as u64);
        }

        /// `summarise` and the JSON output's embedded summary always agree.
        #[test]
        fn summarise_matches_embedded_exit_code(
            checks in proptest::collection::vec(arb_check(), 0..32),
        ) {
            let code = summarise(&checks);
            let s = render_json(&checks, code);
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            prop_assert_eq!(v["summary"]["exit_code"].as_i64().unwrap(), i64::from(code));
        }

        /// `summarise` only returns one of the three documented exit codes.
        #[test]
        fn summarise_only_returns_documented_codes(
            checks in proptest::collection::vec(arb_check(), 0..32),
        ) {
            let code = summarise(&checks);
            prop_assert!(matches!(code, 0..=2), "unexpected exit code {code}");
        }
    }
}
