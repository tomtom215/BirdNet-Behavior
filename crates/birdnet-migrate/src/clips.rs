//! Bringing BirdNET-Pi's clips across, so an imported detection plays.
//!
//! The database import copies rows. Each row keeps its `File_Name`, but the
//! recording it names stayed behind in BirdNET-Pi's clip tree, so the player
//! on an imported detection had nothing behind it. This module finds those
//! files and puts them where this station serves clips from.
//!
//! # BirdNET-Pi's layout, and this station's
//!
//! BirdNET-Pi writes a clip to
//! `<EXTRACTED>/By_Date/<Date>/<Common_Name_Safe>/<File_Name>` — `EXTRACTED`
//! is `~/BirdSongs/Extracted` unless the file moves it — with a spectrogram
//! beside it as `<File_Name>.png`, where the name is
//! `Common_Name-<pct>-<date>-birdnet-[RTSP_n-]<HH:MM:SS>.<ext>` and
//! `Common_Name_Safe` is the common name with its apostrophes removed and its
//! spaces made underscores (`reporting.py: extract_detection`, `play.php`).
//! `By_Date/shifted/` holds on-demand pitch-shifted **copies**, which are not
//! clips of their own and are never imported.
//!
//! This station keeps clips **flat** in one directory under the same
//! `File_Name` (`extractor.rs`), and renders a spectrogram from the audio when
//! asked, so the picture BirdNET-Pi drew is not copied.
//!
//! # What it promises
//!
//! * It only ever reads the rows it is handed, so a clip nobody's detection
//!   names is not copied.
//! * A file name is used only if it is a plain file name: no separator, no
//!   `..`, no NUL, no leading dot, an audio extension. The source is found by
//!   building a path under the clip root from checked parts (or by looking the
//!   name up in a listing of the root), and must resolve **inside** that root
//!   after symbolic links are followed.
//! * It never overwrites. A name already present with the same size is "already
//!   there"; present with another size is a conflict, reported and skipped.
//! * It is resumable. A copy is written under a temporary name in the
//!   destination directory and renamed into place, so a run that stops leaves
//!   either a whole clip or a temporary file the next run removes; running it
//!   again carries on from where it was.
//! * `dry_run` runs the same code with the actions switched off, so the
//!   preview's numbers are the run's numbers.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::progress::ProgressHandle;

/// What to do with each clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Copy it. BirdNET-Pi's tree is left whole. Needs room for the copy.
    Copy,
    /// Link it: the same file, two names, no extra space. Only within one
    /// filesystem. Deleting one name leaves the other.
    Hardlink,
    /// Move it. BirdNET-Pi loses the clip, so it cannot be started again
    /// afterwards. Copies first and removes the original only once the copy is
    /// whole.
    Move,
    /// Touch nothing; only report what is there.
    Leave,
}

impl Mode {
    /// Parse the word the form and the command line use.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "copy" => Some(Self::Copy),
            "hardlink" | "link" => Some(Self::Hardlink),
            "move" => Some(Self::Move),
            "leave" | "none" => Some(Self::Leave),
            _ => None,
        }
    }

    /// The word for the form and the command line.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Hardlink => "hardlink",
            Self::Move => "move",
            Self::Leave => "leave",
        }
    }
}

/// A detection whose clip is wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// `YYYY-MM-DD`, BirdNET-Pi's `Date`.
    pub date: String,
    /// The common name as stored.
    pub com_name: String,
    /// The stored `File_Name`.
    pub file_name: String,
}

/// How much room the destination filesystem has.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiskRoom {
    /// Bytes in use on the filesystem.
    pub used_bytes: u64,
    /// Bytes this user can still write.
    pub available_bytes: u64,
    /// The percentage at which this station's disk manager starts deleting
    /// clips (`DISK_PURGE_THRESHOLD`). Copying past it would start a purge of
    /// the very clips just brought in, and of the station's own.
    pub purge_threshold_percent: f64,
}

impl DiskRoom {
    /// Percentage used if `extra` more bytes were written, as `df` counts it
    /// (`used / (used + available)`).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn percent_after(&self, extra: u64) -> f64 {
        let reachable = self.used_bytes.saturating_add(self.available_bytes);
        if reachable == 0 {
            return 100.0;
        }
        self.used_bytes.saturating_add(extra) as f64 / reachable as f64 * 100.0
    }
}

/// Options for one pass.
#[derive(Debug, Clone)]
pub struct Options {
    /// BirdNET-Pi's clip tree: the `By_Date` directory, the `Extracted`
    /// directory that holds it, or `BirdSongs` that holds that.
    pub source: PathBuf,
    /// This station's recordings directory.
    pub dest: PathBuf,
    /// What to do with each clip.
    pub mode: Mode,
    /// Report only; change nothing.
    pub dry_run: bool,
    /// Room on the destination, for the check; `None` skips it.
    pub room: Option<DiskRoom>,
    /// Clips for detections dated before this (`YYYY-MM-DD`) are older than
    /// the retention this station is set to, and would be removed by its next
    /// maintenance pass. Counted in the report; a [`Mode::Move`] of any such
    /// clip is refused with [`ClipError::MoveWouldBeDeleted`], because it would
    /// leave no copy anywhere.
    pub retention_cutoff: Option<String>,
}

/// What happened, or in a dry run what would.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// Rows asked about.
    pub examined: u64,
    /// Clips copied, linked or moved (in a dry run: that would be).
    pub done: u64,
    /// Bytes of those clips.
    pub bytes: u64,
    /// Already in the destination with the same size.
    pub already_there: u64,
    /// In the destination under that name with a different size. Skipped.
    pub conflicts: u64,
    /// Not found in the tree.
    pub missing: u64,
    /// A few of the missing, as stored, for the report.
    pub missing_sample: Vec<String>,
    /// File names that are not plain file names. Skipped.
    pub unsafe_names: u64,
    /// Failures after a file was found (read, write, link).
    pub failed: u64,
    /// A few of the failures, with the file name and the reason.
    pub failed_sample: Vec<String>,
    /// Originals removed after a move.
    pub originals_removed: u64,
    /// Of `done`, those dated before the retention cutoff.
    pub older_than_retention: u64,
    /// Why the pass stopped before the end, if it did.
    pub stopped: Option<String>,
}

impl Report {
    /// How many failures and misses to keep a sample of.
    const SAMPLE: usize = 12;

    fn note_missing(&mut self, name: &str) {
        self.missing += 1;
        if self.missing_sample.len() < Self::SAMPLE {
            self.missing_sample.push(name.to_owned());
        }
    }

    fn note_failed(&mut self, name: &str, why: &str) {
        self.failed += 1;
        if self.failed_sample.len() < Self::SAMPLE {
            self.failed_sample.push(format!("{name}: {why}"));
        }
    }
}

/// Why a pass could not start.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipError {
    /// The source is not BirdNET-Pi's clip tree.
    NoClipTree(String),
    /// The destination cannot be used.
    BadDestination(String),
    /// Hard links need one filesystem and the two are not.
    DifferentFilesystems,
    /// A move would take BirdNET-Pi's only copy of clips that this station's
    /// retention would delete at its next cleanup.
    MoveWouldBeDeleted {
        /// How many of the clips to be moved are older than the retention.
        older_than_retention: u64,
    },
    /// The copy would not fit, or would take the disk past the point at which
    /// this station starts deleting clips.
    NoRoom {
        /// Bytes the copy needs.
        needed: u64,
        /// Bytes available.
        available: u64,
        /// Whether it would fit but cross the purge threshold.
        crosses_threshold: bool,
        /// The disk's usage after the copy.
        percent_after: f64,
        /// The threshold.
        threshold: f64,
    },
}

impl std::fmt::Display for ClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoClipTree(why) => write!(f, "{why}"),
            Self::BadDestination(why) => {
                write!(f, "the recordings directory cannot be used: {why}")
            }
            Self::DifferentFilesystems => write!(
                f,
                "BirdNET-Pi's clips and this station's recordings are on different disks, so \
                 they cannot be linked; choose copy or move"
            ),
            Self::MoveWouldBeDeleted {
                older_than_retention,
            } => write!(
                f,
                "{older_than_retention} of these recordings are older than the number of days \
                 this station keeps clips for, so its next cleanup would delete them, and moving \
                 them would leave no copy anywhere. Choose copy, or raise the clip retention \
                 first"
            ),
            Self::NoRoom {
                needed,
                available,
                crosses_threshold,
                percent_after,
                threshold,
            } => {
                if *crosses_threshold {
                    write!(
                        f,
                        "copying {} would take the disk to {percent_after:.0} %, past the \
                         {threshold:.0} % at which this station starts deleting its oldest \
                         clips; free some space, raise that threshold, or link instead of copy",
                        human_bytes(*needed),
                    )
                } else {
                    write!(
                        f,
                        "the clips need {} and the disk has {} free",
                        human_bytes(*needed),
                        human_bytes(*available),
                    )
                }
            }
        }
    }
}

impl std::error::Error for ClipError {}

/// Bytes as a person reads them.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Audio extensions BirdNET-Pi and this station write.
const AUDIO_EXTENSIONS: [&str; 5] = ["wav", "mp3", "flac", "ogg", "opus"];

/// Whether `name` is a plain audio file name: safe to join onto a directory.
#[must_use]
pub fn is_plain_audio_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('.')
        || name.contains(['/', '\\', '\0'])
        || name == ".."
    {
        return false;
    }
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The directory name BirdNET-Pi gives a species: apostrophes removed, spaces
/// made underscores (`play.php`). `None` for a name that would not be a single
/// path component.
#[must_use]
pub fn species_dir_name(com_name: &str) -> Option<String> {
    let safe = com_name.replace('\'', "").replace(' ', "_");
    if safe.is_empty() || safe == "." || safe == ".." || safe.contains(['/', '\\', '\0']) {
        None
    } else {
        Some(safe)
    }
}

/// Find the `By_Date` directory under whatever the person pointed at.
///
/// # Errors
///
/// [`ClipError::NoClipTree`] when none of the three shapes BirdNET-Pi's tree
/// can be given as is there, with the reason in words.
pub fn find_by_date(source: &Path) -> Result<PathBuf, ClipError> {
    let candidates = [
        source.to_path_buf(),
        source.join("By_Date"),
        source.join("Extracted").join("By_Date"),
    ];
    for candidate in candidates {
        if candidate.file_name().is_some_and(|n| n == "By_Date") && candidate.is_dir() {
            return candidate.canonicalize().map_err(|e| {
                ClipError::NoClipTree(format!("{} cannot be read: {e}", candidate.display()))
            });
        }
    }
    Err(ClipError::NoClipTree(format!(
        "{} does not hold a BirdNET-Pi clip tree. Point at the directory called By_Date, at \
         Extracted (which holds it) or at BirdSongs (which holds that).",
        source.display()
    )))
}

/// Same filesystem?
#[cfg(unix)]
fn same_filesystem(a: &Path, b: &Path) -> Option<bool> {
    use std::os::unix::fs::MetadataExt as _;
    Some(fs::metadata(a).ok()?.dev() == fs::metadata(b).ok()?.dev())
}

#[cfg(not(unix))]
fn same_filesystem(_a: &Path, _b: &Path) -> Option<bool> {
    None
}

/// A listing of every audio file under the tree, by name, for clips that are
/// not where their row says. Built only if something was not found directly.
fn index_tree(root: &Path) -> HashMap<String, PathBuf> {
    let mut found = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                // The pitch-shifted copies are not clips of their own.
                if path.file_name().is_some_and(|n| n == "shifted") && dir == *root {
                    continue;
                }
                stack.push(path);
            } else if kind.is_file()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
                && is_plain_audio_name(name)
            {
                found.entry(name.to_owned()).or_insert(path);
            }
        }
    }
    found
}

/// Resolve `candidate` to a real file inside `root`.
///
/// Symbolic links are followed, and the result must still be under `root`: a
/// link in the tree that points at `/etc/passwd` is not a clip.
fn inside(root: &Path, candidate: &Path) -> Option<(PathBuf, u64)> {
    let real = candidate.canonicalize().ok()?;
    if !real.starts_with(root) {
        return None;
    }
    let meta = fs::metadata(&real).ok()?;
    meta.is_file().then_some((real, meta.len()))
}

/// Name of the temporary file a copy is written to, beside its destination.
fn temp_name(name: &str) -> String {
    format!(".bnb-import-{name}.part")
}

/// Whether `name` is one [`temp_name`] could have produced.
///
/// The suffix is compared exactly: these are names this module wrote itself,
/// always lower case, and a file someone else named `.PART` is not ours to
/// delete.
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "the names are written by temp_name in lower case; an exact match is the point, and a case-insensitive one would delete a stranger's file"
)]
fn is_temp_name(name: &str) -> bool {
    name.ends_with(".part")
}

/// Whether a line of BirdNET-Pi's protected-clips list names a spectrogram.
///
/// BirdNET-Pi writes the `.png` suffix itself, in lower case, so the match is
/// exact.
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "BirdNET-Pi writes the suffix itself in lower case; the filter is the same exact match it uses"
)]
fn is_png_name(line: &str) -> bool {
    line.ends_with(".png")
}

/// Remove temporary files an earlier, stopped pass left in the destination.
fn clear_stale_temporaries(dest: &Path) {
    let Ok(entries) = fs::read_dir(dest) else {
        return;
    };
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str()
            && name.starts_with(".bnb-import-")
            && is_temp_name(name)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// A page of the detections an import brought in that name a clip, after
/// `after_rowid`, oldest first.
///
/// What both the browser and the command line read the rows through, so they
/// ask the same question: imported (`import_batch_id` set), naming a file, and
/// not already reclaimed (`Clip_Pruned_At` empty).
///
/// # Errors
///
/// The query failed.
pub fn wanted_page(
    conn: &rusqlite::Connection,
    after_rowid: i64,
    limit: i64,
) -> rusqlite::Result<Vec<(i64, Wanted)>> {
    let mut stmt = conn.prepare(
        "SELECT rowid, Date, Com_Name, File_Name FROM detections \
         WHERE import_batch_id IS NOT NULL AND rowid > ?1 \
           AND File_Name IS NOT NULL AND TRIM(File_Name) <> '' \
           AND Clip_Pruned_At IS NULL \
         ORDER BY rowid LIMIT ?2",
    )?;
    stmt.query_map(rusqlite::params![after_rowid, limit], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            Wanted {
                date: r.get(1)?,
                com_name: r.get(2)?,
                file_name: r.get(3)?,
            },
        ))
    })?
    .collect()
}

/// A source of the rows whose clips are wanted, callable once per pass.
///
/// A pass reads the rows twice — once to survey, once to act — and the rows
/// are millions on a station with years of history, so the caller supplies a
/// way to read them again rather than a list held in memory.
pub type WantedRows<'a> = dyn Fn() -> Box<dyn Iterator<Item = Wanted> + 'a> + 'a;

/// Run one pass over the wanted rows.
///
/// With `dry_run` nothing is written, linked, moved or removed, and the report
/// says what would be.
///
/// # Errors
///
/// A [`ClipError`] before anything is touched: no clip tree, an unusable
/// destination, hard links across filesystems, or not enough room.
pub fn run(
    options: &Options,
    wanted: &WantedRows<'_>,
    progress: Option<&ProgressHandle>,
) -> Result<Report, ClipError> {
    let root = find_by_date(&options.source)?;
    let acting = !options.dry_run && options.mode != Mode::Leave;
    if acting {
        fs::create_dir_all(&options.dest).map_err(|e| ClipError::BadDestination(e.to_string()))?;
    }
    if options.mode != Mode::Leave && !acting && !options.dest.is_dir() {
        // A dry run may name a directory that does not exist yet; its parent
        // must, so the preview is not a promise that cannot be kept.
        if !options.dest.parent().is_some_and(Path::is_dir) {
            return Err(ClipError::BadDestination(format!(
                "{} is not a directory",
                options.dest.display()
            )));
        }
    }
    if options.mode == Mode::Hardlink
        && options.dest.is_dir()
        && same_filesystem(&root, &options.dest) == Some(false)
    {
        return Err(ClipError::DifferentFilesystems);
    }

    // First pass, always without side effects: what is there and what it
    // costs. The room check and the progress total come from it, and a dry run
    // ends here, so the preview and the run are one computation.
    let surveyed = pass(&root, options, wanted(), None, None);
    if options.mode == Mode::Move && surveyed.older_than_retention > 0 {
        // The dry run refuses too, so the person is never offered a button for
        // an import that would destroy the only copy.
        return Err(ClipError::MoveWouldBeDeleted {
            older_than_retention: surveyed.older_than_retention,
        });
    }
    if let Some(room) = options.room {
        let needed = match options.mode {
            Mode::Copy | Mode::Move => surveyed.bytes,
            Mode::Hardlink | Mode::Leave => 0,
        };
        if needed > 0 {
            let after = room.percent_after(needed);
            let fits = needed <= room.available_bytes;
            let crosses = after >= room.purge_threshold_percent;
            if !fits || crosses {
                return Err(ClipError::NoRoom {
                    needed,
                    available: room.available_bytes,
                    crosses_threshold: fits && crosses,
                    percent_after: after,
                    threshold: room.purge_threshold_percent,
                });
            }
        }
    }
    if !acting {
        return Ok(surveyed);
    }

    clear_stale_temporaries(&options.dest);
    Ok(pass(
        &root,
        options,
        wanted(),
        Some(surveyed.done),
        progress,
    ))
}

/// Where a wanted clip is, if it is.
fn locate(
    root: &Path,
    w: &Wanted,
    index: &mut Option<HashMap<String, PathBuf>>,
) -> Option<(PathBuf, u64)> {
    let date_ok = w.date.len() == 10
        && w.date.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if date_ok
        && let Some(species) = species_dir_name(&w.com_name)
        && let Some(hit) = inside(root, &root.join(&w.date).join(species).join(&w.file_name))
    {
        return Some(hit);
    }
    // Not where its row says (a renamed species, a moved date folder): look it
    // up by name in a listing of the tree, built once.
    let listing = index.get_or_insert_with(|| index_tree(root));
    listing.get(&w.file_name).and_then(|p| inside(root, p))
}

/// One walk over the rows. With `act_total` it also does what the mode says
/// (and reports progress against that total); without, it only classifies.
fn pass(
    root: &Path,
    options: &Options,
    wanted: impl Iterator<Item = Wanted>,
    act_total: Option<u64>,
    progress: Option<&ProgressHandle>,
) -> Report {
    let mut report = Report::default();
    let mut index: Option<HashMap<String, PathBuf>> = None;
    let mut finished = 0_u64;
    for w in wanted {
        report.examined += 1;
        if !is_plain_audio_name(&w.file_name) {
            report.unsafe_names += 1;
            continue;
        }
        let Some((src, size)) = locate(root, &w, &mut index) else {
            report.note_missing(&w.file_name);
            continue;
        };
        if options.mode == Mode::Leave {
            report.done += 1;
            report.bytes += size;
            continue;
        }
        match fs::metadata(options.dest.join(&w.file_name)) {
            Ok(have) if have.len() == size => {
                report.already_there += 1;
                continue;
            }
            Ok(_) => {
                report.conflicts += 1;
                continue;
            }
            Err(_) => {}
        }
        let older = options
            .retention_cutoff
            .as_deref()
            .is_some_and(|cut| w.date.as_str() < cut);
        if act_total.is_none() {
            report.done += 1;
            report.bytes += size;
            report.older_than_retention += u64::from(older);
            continue;
        }
        match act(options.mode, &src, &options.dest, &w.file_name, size) {
            Ok(removed_original) => {
                report.done += 1;
                report.bytes += size;
                report.originals_removed += u64::from(removed_original);
                report.older_than_retention += u64::from(older);
            }
            Err(e) => report.note_failed(&w.file_name, &e.to_string()),
        }
        finished += 1;
        if let (Some(p), Some(total)) = (progress, act_total)
            && finished.is_multiple_of(25)
        {
            p.update(crate::progress::MigrationProgress {
                stage: crate::progress::MigrationStage::Importing,
                rows_imported: finished,
                rows_total: total,
                message: format!(
                    "Brought across {finished} of {total} recordings ({})",
                    human_bytes(report.bytes)
                ),
                error: None,
            });
        }
    }
    report
}

/// The filesystem calls a copy or move is made of, as a seam: the real thing
/// is [`RealFs`]; a test substitutes one that records the order of the calls
/// and can make one of them fail, which is the only way to prove that a move
/// removes the original *last*.
trait ClipFs {
    /// Copy `src` to `part` (created new, never overwriting), carrying the
    /// modification time across, and return the bytes written.
    fn copy(&self, src: &Path, part: &Path) -> io::Result<u64>;
    /// Flush the file's contents and metadata to the disk (`fsync`).
    fn sync_file(&self, path: &Path) -> io::Result<()>;
    /// Rename.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Flush a directory's entries to the disk, so a rename into it survives a
    /// power cut.
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
    /// The length of the file at `path`.
    fn len(&self, path: &Path) -> io::Result<u64>;
    /// Remove a file.
    fn remove(&self, path: &Path) -> io::Result<()>;
}

/// [`ClipFs`] over `std::fs`.
struct RealFs;

impl ClipFs for RealFs {
    fn copy(&self, src: &Path, part: &Path) -> io::Result<u64> {
        let mut from = fs::File::open(src)?;
        let modified = from.metadata()?.modified().ok();
        // `create_new`: a temporary file that is already there is somebody
        // else's copy in flight, and writing into it would interleave.
        let mut to = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(part)?;
        let n = io::copy(&mut from, &mut to)?;
        if let Some(modified) = modified {
            // BirdNET-Pi's clip keeps its age. The disk manager purges oldest
            // first, and a copy stamped "now" would look newer than every
            // clip this station recorded.
            to.set_modified(modified)?;
        }
        Ok(n)
    }

    fn sync_file(&self, path: &Path) -> io::Result<()> {
        fs::File::open(path)?.sync_all()
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    #[cfg(unix)]
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        fs::File::open(dir)?.sync_all()
    }

    #[cfg(not(unix))]
    fn sync_dir(&self, _dir: &Path) -> io::Result<()> {
        // Windows has no directory fsync; the rename is as durable as the
        // platform makes it.
        Ok(())
    }

    fn len(&self, path: &Path) -> io::Result<u64> {
        fs::metadata(path).map(|m| m.len())
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}

/// Do the action for one clip. Returns whether an original was removed.
fn act(mode: Mode, src: &Path, dest_dir: &Path, name: &str, size: u64) -> io::Result<bool> {
    act_with(&RealFs, mode, src, dest_dir, name, size)
}

/// [`act`] over a given filesystem.
///
/// A move removes the original only after, in this order: the copy is whole
/// (byte count), its contents are on the disk (`fsync` of the temporary file),
/// it is renamed into place, the rename is on the disk (`fsync` of the
/// directory), and the destination is seen to have the original's length. On
/// an SD card a power cut between "renamed" and "flushed" otherwise leaves a
/// zero-length clip *and* no original.
fn act_with(
    fs_ops: &dyn ClipFs,
    mode: Mode,
    src: &Path,
    dest_dir: &Path,
    name: &str,
    size: u64,
) -> io::Result<bool> {
    let target = dest_dir.join(name);
    // Re-check at the last moment: another pass, or the station, may have put
    // a file there since the survey.
    if let Ok(have) = fs_ops.len(&target) {
        return if have == size {
            Ok(false)
        } else {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a different file of that name appeared",
            ))
        };
    }
    match mode {
        Mode::Hardlink => {
            fs::hard_link(src, &target)?;
            Ok(false)
        }
        Mode::Copy | Mode::Move => {
            let part = dest_dir.join(temp_name(name));
            let copied = fs_ops.copy(src, &part).and_then(|n| {
                if n != size {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the copy is shorter than the original",
                    ));
                }
                fs_ops.sync_file(&part)?;
                fs_ops.rename(&part, &target)
            });
            if let Err(e) = copied {
                // `AlreadyExists` is the temporary name being taken: that file
                // is another writer's, not ours to remove.
                if e.kind() != io::ErrorKind::AlreadyExists {
                    let _ = fs_ops.remove(&part);
                }
                return Err(e);
            }
            if mode == Mode::Move {
                // The original is the only other copy: it goes only once the
                // rename is on the disk and the destination has its length.
                fs_ops.sync_dir(dest_dir)?;
                if fs_ops.len(&target)? != size {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the copy in place is not the original's length; the original was kept",
                    ));
                }
                fs_ops.remove(src)?;
                return Ok(true);
            }
            Ok(false)
        }
        Mode::Leave => Ok(false),
    }
}

/// The clip names BirdNET-Pi marked as protected from its disk purge.
///
/// `disk_check_exclude.txt` holds a `##start` line and then one
/// `Date/Species/File_Name` per protected clip, each followed by the same line
/// with `.png` added (`play.php`, `stats.php`). This returns the file names.
#[must_use]
pub fn protected_names(text: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || is_png_name(line) {
            continue;
        }
        let name = line.rsplit('/').next().unwrap_or(line);
        if is_plain_audio_name(name) && seen.insert(name.to_owned()) {
            out.push(name.to_owned());
        }
    }
    out
}

/// Where BirdNET-Pi keeps the list of clips its owner protected, found from the
/// clip tree the import was pointed at.
///
/// BirdNET-Pi writes it to `~/BirdNET-Pi/scripts/disk_check_exclude.txt`, which
/// is not under `BirdSongs`; the standard install has both in one home
/// directory, so the folders above `By_Date` are searched for
/// `BirdNET-Pi/scripts/` (and for `scripts/` itself, for a tree pointed at from
/// inside the BirdNET-Pi folder). `None` when no list is found, which is what a
/// station with nothing protected looks like too.
#[must_use]
pub fn find_protected_list(source: &Path) -> Option<PathBuf> {
    let root = find_by_date(source).unwrap_or_else(|_| source.to_path_buf());
    root.ancestors().skip(1).take(4).find_map(|up| {
        [
            up.join("BirdNET-Pi/scripts/disk_check_exclude.txt"),
            up.join("scripts/disk_check_exclude.txt"),
        ]
        .into_iter()
        .find(|c| c.is_file())
    })
}

/// The most a protected-clips list may be: a name and a path per clip, tens of
/// thousands of clips, and not more.
const MAX_PROTECTED_LIST_BYTES: u64 = 8 * 1024 * 1024;

/// Read BirdNET-Pi's protected-clips list and return the file names on it.
///
/// `path` is the list itself (`disk_check_exclude.txt`), or the BirdNET-Pi
/// folder that holds it (in its own folder or its `scripts/`).
///
/// # Errors
///
/// In words: the file is missing, unreadable or too big to be one.
pub fn read_protected_list(path: &Path) -> Result<Vec<String>, String> {
    use std::io::Read as _;
    let file_path = if path.is_dir() {
        [
            path.join("scripts/disk_check_exclude.txt"),
            path.join("disk_check_exclude.txt"),
        ]
        .into_iter()
        .find(|c| c.is_file())
        .ok_or_else(|| format!("{} holds no disk_check_exclude.txt", path.display()))?
    } else {
        path.to_path_buf()
    };
    let file = fs::File::open(&file_path).map_err(|e| format!("{}: {e}", file_path.display()))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("{}: {e}", file_path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", file_path.display()));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PROTECTED_LIST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", file_path.display()))?;
    if bytes.len() as u64 > MAX_PROTECTED_LIST_BYTES {
        return Err(format!(
            "{} is bigger than a list of protected clips is",
            file_path.display()
        ));
    }
    Ok(protected_names(&String::from_utf8_lossy(&bytes)))
}

/// What marking the protected clips did, or in a preview would do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Protection {
    /// File names on BirdNET-Pi's protected list.
    pub listed: usize,
    /// Imported detections whose clip is on it.
    pub matched: u64,
    /// Of those, the ones not locked yet (locked by this call when it applied).
    pub newly_locked: u64,
}

/// Lock the imported detections whose clip BirdNET-Pi's owner had protected.
///
/// BirdNET-Pi's `disk_check_exclude.txt` keeps a clip out of its purge and its
/// per-species cap (`disk_check.sh`, `disk_species_clean.sh`). The equivalent
/// here is a **locked** detection (`is_locked = 1`): the disk manager re-reads
/// the locked file names each cycle and never removes those clips, and the bulk
/// species actions skip them. Only rows an import brought in
/// (`import_batch_id` set) are touched, by exact `File_Name`; a name that
/// matches none (the history is not imported yet) is not an error and is
/// reported as the difference between `listed` and what matched, so the same
/// call can be repeated after the history is in.
///
/// With `apply` false nothing is written and `newly_locked` is what would be.
///
/// # Errors
///
/// The query failed.
pub fn protect_imported(
    conn: &rusqlite::Connection,
    names: &[String],
    apply: bool,
) -> rusqlite::Result<Protection> {
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS bnb_protected(name TEXT PRIMARY KEY) WITHOUT ROWID; \
         DELETE FROM bnb_protected;",
    )?;
    let result = (|| {
        {
            let tx = conn.unchecked_transaction()?;
            {
                let mut insert =
                    tx.prepare("INSERT OR IGNORE INTO bnb_protected(name) VALUES (?1)")?;
                for name in names {
                    insert.execute([name])?;
                }
            }
            tx.commit()?;
        }
        // One scan of the detections, not one per name: a station with years
        // of history and a few thousand protected clips would otherwise read
        // every row for every name.
        let (matched, unlocked): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN COALESCE(is_locked, 0) = 0 THEN 1 ELSE 0 END), 0) \
             FROM detections \
             WHERE import_batch_id IS NOT NULL AND File_Name IN (SELECT name FROM bnb_protected)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if apply && unlocked > 0 {
            conn.execute(
                "UPDATE detections SET is_locked = 1 \
                 WHERE import_batch_id IS NOT NULL AND COALESCE(is_locked, 0) = 0 \
                   AND File_Name IN (SELECT name FROM bnb_protected)",
                [],
            )?;
        }
        Ok(Protection {
            listed: names.len(),
            matched: u64::try_from(matched).unwrap_or(0),
            newly_locked: u64::try_from(unlocked).unwrap_or(0),
        })
    })();
    let _ = conn.execute_batch("DROP TABLE IF EXISTS bnb_protected;");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_only() {
        assert!(is_plain_audio_name(
            "Eurasian_Blackbird-85-2026-03-01-birdnet-06:30:15.mp3"
        ));
        assert!(is_plain_audio_name(
            "Pica_pica-91-2026-03-01-birdnet-RTSP_1-06:30:15.WAV"
        ));
        for bad in [
            "",
            "..",
            ".hidden.wav",
            "../x.wav",
            "a/b.wav",
            "a\\b.wav",
            "nul\0.wav",
            "clip.png",
            "clip",
            "clip.txt",
        ] {
            assert!(!is_plain_audio_name(bad), "{bad:?}");
        }
        assert!(!is_plain_audio_name(&format!("{}.wav", "a".repeat(260))));
    }

    #[test]
    fn a_species_directory_is_birdnet_pis() {
        assert_eq!(
            species_dir_name("Black-capped Chickadee").as_deref(),
            Some("Black-capped_Chickadee")
        );
        assert_eq!(
            species_dir_name("Cetti's Warbler").as_deref(),
            Some("Cettis_Warbler")
        );
        assert_eq!(species_dir_name("a/b"), None);
        assert_eq!(species_dir_name(".."), None);
        assert_eq!(species_dir_name(""), None);
    }

    #[test]
    fn the_protected_list_gives_names_not_pictures() {
        let text = "##start\n2026-03-01/Eurasian_Blackbird/Eurasian_Blackbird-85-2026-03-01-birdnet-06:30:15.mp3\n\
                    2026-03-01/Eurasian_Blackbird/Eurasian_Blackbird-85-2026-03-01-birdnet-06:30:15.mp3.png\n\
                    ../../etc/passwd\n";
        assert_eq!(
            protected_names(text),
            ["Eurasian_Blackbird-85-2026-03-01-birdnet-06:30:15.mp3"]
        );
    }

    #[test]
    fn disk_room_counts_as_df_does() {
        let room = DiskRoom {
            used_bytes: 50,
            available_bytes: 50,
            purge_threshold_percent: 95.0,
        };
        assert!((room.percent_after(0) - 50.0).abs() < 1e-9);
        assert!((room.percent_after(45) - 95.0).abs() < 1e-9);
    }

    /// A [`ClipFs`] over the real one that records the calls in order and can
    /// be told to fail one of them.
    struct Recording {
        calls: std::cell::RefCell<Vec<&'static str>>,
        fail: Option<&'static str>,
    }

    impl Recording {
        fn new(fail: Option<&'static str>) -> Self {
            Self {
                calls: std::cell::RefCell::new(Vec::new()),
                fail,
            }
        }
        fn step(&self, what: &'static str) -> io::Result<()> {
            self.calls.borrow_mut().push(what);
            if self.fail == Some(what) {
                return Err(io::Error::other(format!("injected {what} failure")));
            }
            Ok(())
        }
        fn calls(&self) -> Vec<&'static str> {
            self.calls.borrow().clone()
        }
    }

    impl ClipFs for Recording {
        fn copy(&self, src: &Path, part: &Path) -> io::Result<u64> {
            self.step("copy")?;
            RealFs.copy(src, part)
        }
        fn sync_file(&self, path: &Path) -> io::Result<()> {
            self.step("sync_file")?;
            RealFs.sync_file(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.step("rename")?;
            RealFs.rename(from, to)
        }
        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            self.step("sync_dir")?;
            RealFs.sync_dir(dir)
        }
        fn len(&self, path: &Path) -> io::Result<u64> {
            // Only the length checks after the rename are interesting; the
            // "is anything there yet" probe is recorded too, so the order of
            // the two is visible.
            self.calls.borrow_mut().push("len");
            RealFs.len(path)
        }
        fn remove(&self, path: &Path) -> io::Result<()> {
            self.step("remove")?;
            RealFs.remove(path)
        }
    }

    fn scratch(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "bnb-clips-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&base);
        let src_dir = base.join("src");
        let dest = base.join("dest");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dest).unwrap();
        let src = src_dir.join("a.wav");
        fs::write(&src, b"0123456789").unwrap();
        (base, src, dest)
    }

    #[test]
    fn a_move_removes_the_original_last_and_only_after_both_flushes() {
        let (base, src, dest) = scratch("order");
        let rec = Recording::new(None);
        let removed = act_with(&rec, Mode::Move, &src, &dest, "a.wav", 10).unwrap();
        assert!(removed);
        assert_eq!(
            rec.calls(),
            [
                "len",
                "copy",
                "sync_file",
                "rename",
                "sync_dir",
                "len",
                "remove"
            ],
            "the original goes only after the file and the directory are flushed and the length is re-read"
        );
        assert!(!src.exists());
        assert_eq!(fs::read(dest.join("a.wav")).unwrap(), b"0123456789");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_failed_flush_keeps_the_original() {
        for failing in ["sync_file", "sync_dir"] {
            let (base, src, dest) = scratch(failing);
            let rec = Recording::new(Some(failing));
            let result = act_with(&rec, Mode::Move, &src, &dest, "a.wav", 10);
            assert!(result.is_err(), "{failing}");
            assert!(src.exists(), "{failing}: the original must survive");
            assert!(
                !rec.calls().contains(&"remove") || failing == "sync_file",
                "{failing}: nothing may be removed after a failed directory flush"
            );
            let _ = fs::remove_dir_all(base);
        }
    }

    #[test]
    fn a_plain_copy_flushes_the_file_and_never_removes_the_original() {
        let (base, src, dest) = scratch("copy");
        let rec = Recording::new(None);
        assert!(!act_with(&rec, Mode::Copy, &src, &dest, "a.wav", 10).unwrap());
        assert_eq!(rec.calls(), ["len", "copy", "sync_file", "rename"]);
        assert!(src.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_copy_keeps_the_age_of_the_original() {
        let (base, src, dest) = scratch("mtime");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(400 * 86_400);
        fs::File::options()
            .write(true)
            .open(&src)
            .unwrap()
            .set_modified(old)
            .unwrap();
        act(Mode::Copy, &src, &dest, "a.wav", 10).unwrap();
        let got = fs::metadata(dest.join("a.wav"))
            .unwrap()
            .modified()
            .unwrap();
        let diff = got.duration_since(old).unwrap_or_else(|e| e.duration());
        assert!(
            diff < std::time::Duration::from_secs(2),
            "the copy was stamped {diff:?} away from the original's time"
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_temporary_name_that_is_taken_is_left_alone() {
        let (base, src, dest) = scratch("taken");
        let part = dest.join(temp_name("a.wav"));
        fs::write(&part, b"another writer's bytes").unwrap();
        let err = act(Mode::Copy, &src, &dest, "a.wav", 10).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&part).unwrap(), b"another writer's bytes");
        let _ = fs::remove_dir_all(base);
    }
}
