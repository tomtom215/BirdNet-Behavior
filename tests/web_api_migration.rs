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
//!
//! # The upload is two steps now
//!
//! `POST /admin/migrate/upload` used to validate and immediately import,
//! discarding every non-required check on the way — including the one that says
//! "this file was recorded 18 700 km from here". It now stages the file and
//! renders the same report the Server Path tab shows, and
//! `POST /admin/migrate/upload/confirm` is what actually imports. Every test
//! below therefore goes through both, and
//! [`uploading_shows_the_report_before_importing_anything`] is the one that
//! pins the gap between them.

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

/// Build a multipart body carrying `db_bytes` as the `source_file` field,
/// plus any plain-text fields the upload form sends alongside it.
fn multipart_upload(db_bytes: &[u8], boundary: &str, fields: &[(&str, &str)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; \
                 name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
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
    post_upload_with(app, db_bytes, &[]).await
}

async fn post_upload_with(
    app: &axum::Router,
    db_bytes: &[u8],
    fields: &[(&str, &str)],
) -> (StatusCode, String) {
    let boundary = "----birdnetmigrationtest";
    let req = Request::builder()
        .method("POST")
        .uri("/admin/migrate/upload")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(multipart_upload(db_bytes, boundary, fields)))
        .expect("build request");
    let resp = app.clone().oneshot(req).await.expect("upload response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Confirm the staged upload — the second half of the upload journey.
async fn post_confirm(app: &axum::Router) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/admin/migrate/upload/confirm")
        .body(Body::empty())
        .expect("build confirm request");
    let resp = app.clone().oneshot(req).await.expect("confirm response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Upload, check the report came back, then confirm — what a browser user does.
async fn upload_and_confirm(app: &axum::Router, db_bytes: &[u8]) -> (StatusCode, String) {
    upload_and_confirm_with(app, db_bytes, &[]).await
}

async fn upload_and_confirm_with(
    app: &axum::Router,
    db_bytes: &[u8],
    fields: &[(&str, &str)],
) -> (StatusCode, String) {
    let (status, body) = post_upload_with(app, db_bytes, fields).await;
    assert_eq!(status, StatusCode::OK, "upload rejected: {body}");
    assert!(
        body.contains("Start Import") || body.contains("Import this file"),
        "the upload did not return a report with a confirm button: {body}"
    );
    post_confirm(app).await
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

    // ---- the operator uploads their database, reviews it, and confirms ----
    let (status, body) = upload_and_confirm(&app, &db_bytes).await;
    assert_eq!(status, StatusCode::OK, "confirm rejected: {body}");
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
    let (status, body) = upload_and_confirm(&app, &db_bytes).await;
    assert_eq!(status, StatusCode::OK, "second confirm rejected: {body}");
    await_import_finished(&app).await;
    assert_eq!(
        detection_count(&dst_path),
        4,
        "a re-upload must not duplicate history — the rows with no File_Name \
         are the ones that used to double, silently, while reporting success"
    );
}

/// A BirdNET-Pi database recorded somewhere else entirely.
///
/// Perth, Western Australia: 18 700 km and eight hours of clock from a station
/// in Boston. Every row carries the source station's coordinates, which is what
/// BirdNET-Pi writes and what makes the origin knowable at all.
fn foreign_birdnet_pi_db(path: &std::path::Path) {
    const PERTH: (f64, f64) = (-31.95, 115.86);
    let conn = Connection::open(path).expect("open source");
    conn.execute_batch(
        "CREATE TABLE detections (
            Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
            Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
            Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
    )
    .expect("create source schema");
    for (date, time, sci, com) in [
        (
            "2025-09-01",
            "06:12:00",
            "Cracticus tibicen",
            "Australian Magpie",
        ),
        (
            "2025-09-01",
            "06:41:00",
            "Malurus splendens",
            "Splendid Fairywren",
        ),
        (
            "2025-09-02",
            "17:08:00",
            "Corvus coronoides",
            "Australian Raven",
        ),
    ] {
        conn.execute(
            "INSERT INTO detections
               (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, File_Name)
             VALUES (?1, ?2, ?3, ?4, 0.9, ?5, ?6, NULL)",
            params![date, time, sci, com, PERTH.0, PERTH.1],
        )
        .expect("seed foreign row");
    }
}

/// The one row `import_batches` should hold after an upload.
fn only_import_batch(
    path: &std::path::Path,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<i64>, i64) {
    let conn = Connection::open(path).expect("open destination");
    conn.query_row(
        "SELECT station_lat, station_lon, distance_km, applied_shift_secs, row_count
           FROM import_batches ORDER BY id DESC LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .expect("an import batch was recorded")
}

/// The upload tab must show the report **before** importing anything.
///
/// # The defect this pins
///
/// `upload_and_run_handler` ran `validate_source_against_station`, refused the
/// file only if a **required** check failed, and then never read the report
/// again — the binding was literally
/// `let (schema, report, _migration_report) = …`. It then started the import.
///
/// `provenance::location_check` is deliberately never `required`; its own
/// comment says why: "merging two sites is a legitimate thing to want … the job
/// is to make that a decision instead of an accident." Being non-required is
/// exactly what meant it could not reach an operator on this tab. So uploading
/// 18 700 km of someone else's history showed no distance warning, no species
/// preview, no duplicate count and no confirmation step — while
/// `docs/book/guides/migration.md` step 4 described the Server-Path preview as
/// if it were both tabs.
///
/// Two assertions, and the second is the one that matters: the report has to
/// come back, *and* nothing may be in the database until the operator says so.
#[tokio::test]
async fn uploading_shows_the_report_before_importing_anything() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_path = dir.path().join("perth.db");
    foreign_birdnet_pi_db(&src_path);
    let db_bytes = std::fs::read(&src_path).expect("read source db");

    let dst_path = dir.path().join("station.db");
    let conn = Connection::open(&dst_path).expect("open dest");
    birdnet_db::migration::migrate(&conn).expect("migrate dest");
    for (key, value) in [("latitude", "42.36"), ("longitude", "-71.06")] {
        birdnet_db::settings::set(
            &conn,
            key,
            value,
            birdnet_db::settings::SettingsCategory::Location,
        )
        .expect("set station coordinate");
    }
    drop(conn);

    let state = AppState::new(dst_path.clone()).expect("state");
    let app = build_router(state);

    let (status, body) = post_upload(&app, &db_bytes).await;
    assert_eq!(status, StatusCode::OK, "upload rejected: {body}");

    // The report reached the operator.
    assert!(
        body.contains("source_location"),
        "the location check is missing from the upload report: {body}"
    );
    assert!(
        body.contains("check-warn") || body.contains("⚠"),
        "an 18 700 km source produced no warning: {body}"
    );
    assert!(
        body.contains("Australian Magpie"),
        "the species preview is missing from the upload report"
    );
    assert!(
        body.contains("Import this file"),
        "the upload report offers no way to proceed"
    );

    // …and nothing has been imported.
    assert_eq!(
        detection_count(&dst_path),
        0,
        "the upload imported before the operator confirmed"
    );
    let batches: i64 = Connection::open(&dst_path)
        .expect("open destination")
        .query_row("SELECT COUNT(*) FROM import_batches", [], |r| r.get(0))
        .expect("count batches");
    assert_eq!(batches, 0, "a batch was recorded before confirmation");

    // Confirming is what imports.
    let (status, body) = post_confirm(&app).await;
    assert_eq!(status, StatusCode::OK, "confirm rejected: {body}");
    await_import_finished(&app).await;
    assert_eq!(detection_count(&dst_path), 3, "confirming did not import");
}

/// A second confirm must not import the same file twice.
///
/// The staged slot is taken, not read, so a double-submitted button — an
/// impatient operator, a browser retry — cannot start two imports.
#[tokio::test]
async fn confirming_twice_imports_once() {
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

    let (status, _) = post_upload(&app, &db_bytes).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_confirm(&app).await;
    assert_eq!(status, StatusCode::OK);
    await_import_finished(&app).await;
    assert_eq!(detection_count(&dst_path), 4);

    // The second confirm has nothing staged and must say so rather than act.
    let (status, body) = post_confirm(&app).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("no longer staged"),
        "a second confirm did not report an empty stage: {body}"
    );
    assert_eq!(
        detection_count(&dst_path),
        4,
        "a second confirm re-imported the file"
    );
}

/// Uploading another station's history must record **where it came from**.
///
/// The station's own coordinates were already being read on this path — to
/// validate the file — and then thrown away: `upload_and_run_handler` called
/// `run_migration`, which is `run_migration_with_options` with
/// `ImportOptions::default()` and `station = (None, None)`. So `station_lat`,
/// `station_lon` and `distance_km` were all NULL on every browser import, and
/// the Patterns note that is supposed to name an imported foreign site — the
/// entire point of migration 26 and the `import_batches` read API — could never
/// fire, because it keys on a distance that was never computed.
///
/// Verified against a real running station before the fix: 3 000 Perth
/// detections uploaded to a Boston station produced `station_lat`,
/// `station_lon` and `distance_km` all NULL, an empty `/pages/provenance-note`,
/// and no warning on any screen.
#[tokio::test]
async fn uploading_another_stations_history_records_where_it_came_from() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_path = dir.path().join("perth.db");
    foreign_birdnet_pi_db(&src_path);
    let db_bytes = std::fs::read(&src_path).expect("read source db");

    let dst_path = dir.path().join("station.db");
    let conn = Connection::open(&dst_path).expect("open dest");
    birdnet_db::migration::migrate(&conn).expect("migrate dest");
    // Boston, as the setup wizard would have written it.
    birdnet_db::settings::set(
        &conn,
        "latitude",
        "42.36",
        birdnet_db::settings::SettingsCategory::Location,
    )
    .expect("set latitude");
    birdnet_db::settings::set(
        &conn,
        "longitude",
        "-71.06",
        birdnet_db::settings::SettingsCategory::Location,
    )
    .expect("set longitude");
    drop(conn);

    let state = AppState::new(dst_path.clone()).expect("state");
    let app = build_router(state);

    let (status, body) = upload_and_confirm(&app, &db_bytes).await;
    assert_eq!(status, StatusCode::OK, "confirm rejected: {body}");
    let progress = await_import_finished(&app).await;
    assert!(!progress.contains("Failed"), "import failed: {progress}");
    assert_eq!(detection_count(&dst_path), 3);

    let (lat, lon, distance, _shift, rows) = only_import_batch(&dst_path);
    assert_eq!(rows, 3, "the batch must count what it imported");
    assert!(
        lat.is_some() && lon.is_some(),
        "the receiving station's own coordinates must be recorded, not NULL"
    );
    let distance = distance.expect("a foreign import must record how far away it was");
    // Perth is nearly antipodal to Boston — its antipode sits in the Atlantic
    // off Bermuda — so the great-circle distance is ~18 700 km, not the
    // ~14 000 km a mental map suggests. The band is tight on purpose: it is
    // wide enough for any reasonable Earth radius and narrow enough that a
    // distance computed from the wrong pair of points would fall outside it.
    assert!(
        (18_000.0..19_500.0).contains(&distance),
        "Perth to Boston is about 18 700 km; got {distance}"
    );
}

/// And it must apply the clock reconciliation the operator asked for.
///
/// The upload tab had no field for it at all — the origin fieldset lived only
/// on the server-path tab, which needs the file to already be on the station's
/// filesystem. A browser user could not reconcile an import's clock even in
/// principle, so an eight-hour-out history merged into their own and every
/// hour-of-day analytic averaged the two together.
#[tokio::test]
async fn an_uploaded_import_applies_the_clock_shift_the_operator_gave() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_path = dir.path().join("perth.db");
    foreign_birdnet_pi_db(&src_path);
    let db_bytes = std::fs::read(&src_path).expect("read source db");

    let dst_path = dir.path().join("station.db");
    let conn = Connection::open(&dst_path).expect("open dest");
    birdnet_db::migration::migrate(&conn).expect("migrate dest");
    drop(conn);
    let state = AppState::new(dst_path.clone()).expect("state");
    let app = build_router(state);

    // Perth is UTC+8. The importer no longer applies a flat shift — it converts
    // each timestamp individually from the source's offset onto this host's
    // clock *for that date* — so what the batch records is the source offset the
    // operator gave, and `applied_shift_secs` stays 0. See `to_local_here`.
    let here = birdnet_db::clock::local_utc_offset_secs();
    let moves_the_clock = here != 8 * 3600;
    let (status, body) = upload_and_confirm_with(
        &app,
        &db_bytes,
        &[
            ("source_label", "Hollow Oak, Perth"),
            ("source_utc_offset_secs", "28800"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "confirm rejected: {body}");
    let progress = await_import_finished(&app).await;
    assert!(!progress.contains("Failed"), "import failed: {progress}");

    let (_, _, _, shift, _) = only_import_batch(&dst_path);
    assert_eq!(
        shift,
        Some(0),
        "no flat shift is applied any more; the source offset is what is recorded"
    );

    let recorded_offset: Option<i64> = Connection::open(&dst_path)
        .expect("open destination")
        .query_row(
            "SELECT source_utc_offset_secs FROM import_batches ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("read offset");
    assert_eq!(
        recorded_offset,
        Some(8 * 3600),
        "the batch must record the offset the conversion used, so the import \
         stays explainable"
    );

    let conn = Connection::open(&dst_path).expect("open destination");
    let label: Option<String> = conn
        .query_row(
            "SELECT source_label FROM import_batches ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("read label");
    assert_eq!(label.as_deref(), Some("Hollow Oak, Perth"));

    // 06:12 in Perth is not 06:12 here, unless the runner happens to be on
    // UTC+8 — in which case nothing moves and there is nothing to check.
    if moves_the_clock {
        let unshifted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Time = '06:12:00'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(
            unshifted, 0,
            "no row should still carry the source station's wall clock"
        );
    }
}
