//! Installing a classifier, without ever installing the wrong one (`G-10`
//! Stage 5).
//!
//! Stage 2 let a station run more than one classifier. Getting a second one
//! onto the station was still "download 400 MB from somewhere and put it in
//! the right directory", which on an unattended box at the end of a domestic
//! uplink is several ways to end up with a station that is quietly broken.
//!
//! # What this refuses to do
//!
//! **Install anything it cannot verify.** Every catalogue entry carries a
//! sha256 measured from the actual file. A download that does not match it is
//! deleted, not installed, and not reported as a warning to be ignored later.
//! This is the same posture [`crate::auto_update`] takes toward the binary,
//! and for the same reason: "we could not check this" and "this is fine" are
//! different sentences.
//!
//! **Download onto a disk that cannot hold it.** A 400 MB model onto a card
//! with 300 MB free does not fail cleanly — it fills the card, and a station
//! whose disk is full stops recording birds, which is a far worse outcome than
//! not having a second classifier. Free space is checked first, with room to
//! spare.
//!
//! **Leave a half-written file where the loader will find it.** The download
//! lands on a `.part` path the station never looks at and is renamed into
//! place only after its digest matches. A power cut mid-download leaves
//! rubbish with a name nothing reads, which the next install overwrites.
//!
//! # Where this differs from the binary updater, and why
//!
//! [`crate::auto_update::apply_update`] verifies the asset **before** writing
//! anything to disk — it reads the whole thing into memory first. That is
//! right for a 20 MB binary and wrong here: the models this installs are 409
//! and 541 MB, and buffering one on a board with a gigabyte of RAM is the
//! out-of-memory kill that `G-33`'s whole memory policy exists to avoid.
//!
//! So the bytes stream to disk and the digest is computed as they go. The
//! safety property is preserved by a different means: what lands on disk
//! unverified has a name the station cannot load, and only a verified file is
//! ever given the real one.
//!
//! # Why the catalogue is compiled in rather than fetched
//!
//! Upstream fetches a catalogue from Hugging Face with a configurable endpoint.
//! A compiled-in list is the safer shape for a station nobody is watching: a
//! fetched catalogue is a second network dependency in the path of installing
//! a model, and — more to the point — it is a remote document that decides
//! which URL the station downloads hundreds of megabytes from and which digest
//! it checks them against. Pinning both here means the checksum is a promise
//! this repository makes, verifiable by anyone reading the file, rather than
//! whatever the endpoint said today.
//!
//! The cost is that a new model needs a release. That is the right cost.

use std::fmt;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

/// How this identifies itself when downloading.
const USER_AGENT: &str = "BirdNet-Behavior-ModelInstaller";

/// Free space required beyond the model's own size, in bytes.
///
/// A download that exactly fits leaves a station with nothing in hand, and the
/// same card carries the database, the recordings and the write-ahead log. 512
/// MiB is roughly one more model's worth of headroom — deliberately generous,
/// because the failure this prevents (a full card stops the station recording)
/// is much worse than the one it causes (an install refused on a tight disk).
const FREE_SPACE_MARGIN: u64 = 512 * 1024 * 1024;

/// Bytes read from the network at a time.
const CHUNK: usize = 1024 * 1024;

/// One classifier a station can install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    /// Stable id, used as the installed filename and as a routing target.
    pub id: &'static str,
    /// What to call it to an operator.
    pub name: &'static str,
    /// Where the ONNX file comes from.
    pub model_url: &'static str,
    /// Lowercase hex sha256 of that file, measured from the file itself.
    pub model_sha256: &'static str,
    /// Its size in bytes, for the free-space check before downloading.
    pub model_bytes: u64,
    /// Where its labels come from, when the catalogue ships them.
    pub labels_url: Option<&'static str>,
    /// Lowercase hex sha256 of the labels file.
    pub labels_sha256: Option<&'static str>,
    /// The sample rate it needs — what `MODEL_n_SAMPLE_RATE` should be set to.
    ///
    /// Carried here because it is not derivable from the tensor: Perch v2's
    /// `[-1, 160_000]` is 5 s at 32 kHz and equally 3⅓ s at 48 kHz. See
    /// `G-10` Stage 4.
    pub sample_rate: u32,
    /// Anything an operator needs to know before installing it.
    pub notes: &'static str,
}

/// The classifiers this build knows how to install.
///
/// Every digest here was measured from the file this session, not copied from
/// a model card. A gate asserts each is a well-formed 64-character lowercase
/// hex digest, because a malformed one would make the entry uninstallable in a
/// way nothing else would notice until somebody tried it in the field.
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "birdnet-v3",
        name: "BirdNET+ V3.0 preview3 (global, 11K species)",
        model_url: "https://github.com/tomtom215/BirdNet-Behavior/releases/download/\
                    models-v3.0-preview3/BirdNET%2B_V3.0-preview3_Global_11K_FP32.onnx",
        model_sha256: "2a0f9efba1a98e3193ad3dfcb8323116a7de88e39545f3619a7ea46e3bb7d743",
        model_bytes: 541_391_777,
        labels_url: Some(
            "https://github.com/tomtom215/BirdNet-Behavior/releases/download/\
             models-v3.0-preview3/BirdNET%2B_V3.0-preview3_Global_11K_Labels.csv",
        ),
        labels_sha256: Some("8124b0ea2d187104c5e2cd95a0f937165647e20349c8fd34d4d5ef991821f8f0"),
        sample_rate: 32_000,
        notes: "The station's default classifier. Reports a dynamic input shape, \
                which resolves to 4.5 s windows at 32 kHz.",
    },
    CatalogEntry {
        id: "perch-v2",
        name: "Google Perch v2 (14 795 classes)",
        model_url: "https://huggingface.co/justinchuby/Perch-onnx/resolve/main/perch_v2.onnx",
        model_sha256: "bf0c8467a924cb074663970ca4a0ab1e143602121930209657d0dff5d5cefa1f",
        model_bytes: 409_148_616,
        // No labels file accompanies the ONNX conversion; an operator supplies
        // one. Saying so here is better than shipping a URL that 404s in the
        // field.
        labels_url: None,
        labels_sha256: None,
        sample_rate: 32_000,
        notes: "Materially better than BirdNET in the tropics. Wants 5 s windows \
                at 32 kHz, which differs from BirdNET's 4.5 s — the two cannot yet \
                run together (see G-10 Stage 4). No labels file ships with the ONNX \
                conversion; supply one with 14 795 rows.",
    },
];

/// Look an entry up by id.
#[must_use]
pub fn find(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// What can go wrong installing a classifier.
#[derive(Debug)]
pub enum InstallError {
    /// No catalogue entry by that id.
    Unknown {
        /// What was asked for.
        id: String,
        /// What is available, so the message is actionable.
        known: Vec<&'static str>,
    },
    /// The digest in the catalogue is not a usable sha256.
    Unverifiable {
        /// Which entry.
        id: &'static str,
        /// Why it cannot be checked.
        why: String,
    },
    /// Not enough free space to hold the download plus a margin.
    NotEnoughSpace {
        /// Bytes needed, including the margin.
        needed: u64,
        /// Bytes free.
        available: u64,
    },
    /// The download failed or the server answered with an error.
    Network(String),
    /// The downloaded bytes do not match the catalogue's digest.
    Integrity {
        /// What the catalogue says.
        expected: &'static str,
        /// What arrived.
        actual: String,
    },
    /// A filesystem operation failed.
    Io(std::io::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { id, known } => write!(
                f,
                "no classifier called `{id}` is in the catalogue (available: {})",
                known.join(", ")
            ),
            Self::Unverifiable { id, why } => write!(
                f,
                "classifier `{id}` cannot be verified: {why}. Refusing to install it — \
                 a model that cannot be checked against anything is a model that could \
                 be anything"
            ),
            Self::NotEnoughSpace { needed, available } => write!(
                f,
                "not enough free space: {} MiB needed (including a {} MiB margin), \
                 {} MiB available. Refusing to download — filling the card would stop \
                 the station recording, which is worse than not having this classifier",
                needed / (1024 * 1024),
                FREE_SPACE_MARGIN / (1024 * 1024),
                available / (1024 * 1024)
            ),
            Self::Network(why) => write!(f, "download failed: {why}"),
            Self::Integrity { expected, actual } => write!(
                f,
                "sha256 mismatch: expected {expected}, got {actual}. The download has \
                 been deleted. Either it was corrupted in transit or the file at that \
                 URL is not the one this build was built against"
            ),
            Self::Io(e) => write!(f, "filesystem error: {e}"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Where an installed classifier ended up, and what to configure it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The catalogue id.
    pub id: &'static str,
    /// The installed ONNX file.
    pub model_path: PathBuf,
    /// The installed labels file, when the catalogue shipped one.
    pub labels_path: Option<PathBuf>,
    /// The sample rate this classifier needs.
    pub sample_rate: u32,
}

/// Check a catalogue digest is a usable sha256.
///
/// A malformed digest must be refused **before** anything is downloaded: an
/// entry that can never verify would otherwise pull hundreds of megabytes over
/// somebody's uplink and then throw them away.
fn require_digest(id: &'static str, hex: &str) -> Result<(), InstallError> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(InstallError::Unverifiable {
            id,
            why: format!("`{hex}` is not a 64-character hex sha256"),
        });
    }
    Ok(())
}

/// Whether a download of `model_bytes` fits, given what the filesystem says
/// is free.
///
/// A pure function so the policy can be tested without arranging a full disk.
/// The first version of this was inline in [`install`], and both gates written
/// for it passed with the check deleted: one accepted an unrelated I/O error
/// from `/proc` as success, and the other only ever exercised
/// [`free_space_bytes`]. Separating the decision from the measurement is what
/// makes it checkable.
///
/// `available: None` is **not** enough room. A filesystem that will not say
/// how much space it has cannot be shown to have any, and the reading that
/// treats unknown as plenty is the one that fills a station's card.
///
/// # Errors
///
/// [`InstallError::NotEnoughSpace`], carrying both numbers so the message can
/// show its working.
pub fn fits(model_bytes: u64, available: Option<u64>) -> Result<(), InstallError> {
    let needed = model_bytes.saturating_add(FREE_SPACE_MARGIN);
    let available = available.unwrap_or(0);
    if available < needed {
        return Err(InstallError::NotEnoughSpace { needed, available });
    }
    Ok(())
}

/// Free bytes on the filesystem holding `dir`.
///
/// `None` when it cannot be read, which callers must treat as "cannot show it
/// fits" rather than "plenty" — the same reading `G-10` Stage 2 takes of an
/// unknown memory ceiling, for the same reason.
#[must_use]
pub fn free_space_bytes(dir: &Path) -> Option<u64> {
    // `statvfs` through the `libc`-free route: read what the shell would.
    // Deliberately not a new dependency for one number.
    let out = std::process::Command::new("df")
        .arg("-kP")
        .arg(dir)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1)?;
    let available_kib: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(available_kib * 1024)
}

/// Download one file, hashing as it streams, and rename it into place only
/// once the digest matches.
fn fetch_verified(
    client: &reqwest::blocking::Client,
    url: &str,
    expected: &'static str,
    dest: &Path,
) -> Result<(), InstallError> {
    let part = dest.with_extension("part");
    // A previous attempt that died mid-stream leaves one of these. It has a
    // name nothing loads, so it is rubbish rather than a hazard — but it is
    // still stale bytes, and appending to them would produce a file that
    // matches no digest at all.
    if part.exists() {
        fs::remove_file(&part)?;
    }

    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| InstallError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(InstallError::Network(format!(
            "{url} answered {}",
            resp.status()
        )));
    }

    let mut file = fs::File::create(&part)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = resp
            .read(&mut buf)
            .map_err(|e| InstallError::Network(format!("read failed: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    file.flush()?;
    // The rename below is only atomic with respect to a crash if the bytes are
    // actually on the device first.
    file.sync_all()?;
    drop(file);

    let actual = {
        use std::fmt::Write as _;
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(64);
        for byte in &digest {
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    };
    if !actual.eq_ignore_ascii_case(expected) {
        // Deleted rather than left for somebody to find and wonder about.
        let _ = fs::remove_file(&part);
        return Err(InstallError::Integrity { expected, actual });
    }

    fs::rename(&part, dest)?;
    Ok(())
}

/// Install a classifier from the catalogue into `dest_dir`.
///
/// # Errors
///
/// [`InstallError`] for an unknown id, a digest that cannot verify, a disk
/// that cannot hold the download, a network failure, or a digest mismatch.
/// Nothing is left under a name the station would load unless it verified.
pub fn install(id: &str, dest_dir: &Path) -> Result<Installed, InstallError> {
    let entry = find(id).ok_or_else(|| InstallError::Unknown {
        id: id.to_owned(),
        known: CATALOG.iter().map(|e| e.id).collect(),
    })?;

    // Before the network: an entry that can never verify must not cost
    // somebody 400 MB of a metered uplink to discover that.
    require_digest(entry.id, entry.model_sha256)?;
    if let Some(labels_sha) = entry.labels_sha256 {
        require_digest(entry.id, labels_sha)?;
    }

    fs::create_dir_all(dest_dir)?;

    fits(entry.model_bytes, free_space_bytes(dest_dir))?;

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        // Long: 400 MB over a domestic uplink is measured in hours, and a
        // transfer killed at 90 % has cost the operator the whole thing.
        .timeout(std::time::Duration::from_secs(6 * 60 * 60))
        .build()
        .map_err(|e| InstallError::Network(e.to_string()))?;

    let model_path = dest_dir.join(format!("{}.onnx", entry.id));
    tracing::info!(
        id = entry.id,
        url = entry.model_url,
        "downloading classifier"
    );
    fetch_verified(&client, entry.model_url, entry.model_sha256, &model_path)?;
    tracing::info!(id = entry.id, path = %model_path.display(), "classifier verified and installed");

    let labels_path = match (entry.labels_url, entry.labels_sha256) {
        (Some(url), Some(sha)) => {
            let path = dest_dir.join(format!("{}_labels.csv", entry.id));
            fetch_verified(&client, url, sha, &path)?;
            Some(path)
        }
        _ => None,
    };

    Ok(Installed {
        id: entry.id,
        model_path,
        labels_path,
        sample_rate: entry.sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CATALOG, FREE_SPACE_MARGIN, InstallError, fetch_verified, find, fits, free_space_bytes,
        install, require_digest,
    };

    /// Every digest in the catalogue must be a well-formed sha256.
    ///
    /// A malformed one makes its entry permanently uninstallable, and nothing
    /// else would notice until an operator in the field tried it and lost the
    /// download. Checked at build time here rather than discovered there.
    ///
    /// Observed failing with one hex character removed from the Perch digest:
    /// the length assertion went red.
    #[test]
    fn every_catalogue_digest_is_a_well_formed_sha256() {
        assert!(!CATALOG.is_empty(), "an empty catalogue installs nothing");
        for entry in CATALOG {
            assert_eq!(
                entry.model_sha256.len(),
                64,
                "{}: model digest is not 64 characters",
                entry.id
            );
            assert!(
                entry
                    .model_sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{}: model digest is not lowercase hex",
                entry.id
            );
            assert!(
                require_digest(entry.id, entry.model_sha256).is_ok(),
                "{}",
                entry.id
            );
            if let Some(sha) = entry.labels_sha256 {
                assert!(require_digest(entry.id, sha).is_ok(), "{} labels", entry.id);
            }
        }
    }

    /// A labels URL without its digest, or the reverse, would either install
    /// an unverified file or silently skip one an operator was promised.
    #[test]
    fn labels_are_either_fully_specified_or_absent() {
        for entry in CATALOG {
            assert_eq!(
                entry.labels_url.is_some(),
                entry.labels_sha256.is_some(),
                "{}: a labels URL and its digest must travel together",
                entry.id
            );
        }
    }

    /// Ids are what an operator types and what the installed file is named, so
    /// two entries sharing one would overwrite each other.
    #[test]
    fn catalogue_ids_are_unique_and_filename_safe() {
        let mut ids: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate catalogue id");
        for entry in CATALOG {
            assert!(
                entry
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "{}: id becomes a filename and a routing target",
                entry.id
            );
            assert!(
                entry.model_bytes > 0,
                "{}: size drives the space check",
                entry.id
            );
            assert!(entry.sample_rate > 0, "{}", entry.id);
        }
    }

    /// Every URL is HTTPS. A model fetched over plain HTTP could be replaced
    /// in transit, and while the digest would catch that, an operator should
    /// not be relying on the last line of defence for the first one.
    #[test]
    fn every_catalogue_url_is_https() {
        for entry in CATALOG {
            assert!(
                entry.model_url.starts_with("https://"),
                "{}: {}",
                entry.id,
                entry.model_url
            );
            if let Some(url) = entry.labels_url {
                assert!(url.starts_with("https://"), "{}: {url}", entry.id);
            }
        }
    }

    /// **A digest that cannot verify is refused before anything downloads.**
    /// Discovering it afterwards costs an operator on a metered uplink the
    /// entire transfer.
    ///
    /// Observed failing with the length and hex checks removed from
    /// `require_digest`: every malformed digest was accepted.
    #[test]
    fn a_malformed_digest_is_refused_before_the_network() {
        for bad in ["", "abc", &"z".repeat(64), &"a".repeat(63), &"A".repeat(65)] {
            let err = require_digest("x", bad).expect_err("must refuse");
            assert!(matches!(err, InstallError::Unverifiable { .. }), "{bad}");
            assert!(
                err.to_string().contains("could be anything"),
                "the reason must be legible: {err}"
            );
        }
        // The real digests pass, or the gate above proves nothing.
        assert!(require_digest("x", &"a".repeat(64)).is_ok());
        assert!(require_digest("x", CATALOG[0].model_sha256).is_ok());
    }

    /// An unknown id names what is available, because an operator who typed it
    /// wrong is one letter away from the right answer.
    #[test]
    fn an_unknown_id_lists_what_is_available() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = install("perch-v3", dir.path()).expect_err("unknown id");
        let msg = err.to_string();
        assert!(msg.contains("perch-v3"), "{msg}");
        assert!(msg.contains("perch-v2"), "it must list the real ids: {msg}");
    }

    /// **The refusal that keeps a station recording.** Filling the card is
    /// worse than not having a second classifier: a full disk stops the
    /// detection write path, and an unattended station cannot clear it.
    ///
    /// Observed failing with the `available < needed` comparison removed from
    /// `fits`: a download far larger than the free space was admitted.
    ///
    /// An earlier version of this gate called `install` against `/proc` and
    /// accepted an `Io` error as success — so it passed with the space check
    /// deleted, because `create_dir_all` failed first. It tested that
    /// something went wrong, not that the right thing did.
    #[test]
    fn a_download_that_would_not_fit_is_refused() {
        let err = fits(400 * 1024 * 1024, Some(100 * 1024 * 1024))
            .expect_err("400 MiB must not fit in 100 MiB");
        match &err {
            InstallError::NotEnoughSpace { needed, available } => {
                assert!(
                    *needed > 400 * 1024 * 1024,
                    "the margin must be counted too"
                );
                assert_eq!(*available, 100 * 1024 * 1024);
            }
            other => panic!("wrong error: {other:?}"),
        }
        assert!(
            err.to_string().contains("stop the station recording"),
            "{err}"
        );
    }

    /// Its counterpart: a download that comfortably fits is admitted, or the
    /// gate above would pass against a `fits` that refused everything.
    #[test]
    fn a_download_with_room_to_spare_is_admitted() {
        assert!(fits(400 * 1024 * 1024, Some(8 * 1024 * 1024 * 1024)).is_ok());
    }

    /// **The margin is real, not decorative.** A download that exactly fits
    /// the free space leaves the station nothing — and the same card carries
    /// the database, the recordings and the write-ahead log.
    ///
    /// Observed failing with `FREE_SPACE_MARGIN` added as 0: an exact fit was
    /// admitted.
    #[test]
    fn a_download_that_exactly_fills_the_disk_is_refused() {
        let size = 400 * 1024 * 1024;
        assert!(
            fits(size, Some(size)).is_err(),
            "exactly filling the card must be refused"
        );
        assert!(fits(size, Some(size + FREE_SPACE_MARGIN)).is_ok());
    }

    /// **Free space that cannot be read is none, never plenty** — the same
    /// reading Stage 2 takes of an unknown memory ceiling, for the same
    /// reason.
    ///
    /// Observed failing with `unwrap_or(0)` changed to `unwrap_or(u64::MAX)`
    /// in `fits`: an unmeasurable filesystem was treated as unlimited.
    #[test]
    fn unreadable_free_space_is_not_read_as_plenty() {
        assert!(
            fits(1, None).is_err(),
            "a filesystem that will not say how much space it has cannot be shown to have any"
        );
        // And the measurement itself reports the unknown honestly.
        assert_eq!(
            free_space_bytes(std::path::Path::new("/nonexistent-path-xyz")),
            None
        );
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(free_space_bytes(dir.path()).is_some());
    }

    /// Serve `body` once over HTTP on localhost and return its URL.
    ///
    /// A real server, because the property under test is what happens to the
    /// bytes on disk after they arrive — which an unreachable host never
    /// exercises. Twenty lines is a cheaper price than a gate that tests the
    /// wrong path.
    fn serve_once(body: &'static [u8]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                use std::io::{Read as _, Write as _};
                let mut scratch = [0u8; 1024];
                let _ = sock.read(&mut scratch);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes());
                let _ = sock.write_all(body);
                let _ = sock.flush();
            }
        });
        format!("http://127.0.0.1:{port}/model.onnx")
    }

    fn test_client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .user_agent("test")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("client")
    }

    /// **The safety property this whole module exists for.** Bytes that do not
    /// match the catalogue's digest are deleted, not installed — and the
    /// staging file goes with them, because a `.part` left behind is stale
    /// bytes the next attempt would have to know to ignore.
    ///
    /// Observed failing with the `remove_file` on mismatch removed (the
    /// `.part` survived) and, separately, with the rename moved above the
    /// digest check (the corrupt file was installed under its real name,
    /// which is the field failure in one line).
    #[test]
    fn bytes_that_do_not_match_the_digest_are_deleted_not_installed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("model.onnx");
        let url = serve_once(b"this is not the model you were promised");

        let err = fetch_verified(
            &test_client(),
            &url,
            "0000000000000000000000000000000000000000000000000000000000000000",
            &dest,
        )
        .expect_err("a digest mismatch must be refused");

        match &err {
            InstallError::Integrity { actual, .. } => {
                assert_eq!(
                    actual.len(),
                    64,
                    "the actual digest must be reported: {actual}"
                );
            }
            other => panic!("wrong error: {other:?}"),
        }
        assert!(
            !dest.exists(),
            "a mismatched download must never be installed"
        );
        assert!(
            !dest.with_extension("part").exists(),
            "the staging file must not survive a mismatch"
        );
        assert!(err.to_string().contains("has been deleted"), "{err}");
    }

    /// Its counterpart: bytes that DO match are installed under the real name
    /// and the staging file is gone. Without this, a `fetch_verified` that
    /// refused everything would pass the gate above.
    #[test]
    fn bytes_that_match_the_digest_are_installed_atomically() {
        const BODY: &[u8] = b"pretend this is an onnx file";
        // sha256("pretend this is an onnx file")
        let expected = {
            use sha2::{Digest as _, Sha256};
            use std::fmt::Write as _;
            let d = Sha256::digest(BODY);
            let mut hex = String::new();
            for b in &d {
                let _ = write!(hex, "{b:02x}");
            }
            hex
        };
        let leaked: &'static str = Box::leak(expected.into_boxed_str());

        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("model.onnx");
        let url = serve_once(BODY);

        fetch_verified(&test_client(), &url, leaked, &dest).expect("a matching digest installs");
        assert!(dest.exists(), "a verified download must be installed");
        assert_eq!(std::fs::read(&dest).expect("read"), BODY);
        assert!(
            !dest.with_extension("part").exists(),
            "the staging file must be renamed away, not left beside the real one"
        );
    }

    /// A network failure leaves nothing behind either.
    #[test]
    fn an_unreachable_host_leaves_nothing_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("model.onnx");
        let err = fetch_verified(
            &test_client(),
            "https://example.invalid/model.onnx",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &dest,
        )
        .expect_err("an unreachable host must fail");
        assert!(matches!(err, InstallError::Network(_)), "{err:?}");
        assert!(!dest.exists());
        assert!(!dest.with_extension("part").exists());
    }

    /// The two entries this build ships, pinned. Changing a URL or a digest
    /// is changing what a station downloads and what it checks it against, and
    /// should be a deliberate act with this test in the diff.
    #[test]
    fn the_catalogue_is_what_this_build_was_verified_against() {
        let birdnet = find("birdnet-v3").expect("birdnet-v3 is in the catalogue");
        assert_eq!(
            birdnet.model_sha256,
            "2a0f9efba1a98e3193ad3dfcb8323116a7de88e39545f3619a7ea46e3bb7d743"
        );
        assert_eq!(birdnet.model_bytes, 541_391_777);
        assert_eq!(birdnet.sample_rate, 32_000);
        assert!(birdnet.labels_url.is_some(), "BirdNET ships its labels");

        let perch = find("perch-v2").expect("perch-v2 is in the catalogue");
        assert_eq!(
            perch.model_sha256,
            "bf0c8467a924cb074663970ca4a0ab1e143602121930209657d0dff5d5cefa1f"
        );
        assert_eq!(perch.model_bytes, 409_148_616);
        assert_eq!(perch.sample_rate, 32_000);
        assert!(
            perch.labels_url.is_none(),
            "no labels ship with the ONNX conversion, and the catalogue says so \
             rather than shipping a URL that 404s in the field"
        );
        assert!(perch.notes.contains("14 795"), "{}", perch.notes);
    }

    #[test]
    fn find_returns_nothing_for_an_id_that_is_not_there() {
        assert!(find("nope").is_none());
    }
}
