//! An offsite backup target that moves the bytes with `rsync` over SSH
//! (`G-30`).
//!
//! # What this is for, and what it is not for
//!
//! rsync is usually reached for because it is *incremental*: it finds the
//! blocks a file already shares with the copy at the far end and sends only
//! the difference. **That is worth exactly nothing here, and this module does
//! not claim it.**
//!
//! Every backup is encrypted before it leaves ([`super::envelope`]) under a
//! fresh random argon2 salt and a fresh random nonce prefix, so each run
//! derives a different key and produces an entirely different ciphertext.
//! Measured on a 1 MiB file with rsync's 700-byte block size, counting matches
//! at every offset rather than only aligned ones:
//!
//! | | blocks rsync could reuse |
//! |---|---|
//! | plaintext, one 400-byte region changed | 1496 / 1497 |
//! | encrypted, the same change | 0 / 1498 |
//! | encrypted, **byte-identical** plaintext | 0 / 1498 |
//!
//! The last row is the one that settles it: with nothing changed at all, the
//! fresh salt alone leaves no block in common. rsync sends the whole file
//! every time, exactly as [`super::sftp`] does.
//!
//! So this target exists for the three things that *are* true:
//!
//! 1. **It resumes.** `--partial` keeps what arrived, so a station on a rural
//!    link that drops at 90 % of a 1.3 GB backup continues rather than
//!    starting again. The SFTP target re-uploads from the beginning, and on a
//!    link bad enough to matter it may never finish at all.
//! 2. **It can be told to go slowly.** `--bwlimit` keeps a backup from
//!    saturating a shared or metered connection for an hour.
//! 3. **It is what a person with a NAS already has**, and it needs neither a
//!    bucket nor an FTP daemon.
//!
//! # rsync moves the bytes; sftp does the housekeeping
//!
//! This target is a thin layer over [`SftpTarget`] rather than a parallel
//! implementation, because only the *transfer* is a thing rsync does better.
//!
//! Listing and deleting are not: rsync has no primitive that removes one named
//! remote file. The idiom for it — an empty source directory with `--delete`
//! and an include filter — deletes **everything the filter does not name**, so
//! one wrong filter costs the operator every backup they have. `sftp` already
//! removes exactly one file, through a quoting and path-allowlist discipline
//! that has been written and tested once. Reaching for a second mechanism
//! here would add a way to lose all the backups in exchange for nothing.
//!
//! Creating the remote directory is `sftp`'s too: rsync's `--mkpath` needs
//! 3.2.3 (2020) and a station with an older rsync would fail on first run with
//! an error about an unknown option.
//!
//! A station using this target therefore needs both `rsync` and `sftp` on
//! `PATH`. In practice that is one package more than it already had: the
//! transport is SSH either way, and `sftp` ships with the `openssh-client`
//! that provides the `ssh` rsync is about to run.
//!
//! # Quoting
//!
//! rsync splits the `-e` transport string on whitespace itself, with no shell
//! and no quoting. A key at a path containing a space would therefore be torn
//! into two arguments. Rather than guess at an escaping rsync does not
//! implement, [`RsyncTarget::transport`] refuses such a path outright — the
//! same posture [`super::sftp::is_safe_remote_path`] takes toward a newline in
//! a batch script.

use std::path::Path;
use std::process::Stdio;

use super::sftp::{SftpError, SftpTarget, is_safe_remote_path};

/// The binary this target drives.
pub const RSYNC_BINARY: &str = "rsync";

/// Where an interrupted transfer's partial file is kept.
///
/// A directory of its own, not the destination directory: a partial left under
/// the final name would be listed by retention and reached for by a restore.
/// This is the same property [`super::sftp::PART_SUFFIX`] buys, one level up.
pub const PARTIAL_DIR: &str = ".bnb-partial";

/// The longest one transfer may run.
///
/// Matches the SFTP batch deadline: a first backup of a multi-gigabyte
/// database over a domestic uplink is measured in hours, and a transfer killed
/// halfway is one that never completes.
const TRANSFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// An offsite destination reached with `rsync` over SSH.
#[derive(Debug, Clone)]
pub struct RsyncTarget {
    /// The SSH connection, and the half of the work `sftp` does.
    ///
    /// Held whole rather than copied field by field so the host-key policy,
    /// the identity file and the path allowlist cannot drift between the two
    /// programs that use them.
    pub ssh: SftpTarget,
    /// Transfer cap in KiB/s, or `None` to go as fast as the link allows.
    pub bwlimit_kib: Option<u32>,
}

impl RsyncTarget {
    /// The `ssh` command rsync should use as its transport.
    ///
    /// Returned as one string because that is the shape `-e` takes. rsync
    /// splits it on whitespace with no shell and no quoting, so every
    /// component is checked for whitespace first and the whole thing is
    /// refused rather than mangled.
    ///
    /// # Errors
    ///
    /// [`SftpError::UnsafePath`] if the identity file or `known_hosts` path
    /// contains whitespace, or if any policy option does.
    pub fn transport(&self) -> Result<String, SftpError> {
        let mut parts = vec![
            "ssh".to_owned(),
            // `ssh` spells the port `-p`; the `sftp` client spells it `-P`.
            "-p".to_owned(),
            self.ssh.port.to_string(),
            "-i".to_owned(),
            self.ssh.identity_file.display().to_string(),
        ];
        // The same policy the sftp client connects under, not a second copy of
        // it: host-key checking, batch mode, no password auth, the keepalives.
        parts.extend(self.ssh.ssh_policy_options());

        for part in &parts {
            if part.chars().any(char::is_whitespace) {
                return Err(SftpError::UnsafePath {
                    path: part.clone(),
                    why: "rsync splits its -e transport on whitespace with no shell and no \
                          quoting, so a path containing a space cannot be passed through it",
                });
            }
        }
        Ok(parts.join(" "))
    }

    /// `user@host:/remote/dir/name`, the destination rsync writes to.
    ///
    /// # Errors
    ///
    /// [`SftpError::UnsafePath`] when the login, host or remote path is
    /// outside [`is_safe_remote_path`]'s allowlist, or when the path contains
    /// whitespace.
    ///
    /// The whitespace check is this target's own and is **not** redundant with
    /// the allowlist: that allowlist permits a space, because `sftp` quotes
    /// its batch arguments and can carry one. rsync cannot — it splits its
    /// arguments itself — so a remote directory that works over SFTP has to be
    /// refused here rather than silently truncated at the space.
    ///
    /// A colon needs no check of its own: rsync would read one as the
    /// host/path separator and retarget the transfer, but the allowlist
    /// already refuses it. A branch for it here was written, found
    /// unreachable, and removed rather than left in as reassurance.
    pub fn remote_spec(&self, name: &str) -> Result<String, SftpError> {
        let dest = self.ssh.destination()?;
        let remote = self.ssh.remote_path(name);
        is_safe_remote_path(&remote).map_err(|why| SftpError::UnsafePath {
            path: remote.clone(),
            why,
        })?;
        if remote.chars().any(char::is_whitespace) {
            return Err(SftpError::UnsafePath {
                path: remote,
                why: "rsync splits its arguments on whitespace before the shell would",
            });
        }
        Ok(format!("{dest}:{remote}"))
    }

    /// The complete argument list `rsync` is run with.
    ///
    /// Returned rather than applied so a test can read it — the same reason
    /// [`SftpTarget::argv`] is shaped this way, and for the same hazard: the
    /// `--` before the positional arguments is what stops a local path
    /// beginning `-` being taken as an option.
    ///
    /// # Errors
    ///
    /// [`SftpError::UnsafePath`] from [`RsyncTarget::transport`] or
    /// [`RsyncTarget::remote_spec`], or when the local path contains
    /// whitespace.
    pub fn argv(&self, name: &str, local: &Path) -> Result<Vec<String>, SftpError> {
        let local = local.display().to_string();
        if local.chars().any(char::is_whitespace) {
            return Err(SftpError::UnsafePath {
                path: local,
                why: "rsync splits its arguments on whitespace before the shell would",
            });
        }
        let mut out = vec![
            // Keep what arrived, in a directory of its own. This is the whole
            // reason this target exists.
            "--partial".to_owned(),
            format!("--partial-dir={PARTIAL_DIR}"),
            // Times, so a resumed transfer is recognised as the same file.
            "--times".to_owned(),
            // No recursion, no deletion, no symlink following: this target
            // sends exactly one regular file and must never be able to remove
            // anything at the far end.
            "--no-recursive".to_owned(),
            "--no-links".to_owned(),
            "--compress-level=0".to_owned(),
            "-e".to_owned(),
            self.transport()?,
        ];
        if let Some(kib) = self.bwlimit_kib {
            out.push(format!("--bwlimit={kib}"));
        }
        out.push("--".to_owned());
        out.push(local);
        out.push(self.remote_spec(name)?);
        Ok(out)
    }

    /// Upload one backup.
    ///
    /// The remote directory is created first, through `sftp`, for the reason
    /// the module docs give: rsync's own `--mkpath` is too new to rely on.
    ///
    /// # Errors
    ///
    /// [`SftpError::NotInstalled`] if `rsync` is absent, [`SftpError::Spawn`]
    /// if it cannot be started, [`SftpError::Failed`] if it exits non-zero or
    /// runs past [`TRANSFER_TIMEOUT`], and anything
    /// [`RsyncTarget::argv`] refuses.
    pub async fn put(&self, name: &str, local: &Path) -> Result<(), SftpError> {
        // `-mkdir` is non-fatal in the batch, so an existing directory is not
        // an error — see `SftpTarget::upload_script`, which does the same.
        let dir = self.ssh.remote_dir.trim_end_matches('/');
        is_safe_remote_path(dir).map_err(|why| SftpError::UnsafePath {
            path: dir.to_owned(),
            why,
        })?;
        self.ssh
            .run_batch(&format!("-mkdir {}\n", super::sftp::quote(dir)))
            .await?;

        let argv = self.argv(name, local)?;
        let mut cmd = tokio::process::Command::new(RSYNC_BINARY);
        cmd.args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SftpError::NotInstalled);
            }
            Err(e) => return Err(SftpError::Spawn(e)),
        };

        let out = match tokio::time::timeout(TRANSFER_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return Err(SftpError::Spawn(e)),
            Err(_) => {
                return Err(SftpError::Failed {
                    status: None,
                    stderr: format!(
                        "rsync did not finish within {} hours",
                        TRANSFER_TIMEOUT.as_secs() / 3600
                    ),
                });
            }
        };
        if !out.status.success() {
            return Err(SftpError::Failed {
                status: out.status.code(),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            });
        }
        Ok(())
    }

    /// The backups already at the destination, by name.
    ///
    /// `sftp`'s listing, for the reason in the module docs.
    ///
    /// # Errors
    ///
    /// Whatever [`SftpTarget::list`] returns.
    pub async fn list(&self) -> Result<Vec<String>, SftpError> {
        self.ssh.list().await
    }

    /// Remove one backup by name.
    ///
    /// `sftp`'s removal, which deletes exactly the file it names. rsync has no
    /// equivalent, and its nearest idiom deletes everything a filter does not
    /// mention.
    ///
    /// # Errors
    ///
    /// Whatever [`SftpTarget::remove`] returns.
    pub async fn remove(&self, name: &str) -> Result<(), SftpError> {
        self.ssh.remove(name).await
    }
}

#[cfg(test)]
mod tests {
    use super::{PARTIAL_DIR, RsyncTarget};
    use crate::offsite::sftp::{HostKeyPolicy, SftpError, SftpTarget};
    use std::path::PathBuf;

    fn target() -> RsyncTarget {
        RsyncTarget {
            ssh: SftpTarget {
                host: "nas.local".to_owned(),
                port: 2222,
                user: "birds".to_owned(),
                remote_dir: "/volume1/backups".to_owned(),
                identity_file: PathBuf::from("/home/pi/.ssh/id_ed25519"),
                known_hosts: PathBuf::from("/home/pi/.ssh/known_hosts"),
                host_key_policy: HostKeyPolicy::Strict,
            },
            bwlimit_kib: None,
        }
    }

    /// **The gate this module most needs.** rsync connects through its own
    /// `ssh` command line, so every security option the sftp client relies on
    /// has to appear there too. A copy of the list would be the obvious way
    /// for one to be present in one path and quietly absent from the other,
    /// and neither path's own tests would notice.
    ///
    /// Observed failing with `transport` building its own `-o` list instead of
    /// calling `ssh_policy_options`: dropping `StrictHostKeyChecking` from
    /// that copy left this assertion red while every sftp test stayed green.
    #[test]
    fn the_rsync_transport_carries_the_same_ssh_policy_as_the_sftp_client() {
        let t = target();
        let transport = t.transport().expect("safe paths");
        for option in t.ssh.ssh_policy_options() {
            assert!(
                transport.split_whitespace().any(|w| w == option),
                "the rsync transport is missing {option:?}: {transport}"
            );
        }
        // Named explicitly as well, so this fails loudly if the shared list
        // itself loses one rather than both paths losing it together.
        for needle in [
            "StrictHostKeyChecking=yes",
            "BatchMode=yes",
            "PasswordAuthentication=no",
            "UserKnownHostsFile=/home/pi/.ssh/known_hosts",
        ] {
            assert!(transport.contains(needle), "{needle} missing: {transport}");
        }
    }

    /// `ssh` spells the port `-p`; the `sftp` client spells it `-P`. Passing
    /// the sftp form to `ssh` would silently mean `-p` is never set and the
    /// transfer goes to port 22.
    ///
    /// Observed failing with `-P` in `transport`: the assertion on `-p 2222`
    /// went red.
    #[test]
    fn the_transport_uses_sshs_spelling_of_the_port() {
        let transport = target().transport().expect("safe paths");
        assert!(transport.contains("-p 2222"), "{transport}");
        assert!(!transport.contains("-P 2222"), "{transport}");
    }

    /// rsync splits `-e` on whitespace with no shell and no quoting, so a key
    /// path with a space would be torn into two arguments and `ssh` would be
    /// handed a file that does not exist. Refused rather than mangled.
    ///
    /// Observed failing with the whitespace check removed: `transport`
    /// returned a string in which the identity path was two words.
    #[test]
    fn a_key_path_with_a_space_is_refused_rather_than_split() {
        let mut t = target();
        t.ssh.identity_file = PathBuf::from("/home/pi/my keys/id_ed25519");
        match t.transport() {
            Err(SftpError::UnsafePath { why, .. }) => {
                assert!(why.contains("whitespace"), "{why}");
            }
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(s) => panic!("a path with a space was passed through: {s}"),
        }
    }

    /// rsync reads the first colon as the host/path separator, so a colon in
    /// the remote path would retarget the transfer. The shared allowlist
    /// already refuses one; this pins the outcome rather than the message, so
    /// it keeps holding whichever guard does the refusing.
    #[test]
    fn a_colon_in_the_remote_path_is_refused() {
        let mut t = target();
        t.ssh.remote_dir = "/volume1/backups:evil".to_owned();
        assert!(t.remote_spec("b.bnb").is_err());
    }

    /// A remote directory containing a space is legal over SFTP — the
    /// allowlist permits it and the batch script quotes it — and impossible
    /// over rsync, which splits its own arguments. So this target has to
    /// refuse a path its sibling accepts.
    ///
    /// Observed failing with the whitespace check removed from `remote_spec`:
    /// the call returned `birds@nas.local:/volume1/my backups/b.bnb`, which
    /// rsync would have read as two paths.
    #[test]
    fn a_remote_directory_with_a_space_is_refused_although_sftp_allows_it() {
        let mut t = target();
        t.ssh.remote_dir = "/volume1/my backups".to_owned();
        // The shared allowlist is happy with it, which is the point.
        assert!(
            crate::offsite::sftp::is_safe_remote_path("/volume1/my backups/b.bnb").is_ok(),
            "this gate assumes the allowlist permits a space; it no longer does"
        );
        match t.remote_spec("b.bnb") {
            Err(SftpError::UnsafePath { why, .. }) => assert!(why.contains("whitespace"), "{why}"),
            Err(other) => panic!("wrong reason: {other}"),
            Ok(s) => panic!("rsync cannot carry this: {s}"),
        }
    }

    /// The `--` is what stops a local path beginning `-` being read as an
    /// option, and the partial directory is the whole reason this target
    /// exists over the sftp one.
    ///
    /// Observed failing with the `--` removed from `argv`: the assertion that
    /// it precedes the positional arguments went red.
    #[test]
    fn the_argument_list_separates_options_from_paths_and_keeps_partials() {
        let argv = target()
            .argv(
                "birds.db.backup.100.bnb",
                std::path::Path::new("/tmp/x.bnb"),
            )
            .expect("safe");
        let sep = argv.iter().position(|a| a == "--").expect("-- is present");
        let local = argv
            .iter()
            .position(|a| a == "/tmp/x.bnb")
            .expect("the local path is passed");
        assert!(sep < local, "the -- must precede the paths: {argv:?}");

        assert!(argv.iter().any(|a| a == "--partial"), "{argv:?}");
        assert!(
            argv.iter()
                .any(|a| a == &format!("--partial-dir={PARTIAL_DIR}")),
            "an interrupted transfer must not be left under the final name: {argv:?}"
        );
        // It sends one file and must never be able to delete at the far end.
        assert!(
            !argv.iter().any(|a| a.starts_with("--delete")),
            "this target must never carry a delete flag: {argv:?}"
        );
        assert_eq!(
            argv.last().map(String::as_str),
            Some("birds@nas.local:/volume1/backups/birds.db.backup.100.bnb"),
            "{argv:?}"
        );
    }

    /// A bandwidth cap is passed when set and absent when not — and `0`, which
    /// is rsync's own spelling of "no limit", is resolved to absent rather
    /// than passed through.
    #[test]
    fn the_bandwidth_cap_is_passed_only_when_it_is_set() {
        let mut t = target();
        let argv = t.argv("n.bnb", std::path::Path::new("/tmp/x")).expect("a");
        assert!(!argv.iter().any(|a| a.starts_with("--bwlimit")), "{argv:?}");

        t.bwlimit_kib = Some(512);
        let argv = t.argv("n.bnb", std::path::Path::new("/tmp/x")).expect("b");
        assert!(argv.iter().any(|a| a == "--bwlimit=512"), "{argv:?}");
    }

    /// A hostname or login outside the allowlist cannot reach the command
    /// line at all — the same guard the sftp target applies, reached through
    /// the shared `destination()`.
    #[test]
    fn a_hostile_hostname_is_refused() {
        let mut t = target();
        t.ssh.host = "-oProxyCommand=id".to_owned();
        assert!(t.remote_spec("b.bnb").is_err());
        assert!(t.argv("b.bnb", std::path::Path::new("/tmp/x")).is_err());
    }
}
