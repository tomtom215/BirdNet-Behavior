//! What the operator types into a search box is text, not a `LIKE` pattern.
//!
//! # The defect (DB9)
//!
//! Every free-text search wrapped the term as `%{term}%` and handed it to
//! `LIKE` unescaped, so `%` and `_` in the term were wildcards. A search for
//! `_` matched every detection, and the legacy exclusion `NOT _` — "hide the
//! names containing an underscore" — hid every detection on the station.
//!
//! # What is guarded
//!
//! Each `birdnet-db` search path finds a name containing the literal character
//! and nothing else: the Search page's filter and count, the Recordings clip
//! list and its count, and the species search. The counterparts check that an
//! ordinary substring still matches, and that the exclusion still excludes.

use birdnet_db::sqlite::{
    DetectionFilter, RecordingsFilter, recent_clips, recent_clips_count, search_detection_count,
    search_detections, search_species,
};
use rusqlite::{Connection, params};

/// Three detections with clips; one name holds a literal `%` and `_`.
fn station() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    for (time, sci, com) in [
        ("06:00:00", "Parus major", "Great Tit"),
        ("07:00:00", "Turdus merula", "Eurasian Blackbird"),
        ("08:00:00", "Unknown sp.", "Test_bird 100%"),
    ] {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-01', ?1, ?2, ?3, 0.8, ?4)",
            params![time, sci, com, format!("{time}.mp3")],
        )
        .unwrap();
    }
    conn
}

fn filter_names(conn: &Connection, text: &str) -> Vec<String> {
    let filter = DetectionFilter {
        text: Some(text.to_owned()),
        ..DetectionFilter::default()
    };
    let rows = search_detections(conn, &filter, 50, 0).unwrap();
    let count = search_detection_count(conn, &filter).unwrap();
    assert_eq!(
        usize::try_from(count).unwrap(),
        rows.len(),
        "count agrees with rows"
    );
    let mut names: Vec<String> = rows.into_iter().map(|r| r.com_name).collect();
    names.sort();
    names
}

fn clip_names(conn: &Connection, text: &str) -> Vec<String> {
    let rows = recent_clips(conn, RecordingsFilter::All, Some(text), 50, 0).unwrap();
    let count = recent_clips_count(conn, RecordingsFilter::All, Some(text)).unwrap();
    assert_eq!(
        usize::try_from(count).unwrap(),
        rows.len(),
        "count agrees with rows"
    );
    let mut names: Vec<String> = rows.into_iter().map(|r| r.com_name).collect();
    names.sort();
    names
}

fn species_names(conn: &Connection, text: &str) -> Vec<String> {
    let mut names: Vec<String> = search_species(conn, text, 50)
        .unwrap()
        .into_iter()
        .map(|s| s.com_name)
        .collect();
    names.sort();
    names
}

const LITERAL: &str = "Test_bird 100%";

#[test]
fn the_search_page_matches_wildcard_characters_literally() {
    let conn = station();
    assert_eq!(filter_names(&conn, "_"), [LITERAL]);
    assert_eq!(filter_names(&conn, "%"), [LITERAL]);
    assert_eq!(
        filter_names(&conn, "NOT _"),
        ["Eurasian Blackbird", "Great Tit"],
        "the exclusion hid every detection"
    );
    // Counterparts: an ordinary substring, and an ordinary exclusion.
    assert_eq!(filter_names(&conn, "tit"), ["Great Tit"]);
    assert_eq!(filter_names(&conn, "NOT tit").len(), 2);
}

#[test]
fn the_clip_list_matches_wildcard_characters_literally() {
    let conn = station();
    assert_eq!(clip_names(&conn, "_"), [LITERAL]);
    assert_eq!(clip_names(&conn, "%"), [LITERAL]);
    assert_eq!(
        clip_names(&conn, "NOT _"),
        ["Eurasian Blackbird", "Great Tit"]
    );
    assert_eq!(clip_names(&conn, "tit"), ["Great Tit"]);
}

#[test]
fn the_species_search_matches_wildcard_characters_literally() {
    let conn = station();
    assert_eq!(species_names(&conn, "_"), [LITERAL]);
    assert_eq!(species_names(&conn, "%"), [LITERAL]);
    assert_eq!(
        species_names(&conn, "TIT"),
        ["Great Tit"],
        "still case-insensitive"
    );
}
