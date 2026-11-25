//! A detection's review history follows it through a relabel, and the history
//! cannot be deleted from (N24-01).
//!
//! `review_events` (migration 64) refused every `UPDATE`, which kept a
//! verdict on record from being rewritten, but it took no `DELETE` guard: a
//! single statement could remove who had verified a record. And because a
//! row's `sci_name` can never change, a relabelled detection's history stayed
//! under the old name, where nothing reading the detection would look:
//! `review_history` for the detection as it is now returned nothing, though
//! two people had judged it.
//!
//! The history is not rewritten to follow the relabel. A relabel appends an
//! event under the new name that says what happened and links back, and the
//! current verdict that event leaves is the one migration 59 leaves: none. A
//! verdict was a judgement of the old species, so the detection is unreviewed
//! under its new one, in the history and in `detection_reviews` alike.

use birdnet_db::sqlite::{
    DetectionKey, ReviewStatus, Reviewer, get_detection_review, relabel_detection_at,
    review_history, set_detection_review_by,
};
use rusqlite::Connection;

const CLIP: &str = "2026-09-08-birdnet-06:00:00.wav";

const OWL: DetectionKey<'static> = DetectionKey {
    date: "2026-09-08",
    time: "06:00:00",
    sci_name: "Strix aluco",
    file_name: Some(CLIP),
};

const LONG_EARED: DetectionKey<'static> = DetectionKey {
    sci_name: "Asio otus",
    ..OWL
};

fn station() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES (?1, ?2, ?3, 'Tawny Owl', 0.81, ?4)",
        rusqlite::params![OWL.date, OWL.time, OWL.sci_name, CLIP],
    )
    .expect("seed detection");
    conn
}

fn review(conn: &Connection, name: &str, status: ReviewStatus) {
    let by = Reviewer {
        user_id: None,
        name: Some(name),
        via: "review",
    };
    assert!(set_detection_review_by(conn, &OWL, "Tawny Owl", status, None, &by).expect("review"));
}

/// `(sci_name, reviewer_name, verdict, via)` of each event in `key`'s history.
fn said(
    conn: &Connection,
    key: &DetectionKey<'_>,
) -> Vec<(String, Option<String>, Option<String>, String)> {
    review_history(conn, key)
        .expect("history")
        .into_iter()
        .map(|e| (e.sci_name, e.reviewer_name, e.verdict, e.via))
        .collect()
}

fn owned(
    rows: &[(&str, Option<&str>, Option<&str>, &str)],
) -> Vec<(String, Option<String>, Option<String>, String)> {
    rows.iter()
        .map(|(s, n, v, via)| {
            (
                (*s).to_owned(),
                n.map(str::to_owned),
                v.map(str::to_owned),
                (*via).to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_review_event_cannot_be_deleted() {
    let conn = station();
    review(&conn, "ada", ReviewStatus::Confirmed);
    let deleted = conn.execute("DELETE FROM review_events", []);
    assert!(deleted.is_err(), "a DELETE removed {deleted:?} event(s)");
    assert_eq!(said(&conn, &OWL).len(), 1, "the event is still there");
}

/// The defect: after a relabel, the detection's history is empty.
#[test]
fn a_relabelled_detection_keeps_its_review_history() {
    let conn = station();
    review(&conn, "ada", ReviewStatus::Confirmed);
    review(&conn, "grace", ReviewStatus::Rejected);

    assert!(relabel_detection_at(&conn, &OWL, LONG_EARED.sci_name, "Long-eared Owl").unwrap());

    assert_eq!(
        said(&conn, &LONG_EARED),
        owned(&[
            ("Strix aluco", Some("ada"), Some("confirmed"), "review"),
            ("Strix aluco", Some("grace"), Some("rejected"), "review"),
            ("Asio otus", None, None, "relabel"),
        ]),
        "the verdicts given under the old name, then the relabel that left the detection \
         unreviewed"
    );
    // The history's last word and the current verdict agree: none.
    assert!(get_detection_review(&conn, &LONG_EARED).unwrap().is_none());
    let mirrored: Option<String> = conn
        .query_row("SELECT review_verdict FROM detections", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mirrored, None);
    let from: Option<String> = conn
        .query_row(
            "SELECT relabelled_from FROM review_events WHERE via = 'relabel'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        from.as_deref(),
        Some("Strix aluco"),
        "the relabel names the old species"
    );

    // Nothing was rewritten: the events under the old name are still there,
    // unchanged.
    assert_eq!(
        said(&conn, &OWL),
        owned(&[
            ("Strix aluco", Some("ada"), Some("confirmed"), "review"),
            ("Strix aluco", Some("grace"), Some("rejected"), "review"),
        ]),
    );

    // A verdict given under the new name follows the relabel event.
    let by = Reviewer {
        user_id: None,
        name: Some("ada"),
        via: "review",
    };
    assert!(
        set_detection_review_by(
            &conn,
            &LONG_EARED,
            "Long-eared Owl",
            ReviewStatus::Confirmed,
            None,
            &by
        )
        .unwrap()
    );
    assert_eq!(said(&conn, &LONG_EARED).len(), 4);
}

/// Relabelled twice, back to the first name: each event appears once, in the
/// order it was written.
#[test]
fn a_detection_relabelled_there_and_back_lists_each_event_once() {
    let conn = station();
    review(&conn, "ada", ReviewStatus::Confirmed);
    assert!(relabel_detection_at(&conn, &OWL, LONG_EARED.sci_name, "Long-eared Owl").unwrap());
    assert!(relabel_detection_at(&conn, &LONG_EARED, OWL.sci_name, "Tawny Owl").unwrap());
    assert_eq!(
        said(&conn, &OWL),
        owned(&[
            ("Strix aluco", Some("ada"), Some("confirmed"), "review"),
            ("Asio otus", None, None, "relabel"),
            ("Strix aluco", None, None, "relabel"),
        ]),
    );
}

/// The counterpart: a relabel of a detection nobody reviewed writes no event.
/// The history is of verdicts, and there were none to follow.
#[test]
fn relabelling_an_unreviewed_detection_writes_no_event() {
    let conn = station();
    assert!(relabel_detection_at(&conn, &OWL, LONG_EARED.sci_name, "Long-eared Owl").unwrap());
    assert!(said(&conn, &LONG_EARED).is_empty());
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM review_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(events, 0);
}
