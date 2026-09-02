//! An offsite backup goes out encrypted and comes back through the CLI.
//!
//! # Why this exists as an end-to-end test
//!
//! The library round-trips are gated in `birdnet-integrations`. What this adds
//! is the half an operator actually touches on the worst day of their station's
//! life: `--decrypt-backup`, run from a shell, against a file they downloaded
//! from a bucket. The pieces that can only break here are the ones between the
//! library and the shell — the flag reaching the dispatcher, the passphrase
//! coming from the environment rather than an argument, the exit code a script
//! would branch on, and what is left on disk when it fails.
//!
//! **An encrypted backup with no working restore path is worse than no backup**,
//! because it looks like insurance for a year and then does not pay out. This
//! is the test that says the insurance pays.

use std::path::Path;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_birdnet-behavior");
const PASS: &str = "correct horse battery staple";

/// Run the binary; return (stdout, stderr, exit code).
fn run(args: &[&str], passphrase: Option<&str>) -> (String, String, i32) {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .arg("--config")
        .arg("/nonexistent/birdnet.conf")
        .env("RUST_LOG", "error")
        .env_remove("BIRDNET_CONFIG")
        .env_remove("BIRDNET_OFFSITE_PASSPHRASE")
        .stdin(Stdio::null());
    if let Some(p) = passphrase {
        cmd.env("BIRDNET_OFFSITE_PASSPHRASE", p);
    }
    let out = cmd.output().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// A plausible station database, and its encrypted form.
///
/// `size` is the plaintext length. Most tests want the small one — argon2 and
/// the AEAD are real work in a debug build. The truncation test wants one
/// larger than [`envelope::CHUNK_LEN`], because the property it is checking is
/// only expressible when there is more than one chunk to remove.
fn fixture_of(dir: &Path, size: usize) -> (Vec<u8>, std::path::PathBuf) {
    let mut plain = b"SQLite format 3\0".to_vec();
    plain.extend_from_slice(b"Turdus merula 2026-03-01 06:41 51.5074,-0.1278");
    plain.resize(size, 7);

    let encrypted = dir.join("birds.db.backup.1733400000.bnb");
    let mut out = std::fs::File::create(&encrypted).expect("create");
    birdnet_integrations::offsite::envelope::encrypt(PASS, &mut &plain[..], &mut out)
        .expect("encrypt");
    (plain, encrypted)
}

/// The small fixture: enough to prove the streaming path, quick to encrypt.
fn fixture(dir: &Path) -> (Vec<u8>, std::path::PathBuf) {
    fixture_of(dir, 50_000)
}

#[test]
fn a_downloaded_backup_decrypts_to_the_database_it_came_from() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plain, encrypted) = fixture(dir.path());
    let restored = dir.path().join("restored.db");

    // What leaves the station is not readable, which is the whole point.
    let cipher = std::fs::read(&encrypted).expect("read");
    assert!(
        !cipher.windows(16).any(|w| w == b"SQLite format 3\0"),
        "the uploaded file carries the database header in the clear"
    );

    let (stdout, stderr, code) = run(
        &[
            "--decrypt-backup",
            &encrypted.display().to_string(),
            "--out",
            &restored.display().to_string(),
        ],
        Some(PASS),
    );
    assert_eq!(code, 0, "decrypt failed: {stderr}");
    assert!(
        stdout.contains(&restored.display().to_string()),
        "the success line should name the file it wrote: {stdout:?}"
    );
    assert_eq!(
        std::fs::read(&restored).expect("read restored"),
        plain,
        "the restored database differs from the one that was backed up"
    );
}

#[test]
fn a_wrong_passphrase_says_so_and_leaves_nothing_behind() {
    // The failure an operator will actually hit. It must not exit 0, must not
    // leave a partial file that looks restorable, and must say which of the two
    // possible causes it is.
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, encrypted) = fixture(dir.path());
    let restored = dir.path().join("restored.db");

    let (_, stderr, code) = run(
        &[
            "--decrypt-backup",
            &encrypted.display().to_string(),
            "--out",
            &restored.display().to_string(),
        ],
        Some("a different passphrase entirely"),
    );
    assert_eq!(code, 1, "a wrong passphrase must not exit 0");
    assert!(
        stderr.contains("passphrase is wrong"),
        "the error should name the likely cause: {stderr}"
    );
    assert!(!restored.exists(), "a failed decrypt wrote the output file");
    let strays: Vec<String> = std::fs::read_dir(dir.path())
        .expect("readdir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("partial"))
        .collect();
    assert!(strays.is_empty(), "left a partial file behind: {strays:?}");
}

#[test]
fn a_truncated_backup_fails_rather_than_restoring_part_of_a_database() {
    // The failure that would otherwise be silent. A truncated download
    // decrypts correctly right up to the point it stops, so a decrypt that
    // wrote as it went would hand the operator a database that opens, contains
    // most of their records, and is missing the end — with no error anywhere.
    //
    // Two shapes of truncation, and they are caught by different things:
    //
    //   * a cut *inside* a chunk breaks that chunk's own authentication tag;
    //   * a cut that removes a *whole* chunk leaves every remaining chunk
    //     authentic, and is caught only by the envelope's final-chunk flag —
    //     the reason nonces here are a STREAM counter rather than random.
    //
    // The first version of this test cut 200 bytes off a single-chunk file, so
    // it exercised only the first shape: deleting the final-chunk flag entirely
    // left it green. The fixture is therefore larger than one chunk, and the
    // cut removes a whole framed chunk.
    use birdnet_integrations::offsite::envelope;

    /// Plaintext bytes in the file's short final chunk.
    const TAIL: usize = 4096;

    let dir = tempfile::tempdir().expect("tempdir");
    let (plain, encrypted) = fixture_of(dir.path(), envelope::CHUNK_LEN as usize * 2 + TAIL);
    let full = std::fs::read(&encrypted).expect("read");
    // The *last* chunk is the short one: TAIL bytes plus its 16-byte tag.
    // Cutting a whole `CHUNK_LEN` off instead would slice into the middle of
    // the previous chunk and break its tag, which proves the weaker property.
    let last_framed = TAIL + 16;
    let truncated = &full[..full.len() - last_framed];
    assert_eq!(
        truncated.len(),
        envelope::HEADER_LEN + 2 * (envelope::CHUNK_LEN as usize + 16),
        "the cut must land exactly on a chunk boundary, or this test is checking \
         a broken tag rather than a missing final chunk"
    );
    std::fs::write(&encrypted, truncated).expect("truncate");

    let restored = dir.path().join("restored.db");
    let (_, stderr, code) = run(
        &[
            "--decrypt-backup",
            &encrypted.display().to_string(),
            "--out",
            &restored.display().to_string(),
        ],
        Some(PASS),
    );
    assert_eq!(code, 1, "a truncated backup must not restore: {stderr}");
    assert!(
        !restored.exists(),
        "a truncated backup produced an output file, which would open in SQLite \
         and be missing its tail"
    );
    let strays: Vec<String> = std::fs::read_dir(dir.path())
        .expect("readdir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("partial"))
        .collect();
    assert!(strays.is_empty(), "left a partial file behind: {strays:?}");

    // Counterpart: the same file untruncated does restore, so this is not
    // satisfied by a decrypt that refuses everything.
    std::fs::write(&encrypted, &full).expect("restore the file");
    let whole = dir.path().join("whole.db");
    let (_, stderr, code) = run(
        &[
            "--decrypt-backup",
            &encrypted.display().to_string(),
            "--out",
            &whole.display().to_string(),
        ],
        Some(PASS),
    );
    assert_eq!(code, 0, "the intact file must still restore: {stderr}");
    assert_eq!(std::fs::read(&whole).expect("read"), plain);
}

#[test]
fn an_existing_destination_is_refused_rather_than_overwritten() {
    // The most likely `--out` on a station is the live `birds.db`. Overwriting
    // it during a restore that then failed would destroy the thing the operator
    // was trying to compare against.
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, encrypted) = fixture(dir.path());
    let existing = dir.path().join("birds.db");
    std::fs::write(&existing, b"the live database").expect("write");

    let (_, stderr, code) = run(
        &[
            "--decrypt-backup",
            &encrypted.display().to_string(),
            "--out",
            &existing.display().to_string(),
        ],
        Some(PASS),
    );
    assert_eq!(code, 1);
    assert!(stderr.contains("already exists"), "{stderr}");
    assert_eq!(
        std::fs::read(&existing).expect("read"),
        b"the live database",
        "the existing file was modified"
    );
}

#[test]
fn a_missing_passphrase_is_reported_rather_than_guessed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, encrypted) = fixture(dir.path());
    let (_, stderr, code) = run(
        &[
            "--decrypt-backup",
            &encrypted.display().to_string(),
            "--out",
            &dir.path().join("out.db").display().to_string(),
        ],
        None,
    );
    assert_eq!(code, 1);
    assert!(
        stderr.contains("OFFSITE_PASSPHRASE"),
        "the error should name the key to set: {stderr}"
    );
}

#[test]
fn the_passphrase_cannot_be_passed_as_an_argument() {
    // Deliberate, and worth a gate at the CLI boundary as well as in the
    // planner's unit test: a secret in `argv` is visible in `ps` to every user
    // on the machine and is copied into the journal by systemd.
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, encrypted) = fixture(dir.path());
    let (_, stderr, code) = run(
        &[
            "--decrypt-backup",
            &encrypted.display().to_string(),
            "--out",
            &dir.path().join("out.db").display().to_string(),
            "--offsite-passphrase",
            PASS,
        ],
        None,
    );
    assert_ne!(code, 0, "`--offsite-passphrase` should not be a flag");
    assert!(
        stderr.contains("unexpected argument") || stderr.contains("--offsite-passphrase"),
        "clap should reject the unknown flag: {stderr}"
    );
}
