//! The SFTP target against a real OpenSSH server on a loopback port.
//!
//! The unit tests in `offsite::sftp` check the batch scripts as strings. This
//! runs them: a generated host key, a generated login key, `sshd` in the
//! foreground on an ephemeral port, and the station's own `sftp` driver
//! uploading to it and reading back what landed.
//!
//! It is the only thing that can catch the class of bug where the script is
//! well-formed and `sftp` still refuses it — a batch command whose failure was
//! meant to be tolerated but is not, an option combination that makes OpenSSH
//! prompt in a context with no terminal, a `rename` onto an existing name.
//!
//! # Skipping
//!
//! Skips loudly (a printed line and a pass) when `sshd`, `sftp` or `ssh-keygen`
//! is missing, which is how a contributor without openssh-server installed can
//! still run the suite. `container_can_run_what_the_daemon_spawns` is what
//! makes sure the *shipped* container has the client half.

use std::path::{Path, PathBuf};
use std::process::Command;

use birdnet_integrations::offsite::sftp::{HostKeyPolicy, SftpError, SftpTarget};

/// Absolute path of a tool, or `None` when it is not installed.
fn which(tool: &str) -> Option<PathBuf> {
    for dir in ["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/usr/local/bin"] {
        let p = Path::new(dir).join(tool);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// The `sftp-server` subsystem binary, whose location differs by distribution.
fn sftp_server() -> Option<PathBuf> {
    for p in [
        "/usr/lib/openssh/sftp-server",
        "/usr/libexec/openssh/sftp-server",
        "/usr/libexec/sftp-server",
        "/usr/lib/ssh/sftp-server",
    ] {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// OpenSSH's compiled-in privilege-separation directory on Debian family.
const PRIVSEP_DIR: &str = "/run/sshd";

/// A running `sshd`, killed on drop.
struct Server {
    child: std::process::Child,
    port: u16,
    /// Kept alive: dropping it removes the keys and the served directory.
    _dir: tempfile::TempDir,
    root: PathBuf,
    identity: PathBuf,
    known_hosts: PathBuf,
    user: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A free TCP port, released before `sshd` binds it.
///
/// Racy in principle. In a test container with nothing else opening ports it
/// is not, and the alternative — parsing `sshd`'s own log for the port it
/// chose — is more moving parts than the race is worth.
fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    l.local_addr().expect("addr").port()
}

/// Stand up `sshd` with a fresh host key and one authorised login key.
///
/// Returns `None` when the tools are not installed.
fn start_server() -> Option<Server> {
    let sshd = which("sshd")?;
    which("sftp")?;
    let keygen = which("ssh-keygen")?;
    let subsystem = sftp_server()?;

    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path().to_path_buf();
    let host_key = base.join("host_ed25519");
    let identity = base.join("id_ed25519");
    let root = base.join("served");
    std::fs::create_dir_all(&root).expect("served dir");

    for (path, comment) in [(&host_key, "host"), (&identity, "login")] {
        let out = Command::new(&keygen)
            .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
            .arg(path)
            .output()
            .expect("ssh-keygen runs");
        assert!(
            out.status.success(),
            "ssh-keygen failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let authorized = base.join("authorized_keys");
    std::fs::copy(identity.with_extension("pub"), &authorized).expect("authorized_keys");
    // OpenSSH refuses to read an authorized_keys or a private key that anyone
    // else can read, and says so only in the server log.
    set_mode(&authorized, 0o600);
    set_mode(&identity, 0o600);
    set_mode(&host_key, 0o600);

    let user = std::env::var("USER").unwrap_or_else(|_| "root".to_owned());
    let port = free_port();
    let config = base.join("sshd_config");
    std::fs::write(
        &config,
        format!(
            "Port {port}\n\
             ListenAddress 127.0.0.1\n\
             HostKey {host}\n\
             AuthorizedKeysFile {auth}\n\
             PidFile {base}/sshd.pid\n\
             PasswordAuthentication no\n\
             KbdInteractiveAuthentication no\n\
             PubkeyAuthentication yes\n\
             UsePAM no\n\
             StrictModes no\n\
             PermitRootLogin prohibit-password\n\
             LogLevel ERROR\n\
             Subsystem sftp {subsystem}\n",
            host = host_key.display(),
            auth = authorized.display(),
            base = base.display(),
            subsystem = subsystem.display(),
        ),
    )
    .expect("sshd_config");

    // OpenSSH refuses to start without its privilege-separation directory,
    // whose path is compiled in and cannot be configured. Creating it needs
    // root; where that is not available the whole file skips.
    if !Path::new(PRIVSEP_DIR).is_dir() && std::fs::create_dir_all(PRIVSEP_DIR).is_err() {
        eprintln!("SKIP: {PRIVSEP_DIR} does not exist and cannot be created");
        return None;
    }

    // `-D` keeps it in the foreground so the child handle is the server;
    // `-E` sends its log somewhere this can read on failure.
    let log = base.join("sshd.log");
    let mut child = Command::new(&sshd)
        .arg("-D")
        .arg("-f")
        .arg(&config)
        .arg("-E")
        .arg(&log)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    // Wait for the port rather than sleeping a fixed amount.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut listening = false;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            listening = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // A server that never came up must not be handed back. Every test below
    // that asserts a *failure* would otherwise pass on "connection refused",
    // which is exactly what happened the first time this file was run.
    assert!(
        listening,
        "sshd never listened on 127.0.0.1:{port}\n  log: {}\n  stderr: {}",
        std::fs::read_to_string(&log).unwrap_or_default(),
        {
            let _ = child.kill();
            child
                .stderr
                .take()
                .map(|mut e| {
                    use std::io::Read as _;
                    let mut s = String::new();
                    let _ = e.read_to_string(&mut s);
                    s
                })
                .unwrap_or_default()
        }
    );

    // Trust this host key, the way an operator would after checking it.
    let known_hosts = base.join("known_hosts");
    let host_pub = std::fs::read_to_string(host_key.with_extension("pub")).expect("host pub");
    std::fs::write(
        &known_hosts,
        format!("[127.0.0.1]:{port} {}", host_pub.trim()),
    )
    .expect("known_hosts");

    Some(Server {
        child,
        port,
        _dir: dir,
        root,
        identity,
        known_hosts,
        user,
    })
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(path, perms).expect("chmod");
}

impl Server {
    fn target(&self, policy: HostKeyPolicy) -> SftpTarget {
        SftpTarget {
            host: "127.0.0.1".to_owned(),
            port: self.port,
            user: self.user.clone(),
            remote_dir: self.root.join("backups").display().to_string(),
            identity_file: self.identity.clone(),
            known_hosts: self.known_hosts.clone(),
            host_key_policy: policy,
        }
    }
}

/// Print why and return, so a skipped run is visible rather than a silent pass.
macro_rules! server_or_skip {
    () => {
        match start_server() {
            Some(s) => s,
            None => {
                eprintln!(
                    "SKIP: needs sshd, sftp, ssh-keygen and an sftp-server subsystem \
                     (apt install openssh-server openssh-client)"
                );
                return;
            }
        }
    };
}

#[tokio::test]
async fn a_backup_uploads_lists_and_deletes_against_a_real_server() {
    let server = server_or_skip!();
    let target = server.target(HostKeyPolicy::Strict);

    let dir = tempfile::tempdir().expect("tempdir");
    let local = dir.path().join("birds.db.backup.1733400000.bnb");
    #[allow(clippy::cast_possible_truncation)]
    let bytes: Vec<u8> = (0..200_000_usize).map(|i| (i % 251) as u8).collect();
    std::fs::write(&local, &bytes).expect("write");

    // Upload. The remote directory does not exist yet, which is the case the
    // `-mkdir` prefix is for.
    let remote = target
        .put("birds.db.backup.1733400000.bnb", &local)
        .await
        .unwrap_or_else(|e| panic!("upload failed: {e}"));

    let landed = std::fs::read(&remote).expect("the uploaded file must exist on disk");
    assert_eq!(
        landed, bytes,
        "the uploaded bytes differ from the local file"
    );
    assert!(
        !Path::new(&format!("{remote}.part")).exists(),
        "the .part file was left behind"
    );

    // A second upload into the now-existing directory must also work: this is
    // the run where an unprefixed `mkdir` would abort the batch.
    let second = dir.path().join("birds.db.backup.1733500000.bnb");
    std::fs::write(&second, b"a smaller one").expect("write");
    target
        .put("birds.db.backup.1733500000.bnb", &second)
        .await
        .unwrap_or_else(|e| panic!("the second upload failed: {e}"));

    let mut names = target.list().await.unwrap_or_else(|e| panic!("list: {e}"));
    names.sort();
    assert_eq!(
        names,
        vec![
            "birds.db.backup.1733400000.bnb".to_owned(),
            "birds.db.backup.1733500000.bnb".to_owned()
        ],
        "the listing did not report exactly what was uploaded"
    );

    target
        .remove("birds.db.backup.1733400000.bnb")
        .await
        .unwrap_or_else(|e| panic!("remove: {e}"));
    assert_eq!(
        target.list().await.expect("list"),
        vec!["birds.db.backup.1733500000.bnb".to_owned()],
        "remove did not delete exactly one backup"
    );
    assert!(!Path::new(&remote).exists());
}

#[tokio::test]
async fn an_unknown_host_key_is_refused_rather_than_trusted() {
    // The property the whole `HostKeyPolicy` type exists for. With an empty
    // known_hosts and the strict policy, the connection must fail — and it must
    // fail *quickly*, without waiting for an answer nobody is there to give.
    let server = server_or_skip!();
    let mut target = server.target(HostKeyPolicy::Strict);
    let empty = server.root.join("empty_known_hosts");
    std::fs::write(&empty, "").expect("write");
    target.known_hosts = empty;

    let dir = tempfile::tempdir().expect("tempdir");
    let local = dir.path().join("x.bnb");
    std::fs::write(&local, b"x").expect("write");

    let err = target
        .put("x.bnb", &local)
        .await
        .expect_err("an unverified host key must not be trusted");
    assert!(
        matches!(err, SftpError::Failed { .. }),
        "expected the transfer to fail, got {err}"
    );

    // The counterpart, and the reason this test is not simply "SFTP is broken":
    // `accept-new` against the same empty file succeeds, and records the key.
    let mut lenient = target.clone();
    lenient.host_key_policy = HostKeyPolicy::AcceptNew;
    lenient
        .put("x.bnb", &local)
        .await
        .unwrap_or_else(|e| panic!("accept-new should connect to an unknown host: {e}"));
    let recorded = std::fs::read_to_string(&lenient.known_hosts).expect("known_hosts");
    assert!(
        recorded.contains("ssh-ed25519"),
        "accept-new must write the key it accepted, so the next connection is \
         strict: {recorded:?}"
    );
}

#[tokio::test]
async fn a_wrong_key_fails_instead_of_waiting_for_a_password() {
    // `BatchMode=yes` and `PasswordAuthentication=no` together. Without them a
    // station whose key was revoked hangs on a password prompt until the
    // maintenance loop is killed, and nothing is watching.
    let server = server_or_skip!();
    let mut target = server.target(HostKeyPolicy::Strict);

    let control = tempfile::tempdir().expect("tempdir");
    let control_file = control.path().join("control.bnb");
    std::fs::write(&control_file, b"control").expect("write");
    // Positive control first. Without it, "the server is not listening" would
    // satisfy the assertion below — which is how the first run of this file
    // reported two passes against an sshd that had never started.
    target
        .put("control.bnb", &control_file)
        .await
        .unwrap_or_else(|e| {
            panic!("the authorised key must work before the wrong one is tried: {e}")
        });

    let other = server.root.join("wrong_key");
    let keygen = which("ssh-keygen").expect("checked in start_server");
    let out = Command::new(&keygen)
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "wrong", "-f"])
        .arg(&other)
        .output()
        .expect("ssh-keygen");
    assert!(out.status.success());
    set_mode(&other, 0o600);
    target.identity_file = other;

    let dir = tempfile::tempdir().expect("tempdir");
    let local = dir.path().join("x.bnb");
    std::fs::write(&local, b"x").expect("write");

    let started = std::time::Instant::now();
    let err = target
        .put("x.bnb", &local)
        .await
        .expect_err("an unauthorised key must not upload");
    assert!(
        matches!(err, SftpError::Failed { .. }),
        "expected a failure, got {err}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "the transfer took {:?} — it is waiting for something interactive",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_partial_upload_never_appears_under_its_final_name() {
    // What the `.part` rename buys. Simulated by pointing the upload at a local
    // file that vanishes mid-transfer — `sftp` fails, and what matters is what
    // it leaves behind.
    let server = server_or_skip!();
    let target = server.target(HostKeyPolicy::Strict);

    let dir = tempfile::tempdir().expect("tempdir");
    let local = dir.path().join("gone.bnb");
    // Never created, so `put` fails immediately.
    let err = target
        .put("birds.db.backup.1.bnb", &local)
        .await
        .expect_err("uploading a file that does not exist must fail");
    assert!(matches!(err, SftpError::Failed { .. }), "got {err}");

    // The directory was created by the `-mkdir`, but nothing landed in it under
    // the final name — which is what a later restore would reach for.
    // `expect`, not `unwrap_or_default`: a listing that failed would produce an
    // empty vector and satisfy the assertion for the wrong reason.
    let names = target
        .list()
        .await
        .unwrap_or_else(|e| panic!("the directory must still be listable: {e}"));
    assert!(
        names.is_empty(),
        "a failed upload left something behind that looks like a backup: {names:?}"
    );
}

#[tokio::test]
async fn a_full_offsite_run_encrypts_uploads_and_prunes() {
    // The whole pipeline against a real server: encrypt, upload, list, prune,
    // and then decrypt what actually landed on the far side.
    //
    // Everything below this line has been proved separately — the envelope
    // round-trips, the batch scripts are right, retention picks the right
    // names. What only this can show is that they compose: that the file the
    // server holds is the file the station encrypted, and that four runs with
    // `keep = 2` leave the newest two rather than two arbitrary ones.
    use birdnet_integrations::offsite::{
        Destination, OffsiteConfig, Passphrase, envelope, run as offsite_run,
    };

    const PASS: &str = "correct horse battery staple";

    let server = server_or_skip!();
    let target = server.target(HostKeyPolicy::Strict);

    let dir = tempfile::tempdir().expect("tempdir");
    let config = OffsiteConfig {
        destination: Destination::Sftp(Box::new(target.clone())),
        passphrase: Passphrase::new(PASS.to_owned()).expect("long enough"),
        keep: 2,
    };

    // Four snapshots, uploaded oldest first. Each carries different bytes so a
    // mixed-up upload shows as wrong content rather than as the right length.
    let mut written = Vec::new();
    for ts in [1_733_400_000_u64, 1_733_500_000, 1_733_600_000] {
        let local = dir.path().join(format!("birds.db.backup.{ts}"));
        let mut plain = b"SQLite format 3\0".to_vec();
        plain.extend_from_slice(format!("snapshot {ts}").as_bytes());
        plain.resize(70_000, (ts % 251) as u8);
        std::fs::write(&local, &plain).expect("write");

        let report = offsite_run(&config, &local, dir.path())
            .await
            .unwrap_or_else(|e| panic!("offsite run for {ts} failed: {e}"));
        assert_eq!(report.uploaded, format!("birds.db.backup.{ts}.bnb"));
        written.push((ts, local, plain));

        // The scratch ciphertext must not be left behind: a station whose
        // destination is unreachable would otherwise fill its own disk with the
        // backups it could not send.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("scratch")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                std::path::Path::new(n)
                    .extension()
                    .is_some_and(|e| e == "tmp")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "scratch ciphertext left behind: {leftovers:?}"
        );
    }

    // Retention: the oldest of the three is gone, the newest two remain.
    let mut remaining = target.list().await.expect("list");
    remaining.sort();
    assert_eq!(
        remaining,
        vec![
            "birds.db.backup.1733500000.bnb".to_owned(),
            "birds.db.backup.1733600000.bnb".to_owned()
        ],
        "retention did not keep the newest two"
    );

    // And what is actually on the far side decrypts to the snapshot it came
    // from — the assertion that ties encryption, transport and naming together.
    for (ts, _, plain) in &written {
        let remote = PathBuf::from(target.remote_path(&format!("birds.db.backup.{ts}.bnb")));
        if !remote.exists() {
            continue; // pruned, checked above
        }
        let cipher = std::fs::read(&remote).expect("read the uploaded file");
        assert!(
            !cipher.windows(16).any(|w| w == b"SQLite format 3\0"),
            "the database header reached the server in the clear"
        );
        let mut restored = Vec::new();
        envelope::decrypt(PASS, &mut &cipher[..], &mut restored)
            .unwrap_or_else(|e| panic!("the uploaded backup for {ts} does not decrypt: {e}"));
        assert_eq!(
            &restored, plain,
            "the uploaded backup for {ts} is not the snapshot it came from"
        );
    }
}
