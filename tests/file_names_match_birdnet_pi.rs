//! Does a recording's file name mean to this project what it means to BirdNET-Pi?
//!
//! The name is the only record of when audio was made: BirdNET-Pi stores
//! `file_date + window start` as a row's date and time and takes the week handed
//! to its range filter from it (`classes.ParseFileName`). `tools/parity/file_names_ref.py`
//! asked BirdNET-Pi's own class about 33 names, good and bad, including the
//! year-boundary ones (`golden/file_names.json`); this asks `RecordingFile::parse`.
//!
//! Where both read a name they must read the same date, time and RTSP id. Where
//! they disagree about *whether* it can be read, the disagreement is listed here, so
//! it is a decision and not an accident:
//!
//! * BirdNET-Pi finds the date and the time with two loose regular expressions, so
//!   it reads `2026-2-16-…`, `…-1:02:03`, a missing or differently-cased `birdnet`,
//!   and a name without an extension. This project's recorder writes the one
//!   fixed shape and `parse` insists on it.
//! * BirdNET-Pi parses the values with `strptime`, so `2026-02-30` and `25:00:00`
//!   raise and the file is skipped. `parse` checks the shape (4-2-2 digits, an
//!   8-character time) and nothing downstream rejects the value: the daemon
//!   analyses the file and stores its rows under the name's date and time as
//!   they are (observed through the real binary: `2026-02-30` is stored as
//!   2026-03-02, `25:00:00` as the time `25:00:00`). The recorders that name
//!   these files take the date from the clock, so neither can arise unless a
//!   person names a file by hand.

use birdnet_core::detection::types::RecordingFile;

const GOLDEN: &str = "tools/parity/golden/file_names.json";

/// Names BirdNET-Pi reads and `RecordingFile::parse` refuses.
const OURS_REFUSES: &[&str] = &[
    "2026-02-16-birdnet-12:00:00",
    "2026-2-16-birdnet-12:00:00.wav",
    "2026-02-16-birdnet-1:02:03.wav",
    "2026-02-16-12:00:00.wav",
    "2026-02-16-bird-12:00:00.wav",
    "2026-02-16-BirdNET-12:00:00.wav",
];

/// Names `RecordingFile::parse` reads and BirdNET-Pi refuses (the value is not a real date or time).
const OURS_ACCEPTS: &[&str] = &[
    "2025-02-29-birdnet-12:00:00.wav",
    "2026-02-16-birdnet-12:00:60.wav",
    "2026-02-16-birdnet-12:60:00.wav",
    "2026-02-16-birdnet-25:00:00.wav",
    "2026-02-30-birdnet-12:00:00.wav",
    "2026-13-01-birdnet-12:00:00.wav",
];

#[test]
fn a_file_name_means_what_it_means_to_birdnet_pi_except_where_listed() {
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("golden")).expect("json");
    let names = golden["names"].as_object().expect("names");
    assert_eq!(names.len(), 33);
    let (mut refuses, mut accepts, mut agreed_on) = (Vec::new(), Vec::new(), 0);
    for (name, pi) in names {
        let ours = RecordingFile::parse(&format!("/recs/{name}"));
        match (pi["accepted"].as_bool().unwrap(), ours) {
            (true, Some(o)) => {
                assert_eq!(o.date, pi["date"].as_str().unwrap(), "{name}: date");
                assert_eq!(o.time, pi["time"].as_str().unwrap(), "{name}: time");
                let pi_rtsp = pi["rtsp_id"].as_str().unwrap().trim_end_matches('-');
                assert_eq!(
                    o.rtsp_id.as_deref().unwrap_or(""),
                    pi_rtsp,
                    "{name}: RTSP id"
                );
                agreed_on += 1;
            }
            (true, None) => refuses.push(name.clone()),
            (false, Some(_)) => accepts.push(name.clone()),
            (false, None) => agreed_on += 1,
        }
    }
    refuses.sort();
    accepts.sort();
    let mut want_refuses: Vec<String> = OURS_REFUSES.iter().map(|s| (*s).to_owned()).collect();
    let mut want_accepts: Vec<String> = OURS_ACCEPTS.iter().map(|s| (*s).to_owned()).collect();
    want_refuses.sort();
    want_accepts.sort();
    assert_eq!(
        refuses, want_refuses,
        "names BirdNET-Pi reads and this project refuses"
    );
    assert_eq!(
        accepts, want_accepts,
        "names this project reads and BirdNET-Pi refuses"
    );
    assert_eq!(agreed_on + refuses.len() + accepts.len(), 33);
}
