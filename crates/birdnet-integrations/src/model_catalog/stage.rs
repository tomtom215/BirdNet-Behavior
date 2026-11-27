//! Putting bytes under an installed name only once they verify: from the
//! binary itself, from a local file, or from an earlier installer's copy.
//! (Downloads stage the same way, in `download.rs`.)

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use super::InstallError;
use super::record::hex;

/// Bytes copied at a time.
const CHUNK: usize = 1024 * 1024;

/// The staging name for a copy of `dest`: `<name>.<pid>.<n>.part`.
///
/// Unique to this process and this call, so two installs of the same file
/// cannot write into one staging file.
pub(super) fn part_path(dest: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = dest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    dest.with_file_name(format!("{name}.{}.{n}.part", std::process::id()))
}

/// Remove staging files an earlier run left for `dest`: the old fixed
/// `<stem>.part`, and `<name>.<pid>.<n>.part` whose process is gone. One whose
/// process is alive may be an install in flight, and is left alone. (A
/// download's `<name>.download` is not rubbish: it is what the next attempt
/// resumes from.)
pub(super) fn clear_stale_parts(dest: &Path) {
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

/// Write bytes the binary carries to `dest`, only if they hash to `expected`.
///
/// The bytes land on a `.part` path nothing loads and are renamed into place
/// once they match, so a binary built with data that disagrees with the digest
/// pinned beside it fails here rather than naming the wrong birds.
pub(super) fn write_verified(
    bytes: &[u8],
    expected: &'static str,
    dest: &Path,
) -> Result<(), InstallError> {
    let actual = hex(&Sha256::digest(bytes));
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
    // Synced, renamed, and the directory synced: whole and durable (F21).
    birdnet_core::atomic_file::commit(&part, dest)?;
    Ok(())
}

/// Refuse `src` without reading it when it is not `bytes` long.
pub(super) fn require_size(src: &Path, bytes: u64) -> Result<(), InstallError> {
    let actual = fs::metadata(src)?.len();
    if actual == bytes {
        Ok(())
    } else {
        Err(InstallError::WrongSize {
            expected: bytes,
            actual,
        })
    }
}

/// Copy `src` to `dest` through a staging file, keeping it only if it is the
/// pinned size and has the pinned digest.
pub(super) fn copy_verified(
    src: &Path,
    expected: &'static str,
    bytes: u64,
    dest: &Path,
) -> Result<(), InstallError> {
    require_size(src, bytes)?;
    let mut from = fs::File::open(src)?;
    clear_stale_parts(dest);
    let part = part_path(dest);
    let mut to = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let copied = (|| -> std::io::Result<()> {
        loop {
            let n = from.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            to.write_all(&buf[..n])?;
        }
        to.flush()?;
        to.sync_all()
    })();
    drop(to);
    if let Err(e) = copied {
        let _ = fs::remove_file(&part);
        return Err(e.into());
    }
    let actual = hex(&hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(&part);
        return Err(InstallError::NotThePinnedFile { expected, actual });
    }
    // Synced, renamed, and the directory synced: whole and durable (F21).
    birdnet_core::atomic_file::commit(&part, dest)?;
    Ok(())
}

/// Give `legacy`, a file an earlier installer wrote, the installed name
/// `dest` — when it is the pinned file. `Ok(false)` when there is no such
/// file or it is not the pinned one (it is then left exactly as it was).
///
/// The file is hard-linked under the new name, not moved and not copied: no
/// disk is spent on a second 541 MB copy, and the old name keeps working until
/// whatever configuration names it has been pointed at the new one (the
/// installer removes it then). A filesystem without hard links gets a verified
/// copy instead.
pub(super) fn adopt(
    legacy: &Path,
    dest: &Path,
    expected: &'static str,
    bytes: u64,
) -> Result<bool, InstallError> {
    if !fs::metadata(legacy).is_ok_and(|m| m.is_file() && m.len() == bytes) {
        return Ok(false);
    }
    clear_stale_parts(dest);
    let part = part_path(dest);
    if fs::hard_link(legacy, &part).is_err() {
        return match copy_verified(legacy, expected, bytes, dest) {
            Ok(()) => Ok(true),
            Err(InstallError::NotThePinnedFile { .. }) => Ok(false),
            Err(e) => Err(e),
        };
    }
    // Hashed through the new link, which is the file that will be installed.
    let verified = super::record::sha256_of_file(&part)
        .is_some_and(|actual| actual.eq_ignore_ascii_case(expected));
    if !verified {
        let _ = fs::remove_file(&part);
        return Ok(false);
    }
    // Synced, renamed, and the directory synced: whole and durable (F21).
    birdnet_core::atomic_file::commit(&part, dest)?;
    Ok(true)
}
