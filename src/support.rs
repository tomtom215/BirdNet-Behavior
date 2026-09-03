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
//! * **By shape** — [`redact_url_credentials`] strips `user:pass@` from
//!   anything URL-shaped in the values that *are* kept, because the key name
//!   `RTSP_URL` says nothing about a secret while its value very often
//!   contains one.
//!
//! Redaction replaces the value rather than dropping the line: "this station
//! has an SMTP password set" is diagnostic information, and a missing line
//! reads identically to a setting that was never configured.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use birdnet_core::config::Config;

use crate::cli::Cli;

/// What the redactor put in place of a secret.
pub const REDACTED: &str = "***REDACTED***";

/// How many journal lines to include.
///
/// Enough to cover a start-up and the failure that followed it; small enough
/// that the bundle stays attachable to an issue.
const JOURNAL_LINES: &str = "2000";

/// Whether a configuration key names a secret.
///
/// Substring matching on the *key*, deliberately over-broad: a false positive
/// costs one redacted line in a support bundle, a false negative costs an
/// operator their admin password in a public issue. `KEY` catches
/// `FLICKR_API_KEY` and would catch a hypothetical `API_KEY`; it also catches
/// innocuous names containing "key", which is the trade being made.
///
/// `PASS` rather than `PASSWORD` because the real key is `EMAIL_SMTP_PASS`, and
/// a needle that only matched the long form would have missed the one SMTP
/// secret a station actually stores. It would also match an audio setting named
/// `HIGH_PASS`, which is not a configuration key today — and if it ever became
/// one, a masked boolean costs a reader nothing while a leaked mail password
/// costs an operator their account.
#[must_use]
pub fn is_secret_key(key: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "PASS",
        "PWD",
        "SECRET",
        "TOKEN",
        "APIKEY",
        "API_KEY",
        "KEY",
        "CREDENTIAL",
        "AUTH",
        "SALT",
        "HASH",
    ];
    let upper = key.to_ascii_uppercase();
    NEEDLES.iter().any(|n| upper.contains(n))
}

/// Strip `user:pass@` from a URL-shaped value.
///
/// Returns the value unchanged when it carries no credentials, so a plain
/// `rtsp://camera.local/stream` stays readable — the hostname and path are
/// exactly what makes an RTSP problem diagnosable.
///
/// Only the authority between `://` and the first `/` (or end) is examined, so
/// an `@` in a path or query is left alone.
#[must_use]
pub fn redact_url_credentials(value: &str) -> String {
    let Some(scheme_end) = value.find("://") else {
        return value.to_owned();
    };
    let authority_start = scheme_end + 3;
    let rest = &value[authority_start..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];

    let Some(at) = authority.rfind('@') else {
        return value.to_owned();
    };
    // Keep the username: "which account is this camera using" is diagnostic,
    // and it is not the secret. Only the password half is replaced.
    let userinfo = &authority[..at];
    let host = &authority[at + 1..];
    let user = userinfo.split_once(':').map_or(userinfo, |(u, _)| u);

    format!(
        "{}{user}:{REDACTED}@{host}{}",
        &value[..authority_start],
        &rest[authority_end..]
    )
}

/// Mask the local part of a value that is a bare email address.
///
/// `me@example.com` becomes `***@example.com`. The domain stays because that
/// is the diagnostic half — "is this pointing at the right mail host", "is the
/// setting populated at all" — and the local part is the half that identifies a
/// person in a bundle attached to a public issue.
///
/// Deliberately narrow: exactly one `@`, no whitespace, a dot in the domain,
/// and something on both sides. A value that is not unambiguously one address
/// is returned untouched, because a broad "looks like an email" rule applied to
/// every configuration value would eventually mangle a setting nobody expected
/// it to touch.
#[must_use]
pub fn redact_email_local_part(value: &str) -> String {
    let Some((local, domain)) = value.split_once('@') else {
        return value.to_owned();
    };
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || !domain.contains('.')
        || value.chars().any(char::is_whitespace)
    {
        return value.to_owned();
    }
    format!("***@{domain}")
}

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
                redact_email_local_part(&redact_url_credentials(v))
            };
            format!("{k}={shown}")
        })
        .collect();
    lines.sort();
    lines.join("\n")
}

/// Run a command and capture its output, or a note explaining why not.
///
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

/// Collect a support bundle and write it to `dest`.
///
/// Returns the process exit code: `0` on success, `2` when the bundle could
/// not be written. The diagnostic itself failing is *not* an error — a station
/// too broken to pass `--doctor` is exactly the one that needs a bundle.
pub fn run(cli: &Cli, config: Option<&Config>, dest: &Path) -> i32 {
    // Staged beside the destination rather than in a temp dir: same
    // filesystem, so `tar` writes the archive without crossing a device, and
    // an operator who ran out of space sees it at the path they chose rather
    // than in `/tmp`. `tempfile` is a dev-dependency here and staying that way
    // is worth a dozen lines.
    let staging = staging_dir(dest);
    let _cleanup = Cleanup(staging.clone());
    let dir = staging.join("birdnet-support");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("support: could not create a staging directory: {e}");
        return 2;
    }

    let mut errors: Vec<String> = Vec::new();
    let mut push = |r: Result<(), String>| {
        if let Err(e) = r {
            errors.push(e);
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

    if !errors.is_empty() {
        for e in &errors {
            eprintln!("support: {e}");
        }
    }

    // `tar` rather than a crate, matching how the web backup builds its archive
    // — one fewer dependency and one fewer way for the two to disagree.
    let status = std::process::Command::new("tar")
        .arg("czf")
        .arg(dest)
        .arg("-C")
        .arg(&staging)
        .arg("birdnet-support")
        .status();

    match status {
        Ok(s) if s.success() => {
            let size = std::fs::metadata(dest).map_or(0, |m| m.len());
            println!(
                "Support bundle written to {} ({size} bytes)",
                dest.display()
            );
            println!();
            println!("It contains the diagnostic report, the station's version, a redacted");
            println!("copy of the configuration, and the last {JOURNAL_LINES} journal lines.");
            println!("Passwords, tokens and URL credentials are masked as {REDACTED} —");
            println!("check it before posting anywhere public all the same.");
            let _ = std::io::stdout().flush();
            0
        }
        Ok(s) => {
            eprintln!("support: tar exited with {s}");
            2
        }
        Err(e) => {
            eprintln!("support: could not run tar: {e}");
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
    fn secret_key_names_are_recognised() {
        for k in [
            "CADDY_PWD",
            "BIRDWEATHER_TOKEN",
            "BNB_SESSION_SECRET",
            "EMAIL_SMTP_PASS",
            "FLICKR_API_KEY",
            "BNB_SHARE_SECRET",
            "caddy_pwd",
        ] {
            assert!(is_secret_key(k), "{k} must be treated as a secret");
        }
    }

    /// The counterpart. Without it "redact everything" would pass the test
    /// above and produce a bundle with no diagnostic value at all.
    #[test]
    fn ordinary_keys_are_not_redacted() {
        for k in [
            "LATITUDE",
            "RECORDING_LENGTH",
            "MODEL_PATH",
            "BIRDNET_LISTEN",
            "SF_THRESH",
        ] {
            assert!(!is_secret_key(k), "{k} must survive into the bundle");
        }
    }

    #[test]
    fn url_credentials_are_stripped_but_the_host_survives() {
        assert_eq!(
            redact_url_credentials("rtsp://admin:hunter2@camera.local:554/stream"),
            format!("rtsp://admin:{REDACTED}@camera.local:554/stream")
        );
    }

    /// A URL without credentials must come through untouched — the hostname
    /// and path are what make an RTSP fault diagnosable.
    #[test]
    fn urls_without_credentials_are_untouched() {
        for url in [
            "rtsp://camera.local:554/stream",
            "http://localhost:8000",
            "not a url at all",
        ] {
            assert_eq!(redact_url_credentials(url), url);
        }
    }

    /// An `@` after the authority is part of the path, not credentials.
    /// Treating it as a userinfo separator would mangle the URL and hide the
    /// host — the opposite of the intent.
    #[test]
    fn an_at_sign_in_the_path_is_not_credentials() {
        let url = "http://example.com/path/@handle";
        assert_eq!(redact_url_credentials(url), url);
    }

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

    /// The email masker is narrow on purpose: a broad rule over every value
    /// would eventually mangle a setting nobody expected it to touch.
    #[test]
    fn only_an_unambiguous_email_address_has_its_local_part_masked() {
        assert_eq!(redact_email_local_part("me@example.com"), "***@example.com");
        assert_eq!(
            redact_email_local_part("first.last+tag@mail.example.co.uk"),
            "***@mail.example.co.uk"
        );
        for untouched in [
            "plughw:1,0",             // no @ at all
            "user@host",              // no dot: a LAN hostname, not an address
            "@example.com",           // no local part
            "me@",                    // no domain
            "a@b@example.com",        // not one address
            "see me@example.com now", // prose, not a value
        ] {
            assert_eq!(
                redact_email_local_part(untouched),
                untouched,
                "{untouched:?} must be left alone"
            );
        }
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
