//! What a full backup holds, written into it, and checked on restore
//! (RES-29, N26-01).
//!
//! A full backup used to be the database, the recordings folder and a
//! `birdnet.conf` only when one sat beside the database, which on a standard
//! install it never does. It now also carries:
//!
//! * the configuration file the station actually reads (`--config`), as
//!   `birdnet.conf`, which a restore puts back where the station reads it;
//! * `clip-manifest.tsv`: one line per detection that names a clip — its
//!   permanent identifier, the clip, the SHA-256 and length recorded when the
//!   clip was written (empty where none was: unknown), whether the clip is in
//!   this archive, and what the station found (`present`, `missing`, `pruned`);
//! * `clips.sha256`: every clip in the archive that has a recorded checksum, in
//!   the format `sha256sum -c` reads, so the evidence can be checked on any
//!   machine with the archive unpacked;
//! * `backup-manifest.json`: what was included and the counts above.
//!
//! The configuration holds the station's secrets in plain text, as the
//! database in the same archive holds its accounts and saved credentials:
//! the archive is to be kept like a password file, as the manual says.
//!
//! Audio is the bulk of a backup. `audio=all` (the default, as before) takes
//! every clip; `audio=kept` takes only the clips the station keeps back from
//! automatic deletion (locked, reviewed or commented on), the evidence a
//! second reviewer needs; `audio=none` takes none. The manifest lists every
//! clip either way, so a study can tell what exists and what was left out.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The configuration file's name inside the archive.
pub const CONFIG_MEMBER: &str = "birdnet.conf";
/// The per-detection clip manifest's name inside the archive.
pub const CLIP_MANIFEST: &str = "clip-manifest.tsv";
/// The `sha256sum -c` list's name inside the archive.
pub const CHECKSUMS: &str = "clips.sha256";
/// The summary's name inside the archive.
pub const SUMMARY: &str = "backup-manifest.json";

/// Members a restore reads and does not place in the data directory.
pub const MANIFEST_MEMBERS: [&str; 3] = [CLIP_MANIFEST, CHECKSUMS, SUMMARY];

/// Which clips a full backup takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioScope {
    /// Every clip in the recordings folder (the default).
    All,
    /// Only the clips the station keeps back from automatic deletion.
    Kept,
    /// No audio; the manifest still lists every clip.
    None,
}

impl AudioScope {
    /// Parse the `audio` query parameter; `None` for a value it does not know.
    #[must_use]
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim) {
            None | Some("" | "all") => Some(Self::All),
            Some("kept") => Some(Self::Kept),
            Some("none") => Some(Self::None),
            Some(_) => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Kept => "kept",
            Self::None => "none",
        }
    }
}

/// What [`write_manifests`] counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestCounts {
    /// Detections that name a clip.
    pub rows: u64,
    /// Of those, rows with a recorded checksum.
    pub with_checksum: u64,
    /// Distinct clips placed in the archive.
    pub included: u64,
    /// Rows whose clip the recordings folder does not have and retention did
    /// not record removing.
    pub missing: u64,
    /// Rows whose clip retention removed.
    pub pruned: u64,
}

/// Write the three manifest files into `staging`, from the database snapshot
/// at `snapshot`, and return the clip names (bare, relative to the
/// recordings folder) the archive is to include for `scope`.
///
/// `recordings_name` is the recordings folder's name inside the archive.
///
/// # Errors
///
/// A failed read of the snapshot or a failed write: the backup is then
/// refused, rather than shipped with a manifest that leaves rows out.
pub fn write_manifests(
    snapshot: &Path,
    recordings_dir: &Path,
    recordings_name: &str,
    staging: &Path,
    scope: AudioScope,
    config_station_path: Option<&Path>,
) -> Result<(Vec<String>, ManifestCounts), String> {
    let conn = birdnet_db::sqlite::open_readonly(snapshot)
        .map_err(|e| format!("could not open the snapshot to list its clips: {e}"))?;
    let kept: BTreeSet<String> = match scope {
        AudioScope::Kept => birdnet_db::sqlite::kept_back_file_names(&conn, true)
            .map_err(|e| format!("could not read which clips are kept: {e}"))?
            .into_iter()
            .filter_map(|n| base_name(&n))
            .collect(),
        AudioScope::All | AudioScope::None => BTreeSet::new(),
    };

    let mut tsv = std::io::BufWriter::new(
        std::fs::File::create(staging.join(CLIP_MANIFEST))
            .map_err(|e| format!("could not write {CLIP_MANIFEST}: {e}"))?,
    );
    let mut sums = std::io::BufWriter::new(
        std::fs::File::create(staging.join(CHECKSUMS))
            .map_err(|e| format!("could not write {CHECKSUMS}: {e}"))?,
    );
    let io = |e: std::io::Error| format!("could not write the clip manifest: {e}");
    tsv.write_all(b"detection_uuid\tfile_name\tclip_sha256\tclip_bytes\tin_archive\tstate\n")
        .map_err(io)?;

    let mut counts = ManifestCounts::default();
    let mut included: BTreeSet<String> = BTreeSet::new();
    let mut summed: BTreeSet<String> = BTreeSet::new();
    let mut line = String::new();
    let mut failure: Option<String> = None;
    birdnet_db::sqlite::for_each_clip_record(&conn, |record| {
        counts.rows += 1;
        let Some(name) = record.file_name.as_deref().and_then(base_name) else {
            return Ok(());
        };
        let state = if record.pruned_at.is_some() {
            counts.pruned += 1;
            "pruned"
        } else {
            match recordings_dir.join(&name).try_exists() {
                Ok(true) => "present",
                Ok(false) => {
                    counts.missing += 1;
                    "missing"
                }
                Err(_) => "unreadable",
            }
        };
        let in_archive = state == "present"
            && match scope {
                AudioScope::All => true,
                AudioScope::Kept => kept.contains(&name),
                AudioScope::None => false,
            };
        if in_archive && included.insert(name.clone()) {
            counts.included += 1;
        }
        if let Some(d) = &record.recorded {
            counts.with_checksum += 1;
            if in_archive && summed.insert(name.clone()) {
                let entry = format!("{}  {recordings_name}/{name}\n", d.sha256);
                if let Err(e) = sums.write_all(entry.as_bytes()) {
                    failure.get_or_insert_with(|| io(e));
                }
            }
        }
        line.clear();
        let _ = writeln!(
            line,
            "{}\t{}\t{}\t{}\t{}\t{state}",
            record.detection_uuid,
            tsv_cell(&name),
            record.recorded.as_ref().map_or("", |d| d.sha256.as_str()),
            record
                .recorded
                .as_ref()
                .map_or_else(String::new, |d| d.bytes.to_string()),
            if in_archive { "yes" } else { "no" },
        );
        if let Err(e) = tsv.write_all(line.as_bytes()) {
            failure.get_or_insert_with(|| io(e));
        }
        Ok(())
    })
    .map_err(|e| format!("could not list the snapshot's clips: {e}"))?;
    if let Some(e) = failure {
        return Err(e);
    }
    tsv.flush().map_err(io)?;
    sums.flush().map_err(io)?;
    drop(conn);

    let summary = serde_json::json!({
        "format": 1,
        "database": snapshot.file_name().map(|n| n.to_string_lossy().into_owned()),
        "config": {
            "included": config_station_path.is_some(),
            "station_path": config_station_path.map(|p| p.display().to_string()),
        },
        "audio": scope.as_str(),
        "recordings": recordings_name,
        "clips": {
            "rows": counts.rows,
            "rows_with_checksum": counts.with_checksum,
            "included": counts.included,
            "missing": counts.missing,
            "pruned": counts.pruned,
        },
        "checksums": "clips.sha256 lists every clip in this archive whose SHA-256 was recorded \
                      when it was written; rows with an empty clip_sha256 in clip-manifest.tsv \
                      have none (unknown, not computed later)",
    });
    let summary = serde_json::to_vec_pretty(&summary)
        .map_err(|e| format!("could not write {SUMMARY}: {e}"))?;
    std::fs::write(staging.join(SUMMARY), summary)
        .map_err(|e| format!("could not write {SUMMARY}: {e}"))?;
    Ok((included.into_iter().collect(), counts))
}

/// A clip's bare name, as the recordings folder holds it.
fn base_name(name: &str) -> Option<String> {
    Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
}

/// A clip name cannot hold a tab or newline that the TSV would split on;
/// any that does is escaped rather than breaking the line.
fn tsv_cell(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

/// Check every clip `clips.sha256` lists, in the unpacked archive at
/// `staging`, against its recorded checksum.
///
/// Returns how many were checked; `Ok(0)` when the archive has no list (one
/// made before backups carried it).
///
/// # Errors
///
/// The first clip that is absent from the archive or is not the clip the
/// list names, or a list that cannot be read: the restore is refused before
/// anything is replaced.
pub fn verify_checksums(staging: &Path) -> Result<usize, String> {
    let list = staging.join(CHECKSUMS);
    let text = match std::fs::read_to_string(&list) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("could not read the archive's {CHECKSUMS}: {e}")),
    };
    let mut checked = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let (sha, rel) = line
            .split_once("  ")
            .ok_or_else(|| format!("unreadable line in the archive's {CHECKSUMS}: {line}"))?;
        let rel_path = Path::new(rel);
        if rel_path.is_absolute()
            || rel_path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "the archive's {CHECKSUMS} names a path outside it: {rel}"
            ));
        }
        let path: PathBuf = staging.join(rel_path);
        match birdnet_db::sqlite::digest_file(&path) {
            Ok(d) if d.sha256 == sha => checked += 1,
            Ok(_) => {
                return Err(format!(
                    "{rel} in the archive is not the clip the backup recorded (its SHA-256 \
                     differs); the archive is damaged, and nothing was replaced"
                ));
            }
            Err(e) => {
                return Err(format!(
                    "{rel} is listed in the archive's {CHECKSUMS} but could not be read from it \
                     ({e}); the archive is incomplete, and nothing was replaced"
                ));
            }
        }
    }
    Ok(checked)
}

/// Put the archive's configuration file at `target`, where the station reads
/// it, keeping the file there now as `<target>.before-restore`.
///
/// Written beside `target` and renamed over it, so the station never reads a
/// half-written file; the permissions of the file it replaces are kept, or
/// owner-only when there was none.
///
/// # Errors
///
/// Why it could not be placed.
pub fn place_config(staged: &Path, target: &Path) -> Result<Option<PathBuf>, String> {
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut keep_name = target.as_os_str().to_os_string();
    keep_name.push(".before-restore");
    let kept = PathBuf::from(keep_name);
    let previous = match std::fs::metadata(target) {
        Ok(m) => Some(m.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("could not read {}: {e}", target.display())),
    };
    if previous.is_some() {
        std::fs::copy(target, &kept).map_err(|e| {
            format!(
                "could not keep the current configuration as {}: {e}",
                kept.display()
            )
        })?;
    }
    let mut tmp = tempfile::Builder::new()
        .prefix(".birdnet.conf.restore-")
        .tempfile_in(dir)
        .map_err(|e| format!("could not write beside {}: {e}", target.display()))?;
    let bytes = std::fs::read(staged)
        .map_err(|e| format!("could not read the archive's configuration: {e}"))?;
    tmp.write_all(&bytes)
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(|e| format!("could not write beside {}: {e}", target.display()))?;
    let had_one = previous.is_some();
    let perms = previous.unwrap_or_else(|| {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::Permissions::from_mode(0o600)
    });
    std::fs::set_permissions(tmp.path(), perms)
        .map_err(|e| format!("could not set the restored configuration's permissions: {e}"))?;
    tmp.persist(target)
        .map_err(|e| format!("could not place {}: {}", target.display(), e.error))?;
    Ok(had_one.then_some(kept))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(dir: &Path) -> PathBuf {
        let db = dir.join("birds.db");
        let conn = birdnet_db::sqlite::open_or_create(&db).unwrap();
        let rec = dir.join("recordings");
        std::fs::create_dir_all(&rec).unwrap();
        for (i, (file, locked)) in [("a.wav", 1), ("b.wav", 0), ("gone.wav", 0)]
            .iter()
            .enumerate()
        {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name,
                                         is_locked)
                 VALUES ('2026-05-01', printf('06:00:%02d', ?1), 'Strix aluco', 'Tawny Owl', 0.9,
                         ?2, ?3)",
                rusqlite::params![i64::try_from(i).unwrap(), file, locked],
            )
            .unwrap();
            if *file != "gone.wav" {
                std::fs::write(rec.join(file), file.as_bytes()).unwrap();
                let d = birdnet_db::sqlite::digest_file(&rec.join(file)).unwrap();
                birdnet_db::sqlite::record_clip_digest(&conn, conn.last_insert_rowid(), &d)
                    .unwrap();
            }
        }
        // A clip written before checksums were recorded.
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-01', '07:00:00', 'Strix aluco', 'Tawny Owl', 0.9, 'old.wav')",
            [],
        )
        .unwrap();
        std::fs::write(rec.join("old.wav"), b"old").unwrap();
        db
    }

    /// Every clip row reaches the manifest, with what is known and what is
    /// not; `clips.sha256` lists only the clips in the archive that have a
    /// recorded checksum; `audio=kept` takes only the kept-back clip.
    #[test]
    fn the_manifest_lists_every_clip_and_the_checksums_of_those_included() {
        let dir = tempfile::tempdir().unwrap();
        let db = station(dir.path());
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let (included, counts) = write_manifests(
            &db,
            &dir.path().join("recordings"),
            "recordings",
            &staging,
            AudioScope::All,
            None,
        )
        .unwrap();
        assert_eq!(included, ["a.wav", "b.wav", "old.wav"]);
        assert_eq!(
            counts,
            ManifestCounts {
                rows: 4,
                with_checksum: 2,
                included: 3,
                missing: 1,
                pruned: 0
            }
        );
        let tsv = std::fs::read_to_string(staging.join(CLIP_MANIFEST)).unwrap();
        assert_eq!(tsv.lines().count(), 5, "{tsv}");
        assert!(
            tsv.lines()
                .any(|l| l.contains("\tgone.wav\t") && l.ends_with("\tno\tmissing")),
            "{tsv}"
        );
        assert!(
            tsv.lines()
                .any(|l| l.contains("\told.wav\t\t\tyes\tpresent")),
            "a clip with no recorded checksum is listed with it empty: {tsv}"
        );
        let sums = std::fs::read_to_string(staging.join(CHECKSUMS)).unwrap();
        assert_eq!(sums.lines().count(), 2, "{sums}");
        assert!(sums.lines().all(|l| l.contains("  recordings/")));

        let staging2 = dir.path().join("staging2");
        std::fs::create_dir_all(&staging2).unwrap();
        let (kept, _) = write_manifests(
            &db,
            &dir.path().join("recordings"),
            "recordings",
            &staging2,
            AudioScope::Kept,
            None,
        )
        .unwrap();
        assert_eq!(kept, ["a.wav"], "only the locked clip");
        let (none, _) = write_manifests(
            &db,
            &dir.path().join("recordings"),
            "recordings",
            &staging2,
            AudioScope::None,
            None,
        )
        .unwrap();
        assert!(none.is_empty());
        assert_eq!(
            std::fs::read_to_string(staging2.join(CHECKSUMS)).unwrap(),
            ""
        );
    }

    /// A clip that differs from its recorded checksum, or is listed and
    /// absent, refuses the restore; the unaltered archive passes.
    #[test]
    fn a_damaged_or_incomplete_archive_fails_its_checksums() {
        let dir = tempfile::tempdir().unwrap();
        let rec = dir.path().join("recordings");
        std::fs::create_dir_all(&rec).unwrap();
        std::fs::write(rec.join("a.wav"), b"abc").unwrap();
        std::fs::write(
            dir.path().join(CHECKSUMS),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  recordings/a.wav\n",
        )
        .unwrap();
        assert_eq!(verify_checksums(dir.path()), Ok(1));
        std::fs::write(rec.join("a.wav"), b"xyz").unwrap();
        assert!(
            verify_checksums(dir.path())
                .unwrap_err()
                .contains("differs")
        );
        std::fs::remove_file(rec.join("a.wav")).unwrap();
        assert!(
            verify_checksums(dir.path())
                .unwrap_err()
                .contains("incomplete")
        );
        std::fs::write(dir.path().join(CHECKSUMS), "00  ../x\n").unwrap();
        assert!(verify_checksums(dir.path()).is_err());
        std::fs::remove_file(dir.path().join(CHECKSUMS)).unwrap();
        assert_eq!(
            verify_checksums(dir.path()),
            Ok(0),
            "an older archive has no list"
        );
    }

    #[test]
    fn the_audio_parameter_is_read_strictly() {
        assert_eq!(AudioScope::parse(None), Some(AudioScope::All));
        assert_eq!(AudioScope::parse(Some("kept")), Some(AudioScope::Kept));
        assert_eq!(AudioScope::parse(Some("none")), Some(AudioScope::None));
        assert_eq!(AudioScope::parse(Some("some")), None);
    }
}
