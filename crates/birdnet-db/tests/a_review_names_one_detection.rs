//! A review is a verdict on one detection, not on a date, time and species.
//!
//! Two microphones that hear one bird in the same second write two detections
//! that share `(Date, Time, Sci_Name)` and differ in the clip they point at
//! (`idx_detections_unique`). A review keyed on the triple reached both: a
//! verdict on the one the reviewer listened to also rejected, and so withdrew
//! from every count, a detection nobody had listened to.
//!
//! And a verdict is about a species. Relabelling the detection changed the
//! name under it and left the verdict behind: the queue then listed the row as
//! unreviewed while the analytics still excluded it as rejected.

use birdnet_db::sqlite::queries::species::species_summary_drift;
use birdnet_db::sqlite::{
    DetectionKey, DetectionRecord, ReviewStatus, detection_at, detection_with_clip,
    get_detection_review, insert_detection, relabel_detection_at, set_detection_review,
    unreviewed_recent_detections,
};
use rusqlite::Connection;

const DATE: &str = "2026-09-08";
const TIME: &str = "06:00:00";
const SCI: &str = "Turdus merula";
const COM: &str = "Eurasian Blackbird";

fn station() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn
}

const fn heard<'a>(clip: &'a str, source: &'a str) -> DetectionRecord<'a> {
    DetectionRecord {
        model_id: None,
        model_agreement: None,
        date: DATE,
        time: TIME,
        sci_name: SCI,
        com_name: COM,
        confidence: 0.9,
        lat: None,
        lon: None,
        cutoff: None,
        week: None,
        sensitivity: None,
        overlap: None,
        file_name: clip,
        chunk_offset_secs: Some(0.0),
        correlation_id: None,
        source: Some(source),
        duration_secs: Some(6.0),
        detected_at_utc: None,
        run_id: None,
        clip_offset_secs: None,
        detection_secs: None,
    }
}

/// The key of the detection that points at `clip`, as heard as `SCI`.
const fn key(clip: &str) -> DetectionKey<'_> {
    DetectionKey {
        date: DATE,
        time: TIME,
        sci_name: SCI,
        file_name: Some(clip),
    }
}

/// `(clip, review_verdict)` of every detection, by clip.
fn verdicts(conn: &Connection) -> Vec<(String, Option<String>)> {
    conn.prepare("SELECT File_Name, review_verdict FROM detections ORDER BY File_Name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The reviewer listened to the clip from microphone A and rejected it. The
/// detection from microphone B was never reviewed and must not carry the
/// verdict.
#[test]
fn a_verdict_on_one_sources_detection_does_not_reach_anothers() {
    let conn = station();
    insert_detection(&conn, &heard("mic_a.wav", "mic-a")).unwrap();
    insert_detection(&conn, &heard("mic_b.wav", "mic-b")).unwrap();
    assert_eq!(verdicts(&conn).len(), 2, "precondition: two detections");

    assert!(
        set_detection_review(&conn, &key("mic_a.wav"), COM, ReviewStatus::Rejected, None).unwrap()
    );

    let after = verdicts(&conn);
    assert_eq!(
        after[1],
        ("mic_b.wav".to_owned(), None),
        "microphone B's detection was never reviewed: {after:?}"
    );
    assert_eq!(after[0].1.as_deref(), Some("rejected"), "and A's was");
    assert!(
        get_detection_review(&conn, &key("mic_b.wav"))
            .unwrap()
            .is_none(),
        "the review log agrees with the mirror"
    );
}

/// A verdict that said "this is not a Blackbird" says nothing about a Great
/// Tit. After a relabel the row is unreviewed everywhere, so the queue and the
/// analytics agree.
#[test]
fn a_relabelled_detection_does_not_keep_a_verdict_about_its_old_species() {
    let conn = station();
    insert_detection(&conn, &heard("clip.wav", "mic-a")).unwrap();
    set_detection_review(&conn, &key("clip.wav"), COM, ReviewStatus::Rejected, None).unwrap();

    assert!(relabel_detection_at(&conn, &key("clip.wav"), "Parus major", "Great Tit").unwrap());

    let verdict: Option<String> = conn
        .query_row(
            "SELECT review_verdict FROM detections WHERE Sci_Name = 'Parus major'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let queued = unreviewed_recent_detections(&conn, 50)
        .unwrap()
        .iter()
        .any(|d| d.sci_name == "Parus major");
    assert!(
        queued && verdict.is_none(),
        "the review queue lists the relabelled row as unreviewed ({queued}) \
         but the row still carries {verdict:?}"
    );
    // What the reviewer decided is history, not erased: the review stays, marked
    // as being about a detection that no longer has that name.
    let history: Vec<(String, Option<String>)> = conn
        .prepare("SELECT sci_name, detached FROM detection_reviews")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        history,
        [(SCI.to_owned(), Some("relabelled".to_owned()))],
        "the verdict on the old species is kept, detached"
    );
}

/// Relabelling back does not bring the old verdict back to life: it was
/// detached, so the detection is unreviewed under its old name too, and the
/// two stores still agree.
#[test]
fn relabelling_back_does_not_resurrect_the_old_verdict() {
    let conn = station();
    insert_detection(&conn, &heard("clip.wav", "mic-a")).unwrap();
    set_detection_review(&conn, &key("clip.wav"), COM, ReviewStatus::Rejected, None).unwrap();
    relabel_detection_at(&conn, &key("clip.wav"), "Parus major", "Great Tit").unwrap();
    let as_tit = DetectionKey {
        sci_name: "Parus major",
        ..key("clip.wav")
    };
    relabel_detection_at(&conn, &as_tit, SCI, COM).unwrap();

    assert_eq!(verdicts(&conn), [("clip.wav".to_owned(), None)]);
    assert!(
        get_detection_review(&conn, &key("clip.wav"))
            .unwrap()
            .is_none(),
        "a verdict that was detached is no detection's verdict"
    );
    // And it can be reviewed afresh, as a new verdict beside the old history.
    assert!(
        set_detection_review(&conn, &key("clip.wav"), COM, ReviewStatus::Confirmed, None).unwrap()
    );
    let reviews: i64 = conn
        .query_row("SELECT COUNT(*) FROM detection_reviews", [], |r| r.get(0))
        .unwrap();
    assert_eq!(reviews, 2, "the old verdict and the new one");
    assert_eq!(
        get_detection_review(&conn, &key("clip.wav"))
            .unwrap()
            .map(|r| r.status),
        Some("confirmed".to_owned())
    );
}

/// The counterpart that keeps the trigger from being a blanket alarm: an
/// update that does not change the species leaves the verdict alone.
#[test]
fn an_update_that_does_not_rename_the_species_keeps_the_verdict() {
    let conn = station();
    insert_detection(&conn, &heard("clip.wav", "mic-a")).unwrap();
    set_detection_review(&conn, &key("clip.wav"), COM, ReviewStatus::Rejected, None).unwrap();

    conn.execute_batch(
        "UPDATE detections SET is_locked = 1, Com_Name = 'Blackbird (edited)';
         UPDATE detections SET Sci_Name = Sci_Name;",
    )
    .unwrap();

    assert_eq!(
        verdicts(&conn),
        [("clip.wav".to_owned(), Some("rejected".to_owned()))]
    );
    assert!(
        get_detection_review(&conn, &key("clip.wav"))
            .unwrap()
            .is_some()
    );
}

/// The corrected detection counts as what it now is: it is in the analytic
/// view under its new name, and the maintained species summary agrees with the
/// table it summarises.
#[test]
fn a_relabelled_detection_counts_under_its_new_species() {
    let conn = station();
    insert_detection(&conn, &heard("clip.wav", "mic-a")).unwrap();
    set_detection_review(&conn, &key("clip.wav"), COM, ReviewStatus::Rejected, None).unwrap();
    relabel_detection_at(&conn, &key("clip.wav"), "Parus major", "Great Tit").unwrap();

    let analytic: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM detections_analytic WHERE Sci_Name = 'Parus major'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(analytic, 1, "the corrected detection must count");
    let drift = species_summary_drift(&conn).unwrap();
    assert!(drift.is_empty(), "the species summary drifted: {drift:#?}");
}

/// A page opened from a list row knows the row's clip, and must get that row:
/// `detection_at` takes whichever of two same-second detections comes first.
#[test]
fn a_page_can_open_the_detection_its_clip_names() {
    let conn = station();
    insert_detection(&conn, &heard("mic_a.wav", "mic-a")).unwrap();
    insert_detection(&conn, &heard("mic_b.wav", "mic-b")).unwrap();

    let b = detection_with_clip(&conn, DATE, TIME, Some(COM), "mic_b.wav")
        .unwrap()
        .expect("the second microphone's detection");
    assert_eq!(b.file_name.as_deref(), Some("mic_b.wav"));
    assert_eq!(b.source.as_deref(), Some("mic-b"));
    assert!(
        detection_with_clip(&conn, DATE, TIME, None, "other.wav")
            .unwrap()
            .is_none(),
        "a clip that names no detection opens none"
    );
    assert_eq!(
        detection_at(&conn, DATE, TIME, Some(COM))
            .unwrap()
            .unwrap()
            .file_name
            .as_deref(),
        Some("mic_a.wav"),
        "the lookup that ignores the clip still takes the first"
    );
}
