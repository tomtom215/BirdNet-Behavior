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

use std::io::Write as _;
use std::path::{Path, PathBuf};

use birdnet_core::config::Config;
pub use birdnet_core::config::redact::{
    REDACTED, is_secret_key, redact_url_credentials, redact_value,
};

use crate::cli::Cli;

/// How many journal lines to include.
///
/// Enough to cover a start-up and the failure that followed it; small enough
/// that the bundle stays attachable to an issue.
const JOURNAL_LINES: &str = "2000";

/// Render the configuration with every secret masked.
///
/// Sorted, so two bundles from the same station diff cleanly.
#[must_use]
pub fn redacted_config(config: &Config) -> String {
    let mut lines: Vec<String> = config
        .iter()
        .map(|(k, v)| {
            let shown = if is_secret_key(k) {
                REDACTED.to_owned()
            } else {
                redact_value(v)
            };
            format!("{k}={shown}")
        })
        .collect();
    lines.sort();
    lines.join("\n")
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
    match std::process::Command::new(cmd).args(args).output() {
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
            || "(no configuration file loaded)\n".to_owned(),
            redacted_config,
        ),
    ));

    // The station's own ERROR/WARN log. This is the member that matters on a
    // default Raspberry Pi OS, where `/var/log/journal` does not exist and the
    // journal is therefore volatile: `journal.log` below is empty for
    // everything before the last boot, which is exactly the boot an operator
    // is filing a bug about. `errors.jsonl` survives it.
    push(stage(
        &dir,
        crate::log_capture::ERROR_LOG_NAME,
        &read_error_log(config),
    ));

    push(stage(&dir, "uname.txt", &capture("uname", &["-a"])));
    push(stage(&dir, "disk.txt", &capture("df", &["-h"])));
    push(stage(
        &dir,
        "journal.log",
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
    ));

    // `tar` rather than a crate, matching how the web backup builds its archive
    // — one fewer dependency and one fewer way for the two to disagree.
    let status = std::process::Command::new("tar")
        .arg("czf")
        .arg(dest)
        .arg("-C")
        .arg(&staging)
        .arg("birdnet-support")
        .status()
        .map_err(|e| format!("could not run tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar exited with {status}"));
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
            println!("copy of the configuration, the station's own error log, and the last");
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

    #[test]
    fn redacted_config_masks_secrets_and_keeps_the_rest() {
        let cfg = Config::parse(
            "LATITUDE=52.52\nCADDY_PWD=hunter2\nRTSP_URL=rtsp://u:p@cam/stream\nSF_THRESH=0.03",
        )
        .unwrap();
        let out = redacted_config(&cfg);

        assert!(out.contains("LATITUDE=52.52"));
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
}
