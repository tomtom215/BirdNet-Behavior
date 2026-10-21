//! BirdNET-Pi's clip tree, brought across so an imported detection plays.
//!
//! The tree is built to BirdNET-Pi's real layout, read from its code:
//! `<EXTRACTED>/By_Date/<Date>/<Common_Name_Safe>/<File_Name>` with a
//! spectrogram `<File_Name>.png` beside each clip, and `By_Date/shifted/` for
//! its on-demand pitch-shifted copies (`reporting.py: extract_detection`,
//! `play.php`).

use std::fs;
use std::path::{Path, PathBuf};

use birdnet_migrate::clips::{ClipError, DiskRoom, Mode, Options, Report, Wanted, WantedRows, run};

/// One synthetic BirdNET-Pi station.
struct Tree {
    home: tempfile::TempDir,
    rows: Vec<Wanted>,
}

fn clip_name(com: &str, pct: u32, date: &str, time: &str, ext: &str) -> String {
    format!(
        "{}-{pct}-{date}-birdnet-{time}.{ext}",
        com.replace('\'', "").replace(' ', "_")
    )
}

impl Tree {
    /// `BirdSongs/Extracted/By_Date/...` with clips of known content.
    fn new() -> Self {
        let home = tempfile::tempdir().expect("tempdir");
        let mut tree = Self {
            home,
            rows: Vec::new(),
        };
        tree.add(
            "Eurasian Blackbird",
            "2026-03-01",
            "06:30:15",
            "mp3",
            b"blackbird-one",
        );
        tree.add(
            "Eurasian Blackbird",
            "2026-03-02",
            "07:01:02",
            "mp3",
            b"blackbird-two!",
        );
        tree.add(
            "Cetti's Warbler",
            "2026-03-02",
            "05:55:55",
            "wav",
            b"cettis",
        );
        tree.add(
            "Black-capped Chickadee",
            "2026-03-03",
            "08:00:00",
            "mp3",
            b"chickadee-audio",
        );
        tree
    }

    fn by_date(&self) -> PathBuf {
        self.home.path().join("BirdSongs/Extracted/By_Date")
    }

    /// Write a clip, its picture, and the row that names it.
    fn add(&mut self, com: &str, date: &str, time: &str, ext: &str, audio: &[u8]) -> String {
        let name = clip_name(com, 85, date, time, ext);
        let dir = self
            .by_date()
            .join(date)
            .join(com.replace('\'', "").replace(' ', "_"));
        fs::create_dir_all(&dir).expect("species dir");
        fs::write(dir.join(&name), audio).expect("clip");
        fs::write(dir.join(format!("{name}.png")), b"png").expect("picture");
        self.rows.push(Wanted {
            date: date.to_owned(),
            com_name: com.to_owned(),
            file_name: name.clone(),
        });
        name
    }
}

fn opts(tree: &Tree, dest: &Path, mode: Mode, dry_run: bool) -> Options {
    Options {
        // The person may point at any of the three levels.
        source: tree.home.path().join("BirdSongs"),
        dest: dest.to_path_buf(),
        mode,
        dry_run,
        room: None,
        retention_cutoff: None,
    }
}

fn go(tree: &Tree, o: &Options) -> Result<Report, ClipError> {
    let rows = tree.rows.clone();
    let wanted: &WantedRows<'_> = &move || Box::new(rows.clone().into_iter());
    run(o, wanted, None)
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_copy_puts_each_clip_flat_in_the_recordings_directory_and_leaves_the_tree_whole() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let report = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");

    assert_eq!(report.examined, 4);
    assert_eq!(report.done, 4, "{report:?}");
    assert_eq!(listing(dest.path()).len(), 4, "four clips and nothing else");
    for w in &tree.rows {
        let copied = fs::read(dest.path().join(&w.file_name)).expect("clip is there");
        let original = fs::read(
            tree.by_date()
                .join(&w.date)
                .join(w.com_name.replace('\'', "").replace(' ', "_"))
                .join(&w.file_name),
        )
        .expect("original");
        assert_eq!(copied, original);
    }
    assert!(
        listing(dest.path())
            .iter()
            .all(|n| Path::new(n).extension().is_none_or(|e| e != "png")),
        "the picture is not copied: this station draws its own"
    );
    // The tree is untouched.
    assert!(
        tree.by_date()
            .join("2026-03-01/Eurasian_Blackbird")
            .is_dir()
    );
}

#[test]
fn a_dry_run_changes_nothing_and_predicts_the_run() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let dry = go(&tree, &opts(&tree, dest.path(), Mode::Copy, true)).expect("dry run");
    assert!(listing(dest.path()).is_empty(), "a dry run wrote something");
    let real = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");
    assert_eq!((dry.done, dry.bytes), (real.done, real.bytes));
    assert_eq!(dry.examined, real.examined);
    assert_eq!(listing(dest.path()).len(), 4, "and the run did write them");
}

/// Resumable: a second pass finds the first's work done, and finishes a run
/// that stopped half-way, including the temporary file it left.
#[test]
fn a_run_can_be_repeated_and_resumes() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    // A previous run copied one clip and died while writing another.
    let first = &tree.rows[0];
    fs::copy(
        tree.by_date()
            .join(&first.date)
            .join("Eurasian_Blackbird")
            .join(&first.file_name),
        dest.path().join(&first.file_name),
    )
    .expect("one clip is already there");
    fs::write(
        dest.path().join(".bnb-import-half.wav.part"),
        b"half a clip",
    )
    .expect("leftover");

    let report = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");
    assert_eq!(report.already_there, 1);
    assert_eq!(report.done, 3);
    assert!(
        !listing(dest.path())
            .iter()
            .any(|n| Path::new(n).extension().is_some_and(|e| e == "part")),
        "the leftover temporary file is removed"
    );

    let again = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("again");
    assert_eq!((again.done, again.already_there), (0, 4));
}

/// A name already taken by a different file is reported and left alone.
#[test]
fn a_clip_is_never_overwritten() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let taken = &tree.rows[1].file_name;
    fs::write(dest.path().join(taken), b"someone else's recording").expect("taken");
    let report = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");
    assert_eq!(report.conflicts, 1);
    assert_eq!(report.done, 3);
    assert_eq!(
        fs::read(dest.path().join(taken)).expect("still there"),
        b"someone else's recording"
    );
}

#[test]
fn a_hard_link_costs_no_space_and_is_the_same_file() {
    let tree = Tree::new();
    // Same filesystem as the tree.
    let dest = tree.home.path().join("recordings");
    let report = go(&tree, &opts(&tree, &dest, Mode::Hardlink, false)).expect("run");
    assert_eq!(report.done, 4, "{report:?}");
    let w = &tree.rows[0];
    let original = tree
        .by_date()
        .join(&w.date)
        .join("Eurasian_Blackbird")
        .join(&w.file_name);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(
            fs::metadata(&original).expect("original").ino(),
            fs::metadata(dest.join(&w.file_name)).expect("link").ino(),
            "one file with two names"
        );
    }
}

#[test]
fn a_move_removes_the_original_only_after_the_copy_is_whole() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let report = go(&tree, &opts(&tree, dest.path(), Mode::Move, false)).expect("run");
    assert_eq!((report.done, report.originals_removed), (4, 4));
    for w in &tree.rows {
        assert!(dest.path().join(&w.file_name).is_file());
        assert!(
            !tree
                .by_date()
                .join(&w.date)
                .join(w.com_name.replace('\'', "").replace(' ', "_"))
                .join(&w.file_name)
                .exists()
        );
    }
    // A dry run of a move removes nothing.
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    go(&tree, &opts(&tree, dest.path(), Mode::Move, true)).expect("dry");
    assert!(
        tree.by_date()
            .join("2026-03-01/Eurasian_Blackbird")
            .read_dir()
            .expect("dir")
            .count()
            > 0
    );
}

#[test]
fn leave_reports_and_touches_nothing() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let report = go(&tree, &opts(&tree, dest.path(), Mode::Leave, false)).expect("run");
    assert_eq!(report.done, 4, "four clips found");
    assert!(listing(dest.path()).is_empty());
}

/// A clip whose row says one species but which lives under another folder (a
/// species renamed by a taxonomy update, or a date folder moved) is found by name.
#[test]
fn a_clip_filed_somewhere_else_in_the_tree_is_found_by_its_name() {
    let mut tree = Tree::new();
    let name = tree.add(
        "Eurasian Blackbird",
        "2026-03-04",
        "06:00:00",
        "mp3",
        b"moved-about",
    );
    let from = tree
        .by_date()
        .join("2026-03-04/Eurasian_Blackbird")
        .join(&name);
    let to_dir = tree.by_date().join("2026-03-05/Common_Blackbird");
    fs::create_dir_all(&to_dir).expect("dir");
    fs::rename(&from, to_dir.join(&name)).expect("move");
    let dest = tempfile::tempdir().expect("dest");
    let report = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");
    assert_eq!(report.done, 5, "{report:?}");
    assert_eq!(
        fs::read(dest.path().join(&name)).expect("clip"),
        b"moved-about"
    );
}

#[test]
fn a_clip_that_is_not_in_the_tree_is_missing_not_an_error() {
    let mut tree = Tree::new();
    tree.rows.push(Wanted {
        date: "2026-03-09".into(),
        com_name: "Ghost Owl".into(),
        file_name: "Ghost_Owl-90-2026-03-09-birdnet-01:00:00.wav".into(),
    });
    let dest = tempfile::tempdir().expect("dest");
    let report = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");
    assert_eq!(report.missing, 1);
    assert_eq!(
        report.missing_sample,
        ["Ghost_Owl-90-2026-03-09-birdnet-01:00:00.wav"]
    );
    assert_eq!(report.done, 4);
}

/// The pitch-shifted copies BirdNET-Pi keeps are not clips.
#[test]
fn the_shifted_directory_is_not_a_source() {
    let tree = Tree::new();
    let w = tree.rows[0].clone();
    let shifted = tree
        .by_date()
        .join("shifted")
        .join(&w.date)
        .join("Eurasian_Blackbird");
    fs::create_dir_all(&shifted).expect("dir");
    fs::write(shifted.join(&w.file_name), b"a shifted copy").expect("shifted");
    // Take the real one away: the row must be missing, not satisfied by the copy.
    fs::remove_file(
        tree.by_date()
            .join(&w.date)
            .join("Eurasian_Blackbird")
            .join(&w.file_name),
    )
    .expect("remove");
    let dest = tempfile::tempdir().expect("dest");
    let report = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");
    assert_eq!(report.missing, 1, "{report:?}");
    assert!(!dest.path().join(&w.file_name).exists());
}

/// A file name that is not a plain name is never joined onto a path.
#[test]
fn a_hostile_file_name_is_refused_and_nothing_is_written_outside() {
    let mut tree = Tree::new();
    let outside = tree.home.path().join("secret.wav");
    fs::write(&outside, b"not a clip").expect("outside");
    for bad in [
        "../../../secret.wav",
        "/etc/passwd",
        "a/../../secret.wav",
        "..",
        ".hidden.wav",
        "x\0.wav",
    ] {
        tree.rows.push(Wanted {
            date: "2026-03-01".into(),
            com_name: "Eurasian Blackbird".into(),
            file_name: bad.into(),
        });
    }
    // And a species name that tries to climb out of the tree.
    tree.rows.push(Wanted {
        date: "2026-03-01".into(),
        com_name: "../../..".into(),
        file_name: "secret.wav".into(),
    });
    let dest = tempfile::tempdir().expect("dest");
    let report = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");
    assert_eq!(report.unsafe_names, 6);
    assert_eq!(report.done, 4, "only the four real clips");
    let names = listing(dest.path());
    assert_eq!(names.len(), 4, "{names:?}");
    assert!(!names.iter().any(|n| n == "secret.wav"));
}

/// A symbolic link in the tree that points out of it is not a clip.
#[cfg(unix)]
#[test]
fn a_link_out_of_the_tree_is_not_followed() {
    let mut tree = Tree::new();
    let outside = tree.home.path().join("private.wav");
    fs::write(&outside, b"private").expect("outside");
    let name = "Eurasian_Blackbird-85-2026-03-07-birdnet-06:00:00.wav";
    let dir = tree.by_date().join("2026-03-07/Eurasian_Blackbird");
    fs::create_dir_all(&dir).expect("dir");
    std::os::unix::fs::symlink(&outside, dir.join(name)).expect("link");
    tree.rows.push(Wanted {
        date: "2026-03-07".into(),
        com_name: "Eurasian Blackbird".into(),
        file_name: name.into(),
    });
    let dest = tempfile::tempdir().expect("dest");
    let report = go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");
    assert_eq!(report.missing, 1, "{report:?}");
    assert!(!dest.path().join(name).exists());
}

#[test]
fn a_source_that_is_not_a_clip_tree_is_said_so() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let mut o = opts(&tree, dest.path(), Mode::Copy, true);
    o.source = tree.home.path().join("nowhere");
    let err = go(&tree, &o).expect_err("no tree");
    assert!(matches!(err, ClipError::NoClipTree(_)), "{err}");
    assert!(err.to_string().contains("By_Date"), "{err}");

    // Each of the three ways of pointing at it works.
    for source in [
        tree.by_date(),
        tree.home.path().join("BirdSongs/Extracted"),
        tree.home.path().join("BirdSongs"),
    ] {
        let mut o = opts(&tree, dest.path(), Mode::Copy, true);
        o.source = source;
        assert_eq!(go(&tree, &o).expect("found").done, 4);
    }
}

#[test]
fn not_enough_room_stops_before_anything_is_written() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let mut o = opts(&tree, dest.path(), Mode::Copy, false);
    o.room = Some(DiskRoom {
        used_bytes: 100,
        available_bytes: 10,
        purge_threshold_percent: 99.0,
    });
    let err = go(&tree, &o).expect_err("no room");
    assert!(
        matches!(
            err,
            ClipError::NoRoom {
                crosses_threshold: false,
                ..
            }
        ),
        "{err}"
    );
    assert!(listing(dest.path()).is_empty());
}

/// There is room, but the copy would take the disk past the point at which this
/// station starts deleting its clips: the very clips just brought in would go.
#[test]
fn a_copy_that_would_start_the_purge_is_refused() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let total_bytes: u64 = tree
        .rows
        .iter()
        .map(|w| {
            fs::metadata(
                tree.by_date()
                    .join(&w.date)
                    .join(w.com_name.replace('\'', "").replace(' ', "_"))
                    .join(&w.file_name),
            )
            .expect("meta")
            .len()
        })
        .sum();
    // 1000 bytes reachable, 920 used: the 48 bytes of clips fit in the 80 left but land at 96.8 %.
    let mut o = opts(&tree, dest.path(), Mode::Copy, false);
    o.room = Some(DiskRoom {
        used_bytes: 920,
        available_bytes: 80,
        purge_threshold_percent: 95.0,
    });
    assert_eq!(total_bytes, 48);
    let err = go(&tree, &o).expect_err("refused");
    assert!(
        matches!(
            err,
            ClipError::NoRoom {
                crosses_threshold: true,
                ..
            }
        ),
        "{err}"
    );
    assert!(err.to_string().contains("starts deleting"), "{err}");
    assert!(listing(dest.path()).is_empty());

    // Counterpart: the same copy with the threshold higher goes ahead.
    o.room = Some(DiskRoom {
        used_bytes: 920,
        available_bytes: 80,
        purge_threshold_percent: 99.0,
    });
    assert_eq!(go(&tree, &o).expect("fits").done, 4);
    // And a link needs no room at all, however full the disk is.
    let dest2 = tree.home.path().join("linked");
    let mut l = opts(&tree, &dest2, Mode::Hardlink, false);
    l.room = Some(DiskRoom {
        used_bytes: 999,
        available_bytes: 1,
        purge_threshold_percent: 95.0,
    });
    assert_eq!(go(&tree, &l).expect("links").done, 4);
}

#[test]
fn clips_older_than_the_retention_are_counted() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let mut o = opts(&tree, dest.path(), Mode::Copy, true);
    o.retention_cutoff = Some("2026-03-03".into());
    let report = go(&tree, &o).expect("dry");
    assert_eq!(
        report.older_than_retention, 3,
        "the 1st and 2nd (x2) are before the 3rd"
    );
}

#[test]
fn a_move_of_clips_the_retention_would_delete_is_refused_before_anything_is_touched() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    for dry_run in [true, false] {
        let mut o = opts(&tree, dest.path(), Mode::Move, dry_run);
        o.retention_cutoff = Some("2026-03-03".into());
        let err = go(&tree, &o).expect_err("a move onto the purge's list must be refused");
        assert!(
            matches!(
                err,
                ClipError::MoveWouldBeDeleted {
                    older_than_retention: 3
                }
            ),
            "{err}"
        );
    }
    assert!(listing(dest.path()).is_empty(), "nothing was written");
    for w in &tree.rows {
        assert!(
            tree.by_date()
                .join(&w.date)
                .join(w.com_name.replace('\'', "").replace(' ', "_"))
                .join(&w.file_name)
                .exists(),
            "the original of {} was removed",
            w.file_name
        );
    }
}

/// The counterpart: the refusal is about moving old clips, not about a cutoff
/// being set. A copy of the same clips, and a move when none is older, run.
#[test]
fn the_move_refusal_discriminates() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let mut copy = opts(&tree, dest.path(), Mode::Copy, true);
    copy.retention_cutoff = Some("2026-03-03".into());
    assert_eq!(go(&tree, &copy).expect("copy is allowed").done, 4);
    let mut mv = opts(&tree, dest.path(), Mode::Move, true);
    mv.retention_cutoff = Some("2000-01-01".into());
    assert_eq!(go(&tree, &mv).expect("nothing is older").done, 4);
}

/// A copy keeps the age of the original, or the oldest-first purge would take
/// this station's own recent clips before the imported ones.
#[test]
fn a_copied_clip_keeps_its_age() {
    let tree = Tree::new();
    let dest = tempfile::tempdir().expect("dest");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(900 * 86_400);
    for w in &tree.rows {
        let src = tree
            .by_date()
            .join(&w.date)
            .join(w.com_name.replace('\'', "").replace(' ', "_"))
            .join(&w.file_name);
        fs::File::options()
            .write(true)
            .open(src)
            .and_then(|f| f.set_modified(old))
            .expect("age the original");
    }
    go(&tree, &opts(&tree, dest.path(), Mode::Copy, false)).expect("run");
    for w in &tree.rows {
        let got = fs::metadata(dest.path().join(&w.file_name))
            .and_then(|m| m.modified())
            .expect("mtime");
        let diff = got.duration_since(old).unwrap_or_else(|e| e.duration());
        assert!(
            diff < std::time::Duration::from_secs(2),
            "{} was stamped {diff:?} from the original",
            w.file_name
        );
    }
}

/// The rows a pass asks about are the imported ones that name a clip still
/// wanted: not this station's own, not blank, not already reclaimed.
#[test]
fn only_imported_detections_that_name_a_clip_are_wanted() {
    use birdnet_migrate::clips::wanted_page;
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for _ in 0..2 {
        conn.execute(
            "INSERT INTO import_batches (source_kind) VALUES ('birdnet-pi')",
            [],
        )
        .expect("batch");
    }
    let insert = |time: &str, file: Option<&str>, batch: Option<i64>, pruned: Option<i64>| {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, \
             Sens, Overlap, File_Name, chunk_offset_secs, import_batch_id, Clip_Pruned_At) \
             VALUES ('2026-03-01', ?1, 'Pica pica', 'Eurasian Magpie', 0.9, 0.7, 9, 1.25, 0.0, ?2, 0, ?3, ?4)",
            rusqlite::params![time, file, batch, pruned],
        )
        .expect("insert");
    };
    insert("06:00:01", Some("a.wav"), Some(1), None);
    insert("06:00:02", Some("b.wav"), None, None); // this station's own recording
    insert("06:00:03", Some("c.wav"), Some(1), Some(1_700_000_000)); // already reclaimed
    insert("06:00:04", Some("  "), Some(1), None); // blank name
    insert("06:00:05", None, Some(1), None); // no name
    insert("06:00:06", Some("d.mp3"), Some(2), None);

    let all = wanted_page(&conn, 0, 100).expect("page");
    let names: Vec<&str> = all.iter().map(|(_, w)| w.file_name.as_str()).collect();
    assert_eq!(names, ["a.wav", "d.mp3"]);
    assert_eq!(all[0].1.com_name, "Eurasian Magpie");

    // Paged by rowid: a page of one, then the rest after it.
    let first = wanted_page(&conn, 0, 1).expect("page");
    assert_eq!(first.len(), 1);
    let rest = wanted_page(&conn, first[0].0, 100).expect("page");
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].1.file_name, "d.mp3");
}
