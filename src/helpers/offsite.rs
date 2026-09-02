//! Turn the offsite-backup config keys into one decided plan.
//!
//! Kept apart from the maintenance loop for the same reason
//! [`crate::helpers::tls`] is kept apart from `app.rs`: the interesting part is
//! not the wiring but what happens when a station is *half* configured, and
//! that is worth deciding in one place a unit test can reach.
//!
//! # Secrets do not get command-line flags
//!
//! `--offsite-backup` names the destination kind and that is the only flag.
//! The passphrase and the S3 secret key are read from the environment or the
//! config file only, deliberately: an argument passed on a command line is
//! visible in `ps` to every user on the box, is copied into the systemd journal
//! by `ExecStart=`, and ends up in shell history. There is no flag to add
//! later, either — [`SECRET_KEYS`] is checked against the CLI's own flag list
//! by a test, so one cannot be introduced without the decision being made
//! again.
//!
//! # Half-configured is an error, not a default
//!
//! A station that names an S3 bucket and forgets the secret key must not
//! quietly fall back to "offsite backups off". That is the shape of the defect
//! that leaves an operator believing they have offsite copies for a year. Every
//! missing piece is collected and reported together, so one restart fixes all
//! of them rather than one per attempt.

use std::path::PathBuf;

use birdnet_core::config::Config;
use birdnet_integrations::offsite::{
    Destination, OffsiteConfig, Passphrase,
    s3::{Addressing, S3Target},
    sftp::{HostKeyPolicy, SftpTarget},
    sigv4::Credentials,
};

use crate::cli::Cli;

/// Backups kept at the destination when the operator names no number.
///
/// Eight weekly snapshots is two months of history, which is long enough to
/// notice and recover from a problem that started quietly, and small enough
/// that it costs pennies a month in any object store.
pub const DEFAULT_KEEP: usize = 8;

/// Region assumed when none is set.
///
/// `us-east-1` rather than a guess from the endpoint: every S3-compatible
/// implementation that does not care about regions accepts it, and AWS itself
/// rejects a wrong one loudly rather than silently writing somewhere else.
pub const DEFAULT_REGION: &str = "us-east-1";

/// Config keys that hold a secret and therefore have no command-line flag.
///
/// Named here rather than only in prose so two tests can check it:
/// `no_secret_key_has_acquired_a_command_line_flag` against clap's own argument
/// list, and `every_secret_key_is_redacted_from_the_support_bundle` against the
/// redactor. Nothing in the running binary reads it — it is the record of a
/// decision, and the tests are what enforce it.
#[cfg_attr(not(test), allow(dead_code))]
pub const SECRET_KEYS: &[&str] = &["OFFSITE_PASSPHRASE", "OFFSITE_S3_SECRET_KEY"];

/// What the station should do about offsite backups.
#[derive(Debug)]
pub enum OffsitePlan {
    /// Nothing is configured. Not an error.
    Off,
    /// Configured and usable.
    On(Box<OffsiteConfig>),
    /// Asked for, but something is missing or wrong.
    ///
    /// Every problem found, not just the first: a station is usually missing
    /// two keys, and reporting them one restart at a time is how an operator
    /// gives up.
    Broken(Vec<String>),
}

/// Read a key from the config file, trimmed, treating empty as absent.
fn get(config: Option<&Config>, key: &str) -> Option<String> {
    config
        .and_then(|c| c.get(key))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Read a key from the environment, trimmed, treating empty as absent.
///
/// Checked before the config file so a container can inject a secret without
/// writing it to a file that ends up in a backup — which for
/// `OFFSITE_PASSPHRASE` would be circular in an unfortunate way.
fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Environment first, then the config file.
fn setting(config: Option<&Config>, key: &str) -> Option<String> {
    env(&format!("BIRDNET_{key}")).or_else(|| get(config, key))
}

/// Decide what to do.
///
/// Never fails: a station with a broken offsite configuration must still record
/// birds, so the failure is reported as [`OffsitePlan::Broken`] and the caller
/// decides how loudly to say so.
#[must_use]
pub fn plan(cli: &Cli, config: Option<&Config>) -> OffsitePlan {
    let mode = cli
        .offsite_backup
        .clone()
        .filter(|m| !m.trim().is_empty())
        .or_else(|| setting(config, "OFFSITE_BACKUP"))
        .unwrap_or_else(|| "off".to_owned());

    let mut problems = Vec::new();
    let destination = match mode.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "disabled" | "" => return OffsitePlan::Off,
        "s3" => s3_destination(config, &mut problems),
        "sftp" | "ssh" => sftp_destination(config, &mut problems),
        other => {
            return OffsitePlan::Broken(vec![format!(
                "`{other}` is not an offsite backup destination; use `off`, `s3` or `sftp`"
            )]);
        }
    };

    let passphrase = if let Some(value) = setting(config, "OFFSITE_PASSPHRASE") {
        match Passphrase::new(value) {
            Ok(p) => Some(p),
            Err(e) => {
                problems.push(e.to_string());
                None
            }
        }
    } else {
        problems.push(
            "OFFSITE_PASSPHRASE is not set. Backups are encrypted before they leave the \
             station and there is no way to turn that off — the passphrase is the only \
             thing standing between a rented bucket and a log of when the house is empty. \
             Choose a long one, write it down somewhere that is not this station, and set \
             it in the config or as BIRDNET_OFFSITE_PASSPHRASE"
                .to_owned(),
        );
        None
    };

    let keep = parsed_or(
        config,
        "OFFSITE_KEEP",
        DEFAULT_KEEP,
        "a number of backups to keep",
        &mut problems,
    );

    if let (Some(destination), Some(passphrase), true) =
        (destination, passphrase, problems.is_empty())
    {
        OffsitePlan::On(Box::new(OffsiteConfig {
            destination,
            passphrase,
            keep,
        }))
    } else {
        if problems.is_empty() {
            problems.push("the offsite destination could not be built".to_owned());
        }
        OffsitePlan::Broken(problems)
    }
}

/// Parse a numeric key, falling back to `default` and recording a problem.
///
/// The fallback matters: a station with `OFFSITE_KEEP=eight` should not stop
/// backing up, it should back up with the default retention and say what it
/// could not read. `plan` refuses the whole configuration anyway once
/// `problems` is non-empty — this exists so the *message* names the key rather
/// than the operator discovering it one restart at a time.
fn parsed_or<T: std::str::FromStr + Copy>(
    config: Option<&Config>,
    key: &str,
    default: T,
    what: &str,
    problems: &mut Vec<String>,
) -> T {
    let Some(raw) = setting(config, key) else {
        return default;
    };
    raw.parse::<T>().unwrap_or_else(|_| {
        problems.push(format!("{key} is `{raw}`, which is not {what}"));
        default
    })
}

/// Require a key, recording a problem when it is missing.
fn require(
    config: Option<&Config>,
    key: &str,
    what: &str,
    problems: &mut Vec<String>,
) -> Option<String> {
    let found = setting(config, key);
    if found.is_none() {
        problems.push(format!("{key} is not set ({what})"));
    }
    found
}

fn s3_destination(config: Option<&Config>, problems: &mut Vec<String>) -> Option<Destination> {
    let endpoint = require(
        config,
        "OFFSITE_S3_ENDPOINT",
        "the store's base URL, e.g. https://s3.eu-west-2.amazonaws.com \
         or http://minio.lan:9000",
        problems,
    );
    let bucket = require(config, "OFFSITE_S3_BUCKET", "the bucket name", problems);
    let access_key = require(
        config,
        "OFFSITE_S3_ACCESS_KEY",
        "the access key ID",
        problems,
    );
    let secret_key = require(
        config,
        "OFFSITE_S3_SECRET_KEY",
        "the secret access key",
        problems,
    );

    let endpoint = endpoint?;
    // The host, for the addressing guess. A malformed endpoint is reported by
    // `S3Target::address` at run time; here it only affects the default.
    let host = endpoint
        .split_once("://")
        .map_or(endpoint.as_str(), |(_, rest)| rest)
        .trim_end_matches('/');

    let addressing = match Addressing::parse(
        &setting(config, "OFFSITE_S3_ADDRESSING").unwrap_or_default(),
        host,
    ) {
        Ok(a) => a,
        Err(bad) => {
            problems.push(format!(
                "OFFSITE_S3_ADDRESSING is `{bad}`; use `auto`, `virtual` or `path`"
            ));
            return None;
        }
    };

    Some(Destination::S3(Box::new(S3Target {
        endpoint,
        bucket: bucket?,
        prefix: setting(config, "OFFSITE_S3_PREFIX").unwrap_or_default(),
        region: setting(config, "OFFSITE_S3_REGION").unwrap_or_else(|| DEFAULT_REGION.to_owned()),
        credentials: Credentials {
            access_key: access_key?,
            secret_key: secret_key?,
        },
        addressing,
    })))
}

fn sftp_destination(config: Option<&Config>, problems: &mut Vec<String>) -> Option<Destination> {
    let host = require(
        config,
        "OFFSITE_SFTP_HOST",
        "the server's hostname",
        problems,
    );
    let user = require(config, "OFFSITE_SFTP_USER", "the login name", problems);
    let dir = require(
        config,
        "OFFSITE_SFTP_DIR",
        "the directory backups are written to",
        problems,
    );
    let identity = require(
        config,
        "OFFSITE_SFTP_IDENTITY",
        "the private key file; password authentication is disabled because a \
         batch upload cannot answer a prompt",
        problems,
    );

    let port = parsed_or(config, "OFFSITE_SFTP_PORT", 22u16, "a port", problems);

    let host_key_policy = match HostKeyPolicy::parse(
        &setting(config, "OFFSITE_SFTP_HOST_KEY_POLICY").unwrap_or_default(),
    ) {
        Ok(p) => p,
        Err(why) => {
            problems.push(format!("OFFSITE_SFTP_HOST_KEY_POLICY: {why}"));
            return None;
        }
    };

    // Defaults to the conventional location beside the identity file, so an
    // operator who set up keys the usual way needs no extra setting.
    let identity = PathBuf::from(identity?);
    let known_hosts = setting(config, "OFFSITE_SFTP_KNOWN_HOSTS").map_or_else(
        || {
            identity
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("known_hosts")
        },
        PathBuf::from,
    );

    Some(Destination::Sftp(Box::new(SftpTarget {
        host: host?,
        port,
        user: user?,
        remote_dir: dir?,
        identity_file: identity,
        known_hosts,
        host_key_policy,
    })))
}

/// Decrypt one offsite backup file, for `--decrypt-backup`.
///
/// Returns a process exit code: `0` on success, `1` on any failure. Every
/// failure prints one line saying which of the three things went wrong — the
/// passphrase, the file, or the destination — because an operator running this
/// is having a bad day already.
///
/// The plaintext is written through a `.partial` name and renamed. An existing
/// destination is refused rather than overwritten, because the most likely
/// `--out` on a station is the live `birds.db`.
///
/// **What the rename does and does not buy.** For every failure this code can
/// observe — a wrong passphrase, a truncated file, a full disk — the partial is
/// removed explicitly, and a gate covers that. The rename adds one thing on top:
/// if the process is killed outright (a power cut, an OOM kill) `--out` never
/// exists half-written, so there is nothing for an operator to `mv` over
/// `birds.db` by mistake. That last property is **not** covered by a test:
/// observing it means killing the process at an arbitrary point mid-write, and a
/// gate built on that timing would be flaky, which is worse than an honest note.
/// Removing the rename does not fail any test in this repository; it is here on
/// the argument, not on the evidence.
pub fn run_decrypt(cli: &Cli, config: Option<&Config>) -> i32 {
    use birdnet_integrations::offsite::envelope;

    let Some(source) = cli.decrypt_backup.as_ref() else {
        eprintln!("--decrypt-backup needs a file");
        return 1;
    };
    let Some(dest) = cli.out.as_ref() else {
        eprintln!("--decrypt-backup needs --out <path> to write the database to");
        return 1;
    };
    if dest.exists() {
        eprintln!(
            "{} already exists. Refusing to overwrite it — write somewhere else and \
             move the file into place yourself.",
            dest.display()
        );
        return 1;
    }

    let Some(passphrase) = setting(config, "OFFSITE_PASSPHRASE") else {
        eprintln!(
            "OFFSITE_PASSPHRASE is not set. It is the only thing that can open this \
             file: set it in the config, or run with \
             BIRDNET_OFFSITE_PASSPHRASE=... in the environment."
        );
        return 1;
    };

    let mut input = match std::fs::File::open(source) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("cannot read {}: {e}", source.display());
            return 1;
        }
    };

    let partial = dest.with_extension("partial");
    let mut output = match std::fs::File::create(&partial) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("cannot write {}: {e}", partial.display());
            return 1;
        }
    };

    match envelope::decrypt(&passphrase, &mut input, &mut output) {
        Ok(bytes) => {
            drop(output);
            if let Err(e) = std::fs::rename(&partial, dest) {
                eprintln!("cannot move {} into place: {e}", partial.display());
                let _ = std::fs::remove_file(&partial);
                return 1;
            }
            println!("wrote {} ({bytes} bytes)", dest.display());
            0
        }
        Err(e) => {
            drop(output);
            let _ = std::fs::remove_file(&partial);
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Cli` with nothing set, so each test names only what it varies.
    fn cli() -> Cli {
        <Cli as clap::Parser>::parse_from(["birdnet-behavior"])
    }

    /// A config built from `KEY=value` lines.
    fn config(lines: &[&str]) -> Config {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("birdnet.conf");
        std::fs::write(&path, lines.join("\n")).expect("write");
        let cfg = Config::load_from(&path).expect("parse");
        // The temp dir is dropped here; `Config` has already read the file.
        drop(dir);
        cfg
    }

    #[test]
    fn nothing_configured_is_off_and_not_an_error() {
        assert!(matches!(plan(&cli(), None), OffsitePlan::Off));
        assert!(matches!(
            plan(&cli(), Some(&config(&["OFFSITE_BACKUP=off"]))),
            OffsitePlan::Off
        ));
    }

    #[test]
    fn a_half_configured_destination_reports_every_missing_piece_at_once() {
        // The defect this exists to stop: a station that names a bucket and
        // forgets the secret key must not fall back to "off" and let an
        // operator believe they have offsite copies. And it must not report the
        // missing keys one restart at a time.
        let cfg = config(&["OFFSITE_BACKUP=s3", "OFFSITE_S3_BUCKET=birdnet"]);
        let OffsitePlan::Broken(problems) = plan(&cli(), Some(&cfg)) else {
            panic!("a half-configured destination must not be reported as off or on");
        };
        let all = problems.join("\n");
        for missing in [
            "OFFSITE_S3_ENDPOINT",
            "OFFSITE_S3_ACCESS_KEY",
            "OFFSITE_S3_SECRET_KEY",
            "OFFSITE_PASSPHRASE",
        ] {
            assert!(
                all.contains(missing),
                "`{missing}` was not reported; an operator would fix one key per \
                 restart:\n{all}"
            );
        }
    }

    #[test]
    fn a_fully_configured_s3_destination_is_usable() {
        // The counterpart: the gate above is satisfied by a planner that
        // reports everything broken.
        let cfg = config(&[
            "OFFSITE_BACKUP=s3",
            "OFFSITE_S3_ENDPOINT=https://s3.eu-west-2.amazonaws.com",
            "OFFSITE_S3_BUCKET=birdnet",
            "OFFSITE_S3_PREFIX=stations/pi-1",
            "OFFSITE_S3_REGION=eu-west-2",
            "OFFSITE_S3_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE",
            "OFFSITE_S3_SECRET_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "OFFSITE_PASSPHRASE=correct horse battery staple",
        ]);
        let OffsitePlan::On(plan) = plan(&cli(), Some(&cfg)) else {
            panic!("a complete configuration must be usable");
        };
        assert_eq!(plan.keep, DEFAULT_KEEP);
        let Destination::S3(t) = &plan.destination else {
            panic!("expected an S3 destination");
        };
        assert_eq!(t.bucket, "birdnet");
        assert_eq!(t.region, "eu-west-2");
        assert_eq!(
            t.addressing,
            Addressing::VirtualHost,
            "an amazonaws.com endpoint should default to virtual-host addressing"
        );
    }

    #[test]
    fn a_self_hosted_endpoint_defaults_to_path_addressing() {
        let cfg = config(&[
            "OFFSITE_BACKUP=s3",
            "OFFSITE_S3_ENDPOINT=http://minio.lan:9000",
            "OFFSITE_S3_BUCKET=birdnet",
            "OFFSITE_S3_ACCESS_KEY=k",
            "OFFSITE_S3_SECRET_KEY=s",
            "OFFSITE_PASSPHRASE=correct horse battery staple",
        ]);
        let OffsitePlan::On(plan) = plan(&cli(), Some(&cfg)) else {
            panic!("expected a usable plan");
        };
        let Destination::S3(t) = &plan.destination else {
            panic!("expected S3");
        };
        assert_eq!(t.addressing, Addressing::Path);
        assert_eq!(t.region, DEFAULT_REGION);
    }

    #[test]
    fn an_sftp_destination_defaults_known_hosts_beside_the_key() {
        let cfg = config(&[
            "OFFSITE_BACKUP=sftp",
            "OFFSITE_SFTP_HOST=backup.example.net",
            "OFFSITE_SFTP_USER=birdnet",
            "OFFSITE_SFTP_DIR=/srv/backups/pi-1",
            "OFFSITE_SFTP_IDENTITY=/var/lib/birdnet/ssh/id_ed25519",
            "OFFSITE_PASSPHRASE=correct horse battery staple",
        ]);
        let OffsitePlan::On(plan) = plan(&cli(), Some(&cfg)) else {
            panic!("expected a usable plan");
        };
        let Destination::Sftp(t) = &plan.destination else {
            panic!("expected SFTP");
        };
        assert_eq!(t.port, 22);
        assert_eq!(
            t.known_hosts,
            PathBuf::from("/var/lib/birdnet/ssh/known_hosts"),
            "known_hosts should default beside the identity file"
        );
        assert_eq!(t.host_key_policy, HostKeyPolicy::Strict);
    }

    #[test]
    fn turning_host_key_checking_off_is_a_broken_plan_not_a_silent_downgrade() {
        // An operator whose first connection failed will reach for this. It has
        // to be refused where they can see it, not accepted and ignored.
        let cfg = config(&[
            "OFFSITE_BACKUP=sftp",
            "OFFSITE_SFTP_HOST=backup.example.net",
            "OFFSITE_SFTP_USER=birdnet",
            "OFFSITE_SFTP_DIR=/srv/backups",
            "OFFSITE_SFTP_IDENTITY=/var/lib/birdnet/ssh/id_ed25519",
            "OFFSITE_SFTP_HOST_KEY_POLICY=no",
            "OFFSITE_PASSPHRASE=correct horse battery staple",
        ]);
        let OffsitePlan::Broken(problems) = plan(&cli(), Some(&cfg)) else {
            panic!("`no` must not produce a working plan");
        };
        assert!(
            problems.join(" ").contains("cannot be turned off"),
            "the refusal should explain itself: {problems:?}"
        );
    }

    #[test]
    fn a_short_passphrase_is_refused_rather_than_used() {
        let cfg = config(&[
            "OFFSITE_BACKUP=s3",
            "OFFSITE_S3_ENDPOINT=https://x.example",
            "OFFSITE_S3_BUCKET=b",
            "OFFSITE_S3_ACCESS_KEY=k",
            "OFFSITE_S3_SECRET_KEY=s",
            "OFFSITE_PASSPHRASE=hunter2",
        ]);
        let OffsitePlan::Broken(problems) = plan(&cli(), Some(&cfg)) else {
            panic!("a 7-character passphrase must not produce a working plan");
        };
        assert!(
            problems.join(" ").contains("at least"),
            "the refusal should say how long it needs to be: {problems:?}"
        );
    }

    #[test]
    fn an_unknown_destination_kind_is_named_rather_than_ignored() {
        let cfg = config(&["OFFSITE_BACKUP=dropbox"]);
        let OffsitePlan::Broken(problems) = plan(&cli(), Some(&cfg)) else {
            panic!("an unknown destination must not be silently off");
        };
        assert!(problems.join(" ").contains("dropbox"), "{problems:?}");
    }

    #[test]
    fn every_secret_key_is_redacted_from_the_support_bundle() {
        // The support bundle is a file operators are asked to attach to public
        // issues. `is_secret_key` is a substring match on a list of needles, so
        // whether a new key is caught is a coincidence of naming until somebody
        // checks — `OFFSITE_PASSPHRASE` is caught by `PASS`, and a rename to
        // `OFFSITE_ENCRYPTION_PHRASE` would silently stop being.
        for key in SECRET_KEYS {
            assert!(
                crate::support::is_secret_key(key),
                "`{key}` holds a secret but the support bundle would print it \
                 verbatim. Add a needle to `support::is_secret_key`, or name the \
                 key so an existing one matches"
            );
        }
        // Counterpart, so this is not satisfied by a redactor that masks
        // everything and makes the bundle useless.
        for ordinary in [
            "OFFSITE_BACKUP",
            "OFFSITE_S3_ENDPOINT",
            "OFFSITE_S3_BUCKET",
            "OFFSITE_SFTP_HOST",
            "OFFSITE_SFTP_DIR",
            "OFFSITE_KEEP",
        ] {
            assert!(
                !crate::support::is_secret_key(ordinary),
                "`{ordinary}` is not a secret and should stay readable in a \
                 support bundle"
            );
        }
    }

    #[test]
    fn no_secret_key_has_acquired_a_command_line_flag() {
        // A secret passed as an argument is visible in `ps` to every user on
        // the box, is copied into the journal by systemd's `ExecStart=`, and
        // lands in shell history. The decision not to offer one is recorded
        // here so adding a flag later fails this test rather than shipping.
        use clap::CommandFactory as _;
        let command = Cli::command();
        let flags: Vec<String> = command
            .get_arguments()
            .filter_map(|a| a.get_long().map(ToOwned::to_owned))
            .collect();
        for key in SECRET_KEYS {
            let flag = key.to_ascii_lowercase().replace('_', "-");
            assert!(
                !flags.contains(&flag),
                "`--{flag}` exists. Secrets must not be passable on the command \
                 line: read {key} from the environment or the config file"
            );
        }
        // Counterpart, so this is not satisfied by the CLI having no flags at
        // all: the non-secret one is present.
        assert!(
            flags.iter().any(|f| f == "offsite-backup"),
            "--offsite-backup should exist: {flags:?}"
        );
    }
}
