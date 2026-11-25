//! A comment stays on its detection when the detection is relabelled (MSC-22,
//! `TECH_DEBT` TD-9).
//!
//! A comment names its detection by date, time, species and clip. Relabelling
//! changes the species, and nothing moved the comment: the relabelled
//! detection's thread was empty, and the old thread was on no detection's page.
//! A comment is about what was heard in the clip, so it follows the clip; often
//! it is the reason for the relabel ("too short for a Hairy, I think Downy").

use birdnet_db::detection_comments::{self, NewComment};
use birdnet_db::sqlite::{DetectionKey, relabel_detection_at};
use rusqlite::Connection;

const DATE: &str = "2026-05-01";
const TIME: &str = "06:00:00";
const HAIRY: &str = "Dryobates villosus";
const DOWNY: &str = "Dryobates pubescens";

fn station() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for clip in ["garden.wav", "camera.wav"] {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES (?1, ?2, ?3, 'Hairy Woodpecker', 0.8, ?4)",
            rusqlite::params![DATE, TIME, HAIRY, clip],
        )
        .expect("seed");
    }
    conn
}

fn comment(conn: &Connection, clip: &str, body: &str) {
    detection_comments::insert(
        conn,
        &NewComment {
            date: DATE,
            time: TIME,
            sci_name: HAIRY,
            file_name: Some(clip),
            user_id: None,
            author: "ada",
            body,
        },
    )
    .expect("comment");
}

fn thread(conn: &Connection, sci: &str, clip: &str) -> Vec<String> {
    detection_comments::list(conn, DATE, TIME, sci, Some(clip))
        .expect("list")
        .into_iter()
        .map(|c| c.body)
        .collect()
}

/// The defect: comment, relabel, and the comment is read from the relabelled
/// detection. The counterpart: the other source's detection of the same
/// second, not relabelled, keeps its own comment under its own name.
#[test]
fn a_relabelled_detection_keeps_its_comments_and_its_neighbour_keeps_its_own() {
    let conn = station();
    comment(&conn, "garden.wav", "Too short for a Hairy; I think Downy.");
    comment(&conn, "camera.wav", "This one is a Hairy.");

    let relabelled = relabel_detection_at(
        &conn,
        &DetectionKey {
            date: DATE,
            time: TIME,
            sci_name: HAIRY,
            file_name: Some("garden.wav"),
        },
        DOWNY,
        "Downy Woodpecker",
    )
    .expect("relabel");
    assert!(relabelled, "precondition: the detection was relabelled");

    assert_eq!(
        thread(&conn, DOWNY, "garden.wav"),
        ["Too short for a Hairy; I think Downy."],
        "the relabelled detection's comment must be on it"
    );
    assert!(
        thread(&conn, HAIRY, "garden.wav").is_empty(),
        "no thread is left behind under a name no detection has"
    );
    assert_eq!(
        thread(&conn, HAIRY, "camera.wav"),
        ["This one is a Hairy."],
        "the other source's detection is untouched"
    );
}

/// A comment still cannot be moved by hand to a detection that does not
/// exist: following a relabel is not licence to rewrite the row.
#[test]
fn a_comment_cannot_be_moved_to_a_detection_that_is_not_there() {
    let conn = station();
    comment(&conn, "garden.wav", "A note.");
    let err = conn
        .execute(
            "UPDATE detection_comments SET sci_name = 'Picus viridis'",
            [],
        )
        .expect_err("a stray rename must be refused");
    assert!(err.to_string().contains("append-only"), "{err}");
    let err = conn
        .execute("UPDATE detection_comments SET body = 'edited'", [])
        .expect_err("an edit must be refused");
    assert!(err.to_string().contains("append-only"), "{err}");
}
