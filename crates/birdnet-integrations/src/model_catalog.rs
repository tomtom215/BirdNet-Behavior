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
//! lands on a `<name>.<pid>.<n>.part` path the station never looks at (unique
//! to the process and the call, opened with `create_new`, so two installs never
//! write into one file) and is renamed into place only after its digest matches
//! and its length is the length received. It is stopped at the file's pinned
//! size, so an origin that never stops cannot fill the disk for the length of
//! the timeout. A power cut mid-download leaves rubbish with a name nothing
//! reads; the next install removes it once its process is gone.
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

/// The catalogue id of the classifier a new station runs: BirdNET V2.4, the
/// network BirdNET-Pi and birdnet-go run.
pub const DEFAULT_CLASSIFIER_ID: &str = "birdnet-v24";

/// The catalogue id of the species-occurrence ("range") filter that pairs with
/// [`DEFAULT_CLASSIFIER_ID`].
pub const DEFAULT_RANGE_FILTER_ID: &str = "birdnet-v24-range-v2";

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

/// The most a labels file may be. They are a few hundred kilobytes; the cap
/// only stops a hostile origin from streaming for the length of the timeout.
const LABELS_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The most a download with no pinned size may be (the test helper's).
#[cfg(test)]
const UNPINNED_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// What an installed file is for, which decides what the operator is told to
/// configure it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// A classifier that can run beside the primary, if it wants the same
    /// sample rate (`MODEL_2_PATH`, ...).
    Classifier,
    /// A classifier that replaces the primary (`MODEL_PATH`, `LABELS_PATH`). It
    /// cannot run beside one that wants a different sample rate: a recording is
    /// resampled once, for every classifier.
    PrimaryReplacement,
    /// A species-occurrence ("range") filter, for `METADATA_MODEL_PATH`.
    RangeFilter,
}

impl Use {
    /// The stable name the API reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Classifier => "classifier",
            Self::PrimaryReplacement => "primary_classifier",
            Self::RangeFilter => "range_filter",
        }
    }
}

/// One model file a station can install.
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
    /// Where its labels are downloaded from, when the catalogue ships them
    /// that way. Never set together with [`Self::labels_embedded`].
    pub labels_url: Option<&'static str>,
    /// The labels file itself, when it is compiled into the binary instead of
    /// downloaded. Written to disk by [`install`] and verified against
    /// [`Self::labels_sha256`] like a download, so a binary that carries the
    /// wrong bytes refuses to install them rather than naming the wrong birds.
    pub labels_embedded: Option<&'static str>,
    /// Lowercase hex sha256 of the labels file.
    pub labels_sha256: Option<&'static str>,
    /// The sample rate it needs — what `MODEL_n_SAMPLE_RATE` should be set to.
    ///
    /// Carried here because it is not derivable from the tensor: Perch v2's
    /// `[-1, 160_000]` is 5 s at 32 kHz and equally 3⅓ s at 48 kHz. See
    /// `G-10` Stage 4.
    pub sample_rate: u32,
    /// What it is for.
    pub usage: Use,
    /// Anything an operator needs to know before installing it.
    pub notes: &'static str,
}

impl CatalogEntry {
    /// Whether installing this entry also installs a labels file.
    #[must_use]
    pub const fn ships_labels(&self) -> bool {
        self.labels_url.is_some() || self.labels_embedded.is_some()
    }
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
        labels_embedded: None,
        labels_sha256: Some("8124b0ea2d187104c5e2cd95a0f937165647e20349c8fd34d4d5ef991821f8f0"),
        sample_rate: 32_000,
        usage: Use::Classifier,
        notes: "The station's default classifier. Reports a dynamic input shape, \
                which resolves to 4.5 s windows at 32 kHz.",
    },
    CatalogEntry {
        id: "perch-v2",
        name: "Google Perch v2 (14 795 classes)",
        model_url: "https://huggingface.co/justinchuby/Perch-onnx/resolve/\
                    da88bf8c37c0b3068fc706fe509f633b9fbd28c1/perch_v2.onnx",
        model_sha256: "bf0c8467a924cb074663970ca4a0ab1e143602121930209657d0dff5d5cefa1f",
        model_bytes: 409_148_616,
        // No labels file accompanies the ONNX conversion; an operator supplies
        // one. Saying so here is better than shipping a URL that 404s in the
        // field.
        labels_url: None,
        labels_embedded: None,
        labels_sha256: None,
        sample_rate: 32_000,
        usage: Use::Classifier,
        notes: "Materially better than BirdNET in the tropics. Wants 5 s windows \
                at 32 kHz where BirdNET wants 4.5 s; the two run together, the chunk \
                cut to the longer window and stepped by the shorter. Set \
                MODEL_n_SAMPLE_RATE=32000 — its tensor shape alone cannot say. No \
                labels file ships with the ONNX conversion; supply one with 14 795 \
                rows.",
    },
    CatalogEntry {
        id: "birdnet-v24",
        name: "BirdNET V2.4 (global, 6 522 species)",
        // birdnet-go's published ONNX build of the network BirdNET-Pi and
        // birdnet-go run, at a fixed revision of the repository rather than
        // `main`, so the bytes behind the URL cannot change under the digest.
        model_url: "https://huggingface.co/tphakala/BirdNET-v2.4/resolve/\
                    acd49768d227c726ac94550c94f0effb3e1dd04c/BirdNET_v2.4_fp32_dfttrunc.onnx",
        model_sha256: "3b72e88b3ad0c310a41adabccf8cf75b1a05daeeb40884ebd38038c91d0e423d",
        model_bytes: 54_068_648,
        // Not downloaded: the birdnet-go file this model is paired with names 460
        // of its 6 522 classes differently from BirdNET-Pi ("Common Magpie"
        // against "Eurasian Magpie"), which would split a species in two the
        // moment a BirdNET-Pi history is imported. The embedded file is that
        // file's class order with BirdNET-Pi's English names
        // (`scripts/build-v24-labels.py`, and a gate that holds the order to
        // birdnet-go's file line for line).
        labels_url: None,
        labels_embedded: Some(birdnet_core::inference::labels::BIRDNET_V24_ENGLISH_LABELS),
        labels_sha256: Some(birdnet_core::inference::labels::BIRDNET_V24_ENGLISH_LABELS_SHA256),
        sample_rate: 48_000,
        usage: Use::PrimaryReplacement,
        notes: "The classifier BirdNET-Pi and birdnet-go run. Its scores are \
                BirdNET-Pi's to within a few thousandths (it is the FP32 build; \
                BirdNET-Pi runs FP16 weights) once CONFIDENCE and SENSITIVITY are \
                set as BirdNET-Pi sets them, and its labels carry BirdNET-Pi's \
                English names. 48 kHz, 3 s windows, no calibration: its scores \
                are raw logits put through SENSITIVITY. Replaces the \
                BirdNET+ V3.0 classifier rather than running beside it, because the \
                two want different sample rates. Pair it with birdnet-v24-range-v2.",
    },
    CatalogEntry {
        id: "birdnet-v24-range-v2",
        name: "BirdNET V2.4 species-occurrence (range) filter, v2",
        // The same file birdnet-go ships for the V2.4 filter ("latest"),
        // converted from the MData V2 FP16 model BirdNET-Pi also ships. Fixed
        // to a commit of that repository for the same reason as above.
        model_url: "https://raw.githubusercontent.com/tphakala/birdnet-go/\
                    f4f019a3d79cb547b432bd8729fb4cf9c71aa2e7/internal/classifier/data/\
                    BirdNET_MData_V2.onnx",
        model_sha256: "b1810073d16c6c39df95a7419f9a11a02f98dbea562c96d4993c6af52b54b271",
        model_bytes: 29_524_170,
        labels_url: None,
        labels_embedded: None,
        labels_sha256: None,
        sample_rate: 0,
        usage: Use::RangeFilter,
        notes: "Takes latitude, longitude and the week and returns, for each V2.4 \
                species, how likely it is to occur there. Indexed like the V2.4 \
                labels, so it needs no labels of its own. Not the filter BirdNET-Pi \
                uses by default (its DATA_MODEL_VERSION=1 is the older v1, which \
                this catalogue does not carry): v2 lets about a quarter more species \
                through at the same SF_THRESH. Do not pair it with the V3.0 \
                classifier; that has its own geomodel.",
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
    /// The server sent more bytes than the file is pinned to. The download was
    /// stopped and deleted.
    TooLarge {
        /// The most the file may be.
        limit: u64,
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
            Self::TooLarge { limit } => write!(
                f,
                "the server sent more than the {} MiB this file is pinned to, so the \
                 download was stopped and deleted",
                limit / (1024 * 1024)
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
    /// The sample rate this classifier needs (0 for a range filter).
    pub sample_rate: u32,
    /// What it is for.
    pub usage: Use,
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
    // Delegated rather than re-spawned. This shelled out to `df -kP` and
    // parsed the second line itself until `every_child_process_has_a_deadline`
    // caught the `.output()`: a bare wait on `df` blocks for as long as a dead
    // network mount does, and this one runs immediately before a multi-hundred
    // megabyte download, so the station would have hung with nothing logged.
    //
    // The core helper already answers exactly this question behind
    // `run_with_timeout`, and it also passes `--`, which the copy here did not:
    // a model directory whose name begins with `-` was being read by `df` as a
    // flag.
    birdnet_core::audio::capture::disk_usage(dir)
        .ok()
        .map(|usage| usage.available_bytes)
}

/// How far along a download is, as [`install_with_progress`] reports it.
///
/// A browser page that offers to install a classifier needs something to show
/// for a minute or two of silence; this is that something. It carries no
/// percentage of its own, because "how far" depends on whether the server sent
/// a length ([`Self::total`] falls back to the catalogue's pinned size).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// The file being fetched, by name (`birdnet-v24.onnx`).
    pub file: String,
    /// Bytes received so far.
    pub done: u64,
    /// Bytes expected in all.
    pub total: u64,
}

/// The staging name for a download of `dest`: `<name>.<pid>.<n>.part`.
///
/// Unique to this process and this call, so two installs of the same file (the
/// command line while the browser's is running, or two browsers) cannot write
/// into one file and produce a stream digest that verified over a file with a
/// hole in it.
fn part_path(dest: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = dest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    dest.with_file_name(format!("{name}.{}.{n}.part", std::process::id()))
}

/// Remove staging files an earlier run left for `dest`: the old fixed
/// `<stem>.part`, and `<name>.<pid>.<n>.part` whose process is gone. One whose
/// process is alive may be an install in flight, and is left alone.
fn clear_stale_parts(dest: &Path) {
    let legacy = dest.with_extension("part");
    if legacy.exists() {
        let _ = fs::remove_file(&legacy);
    }
    let (Some(dir), Some(name)) = (dest.parent(), dest.file_name()) else {
        return;
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        let Some(rest) = file
            .strip_prefix(&prefix)
            .and_then(|r| r.strip_suffix(".part"))
        else {
            continue;
        };
        let Some(pid) = rest.split('.').next().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        let alive = pid == std::process::id() || Path::new(&format!("/proc/{pid}")).exists();
        if !alive {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Download one file, hashing as it streams, and rename it into place only
/// once the digest matches.
#[cfg(test)]
fn fetch_verified(
    client: &reqwest::blocking::Client,
    url: &str,
    expected: &'static str,
    dest: &Path,
) -> Result<(), InstallError> {
    fetch_verified_with(
        client,
        url,
        expected,
        dest,
        0,
        UNPINNED_MAX_BYTES,
        &mut |_| {},
    )
}

/// `fetch_verified`, reporting each megabyte received to `progress`.
///
/// `fallback_total` is used when the server sends no `Content-Length`.
/// `max_bytes` is the most the file may be (its pinned size): a server that
/// sends more is stopped at that many bytes, not allowed to stream for the
/// length of the timeout into the disk.
fn fetch_verified_with(
    client: &reqwest::blocking::Client,
    url: &str,
    expected: &'static str,
    dest: &Path,
    fallback_total: u64,
    max_bytes: u64,
    progress: &mut dyn FnMut(&Progress),
) -> Result<(), InstallError> {
    // A previous attempt that died mid-stream leaves staging files. They have
    // names nothing loads, so they are rubbish rather than a hazard.
    clear_stale_parts(dest);
    let part = part_path(dest);

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

    if resp.content_length().is_some_and(|len| len > max_bytes) {
        return Err(InstallError::TooLarge { limit: max_bytes });
    }

    // `create_new`: the name is unique to this call, so a file already there is
    // something else's and must not be written into.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let name = dest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let total = resp.content_length().unwrap_or(fallback_total);
    let mut done: u64 = 0;
    progress(&Progress {
        file: name.clone(),
        done,
        total,
    });
    // Everything from here until the rename removes the staging file on failure.
    let streamed = (|| -> Result<(), InstallError> {
        loop {
            let n = resp
                .read(&mut buf)
                .map_err(|e| InstallError::Network(format!("read failed: {e}")))?;
            if n == 0 {
                break;
            }
            if done + n as u64 > max_bytes {
                return Err(InstallError::TooLarge { limit: max_bytes });
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            done += n as u64;
            progress(&Progress {
                file: name.clone(),
                done,
                total,
            });
        }
        file.flush()?;
        // The rename below is only atomic with respect to a crash if the bytes are
        // actually on the device first.
        file.sync_all()?;
        // The digest covers the bytes that were streamed; the file must be exactly
        // those bytes.
        if file.metadata()?.len() != done {
            return Err(InstallError::Io(std::io::Error::other(
                "the staging file is not the length that was received",
            )));
        }
        Ok(())
    })();
    drop(file);
    if let Err(e) = streamed {
        let _ = fs::remove_file(&part);
        return Err(e);
    }

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

/// Lowercase hex SHA-256 of a file's contents, or `None` if it cannot be read.
fn sha256_of_file(path: &Path) -> Option<String> {
    use std::fmt::Write as _;
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let mut hex = String::with_capacity(64);
    for byte in &hasher.finalize() {
        let _ = write!(hex, "{byte:02x}");
    }
    Some(hex)
}

/// Whether `dest` already holds exactly the bytes `expected` names.
///
/// What lets `--install-model` be run again, by an installer on every start or
/// by an operator who is not sure it finished, without fetching a 541 MB file it
/// already has. The file is hashed, not trusted for being there: a truncated or
/// substituted file under the right name is fetched again.
fn already_installed(dest: &Path, expected: &str) -> bool {
    sha256_of_file(dest).is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
}

/// [`fetch_verified`], unless `dest` already verifies. `Ok(true)` when bytes
/// were fetched, `Ok(false)` when the file was already there and intact.
#[cfg(test)]
fn fetch_unless_installed(
    client: &reqwest::blocking::Client,
    url: &str,
    expected: &'static str,
    dest: &Path,
) -> Result<bool, InstallError> {
    fetch_unless_installed_with(client, url, expected, dest, UNPINNED_MAX_BYTES, &mut |_| {})
}

/// `fetch_unless_installed`, reporting progress when it does fetch.
fn fetch_unless_installed_with(
    client: &reqwest::blocking::Client,
    url: &str,
    expected: &'static str,
    dest: &Path,
    max_bytes: u64,
    progress: &mut dyn FnMut(&Progress),
) -> Result<bool, InstallError> {
    if already_installed(dest, expected) {
        return Ok(false);
    }
    fetch_verified_with(client, url, expected, dest, 0, max_bytes, progress)?;
    Ok(true)
}

/// Write bytes the binary carries to `dest`, only if they hash to `expected`.
///
/// The same discipline as `fetch_verified` for a file that needs no network:
/// the bytes land on a `.part` path nothing loads and are renamed into place
/// once they match, so a binary built with data that disagrees with the digest
/// pinned beside it fails here rather than naming the wrong birds.
fn write_verified(bytes: &[u8], expected: &'static str, dest: &Path) -> Result<(), InstallError> {
    use std::fmt::Write as _;
    let mut actual = String::with_capacity(64);
    for byte in &Sha256::digest(bytes) {
        let _ = write!(actual, "{byte:02x}");
    }
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(InstallError::Integrity { expected, actual });
    }
    clear_stale_parts(dest);
    let part = part_path(dest);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written {
        let _ = fs::remove_file(&part);
        return Err(e.into());
    }
    fs::rename(&part, dest)?;
    Ok(())
}

/// The name [`install`] gives an entry's labels file, or `None` when the entry
/// ships none.
fn labels_file_name(entry: &CatalogEntry) -> Option<String> {
    if let Some(url) = entry.labels_url {
        let ext = url
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4)
            .unwrap_or("csv");
        return Some(format!("{}_labels.{ext}", entry.id));
    }
    entry
        .labels_embedded
        .map(|_| format!("{}_labels.txt", entry.id))
}

/// Where [`install`] puts an entry's model and (if it ships one) its labels.
///
/// Pure: it names paths and touches nothing, so the setup page can say where a
/// classifier will land, and the daemon can look for it, using the one rule the
/// installer itself follows.
#[must_use]
pub fn installed_paths(entry: &CatalogEntry, dest_dir: &Path) -> (PathBuf, Option<PathBuf>) {
    (
        dest_dir.join(format!("{}.onnx", entry.id)),
        labels_file_name(entry).map(|n| dest_dir.join(n)),
    )
}

/// Whether an entry's files are in `dest_dir` and the model has the size the
/// catalogue pins.
///
/// A cheap look — it does **not** hash the 54 MB, which [`install`] does and a
/// status poll should not. It exists to tell "nothing there" and "a download
/// that stopped halfway" (which can never carry the final name, but a copy
/// made by hand can) from "installed".
#[must_use]
pub fn looks_installed(entry: &CatalogEntry, dest_dir: &Path) -> bool {
    let (model, labels) = installed_paths(entry, dest_dir);
    let model_ok = fs::metadata(&model).is_ok_and(|m| m.is_file() && m.len() == entry.model_bytes);
    let labels_ok = labels
        .as_ref()
        .is_none_or(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0));
    model_ok && labels_ok
}

/// Install a classifier from the catalogue into `dest_dir`.
///
/// # Errors
///
/// [`InstallError`] for an unknown id, a digest that cannot verify, a disk
/// that cannot hold the download, a network failure, or a digest mismatch.
/// Nothing is left under a name the station would load unless it verified.
pub fn install(id: &str, dest_dir: &Path) -> Result<Installed, InstallError> {
    install_with_progress(id, dest_dir, &mut |_| {})
}

/// [`install`], telling `progress` how the download is going.
///
/// The browser's setup page installs a classifier through this: it runs on a
/// worker thread and publishes each report for the page to poll. The callback
/// is called at least once per megabyte of the model, and not at all for a file
/// that already verifies (there is nothing to wait for).
///
/// # Errors
///
/// As [`install`].
pub fn install_with_progress(
    id: &str,
    dest_dir: &Path,
    progress: &mut dyn FnMut(&Progress),
) -> Result<Installed, InstallError> {
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

    let model_path = dest_dir.join(format!("{}.onnx", entry.id));
    // A file that already verifies needs neither the room nor the download.
    if !already_installed(&model_path, entry.model_sha256) {
        fits(entry.model_bytes, free_space_bytes(dest_dir))?;
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        // Long: 400 MB over a domestic uplink is measured in hours, and a
        // transfer killed at 90 % has cost the operator the whole thing.
        .timeout(std::time::Duration::from_secs(6 * 60 * 60))
        .build()
        .map_err(|e| InstallError::Network(e.to_string()))?;

    if already_installed(&model_path, entry.model_sha256) {
        tracing::info!(id = entry.id, path = %model_path.display(), "classifier already installed and verified; not downloading it again");
    } else {
        tracing::info!(
            id = entry.id,
            url = entry.model_url,
            "downloading classifier"
        );
        fetch_verified_with(
            &client,
            entry.model_url,
            entry.model_sha256,
            &model_path,
            entry.model_bytes,
            entry.model_bytes,
            progress,
        )?;
        tracing::info!(id = entry.id, path = %model_path.display(), "classifier verified and installed");
    }

    let labels_path = match (entry.labels_url, entry.labels_embedded, entry.labels_sha256) {
        (Some(url), None, Some(sha)) => {
            let path = dest_dir.join(labels_file_name(entry).unwrap_or_default());
            fetch_unless_installed_with(&client, url, sha, &path, LABELS_MAX_BYTES, progress)?;
            Some(path)
        }
        (None, Some(text), Some(sha)) => {
            let path = dest_dir.join(labels_file_name(entry).unwrap_or_default());
            write_verified(text.as_bytes(), sha, &path)?;
            Some(path)
        }
        _ => None,
    };

    Ok(Installed {
        id: entry.id,
        model_path,
        labels_path,
        sample_rate: entry.sample_rate,
        usage: entry.usage,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CATALOG, FREE_SPACE_MARGIN, InstallError, Use, fetch_verified, find, fits,
        free_space_bytes, install, require_digest,
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

    /// A labels source without its digest, or the reverse, would either
    /// install an unverified file or silently skip one an operator was
    /// promised; and a file that is both downloaded and embedded has no
    /// answer to "which one did I get".
    #[test]
    fn labels_are_either_fully_specified_or_absent() {
        for entry in CATALOG {
            assert_eq!(
                entry.ships_labels(),
                entry.labels_sha256.is_some(),
                "{}: a labels source and its digest must travel together",
                entry.id
            );
            assert!(
                entry.labels_url.is_none() || entry.labels_embedded.is_none(),
                "{}: labels are downloaded or embedded, not both",
                entry.id
            );
        }
    }

    /// The embedded labels are the bytes the pinned digest names, and
    /// installing them writes exactly those bytes — checked by installing into
    /// a scratch directory with no network at all, which is what an embedded
    /// file is for.
    #[test]
    fn embedded_labels_install_without_a_network_and_verify() {
        let entry = find("birdnet-v24").expect("in the catalogue");
        let text = entry.labels_embedded.expect("V2.4's labels are embedded");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("birdnet-v24_labels.txt");
        super::write_verified(text.as_bytes(), entry.labels_sha256.expect("digest"), &path)
            .expect("the embedded bytes match their own digest");
        assert_eq!(std::fs::read_to_string(&path).expect("read back"), text);
        // And the check has teeth: a different byte is refused and leaves
        // nothing under the name the station loads.
        let refused = dir.path().join("refused_labels.txt");
        let err = super::write_verified(
            b"Pica pica_Common Magpie\n",
            entry.labels_sha256.expect("digest"),
            &refused,
        )
        .expect_err("wrong bytes must not verify");
        assert!(matches!(err, InstallError::Integrity { .. }), "{err:?}");
        assert!(!refused.exists());
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
            // A classifier must say what rate it wants; a range filter hears
            // no audio and must say none, so a rate on one would be read as a
            // classifier's by anything that lists the catalogue.
            if entry.usage == Use::RangeFilter {
                assert_eq!(
                    entry.sample_rate, 0,
                    "{}: a filter hears no audio",
                    entry.id
                );
            } else {
                assert!(entry.sample_rate > 0, "{}", entry.id);
            }
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

    /// No staging file of any name is left in `dir`. (Staging names are unique
    /// per call now, so looking for one fixed name would pass for any leak.)
    fn assert_no_part_files(dir: &std::path::Path, after: &str) {
        let left: Vec<String> = std::fs::read_dir(dir)
            .expect("read dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                std::path::Path::new(n)
                    .extension()
                    .is_some_and(|e| e == "part")
            })
            .collect();
        assert!(
            left.is_empty(),
            "staging files left after {after}: {left:?}"
        );
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
        assert_no_part_files(dir.path(), "a mismatch");
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
        assert_no_part_files(dir.path(), "a verified install");
    }

    /// Installing again is cheap and safe: a file that already hashes to the
    /// pinned digest is not fetched (the URL here is unreachable, so a fetch
    /// would fail), and one that does not is fetched again rather than trusted
    /// for having the right name.
    #[test]
    fn a_file_that_already_verifies_is_not_downloaded_again_and_a_damaged_one_is() {
        const BODY: &[u8] = b"already on the disk";
        let expected: &'static str = {
            use sha2::{Digest as _, Sha256};
            use std::fmt::Write as _;
            let mut hex = String::new();
            for b in &Sha256::digest(BODY) {
                let _ = write!(hex, "{b:02x}");
            }
            Box::leak(hex.into_boxed_str())
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("model.onnx");
        std::fs::write(&dest, BODY).expect("seed");

        let fetched = super::fetch_unless_installed(
            &test_client(),
            "https://example.invalid/model.onnx",
            expected,
            &dest,
        )
        .expect("an intact file needs no network");
        assert!(!fetched, "an intact file must not be fetched again");
        assert_eq!(std::fs::read(&dest).expect("read"), BODY);

        // The counterpart: the same name holding other bytes is not trusted.
        std::fs::write(&dest, b"truncated").expect("damage");
        let err = super::fetch_unless_installed(
            &test_client(),
            "https://example.invalid/model.onnx",
            expected,
            &dest,
        )
        .expect_err("a damaged file must be fetched, and here the fetch fails");
        assert!(matches!(err, InstallError::Network(_)), "{err:?}");
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
        assert_no_part_files(dir.path(), "this case");
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

        let v24 = find("birdnet-v24").expect("birdnet-v2.4 is in the catalogue");
        assert_eq!(
            v24.model_sha256,
            "3b72e88b3ad0c310a41adabccf8cf75b1a05daeeb40884ebd38038c91d0e423d"
        );
        assert_eq!(v24.model_bytes, 54_068_648);
        assert_eq!(v24.sample_rate, 48_000);
        assert_eq!(v24.usage, Use::PrimaryReplacement);
        assert_eq!(
            v24.labels_sha256,
            Some("3773334c2ad218bde0ac083dd0b6e839a5cf34e84a6017830b9f446fbb57a215"),
            "BirdNET-Pi's English names on V2.4's class order, embedded"
        );
        assert!(v24.labels_url.is_none() && v24.labels_embedded.is_some());
        assert!(
            v24.model_url
                .contains("acd49768d227c726ac94550c94f0effb3e1dd04c"),
            "pinned to a revision, never `main`: {}",
            v24.model_url
        );

        let range =
            find("birdnet-v24-range-v2").expect("the V2.4 range filter is in the catalogue");
        assert_eq!(
            range.model_sha256,
            "b1810073d16c6c39df95a7419f9a11a02f98dbea562c96d4993c6af52b54b271"
        );
        assert_eq!(range.model_bytes, 29_524_170);
        assert_eq!(range.usage, Use::RangeFilter);
        assert!(range.labels_url.is_none());
        assert!(
            range
                .model_url
                .contains("f4f019a3d79cb547b432bd8729fb4cf9c71aa2e7"),
            "pinned to a commit, never a branch: {}",
            range.model_url
        );
    }

    /// No URL in the catalogue may name a moving ref. A digest pins the bytes,
    /// but a moving ref turns an upstream edit into an install that fails
    /// verification for every station at once, with no change here to point at.
    #[test]
    fn no_catalogue_url_follows_a_branch() {
        for e in CATALOG {
            for url in std::iter::once(e.model_url).chain(e.labels_url) {
                if url.contains("huggingface.co") {
                    assert!(!url.contains("/resolve/main/"), "{}: {url}", e.id);
                }
                if url.contains("raw.githubusercontent.com") {
                    let rev = url.split('/').nth(5).unwrap_or("");
                    assert!(
                        rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()),
                        "{}: not pinned to a commit: {url}",
                        e.id
                    );
                }
            }
        }
    }

    #[test]
    fn find_returns_nothing_for_an_id_that_is_not_there() {
        assert!(find("nope").is_none());
    }

    /// The browser's progress bar is driven by these reports: they begin at
    /// zero, never go backwards, and end at the size that arrived.
    #[test]
    fn a_download_reports_progress_from_zero_to_the_size_it_got() {
        const BODY: &[u8] = &[7u8; 3 * 1024 * 1024 + 123];
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("model.onnx");
        let url = serve_once(BODY);
        let digest: &'static str = Box::leak(
            {
                use sha2::{Digest as _, Sha256};
                use std::fmt::Write as _;
                let mut hex = String::new();
                for b in &Sha256::digest(BODY) {
                    let _ = write!(hex, "{b:02x}");
                }
                hex
            }
            .into_boxed_str(),
        );
        let mut seen: Vec<super::Progress> = Vec::new();
        super::fetch_verified_with(&test_client(), &url, digest, &dest, 0, u64::MAX, &mut |p| {
            seen.push(p.clone());
        })
        .expect("a matching digest installs");

        assert!(seen.len() >= 3, "a 3 MB body is at least three reports");
        assert_eq!(seen[0].done, 0, "the first report is the start");
        assert!(
            seen.windows(2).all(|w| w[0].done <= w[1].done),
            "progress never goes backwards: {seen:?}"
        );
        let last = seen.last().expect("reports");
        assert_eq!(last.done, BODY.len() as u64);
        assert_eq!(
            last.total,
            BODY.len() as u64,
            "total comes from Content-Length"
        );
        assert_eq!(last.file, "model.onnx");
    }

    /// Where the installer puts a file and where the station looks for it are
    /// the same function, so they cannot drift.
    #[test]
    fn the_paths_the_installer_writes_are_the_paths_reported() {
        let dir = std::path::Path::new("/m");
        let v24 = find("birdnet-v24").expect("entry");
        let (model, labels) = super::installed_paths(v24, dir);
        assert_eq!(model, std::path::Path::new("/m/birdnet-v24.onnx"));
        assert_eq!(
            labels.as_deref(),
            Some(std::path::Path::new("/m/birdnet-v24_labels.txt"))
        );
        let range = find("birdnet-v24-range-v2").expect("entry");
        assert_eq!(super::installed_paths(range, dir).1, None);
        let v3 = find("birdnet-v3").expect("entry");
        assert_eq!(
            super::installed_paths(v3, dir).1.as_deref(),
            Some(std::path::Path::new("/m/birdnet-v3_labels.csv")),
            "a downloaded labels file keeps its extension"
        );
    }

    /// `looks_installed` is a size check on the model and a presence check on
    /// the labels, and says no to a file that is one byte short.
    #[test]
    fn looks_installed_wants_the_pinned_size_and_the_labels() {
        let dir = tempfile::tempdir().expect("tempdir");
        let range = find("birdnet-v24-range-v2").expect("entry");
        let (model, _) = super::installed_paths(range, dir.path());
        assert!(!super::looks_installed(range, dir.path()), "nothing there");
        let f = std::fs::File::create(&model).expect("create");
        f.set_len(range.model_bytes - 1).expect("len");
        assert!(!super::looks_installed(range, dir.path()), "one byte short");
        f.set_len(range.model_bytes).expect("len");
        assert!(super::looks_installed(range, dir.path()));

        let v24 = find("birdnet-v24").expect("entry");
        let (model, labels) = super::installed_paths(v24, dir.path());
        std::fs::File::create(&model)
            .and_then(|f| f.set_len(v24.model_bytes))
            .expect("model");
        assert!(
            !super::looks_installed(v24, dir.path()),
            "a model without its labels is not installed"
        );
        std::fs::write(labels.expect("labels"), "x\n").expect("labels");
        assert!(super::looks_installed(v24, dir.path()));
    }

    /// A server that never stops. `chunks` of 64 KiB are written after the
    /// header; the count of bytes it managed to write is returned on join.
    /// `claimed_length` puts a `Content-Length` in the header (a lie).
    fn serve_endless(
        claimed_length: Option<u64>,
        chunks: usize,
    ) -> (String, std::thread::JoinHandle<u64>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let handle = std::thread::spawn(move || {
            let mut written = 0_u64;
            if let Ok((mut sock, _)) = listener.accept() {
                use std::io::{Read as _, Write as _};
                let mut scratch = [0u8; 1024];
                let _ = sock.read(&mut scratch);
                let header = claimed_length.map_or_else(
                    || "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_owned(),
                    |n| {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"
                        )
                    },
                );
                let _ = sock.write_all(header.as_bytes());
                let block = vec![7u8; 64 * 1024];
                for _ in 0..chunks {
                    if sock.write_all(&block).is_err() {
                        break;
                    }
                    written += block.len() as u64;
                }
            }
            written
        });
        (format!("http://127.0.0.1:{port}/model.onnx"), handle)
    }

    const ANY_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    /// A hostile or compromised origin cannot make the station stream for the
    /// whole timeout: the download stops at the pinned size, and nothing is left.
    #[test]
    fn a_server_that_streams_past_the_pinned_size_is_stopped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("model.onnx");
        // No Content-Length: only the byte count can stop it. 3200 x 64 KiB is
        // 200 MiB on offer against a 2 MiB pin.
        let (url, server) = serve_endless(None, 3200);
        let err = super::fetch_verified_with(
            &test_client(),
            &url,
            ANY_DIGEST,
            &dest,
            0,
            2 * 1024 * 1024,
            &mut |_| {},
        )
        .expect_err("more than the pinned size must be refused");
        assert!(
            matches!(err, InstallError::TooLarge { limit } if limit == 2 * 1024 * 1024),
            "{err:?}"
        );
        assert!(!dest.exists());
        assert_no_part_files(dir.path(), "an oversize stream");
        let offered = server.join().expect("server thread");
        assert!(
            offered < 64 * 1024 * 1024,
            "the client kept reading: the server wrote {offered} of 200 MiB"
        );
    }

    /// A `Content-Length` over the pin is refused before a byte is stored.
    #[test]
    fn a_length_over_the_pinned_size_is_refused_before_any_file_is_made() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("model.onnx");
        let (url, server) = serve_endless(Some(10 * 1024 * 1024 * 1024), 4);
        let err = super::fetch_verified_with(
            &test_client(),
            &url,
            ANY_DIGEST,
            &dest,
            0,
            1024 * 1024,
            &mut |_| {},
        )
        .expect_err("a 10 GiB claim against a 1 MiB pin");
        assert!(matches!(err, InstallError::TooLarge { .. }), "{err:?}");
        assert_no_part_files(dir.path(), "an oversize Content-Length");
        let _ = server.join();
    }

    /// The counterpart: a file exactly the pinned size installs. Only *more*
    /// than the pin is refused.
    #[test]
    fn a_file_of_exactly_the_pinned_size_installs() {
        const BODY: &[u8] = b"exactly this many bytes, pinned";
        let digest: &'static str = Box::leak(
            {
                use sha2::Digest as _;
                use std::fmt::Write as _;
                let mut hex = String::new();
                for b in &sha2::Sha256::digest(BODY) {
                    let _ = write!(hex, "{b:02x}");
                }
                hex
            }
            .into_boxed_str(),
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("model.onnx");
        let url = serve_once(BODY);
        super::fetch_verified_with(
            &test_client(),
            &url,
            digest,
            &dest,
            0,
            BODY.len() as u64,
            &mut |_| {},
        )
        .expect("the pinned size is allowed");
        assert_eq!(std::fs::read(&dest).expect("read"), BODY);
        assert_no_part_files(dir.path(), "an install");
    }

    /// Two installs of one file never share a staging file.
    #[test]
    fn two_downloads_of_one_file_stage_under_different_names() {
        let dest = std::path::Path::new("/m/birdnet-v24.onnx");
        let (a, b) = (super::part_path(dest), super::part_path(dest));
        assert_ne!(a, b);
        for p in [&a, &b] {
            let name = p.file_name().expect("name").to_string_lossy();
            assert!(name.starts_with("birdnet-v24.onnx."), "{name}");
            assert!(name.contains(&std::process::id().to_string()), "{name}");
            assert!(name.ends_with(".part"), "{name}");
        }
        // Opening a name that is taken fails instead of writing into it.
        let dir = tempfile::tempdir().expect("tempdir");
        let taken = dir.path().join("x.part");
        std::fs::write(&taken, b"another install's bytes").expect("write");
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&taken);
        assert!(opened.is_err());
    }

    /// Cleaning up after a dead install leaves a live one alone.
    #[test]
    fn a_dead_installs_staging_file_is_cleared_and_a_live_ones_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("m.onnx");
        // 4_000_000_000 is above any pid_max, so no such process exists.
        let dead = dir.path().join("m.onnx.4000000000.0.part");
        let live = dir
            .path()
            .join(format!("m.onnx.{}.9.part", std::process::id()));
        let legacy = dir.path().join("m.part");
        let other = dir.path().join("other.onnx.4000000000.0.part");
        for f in [&dead, &live, &legacy, &other] {
            std::fs::write(f, b"x").expect("write");
        }
        super::clear_stale_parts(&dest);
        assert!(!dead.exists(), "a dead process's staging file");
        assert!(!legacy.exists(), "the old fixed name");
        assert!(live.exists(), "this process's may be in flight");
        assert!(other.exists(), "another file's staging is not ours");
    }
}
