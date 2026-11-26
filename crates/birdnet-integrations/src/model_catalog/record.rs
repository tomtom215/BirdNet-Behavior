//! Whether an installed file is the pinned file, and the record that lets a
//! poll answer that without reading 541 MB (MDL-11).
//!
//! Beside each installed file `<name>` the installer writes `<name>.verified`:
//! the sha256 the file hashed to, and the size, modification time (to the
//! nanosecond) and inode it had when it was hashed. The record is believed only
//! while all three still match the file; a file replaced, rewritten, truncated
//! or touched since is hashed again. What it does not catch — bytes damaged in
//! place without the metadata changing — is what the full re-hash at the
//! station's start and on every `--install-model` is for.
//!
//! The record caches the measurement, not a verdict: it holds the digest that
//! was found, and a file is verified when that digest is the pin. So a file
//! known to be damaged stays known as damaged without being hashed on every
//! poll either.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

/// Bytes hashed at a time.
const CHUNK: usize = 1024 * 1024;

/// What one installed file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// No file at the installed name.
    Missing,
    /// Its bytes hash to the pin, as of the last time they were read.
    Verified,
    /// It is not the pinned file: the wrong size, or bytes that hash to
    /// something else.
    Damaged,
    /// There, the right size, and not hashed since it last changed; asked not
    /// to hash ([`Rehash::Never`]), so nobody can yet say.
    Unchecked,
}

/// Whether [`super::installed_state`] may read a file to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rehash {
    /// Only the record: a status poll. A file the record cannot vouch for is
    /// [`FileState::Unchecked`].
    Never,
    /// Hash a file the record cannot vouch for, and record the answer.
    IfChanged,
    /// Hash every file, whatever its record says, and record the answer.
    Always,
}

/// An entry's files, each as [`FileState`] says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallState {
    /// The model file.
    pub model: FileState,
    /// Its labels file, when the entry ships one.
    pub labels: Option<FileState>,
}

impl InstallState {
    /// Every file the entry installs is there and verified.
    #[must_use]
    pub fn is_verified(&self) -> bool {
        self.model == FileState::Verified && self.labels.is_none_or(|l| l == FileState::Verified)
    }

    /// Every file is there, and none is known not to be the pinned one: all
    /// that a status poll, which reads only the records ([`Rehash::Never`]),
    /// can say. A file nobody has hashed since it last changed passes; the
    /// station re-hashes every installed file at start, after which a damaged
    /// one fails this too.
    #[must_use]
    pub fn is_present(&self) -> bool {
        let usable = |s: FileState| matches!(s, FileState::Verified | FileState::Unchecked);
        usable(self.model) && self.labels.is_none_or(usable)
    }

    /// Some file is there and known not to be the pinned one.
    #[must_use]
    pub fn is_damaged(&self) -> bool {
        self.model == FileState::Damaged || self.labels == Some(FileState::Damaged)
    }
}

/// The record's path for `file`.
pub(super) fn record_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".verified");
    file.with_file_name(name)
}

/// What identifies one version of a file on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    mtime_ns: i128,
    inode: u64,
}

impl Stamp {
    fn of(meta: &fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self {
            size: meta.len(),
            mtime_ns: i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
            inode: meta.ino(),
        }
    }
}

/// The state of `file` against `sha256` and `bytes`; see [`Rehash`].
pub(super) fn check(file: &Path, sha256: &str, bytes: u64, rehash: Rehash) -> FileState {
    let Ok(meta) = fs::metadata(file) else {
        return FileState::Missing;
    };
    if !meta.is_file() {
        return FileState::Missing;
    }
    if meta.len() != bytes {
        return FileState::Damaged;
    }
    let stamp = Stamp::of(&meta);
    if rehash != Rehash::Always
        && let Some(found) = read(file, stamp)
    {
        return verdict(&found, sha256);
    }
    if rehash == Rehash::Never {
        return FileState::Unchecked;
    }
    let Some(found) = sha256_of_file(file) else {
        return FileState::Missing;
    };
    // Recorded only when the file is the one that was hashed: a file replaced
    // while it was being read must not inherit the old one's digest.
    if fs::metadata(file).is_ok_and(|m| Stamp::of(&m) == stamp) {
        store(file, stamp, &found);
    }
    verdict(&found, sha256)
}

/// Record that `file`, as it is now, hashes to `sha256`. Called by the
/// installer right after it put verified bytes there. A record that cannot be
/// written only costs a re-hash later, so the failure is logged, not returned.
pub(super) fn write(file: &Path, sha256: &str) {
    match fs::metadata(file) {
        Ok(meta) => store(file, Stamp::of(&meta), sha256),
        Err(e) => {
            tracing::debug!(file = %file.display(), error = %e, "no verification record written");
        }
    }
}

const fn verdict(found: &str, pinned: &str) -> FileState {
    if found.eq_ignore_ascii_case(pinned) {
        FileState::Verified
    } else {
        FileState::Damaged
    }
}

/// The digest the record holds, when it describes `stamp`.
fn read(file: &Path, stamp: Stamp) -> Option<String> {
    let text = fs::read_to_string(record_path(file)).ok()?;
    let mut sha = None;
    let mut seen = Stamp {
        size: u64::MAX,
        mtime_ns: i128::MIN,
        inode: u64::MAX,
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "sha256" => sha = Some(value.to_owned()),
            "size" => seen.size = value.parse().ok()?,
            "mtime_ns" => seen.mtime_ns = value.parse().ok()?,
            "inode" => seen.inode = value.parse().ok()?,
            _ => {}
        }
    }
    (seen == stamp).then_some(sha?)
}

fn store(file: &Path, stamp: Stamp, sha256: &str) {
    let path = record_path(file);
    let text = format!(
        "# What birdnet-behavior measured when it last read this file. Believed only\n\
         # while the size, modification time and inode below are still the file's.\n\
         sha256={sha256}\nsize={}\nmtime_ns={}\ninode={}\n",
        stamp.size, stamp.mtime_ns, stamp.inode
    );
    let staging = path.with_extension(format!("verified.{}", std::process::id()));
    let written = fs::write(&staging, text).and_then(|()| fs::rename(&staging, &path));
    if let Err(e) = written {
        let _ = fs::remove_file(&staging);
        tracing::debug!(file = %path.display(), error = %e, "verification record not written");
    }
}

/// Lowercase hex SHA-256 of a file's contents, or `None` if it cannot be read.
pub(super) fn sha256_of_file(path: &Path) -> Option<String> {
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
    Some(hex(&hasher.finalize()))
}

/// Lowercase hex of a digest.
pub(super) fn hex(digest: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
