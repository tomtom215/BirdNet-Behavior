//! `--support-bundle`: one file an operator can attach to a bug report.
//!
//! # Why this exists
//!
//! Diagnosing a station over a forum thread costs a round trip per question,
//! and each round trip is a day. `--doctor` already answers most of them, but
//! only for whoever is sitting at the terminal — the answers do not travel.
//! What travels is a file.
//!
//! # Why redaction is the hard part, not the tarball
//!
//! Everything worth collecting is also where the secrets are. `birdnet.conf`
//! holds the admin password, the `BirdWeather` token, the SMTP password and the
//! session secret; an RTSP URL routinely carries `user:pass@` in its authority;
//! `journalctl` output can echo any of them back. A support bundle that leaks
//! those is worse than no support bundle, because the operator has been
//! *encouraged* to post it in public.
//!
//! So redaction is deny-by-default in both directions:
//!
//! * **By key** — [`is_secret_key`] matches on substrings that appear in the
//!   names of secrets (`PASSWORD`, `TOKEN`, `SECRET`, `PWD`, `KEY`, …) rather
//!   than on an allow-list of known keys, so a setting added next year is
//!   redacted before anyone remembers this file exists.
//! * **By shape** — [`redact_value`] handles the values that *are* kept:
//!   `user:pass@` is stripped from anything URL-shaped, an `http(s)` URL
//!   loses its path (a heartbeat ping, an Apprise endpoint and a webhook all
//!   carry their credential there), an Apprise-style notification URL keeps
//!   only its scheme, and a bare email address loses its local part — because
//!   the key name `RTSP_URL` or `HEARTBEAT_URL` says nothing about a secret
//!   while its value very often is one.
//!
//! Redaction replaces the value rather than dropping the line: "this station
//! has an SMTP password set" is diagnostic information, and a missing line
//! reads identically to a setting that was never configured.
//!
//! The three rules themselves now live in [`birdnet_core::config::redact`],
//! because `GET /api/v2/settings` has to apply the same ones from another
//! crate and two copies of "which values are secret" is how a station once
//! shipped an open `/admin` its own diagnostic called protected. They are
//! re-exported here so this module's callers and its own history are
//! unchanged.

mod hardware;

use birdnet_core::process::run_with_timeout;
use std::time::Duration;

/// How long one diagnostic command (`journalctl`, `systemctl`, `df`, …) may
/// take before the bundle records it as unavailable and moves on.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `tar` may take to build the bundle.
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(300);

use std::io::Write as _;
use std::path::{Path, PathBuf};

use birdnet_core::config::Config;
pub use birdnet_core::config::redact::{
    REDACTED, is_secret_key, redact_credentials_in_text, redact_value,
};

use crate::cli::Cli;

/// How many journal lines to include.
///
/// Enough to cover a start-up and the failure that followed it; small enough
/// that the bundle stays attachable to an issue.
const JOURNAL_LINES: &str = "2000";

/// One configuration or settings value as the bundle shows it: a secret by its
/// key is [`REDACTED`], a coordinate is coarsened, anything else is masked by
/// the shape of its value.
fn shown_value(key: &str, value: &str) -> String {
    if is_secret_key(key) {
        REDACTED.to_owned()
    } else if is_coordinate_key(key) {
        coarse_coordinate(value).unwrap_or_else(|| REDACTED.to_owned())
    } else {
        redact_value(value)
    }
}

/// Render the configuration with every secret masked.
///
/// Sorted, so two bundles from the same station diff cleanly.
#[must_use]
pub fn redacted_config(config: &Config) -> String {
    let mut lines: Vec<String> = config
        .iter()
        .map(|(k, v)| format!("{k}={}", shown_value(k, v)))
        .collect();
    lines.sort();
    lines.join("\n")
}

/// The most one value may take in `settings.redacted`, in characters.
///
/// The longest ordinary setting is a notification template; a value past this
/// is a pasted document, and is cut with a note saying how much was left out.
pub const SETTING_VALUE_CAP: usize = 1_024;

/// The most `settings.redacted` and `audio-sources.redacted` may each hold, in
/// bytes. A station has well under a hundred settings, so this is a guard
/// against a table somebody filled with something else, not a limit anyone
/// meets; past it the member ends with a line saying how many rows were left
/// out.
pub const STATE_MEMBER_CAP: usize = 64 * 1024;

/// Keep `lines` to [`STATE_MEMBER_CAP`] bytes, saying what was left out.
fn capped(lines: Vec<String>) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if out.len() + line.len() + 1 > STATE_MEMBER_CAP {
            out.push_str(&format!(
                "(… {} more rows not shown: this member is capped at {STATE_MEMBER_CAP} bytes)\n",
                lines.len() - i
            ));
            return out;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The settings saved from the web interface (the `settings` table), one
/// `key=value` per line and sorted, masked exactly as [`redacted_config`] masks
/// the file: a key that names a secret is [`REDACTED`], a coordinate is
/// coarsened, and every other value is masked by its shape.
///
/// A station configured in the browser has no configuration file at all, so
/// without this member the bundle carried none of its settings. A value is cut
/// at [`SETTING_VALUE_CAP`] characters and a newline inside one is written
/// `\n`, so each setting stays one line.
#[must_use]
pub fn redacted_settings(rows: &[(String, String)]) -> String {
    let mut lines: Vec<String> = rows
        .iter()
        .map(|(k, v)| {
            let shown = shown_value(k, v).replace('\n', "\\n");
            let over = shown.chars().count().saturating_sub(SETTING_VALUE_CAP);
            if over == 0 {
                format!("{k}={shown}")
            } else {
                let kept: String = shown.chars().take(SETTING_VALUE_CAP).collect();
                format!("{k}={kept}(… {over} more characters not shown)")
            }
        })
        .collect();
    lines.sort();
    capped(lines)
}

/// The station's microphones and streams (the `audio_sources` table), one line
/// each. A stream's address is masked by [`redact_value`] — the camera's
/// password never leaves — and every other field is what an audio fault is
/// diagnosed from: the kind, the rate, the gain, the transport, the quiet
/// window, the filters, and whether the row is disabled.
#[must_use]
pub fn redacted_audio_sources(rows: &[birdnet_db::audio_sources::AudioSource]) -> String {
    let lines: Vec<String> = rows
        .iter()
        .map(|s| {
            let quiet = s
                .schedule_quiet
                .as_ref()
                .map_or_else(|| "none".to_owned(), |(a, b)| format!("{a}-{b}"));
            let p = s.pipeline;
            format!(
                "id={} kind={} device={} label={:?} rate={} channels={} bit_depth={} gain_db={} \
                 transport={} media_mode={} quiet={quiet} high_pass={} dc_removal={} agc={} \
                 rtsp_keepalive={} eq_chain={:?} disabled_at={}",
                s.id,
                s.kind.as_str(),
                redact_value(&s.device_id),
                s.label.as_deref().unwrap_or(""),
                s.sample_rate,
                s.channels.as_str(),
                s.bit_depth,
                s.gain_db,
                s.rtsp_transport.as_str(),
                s.media_mode.as_str(),
                p.high_pass,
                p.dc_removal,
                p.agc,
                p.rtsp_keepalive,
                s.eq_chain,
                s.disabled_at.as_deref().unwrap_or("no"),
            )
        })
        .collect();
    if lines.is_empty() {
        return "(no audio sources are stored)\n".to_owned();
    }
    capped(lines)
}

/// The two database-backed members, `(settings.redacted, audio-sources.redacted)`.
///
/// Read through a read-only connection, so building a bundle cannot write to
/// the station's database. A database that cannot be opened or read is said so
/// in the member, with the path: "no settings are stored" and "the bundle could
/// not read them" are different answers.
fn station_state_members(config: Option<&Config>) -> (String, String) {
    let path = crate::helpers::db_path_from_config(config);
    let unreadable = |what: &str, e: &dyn std::fmt::Display| {
        format!("(could not read the {what} from {}: {e})\n", path.display())
    };
    let conn = match birdnet_db::sqlite::open_readonly(&path) {
        Ok(conn) => conn,
        Err(e) => return (unreadable("settings", &e), unreadable("audio sources", &e)),
    };
    let settings = match birdnet_db::settings::list(&conn, None) {
        Ok(rows) => {
            if rows.is_empty() {
                "(no settings have been saved from the web interface)\n".to_owned()
            } else {
                let pairs: Vec<(String, String)> =
                    rows.into_iter().map(|s| (s.key, s.value)).collect();
                redacted_settings(&pairs)
            }
        }
        Err(e) => unreadable("settings", &e),
    };
    let sources = match birdnet_db::audio_sources::AudioSourceStore::list(&conn) {
        Ok(rows) => redacted_audio_sources(&rows),
        Err(e) => unreadable("audio sources", &e),
    };
    (settings, sources)
}

/// The two configuration keys that hold where the station is.
const fn is_coordinate_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("LATITUDE") || key.eq_ignore_ascii_case("LONGITUDE")
}

/// A coordinate as the bundle shows it: one decimal place, about 11 km.
///
/// The bundle is written to be pasted into a public issue, and four decimals of
/// a station's latitude and longitude are a house. One decimal still says which
/// hemisphere, which season's daylight and which region's species the station
/// was configured for, which is everything a support question about the
/// scheduler or the occurrence filter needs. `None` for text that is not a
/// number: it is shown as [`REDACTED`], because an unparseable coordinate may be
/// degrees-minutes-seconds with the exact position in it.
fn coarse_coordinate(raw: &str) -> Option<String> {
    let value: f64 = raw.trim().parse().ok()?;
    value.is_finite().then(|| format!("{value:.1}"))
}

/// Replace the exact coordinates in `text` with their [`coarse_coordinate`]
/// form, wherever the configured value appears as a whole number.
///
/// The journal prints them (BirdWeather's start-up line, a settings change), so
/// coarsening the configuration section alone would leave the position in the
/// bundle anyway. Only values with at least two decimal places are hunted: a
/// coordinate typed to one decimal is already as coarse as the bundle is, and
/// `52.5` is far too common a number to search a log for. "Whole number" is
/// checked on both sides, so `52.5233` is not found inside `152.52331`.
fn coarsen_coordinates_in_text(text: &str, config: &Config) -> String {
    let mut out = text.to_owned();
    for key in ["LATITUDE", "LONGITUDE"] {
        let Some(raw) = config.get(key) else { continue };
        let raw = raw.trim();
        let decimals = raw.split_once('.').map_or(0, |(_, frac)| frac.len());
        let Some(coarse) = coarse_coordinate(raw) else {
            continue;
        };
        if decimals < 2 || coarse == raw {
            continue;
        }
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(raw) {
            let (before, after) = (&rest[..at], &rest[at + raw.len()..]);
            let glued_before = before
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_digit() || c == '.');
            let mut following = after.chars();
            let glued_after = match following.next() {
                Some(c) if c.is_ascii_digit() => true,
                // `52.523431.5` is one malformed number, not a coordinate
                // followed by a full stop; `52.523431.` ends a sentence.
                Some('.') => following.next().is_some_and(|c| c.is_ascii_digit()),
                _ => false,
            };
            result.push_str(before);
            result.push_str(if glued_before || glued_after {
                raw
            } else {
                &coarse
            });
            rest = after;
        }
        result.push_str(rest);
        out = result;
    }
    out
}

/// A log scrubbed for the bundle.
///
/// The journal is the one member nothing upstream redacts: `log_capture`
/// cleans only the lines it captures, the stderr layer writes whatever it is
/// given, and a persistent journal keeps lines an older build wrote before a
/// leak was fixed. So, in this order:
///
/// 1. Every configured secret's literal value becomes [`REDACTED`], and every
///    URL-valued setting its [`redact_value`] form — a token in a path, such as
///    an Apprise `/notify/{key}` or a `tgram://` URL, has no userinfo for the
///    next step to find. Literals first, while they still read as configured.
/// 2. The password of every URL left in the text is replaced
///    ([`redact_credentials_in_text`]).
/// 3. The station's exact coordinates become their one-decimal form
///    ([`coarsen_coordinates_in_text`]).
///
/// Values shorter than four characters are not matched as literals: replacing
/// every `1` or `on` in a log would leave nothing to read. A secret that short
/// survives this step. Settings stored only in the database (webhook
/// addresses, notification URLs entered on the web) are not in the
/// configuration and are covered by step 2 alone.
fn scrub_log(text: &str, config: Option<&Config>) -> String {
    let mut out = text.to_owned();
    if let Some(config) = config {
        for (key, value) in config.iter() {
            let value = value.trim();
            if value.chars().count() < 4 {
                continue;
            }
            if is_secret_key(key) {
                out = out.replace(value, REDACTED);
            } else if value.contains("://") {
                for member in value
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|m| m.contains("://"))
                {
                    let shown = redact_value(member);
                    if shown != member {
                        out = out.replace(member, &shown);
                    }
                }
            }
        }
    }
    let out = redact_credentials_in_text(&out);
    match config {
        Some(config) => coarsen_coordinates_in_text(&out, config),
        None => out,
    }
}

/// Run a command and capture its output, or a note explaining why not.
///
/// The station's persisted ERROR/WARN lines, or a note saying why not.
///
/// A missing file is reported rather than staged empty: "this station has
/// never logged a warning" and "the bundle could not find the log" are
/// different answers, and only one of them is good news.
fn read_error_log(config: Option<&Config>) -> String {
    let path = crate::log_capture::error_log_path(&crate::helpers::db_path_from_config(config));
    match std::fs::read_to_string(&path) {
        Ok(s) if s.is_empty() => format!(
            "(no warnings or errors logged; {} is empty)\n",
            path.display()
        ),
        Ok(s) => s,
        Err(e) => format!("(could not read {}: {e})\n", path.display()),
    }
}

/// Never fails the bundle: a station without `journalctl` (a container, a
/// non-systemd install) should still produce everything else, and "this tool
/// was not available" is itself worth knowing when reading the bundle.
fn capture(cmd: &str, args: &[&str]) -> String {
    match run_with_timeout(std::process::Command::new(cmd).args(args), CAPTURE_TIMEOUT) {
        Ok(out) => {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            if !out.stderr.is_empty() {
                s.push_str("\n--- stderr ---\n");
                s.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            s
        }
        Err(e) => format!("({cmd} unavailable: {e})\n"),
    }
}

/// Write `contents` into the staging directory, reporting the path on failure.
fn stage(dir: &Path, name: &str, contents: &str) -> Result<(), String> {
    let path = dir.join(name);
    std::fs::write(&path, contents).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// What [`build`] produced: the archive's size, and anything it could not
/// stage — a member that failed is reported, not fatal, because a station too
/// broken to answer one question is exactly the one that needs the rest.
#[derive(Debug)]
pub struct Bundle {
    /// Size of the archive at `dest`, in bytes.
    pub size: u64,
    /// Members that could not be staged, one message each.
    pub warnings: Vec<String>,
}

/// Collect a support bundle and write it to `dest`.
///
/// The one implementation behind both `--support-bundle` and
/// `GET /admin/support-bundle` (`OP-1`), so the archive an operator downloads
/// from the browser is byte-for-byte the shape of the one they would have
/// produced at a terminal — same members, same redaction.
///
/// # Errors
///
/// The staging directory could not be created, or `tar` could not produce the
/// archive. The diagnostic itself failing is *not* an error.
pub fn build(cli: &Cli, config: Option<&Config>, dest: &Path) -> Result<Bundle, String> {
    build_with_recent_log(cli, config, dest, None)
}

/// The name of the member carrying the running process's in-memory log ring.
pub const RECENT_LOG_NAME: &str = "recent.log";

/// [`build`], plus the last lines the running process logged.
///
/// `journal.log` is empty on a default Raspberry Pi OS, whose journal is
/// volatile, and on any install not under systemd — both bundles a probe
/// pulled from a container held "No journal files were found" and nothing
/// else — while `/admin/system/logs` was replaying a 200-line in-process ring
/// the bundle never staged. The web hook passes that ring here as
/// `recent_log`; the command line, a separate process, has none and passes
/// `None`, and the member is then absent rather than empty.
///
/// # Errors
///
/// As [`build`].
pub fn build_with_recent_log(
    cli: &Cli,
    config: Option<&Config>,
    dest: &Path,
    recent_log: Option<&str>,
) -> Result<Bundle, String> {
    // Staged beside the destination rather than in a temp dir: same
    // filesystem, so `tar` writes the archive without crossing a device, and
    // an operator who ran out of space sees it at the path they chose rather
    // than in `/tmp`. `tempfile` is a dev-dependency here and staying that way
    // is worth a dozen lines.
    let staging = staging_dir(dest);
    let _cleanup = Cleanup(staging.clone());
    let dir = staging.join("birdnet-support");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create a staging directory: {e}"))?;

    let mut warnings: Vec<String> = Vec::new();
    let mut push = |r: Result<(), String>| {
        if let Err(e) = r {
            warnings.push(e);
        }
    };

    // The diagnostic, both ways round: JSON for a maintainer to grep, text for
    // a human reading the bundle without tooling.
    push(stage(
        &dir,
        "doctor.json",
        &crate::doctor::collect_json(cli, config),
    ));
    push(stage(
        &dir,
        "doctor.txt",
        &crate::doctor::collect_text(cli, config),
    ));

    push(stage(
        &dir,
        "version.txt",
        &format!(
            "birdnet-behavior {}\ntarget: {}\nprofile: {}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
        ),
    ));

    push(stage(
        &dir,
        "config.redacted",
        &config.map_or_else(
            || {
                "(no configuration file loaded; the settings saved from the web interface are \
                 in settings.redacted)\n"
                    .to_owned()
            },
            redacted_config,
        ),
    ));

    // What the browser saved. A station configured there has no file, so this
    // and the audio sources are the whole of its configuration.
    let (settings, audio_sources) = station_state_members(config);
    push(stage(&dir, "settings.redacted", &settings));
    push(stage(&dir, "audio-sources.redacted", &audio_sources));

    // The station's own ERROR/WARN log. This is the member that matters on a
    // default Raspberry Pi OS, where `/var/log/journal` does not exist and the
    // journal is therefore volatile: `journal.log` below is empty for
    // everything before the last boot, which is exactly the boot an operator
    // is filing a bug about. `errors.jsonl` survives it.
    push(stage(
        &dir,
        crate::log_capture::ERROR_LOG_NAME,
        &scrub_log(&read_error_log(config), config),
    ));

    if let Some(recent) = recent_log {
        push(stage(&dir, RECENT_LOG_NAME, &scrub_log(recent, config)));
    }

    push(stage(&dir, "uname.txt", &capture("uname", &["-a"])));
    push(stage(&dir, "disk.txt", &capture("df", &["-h"])));
    // CPU, memory, board and the USB/sound devices, read from /proc and /sys
    // without a serial number (see `hardware`).
    push(stage(
        &dir,
        "hardware.json",
        &hardware::inventory_json(Path::new("/")),
    ));
    push(stage(
        &dir,
        "journal.log",
        &scrub_log(
            &capture(
                "journalctl",
                &[
                    "-u",
                    "birdnet-behavior",
                    "-n",
                    JOURNAL_LINES,
                    "--no-pager",
                    "--output",
                    "short-iso",
                ],
            ),
            config,
        ),
    ));

    // `tar` rather than a crate, matching how the web backup builds its archive
    // — one fewer dependency and one fewer way for the two to disagree.
    let out = run_with_timeout(
        std::process::Command::new("tar")
            .arg("czf")
            .arg(dest)
            .arg("-C")
            .arg(&staging)
            .arg("birdnet-support"),
        ARCHIVE_TIMEOUT,
    )
    .map_err(|e| format!("could not run tar: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "tar exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let size = std::fs::metadata(dest).map_or(0, |m| m.len());
    Ok(Bundle { size, warnings })
}

/// `--support-bundle`: build the archive at `dest` and report on stdout.
///
/// Returns the process exit code: `0` on success, `2` when the bundle could
/// not be written.
pub fn run(cli: &Cli, config: Option<&Config>, dest: &Path) -> i32 {
    match build(cli, config, dest) {
        Ok(bundle) => {
            for w in &bundle.warnings {
                eprintln!("support: {w}");
            }
            println!(
                "Support bundle written to {} ({} bytes)",
                dest.display(),
                bundle.size
            );
            println!();
            println!("It contains the diagnostic report, the station's version, a redacted");
            println!("copy of the configuration file and of the settings and audio sources");
            println!("saved from the web interface, the hardware inventory (CPU, memory, USB and");
            println!(
                "sound devices; no serial numbers), the station's own error log, and the last"
            );
            println!("{JOURNAL_LINES} journal lines.");
            println!("Passwords, tokens and URL credentials are masked as {REDACTED} —");
            println!("check it before posting anywhere public all the same.");
            let _ = std::io::stdout().flush();
            0
        }
        Err(e) => {
            eprintln!("support: {e}");
            2
        }
    }
}

/// Where the bundle's members are assembled before `tar` sees them.
///
/// A sibling of the destination, so the archive never crosses a filesystem and
/// a full disk fails where the operator is looking.
fn staging_dir(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!(".birdnet-support-staging-{}", std::process::id()))
}

/// Removes the staging directory however the collection ends, including on an
/// early return: half a bundle left beside the archive is litter an operator
/// has no reason to recognise.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The default bundle path when the operator names none.
#[must_use]
pub fn default_path() -> PathBuf {
    PathBuf::from("birdnet-support.tar.gz")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── redaction: the half that matters ────────────────────────────────

    /// A journal as an older build left it, with every shape of leak this
    /// code base has had: the RTSP start line, ffmpeg's error, the Apprise
    /// address with its key in the path, a notify URL whose token is the host,
    /// and the SMTP password echoed in an error.
    #[test]
    fn a_journal_leaves_the_bundle_with_no_configured_secret_in_it() {
        let cfg = Config::parse(
            "RTSP_URL=rtsp://admin:camPW99@cam.lan/stream\n\
             APPRISE_URL=http://apprise.lan:8000/notify/k3yValue\n\
             NOTIFY_URLS=tgram://123456:BOTtoken/987\n\
             EMAIL_SMTP_PASS=mailPW77\n\
             LATITUDE=52.52",
        )
        .unwrap();
        let journal = "\
            started RTSP capture via ffmpeg url=rtsp://admin:camPW99@cam.lan/stream\n\
            capture subprocess: Error opening input file rtsp://admin:camPW99@cam.lan/stream.\n\
            Apprise notifications enabled url=http://apprise.lan:8000/notify/k3yValue\n\
            push failed for tgram://123456:BOTtoken/987\n\
            smtp auth failed with password mailPW77\n";
        let out = scrub_log(journal, Some(&cfg));
        // Named for what it is, a string to look for: `CodeQL`'s cleartext-logging
        // query treats a variable called `secret` as sensitive by its name and
        // reports the assertion message that prints it.
        for needle in ["camPW99", "k3yValue", "BOTtoken", "mailPW77"] {
            assert!(!out.contains(needle), "{needle} survived:\n{out}");
        }
        for kept in [
            "cam.lan/stream",
            "apprise.lan:8000",
            "tgram://",
            "smtp auth failed",
        ] {
            assert!(out.contains(kept), "{kept} was lost:\n{out}");
        }
    }

    /// The counterpart: a log with no secret in it is unchanged, and a
    /// short or ordinary setting value is not hunted through the text.
    #[test]
    fn a_clean_journal_is_unchanged() {
        let cfg = Config::parse("OVERLAP=1.25\nCADDY_PWD=on\nSF_THRESH=0.03").unwrap();
        let journal = "detection on 1.25 s overlap, sf 0.03, capture on\n";
        assert_eq!(scrub_log(journal, Some(&cfg)), journal);
        assert_eq!(scrub_log(journal, None), journal);
    }

    /// The bundle is written to be posted publicly; four decimals of latitude and
    /// longitude are a house. Coordinates go out to one decimal, in the
    /// configuration section and wherever the journal printed them.
    #[test]
    fn the_stations_coordinates_leave_the_bundle_at_one_decimal() {
        let cfg =
            Config::parse("LATITUDE=52.523431\nLONGITUDE=-13.405012\nSF_THRESH=0.03").unwrap();
        let out = redacted_config(&cfg);
        assert!(out.contains("LATITUDE=52.5"), "{out}");
        assert!(out.contains("LONGITUDE=-13.4"), "{out}");
        assert!(
            !out.contains("52.523431") && !out.contains("13.405012"),
            "{out}"
        );

        let journal = "BirdWeather uploads enabled lat=52.523431 lon=-13.405012\n\
                       analysed 152.5234312 samples in 52.523431.5 s\n";
        let scrubbed = scrub_log(journal, Some(&cfg));
        assert!(
            scrubbed.contains("lat=52.5 lon=-13.4"),
            "the journal's exact coordinates are coarsened: {scrubbed}"
        );
        // The counterpart: a longer number that merely contains the digits is
        // not a coordinate, and is left alone.
        assert!(
            scrubbed.contains("152.5234312 samples") && scrubbed.contains("52.523431.5 s"),
            "numbers that only contain the coordinate are untouched: {scrubbed}"
        );
    }

    /// A coordinate that cannot be read as a number may be degrees-minutes-
    /// seconds with the exact position in it: it is masked, not shown. And one
    /// already at one decimal is not hunted through a log.
    #[test]
    fn an_unreadable_coordinate_is_masked_and_a_coarse_one_is_not_hunted() {
        let cfg = Config::parse("LATITUDE=52°31'24\"N\nLONGITUDE=13.4").unwrap();
        let out = redacted_config(&cfg);
        assert!(out.contains(&format!("LATITUDE={REDACTED}")), "{out}");
        assert!(out.contains("LONGITUDE=13.4"), "{out}");
        let journal = "capture rate 13.4 MB/s\n";
        assert_eq!(scrub_log(journal, Some(&cfg)), journal);
    }

    #[test]
    fn redacted_config_masks_secrets_and_keeps_the_rest() {
        let cfg = Config::parse(
            "LATITUDE=52.52\nCADDY_PWD=hunter2\nRTSP_URL=rtsp://u:p@cam/stream\nSF_THRESH=0.03",
        )
        .unwrap();
        let out = redacted_config(&cfg);

        assert!(
            out.contains("LATITUDE=52.5") && !out.contains("52.52"),
            "the coordinate is shown to one decimal: {out}"
        );
        assert!(out.contains("SF_THRESH=0.03"));
        assert!(
            out.contains(&format!("CADDY_PWD={REDACTED}")),
            "the admin password must be masked: {out}"
        );
        assert!(
            !out.contains("hunter2") && !out.contains("u:p@"),
            "no secret may survive anywhere in the output: {out}"
        );
        assert!(
            out.contains("rtsp://u:"),
            "the username and host must remain: {out}"
        );
    }

    /// The fixture above has a dotless host, which is the one shape that never
    /// occurs in the field; with a dotted one the old composition returned
    /// `RTSP_URL=***@camera.local/stream` (`OB-11`). And a heartbeat URL's
    /// path is its credential (`OB-10`).
    #[test]
    fn redacted_config_keeps_a_dotted_camera_url_readable_and_hides_a_heartbeat_token() {
        let cfg = Config::parse(
            "RTSP_URL=rtsp://cam:secret@camera.local/stream\n\
             HEARTBEAT_URL=https://hc-ping.com/3f1e9c2a-7b44-4d1e-9c0a-5e6f7a8b9c0d\n\
             APPRISE_URL=http://apprise.local:8000/notify/garden",
        )
        .unwrap();
        let out = redacted_config(&cfg);
        assert!(
            out.contains(&format!(
                "RTSP_URL=rtsp://cam:{REDACTED}@camera.local/stream"
            )),
            "{out}"
        );
        assert!(
            !out.contains("3f1e9c2a"),
            "the heartbeat token leaked: {out}"
        );
        assert!(
            out.contains(&format!("HEARTBEAT_URL=https://hc-ping.com/{REDACTED}")),
            "{out}"
        );
        assert!(
            out.contains(&format!("APPRISE_URL=http://apprise.local:8000/{REDACTED}")),
            "{out}"
        );
    }

    /// The Flickr key is a secret and must not travel in a support bundle
    /// attached to a public issue.
    ///
    /// `is_secret_key`'s doc comment already claims `KEY` catches
    /// `FLICKR_API_KEY`. That claim was written before the setting existed, so
    /// it is asserted here rather than trusted — a prose claim about a key that
    /// did not exist is exactly the kind this repository has been caught out by.
    #[test]
    fn the_flickr_key_is_masked_and_the_rest_of_its_settings_are_not() {
        let cfg = Config::parse(
            "IMAGE_PROVIDER=flickr
FLICKR_API_KEY=abc123secret
FLICKR_FILTER_EMAIL=me@example.com",
        )
        .unwrap();
        let out = redacted_config(&cfg);

        assert!(
            out.contains(&format!("FLICKR_API_KEY={REDACTED}")),
            "the Flickr key must be masked: {out}"
        );
        assert!(
            !out.contains("abc123secret"),
            "and must not survive anywhere in the output: {out}"
        );
        // The counterpart, so this is not satisfied by a redactor that masks
        // everything: which provider a station uses is diagnostic and stays.
        assert!(
            out.contains("IMAGE_PROVIDER=flickr"),
            "the provider choice is diagnostic and must remain: {out}"
        );
        // The filter address is not a secret, but it is a person's email in a
        // bundle that gets attached to public issues. The domain is the
        // diagnostic half and stays; the local part does not.
        assert!(
            out.contains("FLICKR_FILTER_EMAIL=***@example.com"),
            "the address's local part must be masked and its domain kept: {out}"
        );
        assert!(!out.contains("me@example.com"), "{out}");
    }

    /// Sorted output, so two bundles from the same station diff cleanly.
    #[test]
    fn redacted_config_is_sorted() {
        let cfg = Config::parse("ZEBRA=1\nALPHA=2").unwrap();
        assert_eq!(redacted_config(&cfg), "ALPHA=2\nZEBRA=1");
    }

    // ── the bundle itself ───────────────────────────────────────────────

    #[test]
    fn a_bundle_is_written_and_contains_the_expected_members() {
        use clap::Parser as _;
        if !crate::doctor::tool_exists("tar") {
            eprintln!("tar not installed — bundle test skipped");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bundle.tar.gz");
        let cfg = Config::parse("LATITUDE=52.52\nCADDY_PWD=hunter2").unwrap();

        let code = run(&Cli::parse_from(["birdnet-behavior"]), Some(&cfg), &dest);
        assert_eq!(code, 0, "the bundle must be produced");
        assert!(dest.exists());

        let listing = capture("tar", &["tzf", dest.to_str().unwrap()]);
        for member in [
            "doctor.json",
            "doctor.txt",
            "version.txt",
            "config.redacted",
            "journal.log",
            "errors.jsonl",
        ] {
            assert!(
                listing.contains(member),
                "the bundle must contain {member}; got:\n{listing}"
            );
        }
    }

    /// The bundle must not carry the secret it was told about. This is the
    /// gate that would catch a future collector added without redaction.
    #[test]
    fn a_missing_error_log_says_so_rather_than_staging_nothing() {
        // "this station has never logged a warning" and "the bundle could not
        // find the log" are different answers, and only one is good news. An
        // empty member reads as the first whichever it was.
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::parse(&format!("DB_PATH={}/birds.db", dir.path().display())).unwrap();
        let text = read_error_log(Some(&cfg));
        assert!(text.contains("could not read"), "{text}");
        assert!(text.contains("errors.jsonl"), "{text}");
    }

    #[test]
    fn an_existing_error_log_is_carried_verbatim() {
        // Counterpart: the note must not replace real content.
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::parse(&format!("DB_PATH={}/birds.db", dir.path().display())).unwrap();
        std::fs::write(
            dir.path().join("errors.jsonl"),
            "{\"level\":\"ERROR\",\"message\":\"database is corrupt\"}\n",
        )
        .unwrap();
        let text = read_error_log(Some(&cfg));
        assert!(text.contains("database is corrupt"), "{text}");
        assert!(!text.contains("could not read"), "{text}");
    }

    #[test]
    fn no_secret_reaches_the_archive() {
        use clap::Parser as _;
        if !crate::doctor::tool_exists("tar") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bundle.tar.gz");
        let cfg = Config::parse("CADDY_PWD=swordfish-9271\nRTSP_URL=rtsp://a:swordfish-9271@cam/s")
            .unwrap();

        assert_eq!(
            run(&Cli::parse_from(["birdnet-behavior"]), Some(&cfg), &dest),
            0
        );

        // Extract and grep every member, rather than trusting the compressed
        // bytes: a secret could survive in any collector, not just the config.
        let out = dir.path().join("x");
        std::fs::create_dir_all(&out).unwrap();
        let _ = capture(
            "tar",
            &["xzf", dest.to_str().unwrap(), "-C", out.to_str().unwrap()],
        );
        let grep = capture("grep", &["-r", "swordfish-9271", out.to_str().unwrap()]);
        assert!(
            grep.trim().is_empty(),
            "a secret reached the bundle:\n{grep}"
        );
    }

    // ── the settings saved in the browser ───────────────────────────────

    /// A known secret and a known benign setting, as the `settings` table
    /// would hold them on a station that was configured in the browser.
    const KNOWN_SECRET: &str = "swordfish-5521";

    fn browser_settings() -> Vec<(String, String)> {
        [
            ("email_smtp_pass", KNOWN_SECRET),
            ("birdweather_token", "bwtoken-5521"),
            ("mqtt_password", KNOWN_SECRET),
            ("rtsp_url", "rtsp://cam:swordfish-5521@camera.lan/stream"),
            (
                "notify_urls",
                "tgram://123456:swordfish-5521/987\nntfy://my-private-topic",
            ),
            ("heartbeat_url", "https://hc-ping.example/swordfish-5521"),
            ("station_name", "Garden Station"),
            ("confidence_threshold", "0.70"),
            ("latitude", "52.523431"),
            ("notify_title_template", "line one\nline two"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    /// Seen failing with `shown_value` returning the value untouched: every
    /// secret then appeared, and with `redacted_settings` returning an empty
    /// string the benign settings went missing.
    #[test]
    fn the_browsers_settings_are_in_the_bundle_with_every_secret_masked() {
        let out = redacted_settings(&browser_settings());
        for needle in [KNOWN_SECRET, "bwtoken-5521", "my-private-topic", "123456"] {
            assert!(!out.contains(needle), "{needle} survived:\n{out}");
        }
        // Counterpart: what a maintainer needs is still there.
        for kept in [
            "station_name=Garden Station",
            "confidence_threshold=0.70",
            "rtsp_url=rtsp://cam:***REDACTED***@camera.lan/stream",
            "email_smtp_pass=***REDACTED***",
            "latitude=52.5\n",
            "notify_title_template=line one\\nline two",
            "ntfy://***REDACTED***",
        ] {
            assert!(out.contains(kept), "{kept} was lost:\n{out}");
        }
        assert_eq!(
            out.lines().count(),
            browser_settings().len(),
            "one line per setting:\n{out}"
        );
    }

    #[test]
    fn a_long_value_and_a_long_table_are_cut_and_say_so() {
        let long = "x".repeat(SETTING_VALUE_CAP + 40);
        let out = redacted_settings(&[("notes".to_owned(), long)]);
        assert!(out.contains("(… 40 more characters not shown)"), "{out}");

        let many: Vec<(String, String)> = (0..5_000)
            .map(|i| (format!("setting_{i:05}"), "v".repeat(40)))
            .collect();
        let out = redacted_settings(&many);
        assert!(out.len() <= STATE_MEMBER_CAP + 200, "{} bytes", out.len());
        assert!(
            out.contains("more rows not shown"),
            "{}",
            &out[out.len() - 200..]
        );
        // Counterpart: a table under the cap is whole.
        let few = redacted_settings(&many[..10]);
        assert!(!few.contains("not shown"), "{few}");
    }

    /// A station database as the wizard leaves it: settings, and one stream.
    fn browser_configured_station(dir: &Path) -> Config {
        let db = dir.join("birds.db");
        let conn = rusqlite::Connection::open(&db).expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        for (k, v) in browser_settings() {
            birdnet_db::settings::set(
                &conn,
                &k,
                &v,
                birdnet_db::settings::SettingsCategory::System,
            )
            .expect("setting");
        }
        use birdnet_db::audio_sources::{AudioSourceStore as _, NewAudioSource, SourceKind};
        conn.insert(&NewAudioSource::defaults(
            "cam1",
            SourceKind::Rtsp,
            format!("rtsp://viewer:{KNOWN_SECRET}@camera.lan/stream"),
        ))
        .expect("source");
        Config::parse(&format!("DB_PATH={}", db.display())).expect("config")
    }

    /// Seen failing with the two `stage` calls for the database members
    /// removed: the archive then held only the one-line `config.redacted`,
    /// and none of the station's settings.
    #[test]
    fn a_browser_configured_stations_bundle_carries_its_settings_and_no_secret() {
        use clap::Parser as _;
        if !crate::doctor::tool_exists("tar") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let cfg = browser_configured_station(dir.path());
        let dest = dir.path().join("bundle.tar.gz");
        let bundle =
            build(&Cli::parse_from(["birdnet-behavior"]), Some(&cfg), &dest).expect("the bundle");
        assert!(bundle.warnings.is_empty(), "{:?}", bundle.warnings);

        let out = dir.path().join("x");
        std::fs::create_dir_all(&out).unwrap();
        let _ = capture(
            "tar",
            &["xzf", dest.to_str().unwrap(), "-C", out.to_str().unwrap()],
        );
        let member = |name: &str| {
            std::fs::read_to_string(out.join("birdnet-support").join(name))
                .unwrap_or_else(|e| panic!("{name}: {e}"))
        };
        let settings = member("settings.redacted");
        assert!(
            settings.contains("station_name=Garden Station"),
            "{settings}"
        );
        let sources = member("audio-sources.redacted");
        assert!(
            sources.contains(
                "id=cam1 kind=rtsp device=rtsp://viewer:***REDACTED***@camera.lan/stream"
            ),
            "{sources}"
        );
        // Every member, not just these two: the secret is in the database
        // several times over and must be in the archive nowhere.
        let grep = capture("grep", &["-r", KNOWN_SECRET, out.to_str().unwrap()]);
        assert!(
            grep.trim().is_empty(),
            "a secret reached the bundle:\n{grep}"
        );
    }

    /// A database that cannot be read is said so, with where it looked; it is
    /// not "no settings", and it does not stop the rest of the bundle.
    #[test]
    fn an_unreadable_database_is_reported_in_the_members() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::parse(&format!("DB_PATH={}/absent.db", dir.path().display())).unwrap();
        let (settings, sources) = station_state_members(Some(&cfg));
        for text in [&settings, &sources] {
            assert!(text.contains("could not read"), "{text}");
            assert!(text.contains("absent.db"), "{text}");
        }
    }
}
