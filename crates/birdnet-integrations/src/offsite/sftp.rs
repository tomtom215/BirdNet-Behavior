//! An SSH file server as a backup destination.
//!
//! Driven through OpenSSH's own `sftp` binary in batch mode rather than an
//! in-process SSH implementation. A pure-Rust SSH stack is a large dependency
//! and a second place for key handling, host-key policy and cipher selection to
//! be subtly wrong; `sftp` is on every Debian-family image an operator will
//! install this on, is the thing they already have keys and a `known_hosts`
//! entry for, and is maintained by people who work on nothing else.
//!
//! The cost is a subprocess and a batch script, and the batch script is where
//! the sharp edges are. Both are addressed below.
//!
//! # Host keys
//!
//! `StrictHostKeyChecking` defaults to `yes` and there is no setting that turns
//! it off — see [`HostKeyPolicy`]. An SFTP backup with host-key checking
//! disabled encrypts the file in transit to whoever answers, which is a
//! different and much weaker promise than the one an operator thinks they are
//! getting.
//!
//! Backups are encrypted before they leave ([`super::envelope`]) so a
//! misdirected upload does not disclose the database — but it does disclose
//! that the station exists, how large its database is and how often it runs,
//! and it means the backup the operator believes is safe is not where they
//! think it is.
//!
//! # Quoting
//!
//! `sftp`'s batch parser splits on whitespace and understands double quotes and
//! backslash escapes, so a path is quoted before it goes into the script. A
//! newline cannot be escaped at all — it ends the command — so
//! [`is_safe_remote_path`] refuses those outright rather than trying, along with
//! everything else outside a conservative allowlist. Both belt and braces,
//! because the remote directory comes from the operator's config file and the
//! failure mode is arbitrary command execution on the *station's* side of the
//! connection.
//!
//! # Atomicity
//!
//! An upload lands as `<name>.part` and is renamed into place. A backup
//! interrupted by a power cut therefore never appears under its final name, so
//! a later restore cannot reach for a half-written file — and retention, which
//! matches on the final name, does not count it.

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The binary this drives. Named once so the doctor check and the
/// "what does the daemon spawn" gate can refer to the same string.
pub const SFTP_BINARY: &str = "sftp";

/// Seconds to wait for the TCP connection, passed through as `ConnectTimeout`.
const CONNECT_TIMEOUT_SECS: u32 = 30;

/// Seconds between SSH keepalive probes on an idle connection.
///
/// `ConnectTimeout` bounds the connect only. A session that establishes and
/// then stalls — a 4G bearer that has lost its far side, a middlebox that has
/// dropped the flow without RST-ing — is invisible to it, and the upload is
/// awaited with `child.wait_with_output()`, which has no timeout of its own.
/// One wedged session therefore stopped every other maintenance job for the
/// life of the process.
///
/// `ServerAliveInterval` × `ServerAliveCountMax` is OpenSSH's own stall
/// detector: probes that go unanswered this many times tear the session down.
/// 30 × 6 = three minutes of complete silence, which is far longer than any
/// legitimate gap in an SFTP transfer and far shorter than "for ever".
///
/// This is the transport-level counterpart to the `BatchMode=yes` below, which
/// closes the *other* way this hung — a prompt nobody would ever answer.
const SERVER_ALIVE_INTERVAL_SECS: u32 = 30;

/// How many unanswered keepalives end the session. See
/// [`SERVER_ALIVE_INTERVAL_SECS`].
const SERVER_ALIVE_COUNT_MAX: u32 = 6;

/// Suffix an in-flight upload carries until it is renamed into place.
pub const PART_SUFFIX: &str = ".part";

/// How host keys are checked. There is deliberately no "off".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyPolicy {
    /// `StrictHostKeyChecking=yes`: the host must already be in `known_hosts`.
    Strict,
    /// `StrictHostKeyChecking=accept-new`: trust on first use, then strict.
    ///
    /// For an operator setting up a station on a network they control. Still
    /// refuses a host whose key has *changed*, which is the case that matters
    /// after the first connection.
    AcceptNew,
}

impl HostKeyPolicy {
    /// The `ssh_config` value.
    #[must_use]
    pub const fn as_ssh_option(self) -> &'static str {
        match self {
            Self::Strict => "yes",
            Self::AcceptNew => "accept-new",
        }
    }

    /// Parse an operator's setting.
    ///
    /// # Errors
    ///
    /// Returns a message naming the token. `no`/`off` are rejected with their
    /// own explanation rather than silently treated as unknown, because an
    /// operator who wrote one meant it and needs to know why it did not take.
    pub fn parse(token: &str) -> Result<Self, String> {
        match token.trim().to_ascii_lowercase().as_str() {
            "" | "yes" | "strict" => Ok(Self::Strict),
            "accept-new" | "accept_new" | "tofu" => Ok(Self::AcceptNew),
            "no" | "off" | "false" => Err(
                "host key checking cannot be turned off for backups: it would encrypt \
                 the upload to whoever answers. Use `accept-new` for the first \
                 connection, or add the host to the known_hosts file"
                    .to_owned(),
            ),
            other => Err(format!(
                "`{other}` is not a host key policy; use `yes` or `accept-new`"
            )),
        }
    }
}

/// Where offsite backups go over SSH.
#[derive(Debug, Clone)]
pub struct SftpTarget {
    /// Hostname or address of the SSH server.
    pub host: String,
    /// Port. 22 unless the operator says otherwise.
    pub port: u16,
    /// Login name on the server.
    pub user: String,
    /// Absolute or login-relative directory backups are written into.
    pub remote_dir: String,
    /// Private key file. Required: password authentication is disabled, because
    /// a batch-mode `sftp` cannot answer a prompt and would hang until killed.
    pub identity_file: PathBuf,
    /// `known_hosts` file to check the server against.
    pub known_hosts: PathBuf,
    /// How strictly to check.
    pub host_key_policy: HostKeyPolicy,
}

/// What can go wrong.
#[derive(Debug)]
pub enum SftpError {
    /// A path contains something that cannot safely go in a batch script.
    UnsafePath {
        /// The offending path.
        path: String,
        /// Why it was refused.
        why: &'static str,
    },
    /// The `sftp` binary is not installed.
    NotInstalled,
    /// The subprocess could not be started or waited on.
    Spawn(std::io::Error),
    /// `sftp` ran and failed.
    Failed {
        /// Exit status, when the process exited normally.
        status: Option<i32>,
        /// Everything it wrote to stderr, trimmed.
        stderr: String,
    },
    /// `sftp` succeeded but its output could not be read.
    BadOutput(String),
}

impl std::fmt::Display for SftpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsafePath { path, why } => {
                write!(f, "refusing the remote path {path:?}: {why}")
            }
            Self::NotInstalled => write!(
                f,
                "`{SFTP_BINARY}` is not installed; install openssh-client \
                 (`sudo apt install openssh-client`)"
            ),
            Self::Spawn(e) => write!(f, "could not run `{SFTP_BINARY}`: {e}"),
            Self::Failed { status, stderr } => {
                let code =
                    status.map_or_else(|| "killed by a signal".to_owned(), |c| format!("exit {c}"));
                write!(f, "{SFTP_BINARY} failed ({code}): {stderr}")
            }
            Self::BadOutput(e) => write!(f, "could not read the listing: {e}"),
        }
    }
}

impl std::error::Error for SftpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(e) => Some(e),
            _ => None,
        }
    }
}

/// Characters a remote path may contain.
///
/// Deliberately narrower than what SFTP permits. Everything a backup path
/// actually needs — a directory, a database name, a Unix timestamp, an
/// extension — is in here, and everything that makes batch-script quoting a
/// question rather than a formality is not.
///
/// # Errors
///
/// Returns why the path was refused, phrased so it can be shown to an operator.
pub fn is_safe_remote_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("it is empty");
    }
    if path.len() > 1024 {
        return Err("it is longer than 1024 characters");
    }
    for c in path.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '~' | '+' | '=' | ' ') {
            continue;
        }
        return Err(match c {
            '\n' | '\r' => {
                "a newline ends an sftp batch command and cannot be escaped, so a \
                 path containing one could run a second command"
            }
            '"' | '\\' | '\'' => "quoting characters are not allowed in a remote path",
            _ => {
                "it contains a character outside the allowed set \
                  (letters, digits, and / . - _ ~ + = space)"
            }
        });
    }
    // `..` would let a misconfigured prefix write outside the directory the
    // operator granted, which on a shared host is someone else's data.
    if path.split('/').any(|seg| seg == "..") {
        return Err("it contains a `..` segment");
    }
    Ok(())
}

/// Quote a path for an `sftp` batch command.
///
/// Only reached for paths [`is_safe_remote_path`] has already accepted, so the
/// escaping has nothing left to do but the space case — but it is applied
/// anyway, so that loosening the allowlist later does not silently become an
/// injection.
#[must_use]
pub fn quote(path: &str) -> String {
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

impl SftpTarget {
    /// The full remote path a backup name lands at.
    #[must_use]
    pub fn remote_path(&self, name: &str) -> String {
        format!("{}/{name}", self.remote_dir.trim_end_matches('/'))
    }

    /// The `ssh`/`sftp` options this target connects with.
    ///
    /// Split out so the doctor check and the tests can read the exact policy
    /// without running anything, and so there is one place where a security
    /// option could go missing.
    #[must_use]
    pub fn ssh_options(&self) -> Vec<String> {
        vec![
            "-b".to_owned(),
            "-".to_owned(),
            "-P".to_owned(),
            self.port.to_string(),
            "-i".to_owned(),
            self.identity_file.display().to_string(),
            "-o".to_owned(),
            // Never prompt: a batch upload that stops for a passphrase hangs
            // until the maintenance loop is killed, and nothing is watching.
            "BatchMode=yes".to_owned(),
            "-o".to_owned(),
            "PasswordAuthentication=no".to_owned(),
            "-o".to_owned(),
            "PubkeyAuthentication=yes".to_owned(),
            "-o".to_owned(),
            format!(
                "StrictHostKeyChecking={}",
                self.host_key_policy.as_ssh_option()
            ),
            "-o".to_owned(),
            format!("UserKnownHostsFile={}", self.known_hosts.display()),
            "-o".to_owned(),
            format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"),
            "-o".to_owned(),
            format!("ServerAliveInterval={SERVER_ALIVE_INTERVAL_SECS}"),
            "-o".to_owned(),
            format!("ServerAliveCountMax={SERVER_ALIVE_COUNT_MAX}"),
        ]
    }

    /// The complete argument list `sftp` is run with.
    ///
    /// Built in one place, and returned rather than applied, so a test can read
    /// it. The `--` before the destination is the reason: without it, `sftp`
    /// parses a destination beginning `-o` as one of its own options — and
    /// `-oProxyCommand=…` is arbitrary command execution on the *station*.
    /// Verified against the installed OpenSSH: with `--`, `-oProxyCommand=id`
    /// is rejected as "hostname contains invalid characters"; without it,
    /// `sftp` prints its usage, having taken the option.
    ///
    /// [`SftpTarget::destination`]'s allowlist closes the same hole a second
    /// way. Both are here because one guard against command execution is not
    /// enough, and because each is invisible to a test of the other — this
    /// method is what makes the `--` visible.
    ///
    /// # Errors
    ///
    /// [`SftpError::UnsafePath`] from [`SftpTarget::destination`].
    pub fn argv(&self) -> Result<Vec<String>, SftpError> {
        let mut out = self.ssh_options();
        out.push("--".to_owned());
        out.push(self.destination()?);
        Ok(out)
    }

    /// `user@host`, checked against an allowlist.
    ///
    /// # Errors
    ///
    /// [`SftpError::UnsafePath`] when either half contains anything outside
    /// letters, digits, `.`, `-`, `_` — which is everything a DNS name or a
    /// Unix login can legitimately be.
    pub fn destination(&self) -> Result<String, SftpError> {
        for (part, what) in [(&self.user, "login name"), (&self.host, "hostname")] {
            if part.is_empty() {
                return Err(SftpError::UnsafePath {
                    path: part.clone(),
                    why: "it is empty",
                });
            }
            if part.starts_with('-') {
                return Err(SftpError::UnsafePath {
                    path: part.clone(),
                    why: "a leading `-` would be read as a command-line option",
                });
            }
            if !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            {
                let _ = what;
                return Err(SftpError::UnsafePath {
                    path: part.clone(),
                    why: "it contains a character outside the allowed set \
                          (letters, digits, and . - _)",
                });
            }
        }
        Ok(format!("{}@{}", self.user, self.host))
    }

    /// The batch script that uploads one file.
    ///
    /// # Errors
    ///
    /// [`SftpError::UnsafePath`] if the remote directory or name is outside the
    /// allowlist.
    pub fn upload_script(&self, name: &str, local: &Path) -> Result<String, SftpError> {
        let remote = self.remote_path(name);
        is_safe_remote_path(&remote).map_err(|why| SftpError::UnsafePath {
            path: remote.clone(),
            why,
        })?;
        let part = format!("{remote}{PART_SUFFIX}");
        Ok(format!(
            // `-` prefixes a command whose failure is not fatal: the directory
            // usually exists already, and `mkdir` on an existing directory is
            // an error that would abort the whole batch.
            "-mkdir {}\nput {} {}\nrename {} {}\n",
            quote(self.remote_dir.trim_end_matches('/')),
            quote(&local.display().to_string()),
            quote(&part),
            quote(&part),
            quote(&remote),
        ))
    }

    /// The batch script that lists the backup directory.
    ///
    /// `ls -1` rather than `ls -l`: one bare name per line, which every SFTP
    /// server formats the same way. The long form's columns differ between
    /// implementations, and retention only needs names — the time a backup was
    /// *taken* is in the name, and is the ordering that matters.
    ///
    /// # Errors
    ///
    /// [`SftpError::UnsafePath`] as above.
    pub fn list_script(&self) -> Result<String, SftpError> {
        let dir = self.remote_dir.trim_end_matches('/');
        is_safe_remote_path(dir).map_err(|why| SftpError::UnsafePath {
            path: dir.to_owned(),
            why,
        })?;
        Ok(format!("ls -1 {}\n", quote(dir)))
    }

    /// The batch script that removes one backup by name.
    ///
    /// # Errors
    ///
    /// [`SftpError::UnsafePath`] as above.
    pub fn remove_script(&self, name: &str) -> Result<String, SftpError> {
        let remote = self.remote_path(name);
        is_safe_remote_path(&remote).map_err(|why| SftpError::UnsafePath {
            path: remote.clone(),
            why,
        })?;
        Ok(format!("rm {}\n", quote(&remote)))
    }

    /// Run a batch script and return its stdout.
    ///
    /// # Errors
    ///
    /// [`SftpError::NotInstalled`] if `sftp` is absent, [`SftpError::Spawn`] if
    /// it cannot be started, [`SftpError::Failed`] if it exits non-zero.
    pub async fn run_batch(&self, script: &str) -> Result<String, SftpError> {
        use tokio::io::AsyncWriteExt as _;

        let mut cmd = tokio::process::Command::new(SFTP_BINARY);
        cmd.args(self.argv()?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SftpError::NotInstalled);
            }
            Err(e) => return Err(SftpError::Spawn(e)),
        };

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(script.as_bytes())
                .await
                .map_err(SftpError::Spawn)?;
            stdin.shutdown().await.map_err(SftpError::Spawn)?;
        }

        let output = child.wait_with_output().await.map_err(SftpError::Spawn)?;
        if output.status.success() {
            String::from_utf8(output.stdout).map_err(|e| SftpError::BadOutput(e.to_string()))
        } else {
            Err(SftpError::Failed {
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    /// Upload a local file so it appears under `name` only once complete.
    ///
    /// # Errors
    ///
    /// As [`SftpTarget::run_batch`], plus [`SftpError::UnsafePath`].
    pub async fn put(&self, name: &str, local: &Path) -> Result<String, SftpError> {
        let script = self.upload_script(name, local)?;
        self.run_batch(&script).await?;
        Ok(self.remote_path(name))
    }

    /// Names in the backup directory, `.part` files excluded.
    ///
    /// # Errors
    ///
    /// As [`SftpTarget::run_batch`].
    pub async fn list(&self) -> Result<Vec<String>, SftpError> {
        let out = self.run_batch(&self.list_script()?).await?;
        Ok(parse_listing(&out, self.remote_dir.trim_end_matches('/')))
    }

    /// Remove one backup by name.
    ///
    /// # Errors
    ///
    /// As [`SftpTarget::run_batch`].
    pub async fn remove(&self, name: &str) -> Result<(), SftpError> {
        self.run_batch(&self.remove_script(name)?).await?;
        Ok(())
    }
}

/// Turn `ls -1` output into bare file names.
///
/// Servers differ in whether they print bare names or full paths, and `sftp`
/// echoes the command it ran when its input is not a terminal, so both are
/// handled. `.part` files are dropped: an upload still in flight is not a
/// backup, and retention counting one would prune a real backup to make room
/// for it.
#[must_use]
pub fn parse_listing(stdout: &str, remote_dir: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        // `sftp` echoes `sftp> ls -1 "…"` on a non-tty; that is not a file.
        .filter(|line| !line.starts_with("sftp>"))
        .map(|line| line.strip_prefix(remote_dir).unwrap_or(line))
        .map(|line| line.trim_start_matches('/'))
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .filter(|name| !name.ends_with(PART_SUFFIX))
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> SftpTarget {
        SftpTarget {
            host: "backup.example.net".to_owned(),
            port: 2222,
            user: "birdnet".to_owned(),
            remote_dir: "/srv/backups/pi-1".to_owned(),
            identity_file: PathBuf::from("/var/lib/birdnet/ssh/id_ed25519"),
            known_hosts: PathBuf::from("/var/lib/birdnet/ssh/known_hosts"),
            host_key_policy: HostKeyPolicy::Strict,
        }
    }

    #[test]
    fn host_key_checking_cannot_be_turned_off() {
        // The setting an operator reaches for when the first connection fails,
        // and the one that quietly makes the whole thing pointless.
        for token in ["no", "off", "false", "NO", " off "] {
            let Err(err) = HostKeyPolicy::parse(token) else {
                panic!("`{token}` must not be accepted as a host key policy")
            };
            assert!(
                err.contains("cannot be turned off"),
                "`{token}` was rejected, but not with an explanation an operator \
                 can act on: {err}"
            );
        }
        // The two that are allowed, and their ssh_config spellings.
        assert_eq!(HostKeyPolicy::parse("yes").unwrap().as_ssh_option(), "yes");
        assert_eq!(
            HostKeyPolicy::parse("accept-new").unwrap().as_ssh_option(),
            "accept-new"
        );
        assert_eq!(HostKeyPolicy::parse("").unwrap(), HostKeyPolicy::Strict);
    }

    #[test]
    fn the_connection_options_carry_every_security_setting() {
        // One place where an option could go missing, so pin all of them.
        let opts = target().ssh_options().join(" ");
        for required in [
            "BatchMode=yes",
            "PasswordAuthentication=no",
            "PubkeyAuthentication=yes",
            "StrictHostKeyChecking=yes",
            "UserKnownHostsFile=/var/lib/birdnet/ssh/known_hosts",
            "ConnectTimeout=30",
            // The stall detector. ConnectTimeout bounds the connect only, and
            // the upload is awaited with no timeout of its own, so without
            // these a session that establishes and then goes quiet stops every
            // other maintenance job for the life of the process.
            "ServerAliveInterval=30",
            "ServerAliveCountMax=6",
            "-P 2222",
            "-i /var/lib/birdnet/ssh/id_ed25519",
        ] {
            assert!(opts.contains(required), "missing `{required}` in: {opts}");
        }
        // And the option that must never appear, whatever else changes.
        assert!(
            !opts.contains("StrictHostKeyChecking=no"),
            "host key checking was disabled: {opts}"
        );
    }

    #[test]
    fn a_destination_that_could_inject_an_ssh_option_is_refused() {
        // `sftp` has no `--` in its documented usage but does honour one, and
        // without it a destination beginning `-o` is parsed as an option.
        // `-oProxyCommand=…` then runs a command on the station.
        //
        // Two independent guards, and each is invisible to a test of the other:
        // with the allowlist intact no input reaches the `--`, and with the
        // `--` intact the allowlist is belt over braces. So the allowlist is
        // checked by what it rejects, and the `--` by where it sits in the
        // argument list.
        let mut t = target();
        for bad in [
            "-oProxyCommand=id",
            "-obatchmode=no",
            "back up.example.net",
            "host;id",
            "host$(id)",
            "",
        ] {
            t.host = bad.to_owned();
            assert!(
                t.destination().is_err(),
                "`{bad}` should not be an acceptable hostname"
            );
        }
        t.host = "backup.example.net".to_owned();
        for bad in ["-oProxyCommand=id", "user name", "user@other", ""] {
            t.user = bad.to_owned();
            assert!(
                t.destination().is_err(),
                "`{bad}` should not be an acceptable login name"
            );
        }

        // Counterpart: an ordinary destination still builds, or the allowlist
        // is simply an outage.
        t.user = "birdnet".to_owned();
        t.host = "backup.example.net".to_owned();
        assert_eq!(t.destination().unwrap(), "birdnet@backup.example.net");
        t.host = "192.0.2.10".to_owned();
        assert_eq!(t.destination().unwrap(), "birdnet@192.0.2.10");
        t.user = "birdnet_backup-1".to_owned();
        assert!(t.destination().is_ok());
    }

    #[test]
    fn the_destination_is_the_last_argument_and_follows_a_separator() {
        // The second guard, checked where it lives. Deleting the `--` does not
        // fail any behavioural test — the allowlist stops the input first — so
        // without this the separator would be one refactor away from silently
        // disappearing.
        let argv = target().argv().expect("a valid target");
        let last = argv.last().expect("non-empty");
        assert_eq!(last, "birdnet@backup.example.net");
        assert_eq!(
            argv.get(argv.len() - 2).map(String::as_str),
            Some("--"),
            "the destination must be preceded by `--`, or a hostname beginning \
             `-o` becomes an ssh option: {argv:?}"
        );
        // And the separator must come after every option, not before them.
        let sep = argv.iter().position(|a| a == "--").expect("a separator");
        assert!(
            argv[..sep]
                .iter()
                .any(|a| a.starts_with("StrictHostKeyChecking")),
            "the options must precede the separator: {argv:?}"
        );
    }

    #[test]
    fn a_path_that_could_inject_a_second_command_is_refused() {
        // The batch script is a sequence of newline-separated commands, so a
        // newline in a path is a second command. It cannot be escaped, only
        // refused.
        let mut t = target();
        t.remote_dir = "/srv/backups\nrm /srv/everything".to_owned();
        let err = t
            .upload_script("x.bnb", Path::new("/tmp/x"))
            .expect_err("a newline in the remote directory must be refused");
        match err {
            SftpError::UnsafePath { why, .. } => assert!(
                why.contains("newline"),
                "the refusal should name the reason: {why}"
            ),
            other => panic!("expected an unsafe-path refusal, got {other}"),
        }

        // The other shapes, each refused rather than escaped.
        for bad in [
            "/srv/\"quoted\"",
            "/srv/back\\slash",
            "/srv/../../etc",
            "/srv/$(whoami)",
            "/srv/`id`",
            "/srv/a;b",
            "",
        ] {
            assert!(
                is_safe_remote_path(bad).is_err(),
                "`{bad}` should not be an acceptable remote path"
            );
        }

        // Counterpart: everything a real backup path needs must still pass, or
        // the allowlist is just an outage.
        for good in [
            "/srv/backups/pi-1/birds.db.backup.1733400000.bnb",
            "backups/station one/birds.db.backup.1.bnb",
            "~/backups/a-b_c.d+e=f",
        ] {
            assert_eq!(
                is_safe_remote_path(good),
                Ok(()),
                "`{good}` should be an acceptable remote path"
            );
        }
    }

    #[test]
    fn an_upload_lands_under_a_part_name_and_is_renamed() {
        // A power cut mid-upload must not leave something a restore would
        // reach for. The rename is what makes the final name mean "complete".
        let script = target()
            .upload_script("birds.db.backup.1733400000.bnb", Path::new("/tmp/b.bnb"))
            .expect("script");
        let lines: Vec<&str> = script.lines().collect();
        assert_eq!(lines.len(), 3, "unexpected script: {script}");
        assert!(lines[0].starts_with("-mkdir "), "{}", lines[0]);
        assert_eq!(
            lines[1],
            "put \"/tmp/b.bnb\" \"/srv/backups/pi-1/birds.db.backup.1733400000.bnb.part\"",
            "the upload must go to a .part name"
        );
        assert_eq!(
            lines[2],
            "rename \"/srv/backups/pi-1/birds.db.backup.1733400000.bnb.part\" \
             \"/srv/backups/pi-1/birds.db.backup.1733400000.bnb\"",
            "the .part file must be renamed into place"
        );
    }

    #[test]
    fn the_mkdir_is_the_only_command_allowed_to_fail() {
        // `sftp -b` aborts on the first error. The directory usually exists, so
        // an unprefixed mkdir would abort every run after the first — but a
        // failed `put` must still abort, or a rename would promote a stale
        // .part file from a previous attempt to the final name.
        let script = target()
            .upload_script("x.bnb", Path::new("/tmp/x"))
            .expect("script");
        let failable: Vec<&str> = script.lines().filter(|l| l.starts_with('-')).collect();
        assert_eq!(
            failable,
            vec!["-mkdir \"/srv/backups/pi-1\""],
            "exactly one command may be allowed to fail: {script}"
        );
    }

    #[test]
    fn a_listing_drops_echoes_paths_and_in_flight_uploads() {
        let out = "sftp> ls -1 \"/srv/backups/pi-1\"\n\
                   /srv/backups/pi-1/birds.db.backup.100.bnb\n\
                   /srv/backups/pi-1/birds.db.backup.200.bnb\n\
                   /srv/backups/pi-1/birds.db.backup.300.bnb.part\n\
                   \n";
        let names = parse_listing(out, "/srv/backups/pi-1");
        assert_eq!(
            names,
            vec![
                "birds.db.backup.100.bnb".to_owned(),
                "birds.db.backup.200.bnb".to_owned()
            ],
            "an in-flight .part upload is not a backup and must not be listed"
        );

        // A server that prints bare names rather than full paths.
        let bare = "birds.db.backup.100.bnb\nbirds.db.backup.200.bnb\n";
        assert_eq!(parse_listing(bare, "/srv/backups/pi-1").len(), 2);

        // And `.`/`..`, which some servers include.
        let dots = ".\n..\nbirds.db.backup.1.bnb\n";
        assert_eq!(
            parse_listing(dots, "/srv/backups/pi-1"),
            vec!["birds.db.backup.1.bnb".to_owned()]
        );
    }
}
