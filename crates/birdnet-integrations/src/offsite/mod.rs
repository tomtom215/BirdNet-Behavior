//! Sending backups somewhere other than the SD card they were written on.
//!
//! A station's rolling backups live beside its database, on the same card.
//! That covers the failures this project has always covered — a corrupt page, a
//! bad VACUUM, an interrupted write — and none of the ones that actually end a
//! station's records: the card wears out, the enclosure floods, the Pi is
//! stolen. This module is the other half.
//!
//! # Shape
//!
//! ```text
//!   backups/birds.db.backup.1733400000     the local snapshot, unchanged
//!            │
//!            ├─ envelope::encrypt ─────►  birds.db.backup.1733400000.bnb
//!            │                             (argon2id + ChaCha20-Poly1305)
//!            └─ s3::put_object / sftp::put
//! ```
//!
//! Encryption is not optional and not configurable. A station's database is a
//! log of where somebody lives and when they are home; "server-side encryption"
//! on the bucket means the provider holds the key, and an SFTP host means its
//! administrator does. The only useful promise is that the plaintext exists on
//! the station and on whatever machine the operator restores it to, and nowhere
//! else — so [`OffsiteConfig`] cannot be built without a passphrase.
//!
//! # Retention
//!
//! Ordered by the Unix timestamp in the station's own filename, not by the
//! store's `LastModified`. Those differ whenever a station uploads a backlog:
//! four backups uploaded in one catch-up all carry today's `LastModified`, and
//! pruning by that would keep an arbitrary four rather than the newest four.
//!
//! Retention runs after a successful upload and never before, so a run that
//! could not upload cannot also delete — which is the sequence that turns "the
//! offsite copy is stale" into "there is no offsite copy".

pub mod envelope;
pub mod s3;
pub mod sftp;
pub mod sigv4;

use std::path::{Path, PathBuf};

/// Extension an encrypted backup carries offsite.
///
/// Distinct from the local snapshot's name so a file that reached the store and
/// a file that did not cannot be confused, and so `file(1)`, a bucket listing
/// and an operator all see immediately that this is not a database they can
/// open.
pub const ENCRYPTED_SUFFIX: &str = ".bnb";

/// Where offsite backups go.
#[derive(Debug, Clone)]
pub enum Destination {
    /// An S3-compatible object store.
    S3(Box<s3::S3Target>),
    /// An SSH file server.
    Sftp(Box<sftp::SftpTarget>),
}

impl Destination {
    /// A short, log-safe description. Carries no credentials.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::S3(t) => format!("s3 {}/{}", t.endpoint, t.bucket),
            Self::Sftp(t) => format!("sftp {}@{}:{}", t.user, t.host, t.remote_dir),
        }
    }
}

/// Everything an offsite run needs.
#[derive(Debug, Clone)]
pub struct OffsiteConfig {
    /// Where the backups go.
    pub destination: Destination,
    /// The passphrase every backup is encrypted under.
    ///
    /// Not `Debug`-printable — see the manual `Debug` on [`Passphrase`].
    pub passphrase: Passphrase,
    /// How many backups to keep at the destination. `0` keeps everything.
    pub keep: usize,
}

/// A passphrase that does not print itself.
#[derive(Clone)]
pub struct Passphrase(String);

impl Passphrase {
    /// Wrap a passphrase, rejecting one too short to be worth encrypting under.
    ///
    /// # Errors
    ///
    /// [`envelope::EnvelopeError::PassphraseTooShort`] below
    /// [`envelope::MIN_PASSPHRASE_LEN`].
    pub fn new(value: String) -> Result<Self, envelope::EnvelopeError> {
        let len = value.chars().count();
        if len < envelope::MIN_PASSPHRASE_LEN {
            return Err(envelope::EnvelopeError::PassphraseTooShort { len });
        }
        Ok(Self(value))
    }

    /// The passphrase itself. Only [`envelope`] should need this.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

/// What can go wrong sending a backup offsite.
#[derive(Debug)]
pub enum OffsiteError {
    /// Encryption failed, or the passphrase was refused.
    Envelope(envelope::EnvelopeError),
    /// The local backup could not be read, or the encrypted copy written.
    Io(std::io::Error),
    /// The S3 destination refused.
    S3(s3::S3Error),
    /// The SFTP destination refused.
    Sftp(sftp::SftpError),
    /// The encryption task could not be run.
    Join(String),
}

impl std::fmt::Display for OffsiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Envelope(e) => write!(f, "encrypting the backup: {e}"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::S3(e) => write!(f, "{e}"),
            Self::Sftp(e) => write!(f, "{e}"),
            Self::Join(e) => write!(f, "the encryption task failed: {e}"),
        }
    }
}

impl std::error::Error for OffsiteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Envelope(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::S3(e) => Some(e),
            Self::Sftp(e) => Some(e),
            Self::Join(_) => None,
        }
    }
}

impl From<envelope::EnvelopeError> for OffsiteError {
    fn from(e: envelope::EnvelopeError) -> Self {
        Self::Envelope(e)
    }
}
impl From<std::io::Error> for OffsiteError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<s3::S3Error> for OffsiteError {
    fn from(e: s3::S3Error) -> Self {
        Self::S3(e)
    }
}
impl From<sftp::SftpError> for OffsiteError {
    fn from(e: sftp::SftpError) -> Self {
        Self::Sftp(e)
    }
}

/// What one offsite run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsiteReport {
    /// Name the backup landed under at the destination.
    pub uploaded: String,
    /// Bytes of ciphertext sent.
    pub bytes: u64,
    /// Names removed by retention.
    pub pruned: Vec<String>,
    /// Backups at the destination after the run.
    pub kept: usize,
}

/// The name an offsite copy of a local snapshot carries.
#[must_use]
pub fn offsite_name(local: &Path) -> String {
    let base = local
        .file_name()
        .map_or_else(|| "backup".to_owned(), |n| n.to_string_lossy().into_owned());
    format!("{base}{ENCRYPTED_SUFFIX}")
}

/// The station-recorded timestamp in a backup name, for ordering.
///
/// `birds.db.backup.1733400000.bnb` → `1733400000`. `None` for a name this
/// station did not write, which retention treats as "not mine, do not touch" —
/// an operator who keeps other files in the same bucket prefix should not find
/// them deleted.
#[must_use]
pub fn backup_timestamp(name: &str) -> Option<u64> {
    let stem = name.strip_suffix(ENCRYPTED_SUFFIX)?;
    let (_, ts) = stem.rsplit_once(".backup.")?;
    ts.parse().ok()
}

/// Which names retention should remove, newest kept.
///
/// `keep == 0` prunes nothing. Names this station did not write are never
/// returned; see [`backup_timestamp`].
#[must_use]
pub fn prune_list(names: &[String], keep: usize) -> Vec<String> {
    if keep == 0 {
        return Vec::new();
    }
    let mut ours: Vec<(u64, &String)> = names
        .iter()
        .filter_map(|n| backup_timestamp(n).map(|ts| (ts, n)))
        .collect();
    // Newest first, and by name on a tie so the answer does not depend on the
    // order the store happened to list them in.
    ours.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(a.1)));
    ours.into_iter()
        .skip(keep)
        .map(|(_, n)| n.clone())
        .collect()
}

/// Encrypt `local` into `dest`, returning the ciphertext length.
///
/// Blocking: argon2 and the AEAD are CPU work. Callers on an async runtime must
/// wrap this in `spawn_blocking`, which [`run`] does.
///
/// # Errors
///
/// [`OffsiteError::Io`] or [`OffsiteError::Envelope`].
pub fn encrypt_to(passphrase: &Passphrase, local: &Path, dest: &Path) -> Result<u64, OffsiteError> {
    let mut src = std::fs::File::open(local)?;
    let mut out = std::fs::File::create(dest)?;
    envelope::encrypt(passphrase.expose(), &mut src, &mut out)?;
    Ok(std::fs::metadata(dest)?.len())
}

/// Encrypt one local backup, send it, then prune the destination.
///
/// `scratch` is a directory the ciphertext is written to and removed from; on a
/// station that is the backup directory itself, so the temporary file lands on
/// the same filesystem as the thing it is a copy of and a full disk fails here
/// rather than half-way through an upload.
///
/// # Errors
///
/// Any of [`OffsiteError`]. Retention runs only after a successful upload, so
/// an error leaves whatever was already at the destination alone.
pub async fn run(
    config: &OffsiteConfig,
    local: &Path,
    scratch: &Path,
) -> Result<OffsiteReport, OffsiteError> {
    let name = offsite_name(local);
    let ciphertext: PathBuf = scratch.join(format!("{name}.tmp"));

    // Encrypt off the runtime: argon2id is 19 MiB and ~30 ms, and the AEAD runs
    // over the whole database.
    let bytes = {
        let passphrase = config.passphrase.clone();
        let local = local.to_path_buf();
        let dest = ciphertext.clone();
        tokio::task::spawn_blocking(move || encrypt_to(&passphrase, &local, &dest))
            .await
            .map_err(|e| OffsiteError::Join(e.to_string()))??
    };

    let result = send_and_prune(config, &name, &ciphertext).await;

    // The ciphertext is a copy; remove it whether or not the upload worked, or
    // a station with an unreachable destination fills its own disk with the
    // backups it could not send.
    let _ = std::fs::remove_file(&ciphertext);

    let (pruned, kept) = result?;
    Ok(OffsiteReport {
        uploaded: name,
        bytes,
        pruned,
        kept,
    })
}

/// How many of *this station's* backups remain after a prune.
///
/// Counted rather than subtracted from the listing length: an operator sharing
/// a bucket prefix with other files would otherwise see those counted as
/// backups, and a report saying "8 kept" when there are two is worse than no
/// report at all.
fn kept_count(names: &[String], pruned: &[String]) -> usize {
    names
        .iter()
        .filter(|n| backup_timestamp(n).is_some())
        .filter(|n| !pruned.contains(n))
        .count()
}

/// Upload, then apply retention. Split out so [`run`] can clean up either way.
async fn send_and_prune(
    config: &OffsiteConfig,
    name: &str,
    ciphertext: &Path,
) -> Result<(Vec<String>, usize), OffsiteError> {
    match &config.destination {
        Destination::S3(t) => {
            let client = s3::client()?;
            let (digest, len) = s3::file_sha256(ciphertext)?;
            t.put_object(&client, name, ciphertext, &digest, len)
                .await?;

            let objects = t.list_objects(&client).await?;
            let prefix = t.prefix.trim_matches('/');
            let names: Vec<String> = objects
                .iter()
                .map(|o| {
                    o.key
                        .strip_prefix(prefix)
                        .unwrap_or(&o.key)
                        .trim_start_matches('/')
                        .to_owned()
                })
                .collect();
            let doomed = prune_list(&names, config.keep);
            for victim in &doomed {
                t.delete_object(&client, &t.key_for(victim)).await?;
            }
            Ok((doomed.clone(), kept_count(&names, &doomed)))
        }
        Destination::Sftp(t) => {
            t.put(name, ciphertext).await?;
            let names = t.list().await?;
            let doomed = prune_list(&names, config.keep);
            for victim in &doomed {
                t.remove(victim).await?;
            }
            Ok((doomed.clone(), kept_count(&names, &doomed)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offsite_name_is_the_local_name_plus_the_suffix() {
        assert_eq!(
            offsite_name(Path::new("/data/backups/birds.db.backup.1733400000")),
            "birds.db.backup.1733400000.bnb"
        );
    }

    #[test]
    fn retention_orders_by_the_stations_own_timestamp() {
        // Not by the store's LastModified, which is when the upload finished. A
        // station catching up on a backlog uploads four backups in one minute;
        // ordering by upload time would keep an arbitrary subset.
        let names: Vec<String> = ["100", "300", "200", "400"]
            .iter()
            .map(|t| format!("birds.db.backup.{t}.bnb"))
            .collect();
        let mut pruned = prune_list(&names, 2);
        pruned.sort();
        assert_eq!(
            pruned,
            vec![
                "birds.db.backup.100.bnb".to_owned(),
                "birds.db.backup.200.bnb".to_owned()
            ],
            "the two oldest by station timestamp must go"
        );
    }

    #[test]
    fn retention_never_touches_a_file_this_station_did_not_write() {
        // An operator may keep other things in the same bucket prefix — an
        // export, a photo, another station's backups. Deleting one of those
        // because it sorted oldest would be unrecoverable, and it is the kind
        // of thing nobody notices until they need the file.
        let names: Vec<String> = vec![
            "birds.db.backup.100.bnb".to_owned(),
            "birds.db.backup.200.bnb".to_owned(),
            "birds.db.backup.300.bnb".to_owned(),
            "notes.txt".to_owned(),
            "an-export.csv".to_owned(),
            // Plausible but not ours: no `.backup.` segment, and the wrong
            // suffix respectively.
            "birds.db.1733400000.bnb".to_owned(),
            "birds.db.backup.1733400000".to_owned(),
        ];
        let pruned = prune_list(&names, 1);
        assert_eq!(
            pruned,
            vec![
                "birds.db.backup.200.bnb".to_owned(),
                "birds.db.backup.100.bnb".to_owned()
            ],
            "only this station's own backups may be pruned: {pruned:?}"
        );
    }

    #[test]
    fn keeping_zero_means_keep_everything_not_delete_everything() {
        // The reading that costs an operator their whole offsite history. `0`
        // is the "unset" value in a config file, and a station that took it as
        // "keep none" would delete the backup it had just uploaded.
        let names: Vec<String> = (1..=5)
            .map(|i| format!("birds.db.backup.{i}.bnb"))
            .collect();
        assert!(
            prune_list(&names, 0).is_empty(),
            "keep=0 must prune nothing"
        );
        // Counterpart: a real limit does prune.
        assert_eq!(prune_list(&names, 3).len(), 2);
        // And a limit at or above the count prunes nothing.
        assert!(prune_list(&names, 5).is_empty());
        assert!(prune_list(&names, 50).is_empty());
    }

    #[test]
    fn retention_is_stable_when_two_backups_share_a_timestamp() {
        // Two snapshots in the same second is possible on a fast disk, and a
        // sort that left the order to the store's listing would prune a
        // different one on each run — so a station could delete both across two
        // passes while believing it kept one.
        let names: Vec<String> = vec![
            "birds.db.backup.100.bnb".to_owned(),
            "other.db.backup.100.bnb".to_owned(),
            "birds.db.backup.200.bnb".to_owned(),
        ];
        let first = prune_list(&names, 2);
        let mut reversed = names;
        reversed.reverse();
        assert_eq!(
            first,
            prune_list(&reversed, 2),
            "the answer must not depend on the order the store listed them in"
        );
    }

    #[test]
    fn a_timestamp_is_read_only_from_the_shape_this_station_writes() {
        assert_eq!(
            backup_timestamp("birds.db.backup.1733400000.bnb"),
            Some(1_733_400_000)
        );
        // The last `.backup.` wins, so a database that itself contains the word
        // still parses.
        assert_eq!(backup_timestamp("my.backup.db.backup.42.bnb"), Some(42));
        for not_ours in [
            "birds.db.backup.1733400000", // not encrypted
            "birds.db.1733400000.bnb",    // no `.backup.`
            "birds.db.backup.notanumber.bnb",
            "notes.txt",
            "",
        ] {
            assert_eq!(
                backup_timestamp(not_ours),
                None,
                "`{not_ours}` is not a name this station writes"
            );
        }
    }

    #[test]
    fn the_kept_count_reports_backups_and_not_bucket_contents() {
        // A shared prefix with an operator's own files in it. Reporting "5
        // kept" when there are two backups and three photographs is a number
        // nobody can act on.
        let names: Vec<String> = vec![
            "birds.db.backup.100.bnb".to_owned(),
            "birds.db.backup.200.bnb".to_owned(),
            "birds.db.backup.300.bnb".to_owned(),
            "holiday.jpg".to_owned(),
            "notes.txt".to_owned(),
        ];
        let pruned = vec!["birds.db.backup.100.bnb".to_owned()];
        assert_eq!(
            kept_count(&names, &pruned),
            2,
            "only this station's own backups count towards the retention report"
        );
        // Counterpart: with nothing pruned it is the count of ours, not zero
        // and not the whole listing.
        assert_eq!(kept_count(&names, &[]), 3);
    }

    #[test]
    fn a_passphrase_does_not_print_itself() {
        // It reaches a config struct that `tracing` formats and the support
        // bundle includes.
        let p = Passphrase::new("correct horse battery staple".to_owned()).expect("long enough");
        let shown = format!("{p:?}");
        assert!(!shown.contains("correct"), "the passphrase leaked: {shown}");
        assert_eq!(p.expose(), "correct horse battery staple");

        assert!(matches!(
            Passphrase::new("short".to_owned()),
            Err(envelope::EnvelopeError::PassphraseTooShort { len: 5 })
        ));
    }

    #[test]
    fn a_destination_description_carries_no_credentials() {
        // It is logged on every run and printed by `--doctor`.
        let s3 = Destination::S3(Box::new(s3::S3Target {
            endpoint: "https://s3.eu-west-2.amazonaws.com".to_owned(),
            bucket: "birdnet".to_owned(),
            prefix: "pi-1".to_owned(),
            region: "eu-west-2".to_owned(),
            credentials: sigv4::Credentials {
                access_key: "AKIAIOSFODNN7EXAMPLE".to_owned(),
                secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned(),
            },
            addressing: s3::Addressing::VirtualHost,
        }));
        let shown = s3.describe();
        assert!(
            !shown.contains("wJalrXUtnFEMI"),
            "secret key leaked: {shown}"
        );
        assert!(
            shown.contains("birdnet"),
            "should still identify the bucket"
        );
    }

    #[test]
    fn encrypt_to_writes_something_that_is_not_the_database() {
        // The one thing this whole module exists to guarantee, checked rather
        // than assumed: what leaves the station is not readable.
        let dir = tempfile::tempdir().expect("tempdir");
        let local = dir.path().join("birds.db.backup.1");
        // A recognisable SQLite header, plus a species name in the body.
        let mut plain = b"SQLite format 3\0".to_vec();
        plain.extend_from_slice(b"Turdus merula was heard at 51.5074,-0.1278");
        plain.resize(4096, 0);
        std::fs::write(&local, &plain).expect("write");

        let out = dir.path().join("out.bnb");
        let pass = Passphrase::new("correct horse battery staple".to_owned()).expect("ok");
        let len = encrypt_to(&pass, &local, &out).expect("encrypt");

        let cipher = std::fs::read(&out).expect("read");
        assert_eq!(cipher.len() as u64, len);
        assert!(
            !cipher.windows(16).any(|w| w == b"SQLite format 3\0"),
            "the database header survived into the uploaded file"
        );
        assert!(
            !cipher.windows(14).any(|w| w == b"Turdus merula "),
            "a species name survived into the uploaded file"
        );
        assert!(
            envelope::is_envelope(&cipher),
            "the uploaded file must be a recognisable envelope"
        );

        // And it comes back.
        let mut restored = Vec::new();
        envelope::decrypt(pass.expose(), &mut &cipher[..], &mut restored).expect("decrypt");
        assert_eq!(restored, plain);
    }
}
