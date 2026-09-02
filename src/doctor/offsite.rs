//! Offsite backup checks: is there a copy anywhere but this SD card?
//!
//! The question this answers is not "is the config syntactically valid" but
//! "will the weekly backup actually leave the station" — and the ways it does
//! not are all quiet. A missing key, a `known_hosts` file that was never
//! written, a private key the SSH client will refuse to read because its mode
//! is `0644`: each of those produces a warning in a log nobody tails, once a
//! week, while the operator believes their records are safe.
//!
//! Deliberately no connection is made. `--doctor` runs from `ExecStartPre` on
//! every start, and a diagnostic that dials a remote host fails whenever the
//! uplink is down — which on a field station is often, and has nothing to do
//! with whether the configuration is right. Everything here is local.

use std::path::Path;

use birdnet_core::config::Config;
use birdnet_integrations::offsite::{Destination, sftp::HostKeyPolicy};

use super::Check;
use crate::cli::Cli;
use crate::helpers::offsite::{OffsitePlan, plan};

/// Name shared by the checks so the report reads as one group.
const NAME: &str = "Offsite backup";

pub(super) fn check_offsite(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    match plan(cli, config) {
        OffsitePlan::Off => vec![Check::warn(
            NAME,
            "off — backups exist only on this station's own storage",
            "an SD card that fails takes the database and every local backup with it. \
             Set OFFSITE_BACKUP to `s3` or `sftp` (with OFFSITE_PASSPHRASE) to keep a \
             copy somewhere else; see the Backups section of the manual",
        )],
        OffsitePlan::Broken(problems) => problems
            .into_iter()
            .map(|problem| {
                Check::fail(
                    NAME,
                    problem,
                    "offsite backups were asked for but cannot run; the weekly backup \
                     will stay on this station until this is fixed",
                )
            })
            .collect(),
        OffsitePlan::On(config) => {
            let mut out = vec![Check::pass(
                NAME,
                format!(
                    "{} — encrypted here, keeping the newest {}",
                    config.destination.describe(),
                    config.keep
                ),
            )];
            if let Destination::Sftp(t) = &config.destination {
                out.extend(check_ssh_material(
                    &t.identity_file,
                    &t.known_hosts,
                    t.host_key_policy,
                ));
            }
            out
        }
    }
}

/// The two files OpenSSH will refuse to work without, and the permission bit
/// it refuses on.
fn check_ssh_material(identity: &Path, known_hosts: &Path, policy: HostKeyPolicy) -> Vec<Check> {
    let mut out = Vec::new();

    if identity.exists() {
        if let Some(mode) = world_or_group_readable(identity) {
            out.push(Check::fail(
                NAME,
                format!(
                    "the private key {} is mode {mode:04o}; OpenSSH refuses a key \
                     other users can read",
                    identity.display()
                ),
                "run `chmod 600` on it. The refusal appears only in the SSH client's \
                 own stderr, so the weekly backup would fail silently",
            ));
        }
    } else {
        out.push(Check::fail(
            NAME,
            format!("the private key {} does not exist", identity.display()),
            "generate one with `ssh-keygen -t ed25519 -f <path>` and add its `.pub` \
             to the server's authorized_keys, or fix OFFSITE_SFTP_IDENTITY",
        ));
    }

    if known_hosts.exists() {
        return out;
    }
    out.push(match policy {
        HostKeyPolicy::Strict => Check::fail(
            NAME,
            format!("{} does not exist", known_hosts.display()),
            "with strict host key checking the server must already be known. Run \
             `ssh-keyscan -p <port> <host> > <that file>` and **check the \
             fingerprint against the server** before trusting it, or set \
             OFFSITE_SFTP_HOST_KEY_POLICY=accept-new for the first connection",
        ),
        HostKeyPolicy::AcceptNew => Check::warn(
            NAME,
            format!(
                "{} does not exist yet; the first connection will trust whatever \
                 answers",
                known_hosts.display()
            ),
            "that is what `accept-new` means, and it is reasonable on a network you \
             control. Once the file exists, set OFFSITE_SFTP_HOST_KEY_POLICY=yes so a \
             changed key is refused",
        ),
    });
    out
}

/// The mode, when anyone but the owner can read the file.
fn world_or_group_readable(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(path).ok()?.permissions().mode() & 0o777;
    (mode & 0o077 != 0).then_some(mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;

    fn write(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::write(path, "key").expect("write");
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(mode);
        std::fs::set_permissions(path, perms).expect("chmod");
    }

    #[test]
    fn a_private_key_others_can_read_is_a_failure_not_a_warning() {
        // OpenSSH refuses the key outright and says so only on its own stderr,
        // which for a weekly batch job nobody sees. A station in this state has
        // no offsite backups and no indication of it.
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = dir.path().join("id_ed25519");
        let known = dir.path().join("known_hosts");
        write(&identity, 0o644);
        write(&known, 0o644);

        let checks = check_ssh_material(&identity, &known, HostKeyPolicy::Strict);
        assert!(
            checks
                .iter()
                .any(|c| c.status == Status::Fail && c.message.contains("0644")),
            "a world-readable key must be reported as a failure: {checks:?}"
        );

        // Counterpart: 0600 is silent, or the check is a permanent alarm.
        write(&identity, 0o600);
        let checks = check_ssh_material(&identity, &known, HostKeyPolicy::Strict);
        assert!(
            checks.is_empty(),
            "a correctly configured pair should produce no findings: {checks:?}"
        );
    }

    #[test]
    fn a_missing_known_hosts_is_fatal_under_strict_and_a_warning_under_accept_new() {
        // The distinction is the whole point of having two policies: under
        // `yes` the connection cannot succeed, under `accept-new` it can and
        // the operator is told what that means.
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = dir.path().join("id_ed25519");
        write(&identity, 0o600);
        let missing = dir.path().join("known_hosts");

        let strict = check_ssh_material(&identity, &missing, HostKeyPolicy::Strict);
        assert!(
            strict.iter().any(|c| c.status == Status::Fail),
            "under strict checking a missing known_hosts cannot connect: {strict:?}"
        );

        let lenient = check_ssh_material(&identity, &missing, HostKeyPolicy::AcceptNew);
        assert!(
            lenient.iter().any(|c| c.status == Status::Warn),
            "under accept-new it is a warning, not a failure: {lenient:?}"
        );
        assert!(
            !lenient.iter().any(|c| c.status == Status::Fail),
            "accept-new must not be reported as fatal: {lenient:?}"
        );
    }

    #[test]
    fn a_missing_identity_file_is_reported_before_anything_tries_to_use_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let checks = check_ssh_material(
            &dir.path().join("nope"),
            &dir.path().join("known_hosts"),
            HostKeyPolicy::AcceptNew,
        );
        assert!(
            checks
                .iter()
                .any(|c| c.status == Status::Fail && c.message.contains("does not exist")),
            "{checks:?}"
        );
    }
}
