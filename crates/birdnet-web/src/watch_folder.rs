//! What is sitting in the folder the station watches for recordings, and was
//! never going to be analysed.
//!
//! The detection daemon reads the date and time of a recording from its file
//! name, in BirdNET-Pi's form `YYYY-MM-DD-birdnet-HH:MM:SS.wav` (with a stream
//! id before the time for a camera). A file in the watch folder whose name does
//! not look like that is refused with a warning in the log and left where it is,
//! so someone who drops a recording in under any other name sees nothing happen
//! and has no page to tell them why.
//!
//! This is the scan behind the Station Health condition that tells them. It is
//! a real directory read, capped, so a folder with a hundred thousand files in it
//! costs the same as one with a thousand, and it looks at names only.

use std::path::{Path, PathBuf};

/// The name the daemon expects, as the people reading the page should see it.
pub const EXPECTED_NAME: &str = "YYYY-MM-DD-birdnet-HH:MM:SS.wav";

/// An example of that name, for the sentence that explains it.
pub const EXAMPLE_NAME: &str = "2026-08-12-birdnet-11:30:16.wav";

/// How many directory entries one scan looks at before it stops.
pub const SCAN_CAP: usize = 2_000;

/// The result of one scan of the watch folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderScan {
    /// The folder that was read.
    pub dir: PathBuf,
    /// Audio files (`.wav`, `.flac`, `.mp3`) whose names the daemon cannot read.
    pub unnamed: usize,
    /// The scan stopped at [`SCAN_CAP`] entries, so `unnamed` is a floor.
    pub capped: bool,
}

/// Whether the daemon would take `name` as a recording to analyse: an audio
/// extension and a name [`birdnet_core::detection::types::RecordingFile`] reads.
///
/// Returns `None` for a file the daemon would not look at at all (not audio),
/// `Some(true)` for one it can read and `Some(false)` for one it refuses.
fn readable_recording(name: &str) -> Option<bool> {
    let (_, ext) = name.rsplit_once('.')?;
    let audio = ["wav", "flac", "mp3"]
        .iter()
        .any(|known| ext.eq_ignore_ascii_case(known));
    audio.then(|| birdnet_core::detection::types::RecordingFile::parse(name).is_some())
}

/// Read `dir` and count the audio files the daemon refuses by name.
///
/// `None` when the folder cannot be read (it is not there yet, or not
/// readable): that is not a finding about names, and other pages already say
/// when the folder is missing.
#[must_use]
pub fn scan(dir: &Path) -> Option<FolderScan> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut unnamed = 0;
    let mut seen = 0;
    let mut capped = false;
    for entry in entries.flatten() {
        if seen >= SCAN_CAP {
            capped = true;
            break;
        }
        seen += 1;
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() {
            continue;
        }
        let name = entry.file_name();
        if readable_recording(&name.to_string_lossy()) == Some(false) {
            unnamed += 1;
        }
    }
    Some(FolderScan {
        dir: dir.to_path_buf(),
        unnamed,
        capped,
    })
}

/// The paragraph on the Capture tab that says where recordings are read from
/// and what a file placed there has to be called.
///
/// Until this, the folder was findable only in the log, and the name only in the
/// source.
#[must_use]
pub fn capture_note(dir: Option<&Path>) -> String {
    let place = dir.map_or_else(
        || "The station's watch folder".to_owned(),
        |d| {
            format!(
                "Recordings are read from <code>{}</code>.",
                crate::routes::pages::escape_html(&d.display().to_string())
            )
        },
    );
    format!(
        r#"<p class="hint" id="watch-folder-note">{place} The station takes the date and time of a recording from its file name, so a file you place there yourself has to be called <code>{EXPECTED_NAME}</code> (for example <code>{EXAMPLE_NAME}</code>). A file with any other name is skipped, and Station Health says how many.</p>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"RIFF").expect("write");
    }

    #[test]
    fn only_audio_files_with_names_the_daemon_refuses_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        // Names the daemon reads, in both forms.
        touch(dir.path(), "2026-08-12-birdnet-11:30:16.wav");
        touch(dir.path(), "2026-08-12-birdnet-RTSP_1-11:30:16.wav");
        // Audio under a name it cannot read.
        touch(dir.path(), "dawn chorus.wav");
        touch(dir.path(), "recording-1.mp3");
        touch(dir.path(), "20260812_113016.FLAC");
        // Not audio: not the daemon's business, and not counted.
        touch(dir.path(), "notes.txt");
        touch(dir.path(), "README");
        // A folder is not a file.
        std::fs::create_dir(dir.path().join("old.wav")).unwrap();

        let scan = scan(dir.path()).expect("readable");
        assert_eq!(scan.unnamed, 3, "{scan:?}");
        assert!(!scan.capped);
    }

    #[test]
    fn a_folder_of_good_names_reports_none() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "2026-08-12-birdnet-11:30:16.wav");
        assert_eq!(scan(dir.path()).expect("readable").unnamed, 0);
    }

    #[test]
    fn a_folder_that_is_not_there_is_not_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(scan(&dir.path().join("not-created-yet")), None);
    }

    #[test]
    fn the_scan_stops_at_its_cap_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(SCAN_CAP + 25) {
            touch(dir.path(), &format!("bad-{i}.wav"));
        }
        let scan = scan(dir.path()).expect("readable");
        assert!(scan.capped, "{scan:?}");
        assert_eq!(scan.unnamed, SCAN_CAP, "it looked at exactly the cap");
    }

    #[test]
    fn the_capture_note_names_the_folder_escaped_and_the_name() {
        let html = capture_note(Some(Path::new("/tmp/<b>x</b>")));
        assert!(html.contains("/tmp/&lt;b&gt;x&lt;/b&gt;"), "{html}");
        assert!(
            html.contains(EXPECTED_NAME) && html.contains(EXAMPLE_NAME),
            "{html}"
        );
        let unknown = capture_note(None);
        assert!(
            unknown.contains(EXPECTED_NAME) && !unknown.contains("are read from"),
            "{unknown}"
        );
    }

    /// The expected name is itself a name the daemon reads, so the page cannot
    /// tell people to use a form it would refuse.
    #[test]
    fn the_example_name_is_one_the_daemon_reads() {
        assert_eq!(readable_recording(EXAMPLE_NAME), Some(true));
        assert_eq!(
            readable_recording("2026-08-12-birdnet-11-30-16.wav"),
            Some(false)
        );
    }
}
