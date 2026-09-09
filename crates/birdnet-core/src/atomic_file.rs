//! Files that are either whole or absent: write to a sibling `.part` name,
//! sync, rename, sync the directory (PS-7, S-4).
//!
//! A clip written straight to its final name is, for the duration of the
//! write, a file with the right name and the wrong contents. A power cut in
//! that window — the field station's ordinary way of stopping — leaves a
//! truncated WAV the database row points at for ever, and nothing can tell it
//! from a short recording. `rename(2)` is atomic on the same filesystem, so
//! a reader sees either no file or the whole one; the two `fsync`s are what
//! make "the whole one" survive the cut: the data before the rename, the
//! directory entry after it. A rename with no sync buys atomicity of the
//! name and not of the bytes, which is the half that matters.

use std::path::{Path, PathBuf};

/// The suffix a file carries while it is being written.
pub const PART_SUFFIX: &str = ".part";

/// The in-progress name for `final_path`: `name.part.ext` when the file has
/// an extension, `name.part` when it does not.
///
/// The extension stays last so a tool that infers the format from it
/// (`ffmpeg`, `sox`) still does; the marker sits before it so nothing that
/// lists `*.wav` or serves recordings by name picks the file up half-written.
#[must_use]
pub fn part_path(final_path: &Path) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part_name = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}{PART_SUFFIX}.{ext}"),
        _ => format!("{name}{PART_SUFFIX}"),
    };
    final_path.with_file_name(part_name)
}

/// Whether `path` is a file some writer has not finished.
#[must_use]
pub fn is_part_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(PART_SUFFIX) || n.contains(&format!("{PART_SUFFIX}.")))
}

/// Make `part` durable, then give it `final_path`'s name, then make the
/// name durable.
///
/// # Errors
///
/// Any of the three steps failing. On error `part` is left where it was, so
/// the caller can remove it; `final_path` is untouched unless the rename
/// itself succeeded, in which case the file is whole and only the directory
/// sync failed — which is reported, and which a later sync of the directory
/// by anything else also settles.
pub fn commit(part: &Path, final_path: &Path) -> std::io::Result<()> {
    std::fs::File::open(part)?.sync_all()?;
    std::fs::rename(part, final_path)?;
    sync_dir(final_path.parent().unwrap_or_else(|| Path::new(".")))
}

/// `fsync` a directory, so a rename inside it is on disk.
///
/// Opening a directory for reading and syncing it is how POSIX exposes this;
/// a platform that refuses is reported rather than papered over.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_name_keeps_the_extension_last() {
        assert_eq!(
            part_path(Path::new("/r/Pica_pica-91-2026-05-19-birdnet-09:00:00.wav")),
            PathBuf::from("/r/Pica_pica-91-2026-05-19-birdnet-09:00:00.part.wav")
        );
        assert_eq!(
            part_path(Path::new("/r/clip.mp3")),
            PathBuf::from("/r/clip.part.mp3")
        );
        assert_eq!(
            part_path(Path::new("/r/noext")),
            PathBuf::from("/r/noext.part")
        );
        assert_eq!(
            part_path(Path::new("/r/.hidden")),
            PathBuf::from("/r/.hidden.part")
        );
        assert!(is_part_path(Path::new("/r/clip.part.wav")));
        assert!(is_part_path(Path::new("/r/noext.part")));
        assert!(!is_part_path(Path::new("/r/clip.wav")));
        assert!(!is_part_path(Path::new("/r/partial.wav")));
    }

    #[test]
    fn commit_moves_the_bytes_under_the_final_name_and_removes_the_part() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("clip.wav");
        let part = part_path(&final_path);
        std::fs::write(&part, b"whole").unwrap();
        commit(&part, &final_path).unwrap();
        assert_eq!(std::fs::read(&final_path).unwrap(), b"whole");
        assert!(!part.exists());
    }

    #[test]
    fn commit_of_a_missing_part_leaves_the_final_name_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("clip.wav");
        std::fs::write(&final_path, b"previous").unwrap();
        let err = commit(&part_path(&final_path), &final_path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(std::fs::read(&final_path).unwrap(), b"previous");
    }
}
