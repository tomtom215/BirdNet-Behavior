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
    if let Some(name) = path.file_name().map(|n| n.to_string_lossy())
        && locked_file_names.iter().any(|l| l == name.as_ref())
    {
        return true;
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
    } else if !all_files.is_empty() {
        // We had files to purge but couldn't remove any — typically every
        // oldest-bucket file is protected (exclude_paths / locked_file_names),
        // or remove_file failed for all of them (permission errors). Without
        // this signal `check_and_purge` would warn "disk full" on every
        // interval indefinitely with no clue why. Matches BirdNET-Pi's
        // disk_check.sh secondary-check escalation pattern.
        tracing::warn!(
            candidates = all_files.len(),
            "disk purge made no progress — all candidate files are protected \
             (exclude_paths / locked_file_names) or could not be removed; \
             recording may keep filling the disk"
        );
    }

    removed
}

/// What an extracted clip's name says about it.
///
/// The extractor names clips `Common_Name-ConfPct-YYYY-MM-DD-birdnet-[RTSP_ID-]HH:MM:SS.ext`,
/// with a `-n` suffix when the name was already taken. That is the only
/// record of species and confidence the disk manager can reach: it runs in
/// the capture crate, with no database, and the purge has to keep the
/// station's rarest birds from going first (S-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ClipName<'a> {
    /// The common name as the extractor wrote it (`Great_Tit`).
    pub species: &'a str,
    /// The confidence, in whole percent.
    pub confidence_pct: u8,
}

/// Parse the extractor's clip naming; `None` for anything else (a raw
/// segment, a BirdNET-Pi import with another shape, a stray file).
pub(super) fn parse_clip_name(name: &str) -> Option<ClipName<'_>> {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let (head, _) = stem.split_once("-birdnet-")?;
    // `head` is `Common_Name-ConfPct-YYYY-MM-DD`; the name may itself contain
    // dashes (`Black-capped_Chickadee`), so take the fixed parts from the right.
    let date_start = head.len().checked_sub(10)?;
    let date = &head[date_start..];
    let date_ok = date.bytes().enumerate().all(|(i, b)| {
        if i == 4 || i == 7 {
            b == b'-'
        } else {
            b.is_ascii_digit()
        }
    });
    if !date_ok || !head[..date_start].ends_with('-') {
        return None;
    }
    let (species, conf) = head[..date_start - 1].rsplit_once('-')?;
    let confidence_pct: u8 = conf.parse().ok().filter(|c| *c <= 100)?;
    if species.is_empty() {
        return None;
    }
    Some(ClipName {
        species,
        confidence_pct,
    })
}

/// One species' clips, in the order the purge takes them.
struct SpeciesClips {
    /// The clips: lowest confidence first, then oldest first.
    files: Vec<(PathBuf, u8, std::time::SystemTime)>,
    /// How many of them still exist (protected ones included: a locked clip is
    /// still a clip of this species).
    count: usize,
    /// The next file to try.
    next: usize,
    /// Whether the per-species floor applies: it does not for files whose
    /// name is not an extracted clip's, which have no species to protect.
    floored: bool,
}

/// Purge extracted clips from `base_dir` to free space, keeping every species
/// (S-2).
///
/// Walks the clips flat in `base_dir` and, for a migrated BirdNET-Pi tree,
/// under `base_dir/By_Date/`. Removes 10 % of them (minimum 1), chosen so that
/// what remains stays as diverse as it was:
///
/// - the species with the most clips gives up a clip first, and keeps giving
///   until another has more;
/// - within a species, the lowest-confidence clip goes first, then the oldest;
/// - no species is ever taken below `floor` clips. With `floor` 0 nothing is
///   protected; between species with equal counts the lower confidence, then
///   the older clip, goes first.
///
/// Files under `exclude_paths` or named in `locked_file_names` are never
/// removed, and still count towards their species. Files whose name is not an
/// extracted clip's are ordered oldest-first and have no floor.
///
/// Returns the number of files removed.
pub(super) fn purge_recordings(
    base_dir: &Path,
    floor: u32,
    exclude_paths: &[PathBuf],
    locked_file_names: &[String],
) -> u32 {
    let mut all_files: Vec<(PathBuf, std::time::SystemTime)> = collect_flat_audio(base_dir)
        .into_iter()
        .map(|(p, m, _)| (p, m))
        .collect();
    let by_date_dir = base_dir.join("By_Date");
    if by_date_dir.is_dir() {
        collect_audio_files_recursive(&by_date_dir, &mut all_files);
    }
    if all_files.is_empty() {
        return 0;
    }
    let quota = (all_files.len() / 10).max(1);

    let mut groups: std::collections::BTreeMap<String, SpeciesClips> =
        std::collections::BTreeMap::new();
    for (path, modified) in all_files {
        let parsed = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(parse_clip_name)
            .map(|c| (c.species.to_owned(), c.confidence_pct));
        let (key, conf, floored) = match parsed {
            Some((species, conf)) => (species, conf, true),
            None => (String::new(), 0, false),
        };
        let group = groups.entry(key).or_insert_with(|| SpeciesClips {
            files: Vec::new(),
            count: 0,
            next: 0,
            floored,
        });
        group.files.push((path, conf, modified));
        group.count += 1;
    }
    for group in groups.values_mut() {
        group
            .files
            .sort_by_key(|(_, conf, modified)| (*conf, *modified));
    }

    let floor = usize::try_from(floor).unwrap_or(usize::MAX);
    let mut removed = 0_u32;
    let mut skipped_protected = 0_usize;
    let mut skipped_floor = 0_usize;
    while (removed as usize) < quota {
        // The species to take from: most clips first; among equals, the one
        // whose next clip is least confident, then oldest. A species at its
        // floor, or with nothing left to try, is out.
        let pick = groups
            .iter()
            .filter(|(_, g)| g.next < g.files.len() && (!g.floored || g.count > floor))
            .min_by_key(|(name, g)| {
                let (_, conf, modified) = &g.files[g.next];
                (
                    std::cmp::Reverse(g.count),
                    *conf,
                    *modified,
                    (*name).clone(),
                )
            })
            .map(|(name, _)| name.clone());
        let Some(name) = pick else { break };
        let group = groups.get_mut(&name).expect("picked from the map");
        let (path, _, _) = &group.files[group.next];
        group.next += 1;
        if is_protected(path, exclude_paths, locked_file_names) {
            skipped_protected += 1;
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            tracing::debug!(path = %path.display(), species = %name, "purged clip");
            removed += 1;
            group.count -= 1;
        }
    }
    for g in groups.values() {
        if g.floored && g.count <= floor {
            skipped_floor += g.count;
        }
    }

    if removed > 0 {
        tracing::info!(
            count = removed,
            floor,
            "purged clips, most-recorded species first, none below the floor"
        );
    } else {
        // Nothing could go: every species is at its floor, or everything
        // reachable is protected. Say which, or `check_and_purge` warns "disk
        // full" every minute with no clue why.
        tracing::warn!(
            floor,
            protected_by_floor = skipped_floor,
            protected_or_failed = skipped_protected,
            "disk purge made no progress: every species is at its floor of clips, or \
             every candidate is protected (exclude_paths / locked) or could not be \
             removed; the space is not in expendable recordings"
        );
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

/// Collect flat (top-level, non-recursive) audio files in `dir` with their
/// modification time and size.
///
/// Used for transient raw-capture segment cleanup: those segments sit *flat* in
/// the watch/stream dir, unlike the recursive `By_Date/` tree that
/// [`purge_oldest_files`] walks. Keeping this non-recursive is deliberate — it
/// must never descend into (and delete from) a persistent recordings tree.
fn collect_flat_audio(dir: &Path) -> Vec<(PathBuf, std::time::SystemTime, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_audio_file(&path) {
            let meta = entry.metadata().ok();
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            let size = meta.as_ref().map_or(0, std::fs::Metadata::len);
            out.push((path, modified, size));
        }
    }
    out
}

/// Purge the oldest *flat* (top-level) audio files in `dir` — the oldest 10 %
/// (minimum 1). The flat-dir analogue of [`purge_oldest_files`], for the
/// transient raw-capture segments that never live under `By_Date/`.
///
/// Without this the disk-full safety net reclaims nothing on the RAM-backed
/// stream dir (its segments are flat, so the `By_Date/` walk finds nothing) and
/// the tmpfs runs to 100 %. Skips protected / locked files. Returns the count.
pub(super) fn purge_oldest_flat_files(
    dir: &Path,
    exclude_paths: &[PathBuf],
    locked_file_names: &[String],
) -> u32 {
    let mut files = collect_flat_audio(dir);
    if files.is_empty() {
        return 0;
    }
    files.sort_by_key(|(_, modified, _)| *modified);
    let to_remove = (files.len() / 10).max(1);
    let mut removed = 0_u32;
    for (path, _, _) in &files {
        if removed >= u32::try_from(to_remove).unwrap_or(u32::MAX) {
            break;
        }
        if is_protected(path, exclude_paths, locked_file_names) {
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(count = removed, dir = %dir.display(), "purged oldest raw stream segments");
    }
    removed
}

/// Remove flat (top-level) audio files in `dir` older than `max_age`.
///
/// Drains processed raw-capture segments so the transient stream dir self-empties
/// and cannot fill its (RAM-backed) filesystem. `max_age` is chosen far larger
/// than the detection pipeline's processing latency, so an in-flight segment
/// (still being decoded / extracted) is never deleted. Skips protected / locked
/// files. Returns the count removed.
pub(super) fn purge_flat_older_than(
    dir: &Path,
    max_age: std::time::Duration,
    exclude_paths: &[PathBuf],
    locked_file_names: &[String],
) -> u32 {
    let now = std::time::SystemTime::now();
    let mut removed = 0_u32;
    for (path, modified, _) in collect_flat_audio(dir) {
        let old_enough = now.duration_since(modified).is_ok_and(|age| age > max_age);
        if !old_enough || is_protected(&path, exclude_paths, locked_file_names) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(count = removed, dir = %dir.display(), "drained aged raw stream segments");
    }
    removed
}

/// Keep the flat (top-level) audio in `dir` under `max_bytes` by deleting the
/// oldest segments first.
///
/// A hard ceiling on the transient stream dir for the high-ingest case (many
/// RTSP streams) or a backed-up pipeline, independent of the filesystem-percent
/// backstop. Skips protected / locked files. Returns the count removed.
pub(super) fn purge_flat_over_size(
    dir: &Path,
    max_bytes: u64,
    exclude_paths: &[PathBuf],
    locked_file_names: &[String],
) -> u32 {
    let mut files = collect_flat_audio(dir);
    let mut total: u64 = files.iter().map(|(_, _, size)| *size).sum();
    if total <= max_bytes {
        return 0;
    }
    files.sort_by_key(|(_, modified, _)| *modified); // oldest first
    let mut removed = 0_u32;
    for (path, _, size) in &files {
        if total <= max_bytes {
            break;
        }
        if is_protected(path, exclude_paths, locked_file_names) {
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            total = total.saturating_sub(*size);
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(count = removed, dir = %dir.display(), "capped raw stream segment size");
    }
    removed
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
    fn purge_returns_zero_when_all_candidates_are_protected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let species_dir = dir.path().join("By_Date/2026-03-14/Test_Bird");
        std::fs::create_dir_all(&species_dir).expect("create dirs");

        // Create 3 files (10%=1 candidate, +1 minimum=1 to_remove), all locked.
        let mut names = Vec::new();
        for i in 0..3_u32 {
            let name = format!("clip_{i:02}.wav");
            let wav_path = species_dir.join(&name);
            std::fs::write(&wav_path, create_minimal_wav_header()).expect("write wav");
            let mtime = filetime::FileTime::from_unix_time(1_000_000 + i64::from(i), 0);
            filetime::set_file_mtime(&wav_path, mtime).expect("set mtime");
            names.push(name);
        }

        let removed = purge_oldest_files(dir.path(), &[], &names);
        assert_eq!(
            removed, 0,
            "all candidates are locked; nothing should be removed"
        );
        // Every locked file still present (the loop iterated past them all
        // looking for unprotected files, but found none).
        for name in &names {
            assert!(species_dir.join(name).exists(), "{name} should still exist");
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

    #[test]
    fn a_clip_name_yields_its_species_and_confidence() {
        let c = parse_clip_name("Great_Tit-95-2026-03-14-birdnet-10:00:00.wav").unwrap();
        assert_eq!((c.species, c.confidence_pct), ("Great_Tit", 95));
        // An RTSP source, a collision suffix, a dash in the name, another
        // extension: all the extractor's own spellings.
        let c =
            parse_clip_name("Black-capped_Chickadee-71-2026-03-14-birdnet-RTSP_2-10:00:00-3.mp3")
                .unwrap();
        assert_eq!(
            (c.species, c.confidence_pct),
            ("Black-capped_Chickadee", 71)
        );
        // Not clips: a raw segment, a bare name, a date in the wrong place.
        assert_eq!(parse_clip_name("2026-03-14-birdnet-10:00:00.wav"), None);
        assert_eq!(parse_clip_name("clip_01.wav"), None);
        assert_eq!(parse_clip_name("Great_Tit-95-birdnet-10:00:00.wav"), None);
        assert_eq!(parse_clip_name("-95-2026-03-14-birdnet-10:00:00.wav"), None);
        assert_eq!(
            parse_clip_name("Great_Tit-101-2026-03-14-birdnet-10:00:00.wav"),
            None
        );
    }

    /// A clip named as the extractor names them, with a fixed mtime.
    fn write_clip(dir: &Path, species: &str, conf: u8, n: u32, mtime_secs: i64) -> PathBuf {
        let path = dir.join(format!(
            "{species}-{conf}-2026-03-14-birdnet-{:02}:{:02}:00.wav",
            n / 60,
            n % 60
        ));
        write_sized_wav(&path, 64, mtime_secs);
        path
    }

    /// The gate for S-2: the register's exact case. Ninety-seven clips of the
    /// commonest bird and one each of three rare ones, the rare clips older
    /// and less confident than every common one — the old purge, oldest 10 %
    /// first, took all three rare clips in its first pass.
    #[test]
    fn the_purge_takes_from_the_commonest_species_and_never_below_the_floor() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Corncrake is the oldest; Hoopoe the least confident.
        let rare: Vec<PathBuf> = [("Corncrake", 40), ("Wryneck", 40), ("Hoopoe", 30)]
            .iter()
            .enumerate()
            .map(|(i, (sp, conf))| {
                let i = u32::try_from(i).unwrap();
                write_clip(dir.path(), sp, *conf, i, 1_000 + i64::from(i))
            })
            .collect();
        let mut common: Vec<(PathBuf, u8)> = Vec::new();
        for i in 0..97_u32 {
            // Confidence climbs with time, so "lowest confidence" and "oldest"
            // agree except where the test breaks the tie on purpose below.
            let conf = 60 + (i % 40) as u8;
            common.push((
                write_clip(dir.path(), "Great_Tit", conf, 10 + i, 10_000 + i64::from(i)),
                conf,
            ));
        }

        let removed = purge_recordings(dir.path(), 5, &[], &[]);
        assert_eq!(removed, 10, "a tenth of 100");
        for r in &rare {
            assert!(
                r.exists(),
                "{} must survive: its species is at the floor",
                r.display()
            );
        }
        let gone: Vec<u8> = common
            .iter()
            .filter(|(p, _)| !p.exists())
            .map(|(_, c)| *c)
            .collect();
        assert_eq!(gone.len(), 10);
        assert!(
            gone.iter()
                .all(|c| *c == 60 || *c == 61 || *c == 62 || *c == 63 || *c == 64),
            "the least confident common clips go first, not the oldest: {gone:?}"
        );
        // Counterpart: the floor really is a floor, not a preference. With the
        // common species purged down to it, the next pass finds nothing to take
        // and says so, rather than reaching for the rare birds.
        for (p, _) in &common {
            let _ = std::fs::remove_file(p);
        }
        for i in 0..5 {
            write_clip(dir.path(), "Great_Tit", 90, 200 + i, 20_000 + i64::from(i));
        }
        assert_eq!(
            purge_recordings(dir.path(), 5, &[], &[]),
            0,
            "every species is at the floor"
        );
        for r in &rare {
            assert!(r.exists());
        }
        // And with no floor, the purge frees space again — still from the
        // commonest species, which at five clips is still the commonest.
        assert_eq!(purge_recordings(dir.path(), 0, &[], &[]), 1);
        assert!(
            rare.iter().all(|r| r.exists()),
            "the Great Tit clip went, not a rare one"
        );
        // Among species with equal counts the least confident clip goes, and
        // age only breaks a tie in confidence.
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            if entry.file_name().to_string_lossy().starts_with("Great_Tit") {
                std::fs::remove_file(entry.path()).unwrap();
            }
        }
        assert_eq!(purge_recordings(dir.path(), 0, &[], &[]), 1);
        assert!(
            !rare[2].exists(),
            "Hoopoe: least confident, though the newest"
        );
        assert_eq!(purge_recordings(dir.path(), 0, &[], &[]), 1);
        assert!(
            !rare[0].exists(),
            "Corncrake: same confidence as Wryneck, and older"
        );
        assert!(rare[1].exists());
    }

    #[test]
    fn a_locked_clip_is_skipped_and_still_counts_for_its_species() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Six clips of one species, floor 5: exactly one may go. The least
        // confident is locked, so the next-least goes instead — and the locked
        // one still counts, so a second pass is refused.
        let locked = write_clip(dir.path(), "Robin", 50, 0, 1_000);
        let next = write_clip(dir.path(), "Robin", 55, 1, 1_001);
        for i in 2..6 {
            write_clip(dir.path(), "Robin", 80, i, 1_000 + i64::from(i));
        }
        let locked_names = vec![locked.file_name().unwrap().to_string_lossy().into_owned()];
        assert_eq!(purge_recordings(dir.path(), 5, &[], &locked_names), 1);
        assert!(locked.exists());
        assert!(!next.exists());
        assert_eq!(purge_recordings(dir.path(), 5, &[], &locked_names), 0);
    }

    #[test]
    fn a_migrated_by_date_tree_is_purged_with_the_same_policy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("By_Date/2026-03-14/Great_Tit");
        std::fs::create_dir_all(&tree).expect("mkdir");
        for i in 0..20 {
            write_clip(&tree, "Great_Tit", 90, i, 1_000 + i64::from(i));
        }
        let rare = write_clip(dir.path(), "Wryneck", 40, 99, 10);
        assert_eq!(purge_recordings(dir.path(), 5, &[], &[]), 2);
        assert!(rare.exists());
    }

    /// Write `n` bytes of `.wav` at `path` with a fixed mtime (unix seconds).
    fn write_sized_wav(path: &Path, bytes: usize, mtime_secs: i64) {
        std::fs::write(path, vec![0_u8; bytes]).expect("write wav");
        filetime::set_file_mtime(path, filetime::FileTime::from_unix_time(mtime_secs, 0))
            .expect("set mtime");
    }

    #[test]
    fn purge_oldest_flat_files_removes_ten_percent() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..20 {
            write_sized_wav(
                &dir.path().join(format!("seg_{i:02}.wav")),
                64,
                1_000_000 + i,
            );
        }
        // 10% of 20 = 2, oldest first.
        assert_eq!(purge_oldest_flat_files(dir.path(), &[], &[]), 2);
        assert!(!dir.path().join("seg_00.wav").exists());
        assert!(!dir.path().join("seg_01.wav").exists());
        assert!(dir.path().join("seg_02.wav").exists());
    }

    #[test]
    fn purge_flat_older_than_drains_only_aged_segments() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Three ancient (1970) segments, two fresh (now).
        for i in 0..3 {
            write_sized_wav(&dir.path().join(format!("old_{i}.wav")), 64, 1_000_000 + i);
        }
        for i in 0..2 {
            // Natural mtime = now; well under the max_age below.
            std::fs::write(dir.path().join(format!("new_{i}.wav")), vec![0_u8; 64]).expect("write");
        }
        let removed =
            purge_flat_older_than(dir.path(), std::time::Duration::from_secs(3600), &[], &[]);
        assert_eq!(removed, 3, "only the three aged segments should drain");
        assert!(dir.path().join("new_0.wav").exists());
        assert!(dir.path().join("new_1.wav").exists());
    }

    #[test]
    fn purge_flat_over_size_caps_oldest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        // 5 × 1000 B = 5000 B total; cap at 2500 → drop oldest until ≤ 2500.
        for i in 0..5 {
            write_sized_wav(
                &dir.path().join(format!("seg_{i}.wav")),
                1000,
                1_000_000 + i,
            );
        }
        let removed = purge_flat_over_size(dir.path(), 2500, &[], &[]);
        assert_eq!(removed, 3); // 5000 → 4000 → 3000 → 2000 (≤ 2500)
        assert!(!dir.path().join("seg_0.wav").exists()); // oldest gone
        assert!(dir.path().join("seg_3.wav").exists()); // newest kept
        assert!(dir.path().join("seg_4.wav").exists());
    }

    #[test]
    fn purge_flat_over_size_noop_when_under_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_sized_wav(&dir.path().join("seg.wav"), 100, 1_000_000);
        assert_eq!(purge_flat_over_size(dir.path(), 10_000, &[], &[]), 0);
        assert!(dir.path().join("seg.wav").exists());
    }

    #[test]
    fn flat_helpers_skip_locked_and_ignore_nested_by_date() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A persistent By_Date clip nested below the flat dir — flat helpers must
        // NOT descend into it (that tree belongs to purge_oldest_files).
        let nested = dir.path().join("By_Date/2026-03-14/Test_Bird");
        std::fs::create_dir_all(&nested).expect("mkdir");
        write_sized_wav(&nested.join("keep.wav"), 1000, 1_000_000);
        // Flat aged segments, one locked.
        write_sized_wav(&dir.path().join("locked.wav"), 1000, 1_000_001);
        write_sized_wav(&dir.path().join("drop.wav"), 1000, 1_000_002);

        let locked = vec!["locked.wav".to_string()];
        let removed =
            purge_flat_older_than(dir.path(), std::time::Duration::from_secs(1), &[], &locked);
        assert_eq!(removed, 1, "only the unlocked flat segment drains");
        assert!(dir.path().join("locked.wav").exists(), "locked file kept");
        assert!(
            nested.join("keep.wav").exists(),
            "nested By_Date clip untouched"
        );
        assert!(!dir.path().join("drop.wav").exists());
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
