//! The BirdNET-Pi migration journey, driven through its real HTTP endpoints.
//!
//! Everything below this layer was already covered — the importer, the CSV
//! parser, the validator all have their own tests. The journey an operator
//! actually takes was not: nothing posted a database to `/admin/migrate/upload`
//! or read back what the page then claims. That gap is why a re-import could
//! silently double someone's history while reporting success.
//!
//! The re-upload case is the point of this file. A first import proves the
//! plumbing; a second one over the same database is what a real person does
//! after a timeout, a browser refresh, or simple uncertainty about whether it
//! worked — and it must leave the data exactly as the first one did.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use rusqlite::{Connection, params};
use tower::ServiceExt as _;

/// A BirdNET-Pi database as the wild produces them: `File_Name` populated on
/// some rows, absent on others (extraction disabled, older schema, hand-edited
/// exports). Both kinds must survive a re-import without multiplying.
fn birdnet_pi_db(path: &std::path::Path) {
    let conn = Connection::open(path).expect("open source");
    conn.execute_batch(
        "CREATE TABLE detections (
            Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
            Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
            Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
    )
    .expect("create source schema");
    for (date, time, sci, com, file) in [
        (
            "2026-01-01",
            "06:00:00",
            "Turdus merula",
            "Blackbird",
            Some("a.wav"),
        ),
        (
            "2026-01-01",
            "06:05:00",
            "Parus major",
            "Great Tit",
            Some("b.wav"),
        ),
        ("2026-01-02", "07:00:00", "Turdus merula", "Blackbird", None),
        (
            "2026-01-02",
            "07:05:00",
            "Erithacus rubecula",
            "Robin",
            None,
        ),
    ] {
        conn.execute(
            "INSERT INTO detections
               (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES (?1, ?2, ?3, ?4, 0.9, ?5)",
            params![date, time, sci, com, file],
        )
        .expect("seed source row");
    }
}

/// Build a multipart body carrying `db_bytes` as the `source_file` field.
fn multipart_upload(db_bytes: &[u8], boundary: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"source_file\"; \
             filename=\"birds.db\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(db_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

async fn post_upload(app: &axum::Router, db_bytes: &[u8]) -> (StatusCode, String) {
    let boundary = "----birdnetmigrationtest";
    let req = Request::builder()
        .method("POST")
        .uri("/admin/migrate/upload")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(multipart_upload(db_bytes, boundary)))
        .expect("build request");
    let resp = app.clone().oneshot(req).await.expect("upload response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Poll `/admin/migrate/progress` the way the page does, until the import
/// reaches a terminal stage.
///
/// The import runs in the background — `/upload` returns a polling shell, not a
/// finished result — so a test that asserted straight after the upload would be
/// racing it. Terminal state is signalled exactly as the browser sees it: the
/// fragment stops carrying its `hx-trigger`, which is what makes htmx stop
/// asking.
async fn await_import_finished(app: &axum::Router) -> String {
    for _ in 0..200 {
        let req = Request::builder()
            .uri("/admin/migrate/progress")
            .body(Body::empty())
            .expect("build progress request");
        let resp = app.clone().oneshot(req).await.expect("progress response");
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read progress body");
        let body = String::from_utf8_lossy(&bytes).into_owned();
        if !body.contains("hx-trigger") {
            return body;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("import did not reach a terminal stage");
}

fn detection_count(path: &std::path::Path) -> i64 {
    let conn = Connection::open(path).expect("open destination");
    conn.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count detections")
}

#[tokio::test]
async fn uploading_a_birdnet_pi_database_imports_it_once_and_only_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_path = dir.path().join("birds-pi.db");
    birdnet_pi_db(&src_path);
    let db_bytes = std::fs::read(&src_path).expect("read source db");

    let dst_path = dir.path().join("station.db");
    let conn = Connection::open(&dst_path).expect("open dest");
    birdnet_db::migration::migrate(&conn).expect("migrate dest");
    drop(conn);
    let state = AppState::new(dst_path.clone()).expect("state");
    let app = build_router(state);

    // ---- the operator uploads their database -----------------------------
    let (status, body) = post_upload(&app, &db_bytes).await;
    assert_eq!(status, StatusCode::OK, "upload rejected: {body}");
    let progress = await_import_finished(&app).await;
    assert!(
        !progress.contains("Failed") && !progress.contains("err"),
        "first import did not succeed: {progress}"
    );
    assert_eq!(
        detection_count(&dst_path),
        4,
        "all four BirdNET-Pi rows should land, clip-less ones included"
    );

    // ---- and then, unsure it worked, uploads it again ---------------------
    let (status, body) = post_upload(&app, &db_bytes).await;
    assert_eq!(status, StatusCode::OK, "second upload rejected: {body}");
    await_import_finished(&app).await;
    assert_eq!(
        detection_count(&dst_path),
        4,
        "a re-upload must not duplicate history — the rows with no File_Name \
         are the ones that used to double, silently, while reporting success"
    );
}
