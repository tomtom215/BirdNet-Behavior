//! A bulk species action the database refused is not reported as "0 removed",
//! and refusing it leaves the audio alone.
//!
//! `/admin/species/manage/{detections,clips}` defaulted a database error to
//! `BulkOutcome { 0, 0 }` with `.unwrap_or_default()`. The page then said
//! "…: 0 clips reclaimed.", the audit log recorded `deleted=0` as though the
//! action had run and found nothing, and — worse — the clips handler went on
//! to unlink every file of the species even though marking the rows reclaimed
//! had failed. That is the order `prune_species_clips` documents as the one to
//! avoid ("a file removed without the mark offers an operator a player for
//! audio that is gone"), and an unlink cannot be taken back.
//!
//! The refusal is a trigger, which is how this suite makes SQLite say "no" to
//! one statement and not another. A refused write is not audited at all: the
//! audit log records what changed, and the `warn` that carries the refusal is
//! persisted.

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

const SCI: &str = "Troglodytes troglodytes";
const CLIPS: [&str; 3] = [
    "By_Date/2026-05-01/wren-1.wav",
    "By_Date/2026-05-01/wren-2.wav",
    "By_Date/2026-05-01/wren-3.wav",
];

struct Station {
    state: AppState,
    dir: tempfile::TempDir,
}

/// A station with three wren detections, each with a clip on disk, and
/// optionally a trigger that refuses `event` on `detections`.
fn station(refuse: Option<&str>) -> Station {
    let dir = tempfile::tempdir().expect("recordings dir");
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for (i, file) in CLIPS.iter().enumerate() {
        let path = dir.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(&path, b"RIFF....WAVE").expect("clip");
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-01', ?1, ?2, 'Eurasian Wren', 0.9, ?3)",
            rusqlite::params![format!("06:1{i}:00"), SCI, file],
        )
        .expect("seed");
    }
    if let Some(event) = refuse {
        conn.execute_batch(&format!(
            "CREATE TRIGGER refuse BEFORE {event} ON detections
             BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;"
        ))
        .expect("trigger");
    }
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_recording_dir(dir.path().to_path_buf());
    Station { state, dir }
}

async fn post(station: &Station, uri: &str) -> (u16, Option<String>, String) {
    let res = build_router(station.state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("sci_name={}", SCI.replace(' ', "+"))))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let reswap = res
        .headers()
        .get("hx-reswap")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, reswap, String::from_utf8_lossy(&body).into_owned())
}

fn files_on_disk(station: &Station) -> usize {
    CLIPS
        .iter()
        .filter(|f| station.dir.path().join(f).exists())
        .count()
}

fn rows(station: &Station) -> i64 {
    station.state.with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap()
    })
}

fn marked(station: &Station) -> i64 {
    station.state.with_db(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM detections WHERE Clip_Pruned_At IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
    })
}

fn audit_rows(station: &Station, action_like: &str) -> Vec<(String, Option<String>)> {
    station.state.with_db(|c| {
        let mut stmt = c
            .prepare("SELECT action, metadata FROM audit_log WHERE action LIKE ?1 ORDER BY id")
            .unwrap();
        stmt.query_map([action_like], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    })
}

// ── the clips ───────────────────────────────────────────────────────────

/// **The irreversible one.** The mark is an `UPDATE`; the store refuses it.
/// No file may be unlinked, the page must not say "0 clips reclaimed", and the
/// audit log must not carry a successful `species.clips.delete`.
#[tokio::test]
async fn a_clip_mark_the_database_refused_removes_no_file() {
    let st = station(Some("UPDATE"));
    let (status, reswap, body) = post(&st, "/admin/species/manage/clips").await;

    assert_eq!(status, 200, "htmx discards the body of a 4xx/5xx: {body}");
    assert_eq!(
        files_on_disk(&st),
        CLIPS.len(),
        "a clip was unlinked although marking its row failed"
    );
    assert_eq!(
        reswap.as_deref(),
        Some("none"),
        "the page was swapped as if the action had run: {body}"
    );
    assert!(
        !body.contains("0 clips reclaimed"),
        "a refused write was reported as a quiet success: {body}"
    );
    assert!(
        body.contains("were not deleted") && body.contains("No audio file was removed"),
        "the toast must say what is certainly true: {body}"
    );
    assert_eq!(marked(&st), 0);
    assert!(
        audit_rows(&st, "species.%").is_empty(),
        "audited as a delete that ran: {:?}",
        audit_rows(&st, "species.%")
    );
}

/// The counterpart, and the proof the test above has teeth: with nothing
/// refusing, the same request removes all three files and says so.
#[tokio::test]
async fn a_clip_reclaim_that_worked_removes_the_files_and_says_how_many() {
    let st = station(None);
    let (status, reswap, body) = post(&st, "/admin/species/manage/clips").await;

    assert_eq!(status, 200);
    assert_eq!(reswap, None, "{body}");
    assert!(body.contains("3 clips reclaimed"), "{body}");
    assert_eq!(files_on_disk(&st), 0, "the clips should be gone");
    assert_eq!(marked(&st), 3);
    assert_eq!(rows(&st), 3, "the rows stay: only the audio goes");
    let ok = audit_rows(&st, "species.clips.delete");
    assert_eq!(ok.len(), 1, "{ok:?}");
    assert_eq!(ok[0].0, "species.clips.delete");
    assert!(
        ok[0]
            .1
            .as_deref()
            .unwrap_or("")
            .contains("marked=3 files_removed=3"),
        "{ok:?}"
    );
}

// ── the detections ──────────────────────────────────────────────────────

/// A `DELETE` the store refuses is "the database refused", not "0 detections
/// removed" — and not an audit row that says `deleted=0`.
#[tokio::test]
async fn a_detection_delete_the_database_refused_is_not_reported_as_zero() {
    let st = station(Some("DELETE"));
    let (status, reswap, body) = post(&st, "/admin/species/manage/detections").await;

    assert_eq!(status, 200, "htmx discards the body of a 4xx/5xx: {body}");
    assert_eq!(reswap.as_deref(), Some("none"), "{body}");
    assert!(
        !body.contains("0 detections removed"),
        "a refused write was reported as a quiet success: {body}"
    );
    assert!(body.contains("were not deleted"), "{body}");
    assert_eq!(rows(&st), 3, "nothing may have been deleted");
    assert!(
        audit_rows(&st, "species.%").is_empty(),
        "audited as a delete that ran: {:?}",
        audit_rows(&st, "species.%")
    );
}

/// The counterpart: a delete that works is shown, counted and audited, and the
/// table is re-rendered (no `HX-Reswap`).
#[tokio::test]
async fn a_detection_delete_that_worked_is_counted_and_audited() {
    let st = station(None);
    let (status, reswap, body) = post(&st, "/admin/species/manage/detections").await;

    assert_eq!(status, 200);
    assert_eq!(reswap, None, "{body}");
    assert!(body.contains("3 detections removed"), "{body}");
    assert_eq!(rows(&st), 0);
    let ok = audit_rows(&st, "species.detections.delete");
    assert_eq!(ok.len(), 1, "{ok:?}");
    assert!(
        ok[0].1.as_deref().unwrap_or("").contains("deleted=3"),
        "{ok:?}"
    );
}
