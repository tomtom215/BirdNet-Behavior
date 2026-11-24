//! Every verdict is kept, with who gave it and when, for as long as the
//! detection's data is kept (RES-06).
//!
//! `detection_reviews` holds one row per detection and is written with
//! `INSERT … ON CONFLICT DO UPDATE`, and it has no reviewer column. The only
//! place a reviewer was named was `audit_log`, which the maintenance job prunes
//! after `AUDIT_RETENTION_DAYS`. So a second reviewer silently replaced the
//! first, and six months later the station could not say who had verified any
//! record. A methods section has to say both.
//!
//! The history is `review_events`: append-only, one row per verdict given or
//! withdrawn. The detection's current verdict is derived from it by "the latest
//! wins", which is what `detection_reviews` and the mirrored
//! `detections.review_verdict` already hold.

use birdnet_db::accounts::{AuditLog as _, UserStore as _};
use birdnet_db::sqlite::{
    AUDIT_RETENTION_DAYS, DetectionKey, ReviewStatus, Reviewer, clear_detection_review_by,
    get_detection_review, review_history, set_detection_review_by,
};
use rusqlite::Connection;

const DATE: &str = "2026-09-08";
const TIME: &str = "06:00:00";
const SCI: &str = "Strix aluco";
const COM: &str = "Tawny Owl";
const CLIP: &str = "2026-09-08-birdnet-06:00:00.wav";

const KEY: DetectionKey<'static> = DetectionKey {
    date: DATE,
    time: TIME,
    sci_name: SCI,
    file_name: Some(CLIP),
};

fn station() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES (?1, ?2, ?3, ?4, 0.81, ?5)",
        rusqlite::params![DATE, TIME, SCI, COM, CLIP],
    )
    .expect("seed detection");
    conn
}

/// A user account, as the accounts page creates one; its id.
fn account(conn: &Connection, name: &str) -> i64 {
    conn.create_user(name, "", birdnet_db::accounts::Role::Admin, None)
        .expect("create user")
        .id
}

/// A verdict given the way the review page gives one: the verdict, then the
/// audit row naming the user.
fn review(conn: &Connection, user_id: i64, name: &str, status: ReviewStatus) {
    let recorded = set_detection_review_by(
        conn,
        &KEY,
        COM,
        status,
        None,
        &Reviewer {
            user_id: Some(user_id),
            name: Some(name),
            via: "review",
        },
    )
    .expect("review");
    assert!(
        recorded,
        "precondition: the detection exists, so the verdict is recorded"
    );
    conn.record(
        Some(user_id),
        "detection.review",
        Some(&format!("{DATE} {TIME} {SCI}")),
        Some(&format!(
            "via=review verdict={} clip={CLIP}",
            status.as_str()
        )),
    )
    .expect("audit");
}

/// Age every audit row past retention and run the real pruner.
fn prune_the_audit_log(conn: &Connection) {
    conn.execute(
        "UPDATE audit_log SET at = datetime('now', '-' || ?1 || ' days')",
        [AUDIT_RETENTION_DAYS + 30],
    )
    .expect("age audit rows");
    let pruned = conn.prune(AUDIT_RETENTION_DAYS).expect("prune");
    assert_eq!(
        pruned, 2,
        "precondition: both reviews' audit rows were pruned"
    );
}

/// The defect: two reviewers disagree in sequence, the audit log is pruned,
/// and both verdicts must still be readable with the names of who gave them.
#[test]
fn two_reviewers_who_disagree_both_stay_on_record_after_the_audit_log_is_pruned() {
    let conn = station();
    let ada = account(&conn, "ada");
    let grace = account(&conn, "grace");

    review(&conn, ada, "ada", ReviewStatus::Confirmed);
    review(&conn, grace, "grace", ReviewStatus::Rejected);
    prune_the_audit_log(&conn);

    let history = review_history(&conn, &KEY).expect("history");
    let said: Vec<(Option<&str>, Option<&str>)> = history
        .iter()
        .map(|e| (e.reviewer_name.as_deref(), e.verdict.as_deref()))
        .collect();
    assert_eq!(
        said,
        [
            (Some("ada"), Some("confirmed")),
            (Some("grace"), Some("rejected")),
        ],
        "both verdicts, oldest first, each with its reviewer's name"
    );
    assert_eq!(history[0].reviewer_user_id, Some(ada));
    assert_eq!(history[1].reviewer_user_id, Some(grace));
}

/// The rule the current verdict follows: the latest event wins, and a
/// withdrawn verdict leaves the detection unreviewed while the history keeps
/// both what was said and that it was withdrawn.
#[test]
fn the_current_verdict_is_the_latest_one_and_a_withdrawal_is_recorded_too() {
    let conn = station();
    let ada = account(&conn, "ada");
    let grace = account(&conn, "grace");

    review(&conn, ada, "ada", ReviewStatus::Confirmed);
    review(&conn, grace, "grace", ReviewStatus::Rejected);
    let current = get_detection_review(&conn, &KEY)
        .expect("read")
        .expect("reviewed");
    assert_eq!(current.status, "rejected", "the latest verdict wins");
    let mirrored: Option<String> = conn
        .query_row("SELECT review_verdict FROM detections", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mirrored.as_deref(), Some("rejected"));

    let withdrawn = clear_detection_review_by(
        &conn,
        &KEY,
        &Reviewer {
            user_id: Some(ada),
            name: Some("ada"),
            via: "review",
        },
    )
    .expect("clear");
    assert!(withdrawn);
    assert!(get_detection_review(&conn, &KEY).unwrap().is_none());

    let history = review_history(&conn, &KEY).expect("history");
    let said: Vec<(Option<&str>, Option<&str>)> = history
        .iter()
        .map(|e| (e.reviewer_name.as_deref(), e.verdict.as_deref()))
        .collect();
    assert_eq!(
        said,
        [
            (Some("ada"), Some("confirmed")),
            (Some("grace"), Some("rejected")),
            (Some("ada"), None),
        ],
        "a withdrawal is an event, recorded after the verdicts it withdrew"
    );
}

/// The counterpart: a verdict about a detection that does not exist writes no
/// event either. The history is of verdicts given, not of requests made.
#[test]
fn a_verdict_about_no_detection_leaves_no_event() {
    let conn = station();
    let missing = DetectionKey {
        file_name: Some("not-a-clip.wav"),
        ..KEY
    };
    let recorded = set_detection_review_by(
        &conn,
        &missing,
        COM,
        ReviewStatus::Rejected,
        None,
        &Reviewer {
            user_id: None,
            name: Some("ada"),
            via: "review",
        },
    )
    .expect("review");
    assert!(!recorded);
    assert!(review_history(&conn, &missing).unwrap().is_empty());
    assert!(review_history(&conn, &KEY).unwrap().is_empty());
}

/// The history is a record: an event cannot be rewritten after it is written,
/// by this crate's API or by a stray statement.
#[test]
fn a_review_event_can_never_be_rewritten() {
    let conn = station();
    let ada = account(&conn, "ada");
    review(&conn, ada, "ada", ReviewStatus::Confirmed);
    let err = conn
        .execute("UPDATE review_events SET reviewer_name = 'mallory'", [])
        .expect_err("an event must not be rewritten");
    assert!(err.to_string().contains("append-only"), "{err}");
    let err = conn
        .execute("UPDATE review_events SET verdict = 'rejected'", [])
        .expect_err("an event must not be rewritten");
    assert!(err.to_string().contains("append-only"), "{err}");
}
