//! Internal helpers for disk purge operations.
//!
//! Contains file protection checks, recursive file collection,
//! oldest-file purging, and empty directory cleanup.

use std::path::{Path, PathBuf};

use crate::audio::capture::process::is_audio_file;

/// Check whether a file is protected from purge.
///
/// A file is protected if its path starts with any of `exclude_paths`,
/// or its filename appears in `locked_file_names`.
pub(super) fn is_protected(
    path: &Path,
    exclude_paths: &[PathBuf],
    locked_file_names: &[String],
) -> bool {
    for prefix in exclude_paths {
        if path.starts_with(prefix) {
            return true;
        }
    }
    if let Some(name) = path.file_name().map(|n| n.to_string_lossy()) {
        if locked_file_names.iter().any(|l| l == name.as_ref()) {
            return true;
        }
    }
    false
}

/// Purge the oldest audio files under `base_dir/By_Date/` to free space.
///
/// Collects all audio files, sorts by modification time, and deletes the
/// oldest 10% (minimum 1 file). Files under `exclude_paths` or with names
/// in `locked_file_names` are skipped.
///
/// Returns the number of files removed.
pub(super) fn purge_oldest_files(
    base_dir: &Path,
    exclude_paths: &[PathBuf],
    locked_file_names: &[String],
) -> u32 {
    let by_date_dir = base_dir.join("By_Date");
    if !by_date_dir.is_dir() {
        return 0;
    }

    let mut all_files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    collect_audio_files_recursive(&by_date_dir, &mut all_files);

    if all_files.is_empty() {
        return 0;
    }

    // Sort by modification time, oldest first.
    all_files.sort_by_key(|(_, modified)| *modified);

    // Delete oldest 10% (minimum 1), skipping protected files.
    let to_remove = (all_files.len() / 10).max(1);
    let mut removed = 0_u32;

    for (path, _) in &all_files {
        if removed >= u32::try_from(to_remove).unwrap_or(u32::MAX) {
            break;
        }
        if is_protected(path, exclude_paths, locked_file_names) {
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            tracing::debug!(path = %path.display(), "purged old file");
            removed += 1;
        }
    }

    if removed > 0 {
        tracing::info!(count = removed, "purged oldest audio files");
    }

    removed
}

/// Recursively collect audio files and their modification times.
pub(super) fn collect_audio_files_recursive(
    dir: &Path,
    out: &mut Vec<(PathBuf, std::time::SystemTime)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_audio_files_recursive(&path, out);
        } else if path.is_file() && is_audio_file(&path) {
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            out.push((path, modified));
        }
    }
}

/// Remove empty directories under `base_dir` (depth-first).
pub(super) fn cleanup_empty_dirs(base_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            cleanup_empty_dirs(&path);
            // Try to remove; will fail if non-empty, which is fine.
            if std::fs::remove_dir(&path).is_ok() {
                tracing::debug!(path = %path.display(), "removed empty directory");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purge_oldest_files_removes_oldest() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create By_Date directory with 20 files.
        let species_dir = dir.path().join("By_Date/2026-03-14/Test_Bird");
        std::fs::create_dir_all(&species_dir).expect("create dirs");

        for i in 0..20 {
            let wav_path = species_dir.join(format!("clip_{i:02}.wav"));
            let header = create_minimal_wav_header();
            std::fs::write(&wav_path, &header).expect("write wav");
            let mtime = filetime::FileTime::from_unix_time(1_000_000 + i64::from(i), 0);
            filetime::set_file_mtime(&wav_path, mtime).expect("set mtime");
        }

        let removed = purge_oldest_files(dir.path(), &[], &[]);
        // 10% of 20 = 2
        assert_eq!(removed, 2);
    }

    #[test]
    fn cleanup_empty_dirs_removes_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).expect("create dirs");

        cleanup_empty_dirs(dir.path());

        // All empty nested dirs should be gone.
        assert!(!dir.path().join("a").exists());
    }

    #[test]
    fn purge_skips_locked_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let species_dir = dir.path().join("By_Date/2026-03-14/Test_Bird");
        std::fs::create_dir_all(&species_dir).expect("create dirs");

        // Create 20 files; lock the 5 oldest.
        for i in 0..20_u32 {
            let wav_path = species_dir.join(format!("clip_{i:02}.wav"));
            let header = create_minimal_wav_header();
            std::fs::write(&wav_path, &header).expect("write wav");
            let mtime = filetime::FileTime::from_unix_time(1_000_000 + i64::from(i), 0);
            filetime::set_file_mtime(&wav_path, mtime).expect("set mtime");
        }

        // Lock the 5 oldest (clip_00 through clip_04).
        let locked: Vec<String> = (0..5_u32).map(|i| format!("clip_{i:02}.wav")).collect();

        let removed = purge_oldest_files(dir.path(), &[], &locked);
        // 10% of 20 = 2, but oldest 2 are locked so it should skip them
        // and remove the next 2 unlocked.
        assert_eq!(removed, 2);

        // Locked files must still exist.
        for name in &locked {
            assert!(species_dir.join(name).exists(), "{name} should be locked");
        }
    }

    #[test]
    fn is_protected_by_exclude_path() {
        let exclude = vec![PathBuf::from("/protected")];
        assert!(is_protected(
            Path::new("/protected/subdir/file.wav"),
            &exclude,
            &[]
        ));
        assert!(!is_protected(
            Path::new("/other/subdir/file.wav"),
            &exclude,
            &[]
        ));
    }

    #[test]
    fn is_protected_by_locked_name() {
        let locked = vec!["important.wav".to_string()];
        assert!(is_protected(
            Path::new("/any/dir/important.wav"),
            &[],
            &locked
        ));
        assert!(!is_protected(Path::new("/any/dir/other.wav"), &[], &locked));
    }

    /// Create a minimal valid WAV file (44-byte header, no data).
    fn create_minimal_wav_header() -> Vec<u8> {
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&36_u32.to_le_bytes()); // file size - 8
        header.extend_from_slice(b"WAVE");
        header.extend_from_slice(b"fmt ");
        header.extend_from_slice(&16_u32.to_le_bytes()); // fmt chunk size
        header.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        header.extend_from_slice(&1_u16.to_le_bytes()); // mono
        header.extend_from_slice(&48000_u32.to_le_bytes()); // sample rate
        header.extend_from_slice(&96000_u32.to_le_bytes()); // byte rate
        header.extend_from_slice(&2_u16.to_le_bytes()); // block align
        header.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
        header.extend_from_slice(b"data");
        header.extend_from_slice(&0_u32.to_le_bytes()); // data size
        header
    }
}
