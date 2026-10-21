//! The audit log, written by the handlers rather than only by its own tests.
//!
//! Table, store, admin page and pruner all existed. `AuditLog::record` had
//! **zero production callers** — every call site was inside its own
//! `#[cfg(test)]` block — so `/admin/audit` was permanently empty. On a shared
//! station that does not read as "the log is broken"; it reads as "nothing
//! happened".
//!
//! The repo had already caught half of this: the *pruner* was wired after
//! being found to have no caller, and a 180-day retention constant was written
//! for it. Six months of retention on rows nobody wrote.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::Connection;
use tower::ServiceExt as _;

use birdnet_db::accounts::{self, AccountsError, AuditEntry, AuditLog, SessionStore, UserStore};
use birdnet_web::server::build_router;
use birdnet_web::session;
use birdnet_web::state::AppState;

/// The seed admin's password, built per process: the `CodeQL` query on
/// hard-coded cryptographic values follows a literal into any parameter called
/// `password` (CLAUDE.md); `format!` over a runtime value is not one.
fn admin_password() -> String {
    format!("admin-pw-{}", std::process::id())
}

/// A station with an admin account and a bound session cookie for it.
fn station() -> (AppState, String) {
    let conn = Connection::open_in_memory().expect("memory db");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let cookie = admin_session(&state);
    (state, cookie)
}

/// Give `state`'s seed admin a password and a session; return its cookie.
fn admin_session(state: &AppState) -> String {
    let sid = state
        .with_db(|conn| -> Result<String, AccountsError> {
            let admin = conn.find_user_by_name("admin")?;
            conn.set_password(admin.id, &accounts::hash_password(&admin_password())?)?;
            let sid = session::generate_session_id();
            conn.create_session(&sid, admin.id, "2999-01-01 00:00:00", None, None)?;
            Ok(sid)
        })
        .expect("seed admin");

    format!(
        "{}={}",
        session::COOKIE_NAME,
        session::issue_token(&sid, session::DEFAULT_TTL_MS)
    )
}

fn rows(state: &AppState) -> Vec<AuditEntry> {
    state
        .with_db(|conn| conn.recent(100))
        .expect("read the audit log")
}

fn actions(state: &AppState) -> Vec<String> {
    rows(state).into_iter().map(|e| e.action).collect()
}

async fn post_form(state: &AppState, uri: &str, cookie: Option<&str>, body: &str) -> StatusCode {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(axum::http::header::COOKIE, c);
    }
    build_router(state.clone())
        .oneshot(builder.body(Body::from(body.to_owned())).expect("request"))
        .await
        .expect("response")
        .status()
}

// ── the gate the finding named ──────────────────────────────────────────

#[tokio::test]
async fn a_bad_password_writes_exactly_one_failure_row() {
    let (state, _cookie) = station();
    assert!(rows(&state).is_empty(), "the log starts empty");

    post_form(
        &state,
        "/login",
        None,
        "username=admin&password=wrong-password",
    )
    .await;

    let entries = rows(&state);
    assert_eq!(entries.len(), 1, "exactly one row: {entries:?}");
    assert_eq!(entries[0].action, "auth.login.fail");
    assert_eq!(
        entries[0].target.as_deref(),
        Some("admin"),
        "the submitted username, so a name that does not exist is still recorded"
    );
    assert_eq!(
        entries[0].user_id, None,
        "there is no actor — that is why the column is nullable"
    );
}

#[tokio::test]
async fn a_good_password_writes_ok_and_not_fail() {
    // The counterpart the finding asks for, and the discrimination: a gate
    // that only checked "a login writes a row" would pass against a handler
    // that recorded `fail` for every attempt.
    let (state, _cookie) = station();

    post_form(
        &state,
        "/login",
        None,
        &format!("username=admin&password={}", admin_password()),
    )
    .await;

    let entries = rows(&state);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].action, "auth.login.ok");
    assert!(
        entries[0].user_id.is_some(),
        "a successful login has an actor"
    );
    assert!(
        !actions(&state).iter().any(|a| a == "auth.login.fail"),
        "and writes no failure row"
    );
}

#[tokio::test]
async fn every_attempt_is_recorded_not_just_the_first() {
    // A station being brute-forced is the case this log exists for. Three
    // attempts must leave three rows, not one deduplicated one.
    let (state, _cookie) = station();
    for _ in 0..3 {
        post_form(&state, "/login", None, "username=root&password=nope").await;
    }
    assert_eq!(
        actions(&state)
            .iter()
            .filter(|a| *a == "auth.login.fail")
            .count(),
        3
    );
}

// ── the other mutating surfaces ─────────────────────────────────────────

#[tokio::test]
async fn a_settings_save_records_which_keys_changed_and_never_their_values() {
    // `rtsp_url` is the setting that makes this matter: an RTSP URL routinely
    // carries `user:pass@` in its authority, which is why the support bundle
    // has a redactor for exactly this shape. Recording the value here would
    // put a camera password into a table `/admin/audit` renders in full.
    let (state, cookie) = station();

    let status = post_form(
        &state,
        "/admin/settings",
        Some(&cookie),
        "latitude=48.1&rtsp_url=rtsp%3A%2F%2Fadmin%3Ahunter2%40cam.local%2Fstream",
    )
    .await;
    assert!(status.is_success(), "{status}");

    let entries = rows(&state);
    let save = entries
        .iter()
        .find(|e| e.action == "settings.update")
        .unwrap_or_else(|| panic!("no settings.update row in {entries:?}"));
    let meta = save.metadata.as_deref().unwrap_or_default();
    assert!(
        meta.contains("latitude"),
        "the changed key is named: {meta}"
    );
    assert!(meta.contains("rtsp_url"), "and so is this one: {meta}");
    assert!(!meta.contains("48.1"), "values do not appear: {meta}");
    assert!(
        !meta.contains("hunter2") && !meta.contains("cam.local"),
        "and least of all this one: {meta}"
    );
}

#[tokio::test]
async fn saving_a_settings_form_that_changed_nothing_records_nothing() {
    // The discrimination. The settings page posts every field on every save,
    // so recording each submission turns the audit log into a click counter
    // and buries the save that moved the recording schedule.
    let (state, cookie) = station();
    post_form(&state, "/admin/settings", Some(&cookie), "latitude=48.1").await;
    let after_first = actions(&state);

    post_form(&state, "/admin/settings", Some(&cookie), "latitude=48.1").await;
    let after_second = actions(&state);

    assert_eq!(
        after_first, after_second,
        "the second, identical save added a row: {after_second:?}"
    );
}

#[tokio::test]
async fn clearing_the_detection_history_is_recorded_against_the_operator() {
    let (state, cookie) = station();
    let status = post_form(&state, "/admin/system/clear-detections", Some(&cookie), "").await;
    assert!(status.is_success(), "{status}");

    let entries = rows(&state);
    let cleared = entries
        .iter()
        .find(|e| e.action == "data.detections.clear")
        .unwrap_or_else(|| panic!("no clear row in {entries:?}"));
    assert!(
        cleared.user_id.is_some(),
        "an authenticated destructive action has an actor"
    );
}

#[tokio::test]
async fn an_unauthenticated_mutation_writes_no_row_because_it_never_runs() {
    // The counterpart that stops the audit log becoming a log of rejections:
    // the RBAC layer refuses these before any handler sees them, so the
    // absence here is the auth middleware working, not the audit call missing.
    let (state, _cookie) = station();
    let status = post_form(&state, "/admin/system/clear-detections", None, "").await;
    assert!(
        status.is_client_error() || status.is_redirection(),
        "must not be allowed: {status}"
    );
    assert!(
        !actions(&state).iter().any(|a| a == "data.detections.clear"),
        "{:?}",
        actions(&state)
    );
}

#[tokio::test]
async fn the_admin_audit_page_shows_what_was_recorded() {
    // End to end: the page that was permanently empty.
    let (state, cookie) = station();
    post_form(&state, "/login", None, "username=admin&password=wrong").await;

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/admin/audit")
                .header(axum::http::header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("auth.login.fail"),
        "the page must render the row that was written"
    );
}

// ── a detection changed from a page, and the other quiet mutations ──────
//
// The audit module's claim is that mutations are recorded. The API's write
// endpoints did record theirs; the pages that do the same things did not, and
// neither did four admin actions that delete or block something: so the same
// detection deleted through `/api/v2/detections/delete` left a row and through
// the Today list left nothing, and "who deleted the 40 wren clips?" could be
// answered only for the API.

/// A detection, as the page forms name it.
const ROW: &str = "date=2026-05-01&time=06%3A10%3A00&sci_name=Turdus+merula";
/// The same detection as an audit target: the schema's key, in its order.
const TARGET: &str = "2026-05-01 06:10:00 Turdus merula";

fn seed_detection(state: &AppState, time: &str, sci: &str) {
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-01', ?1, ?2, 'Eurasian Blackbird', 0.91, 'b.wav')",
            rusqlite::params![time, sci],
        )
        .expect("seed a detection");
    });
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: &str,
) -> StatusCode {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(axum::http::header::COOKIE, c);
    }
    build_router(state.clone())
        .oneshot(builder.body(Body::from(body.to_owned())).expect("request"))
        .await
        .expect("response")
        .status()
}

fn rows_of(state: &AppState, action: &str) -> Vec<AuditEntry> {
    rows(state)
        .into_iter()
        .filter(|e| e.action == action)
        .collect()
}

/// Every page that changes one detection, with the row it must leave.
#[tokio::test]
async fn a_detection_changed_from_a_page_is_recorded_against_the_operator() {
    for (uri, body, action, via) in [
        ("/pages/today-delete", ROW, "detection.delete", "via=today"),
        ("/pages/today-lock", ROW, "detection.lock", "via=today"),
        ("/pages/today-unlock", ROW, "detection.unlock", "via=today"),
        (
            "/pages/recordings-delete",
            ROW,
            "detection.delete",
            "via=recordings",
        ),
        (
            "/pages/recordings-lock",
            ROW,
            "detection.lock",
            "via=recordings",
        ),
        (
            "/pages/recordings-unlock",
            ROW,
            "detection.unlock",
            "via=recordings",
        ),
    ] {
        let (state, cookie) = station();
        seed_detection(&state, "06:10:00", "Turdus merula");
        let status = send(&state, "POST", uri, Some(&cookie), body).await;
        assert!(status.is_success(), "{uri}: {status}");

        let entries = rows_of(&state, action);
        assert_eq!(entries.len(), 1, "{uri}: {:?}", rows(&state));
        assert_eq!(entries[0].target.as_deref(), Some(TARGET), "{uri}");
        assert!(
            entries[0].user_id.is_some(),
            "{uri}: a change made from a signed-in page has an actor"
        );
        assert!(
            entries[0].metadata.as_deref().unwrap_or("").contains(via),
            "{uri}: {:?}",
            entries[0].metadata
        );
    }
}

#[tokio::test]
async fn relabelling_a_detection_records_what_it_became() {
    let (state, cookie) = station();
    seed_detection(&state, "06:10:00", "Turdus merula");
    let status = send(
        &state,
        "POST",
        "/pages/today-relabel",
        Some(&cookie),
        "date=2026-05-01&time=06%3A10%3A00&old_sci_name=Turdus+merula\
         &new_sci_name=Turdus+philomelos&new_com_name=Song+Thrush",
    )
    .await;
    assert!(status.is_success(), "{status}");

    let entries = rows_of(&state, "detection.relabel");
    assert_eq!(entries.len(), 1, "{:?}", rows(&state));
    assert_eq!(
        entries[0].target.as_deref(),
        Some(TARGET),
        "the target is what the row was"
    );
    assert!(
        entries[0]
            .metadata
            .as_deref()
            .unwrap_or("")
            .contains("to=Turdus philomelos"),
        "and the metadata what it became: {:?}",
        entries[0].metadata
    );
}

/// The discrimination: a row that is not there, or a write the store refused,
/// changed nothing and so records nothing — otherwise the log fills with
/// button presses and the entry that matters is buried in them.
#[tokio::test]
async fn a_page_change_that_changed_nothing_records_nothing() {
    // No such detection.
    let (state, cookie) = station();
    for uri in [
        "/pages/today-delete",
        "/pages/today-lock",
        "/pages/recordings-delete",
        "/pages/recordings-unlock",
    ] {
        let status = send(&state, "POST", uri, Some(&cookie), ROW).await;
        assert!(status.is_success(), "{uri}: {status}");
    }
    assert!(
        !actions(&state).iter().any(|a| a.starts_with("detection.")),
        "{:?}",
        rows(&state)
    );

    // A store that refuses the write.
    let (state, cookie) = station();
    seed_detection(&state, "06:10:00", "Turdus merula");
    state
        .with_db(|c| {
            c.execute_batch(
                "CREATE TRIGGER refuse BEFORE DELETE ON detections
                 BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
            )
        })
        .expect("trigger");
    let status = send(&state, "POST", "/pages/today-delete", Some(&cookie), ROW).await;
    assert!(status.is_success(), "{status}");
    assert!(
        rows_of(&state, "detection.delete").is_empty(),
        "audited as a delete that did not happen: {:?}",
        rows(&state)
    );
}

#[tokio::test]
async fn a_verdict_set_or_undone_from_a_page_is_recorded() {
    for (uri, body, verdict, via) in [
        (
            "/pages/detection-review",
            format!("{ROW}&com_name=Eurasian+Blackbird&status=rejected"),
            "verdict=rejected",
            "via=review",
        ),
        (
            "/pages/detection-review-inline",
            format!("{ROW}&com_name=Eurasian+Blackbird&status=confirmed"),
            "verdict=confirmed",
            "via=detection",
        ),
        (
            "/pages/detection-review-clear",
            ROW.to_owned(),
            "verdict=cleared",
            "via=review",
        ),
    ] {
        let (state, cookie) = station();
        seed_detection(&state, "06:10:00", "Turdus merula");
        let status = send(&state, "POST", uri, Some(&cookie), &body).await;
        assert!(status.is_success(), "{uri}: {status}");

        let entries = rows_of(&state, "detection.review");
        assert_eq!(entries.len(), 1, "{uri}: {:?}", rows(&state));
        assert_eq!(entries[0].target.as_deref(), Some(TARGET), "{uri}");
        assert!(entries[0].user_id.is_some(), "{uri}");
        let meta = entries[0].metadata.as_deref().unwrap_or("");
        assert!(
            meta.contains(verdict) && meta.contains(via),
            "{uri}: {meta}"
        );
    }

    // A status the handler ignores is not a verdict.
    let (state, cookie) = station();
    seed_detection(&state, "06:10:00", "Turdus merula");
    let status = send(
        &state,
        "POST",
        "/pages/detection-review",
        Some(&cookie),
        &format!("{ROW}&com_name=x&status=maybe"),
    )
    .await;
    assert!(status.is_success(), "{status}");
    assert!(rows_of(&state, "detection.review").is_empty());
}

#[tokio::test]
async fn the_search_pages_bulk_action_records_each_detection_it_changed() {
    let (state, cookie) = station();
    seed_detection(&state, "06:10:00", "Turdus merula");
    seed_detection(&state, "06:20:00", "Turdus merula");
    let status = send(
        &state,
        "POST",
        "/pages/search-bulk",
        Some(&cookie),
        "action=delete\
         &selected=2026-05-01%7C06%3A10%3A00%7CTurdus+merula\
         &selected=2026-05-01%7C06%3A20%3A00%7CTurdus+merula\
         &selected=2026-05-01%7C06%3A30%3A00%7CNo+such+bird",
    )
    .await;
    assert!(status.is_success(), "{status}");

    let mut targets: Vec<String> = rows_of(&state, "detection.delete")
        .into_iter()
        .map(|e| e.target.unwrap_or_default())
        .collect();
    targets.sort();
    assert_eq!(
        targets,
        vec![
            "2026-05-01 06:10:00 Turdus merula".to_owned(),
            "2026-05-01 06:20:00 Turdus merula".to_owned(),
        ],
        "one row per detection that went, none for the one that was not there: {:?}",
        rows(&state)
    );
    assert!(
        rows(&state)
            .iter()
            .all(|e| e.user_id.is_some() && e.metadata.as_deref() == Some("via=search")),
        "{:?}",
        rows(&state)
    );

    // An action the page does not offer does nothing and records nothing.
    let before = rows(&state).len();
    send(
        &state,
        "POST",
        "/pages/search-bulk",
        Some(&cookie),
        "action=explode&selected=2026-05-01%7C06%3A10%3A00%7CTurdus+merula",
    )
    .await;
    assert_eq!(rows(&state).len(), before);
}

#[tokio::test]
async fn deciding_a_quarantined_detection_is_recorded() {
    for (uri, action) in [
        ("/pages/quarantine-approve", "quarantine.approve"),
        ("/pages/quarantine-reject", "quarantine.reject"),
        ("/pages/quarantine-delete", "quarantine.delete"),
    ] {
        let (state, cookie) = station();
        let id = state.with_db(|conn| {
            birdnet_db::sqlite::insert_quarantine(
                conn,
                &birdnet_db::sqlite::QuarantineRecord {
                    date: "2026-05-01",
                    time: "06:10:00",
                    sci_name: "Upupa epops",
                    com_name: "Eurasian Hoopoe",
                    confidence: 0.42,
                    sf_probability: Some(0.61),
                    reason: birdnet_db::sqlite::QuarantineReason::LowConfidence,
                    file_name: Some("h.wav"),
                    lat: None,
                    lon: None,
                    week: Some(18),
                    run_id: None,
                    cutoff: Some(0.8),
                    sensitivity: Some(1.25),
                    overlap: Some(0.5),
                },
            )
            .expect("quarantine a detection");
            birdnet_db::sqlite::list_quarantine(
                conn,
                birdnet_db::sqlite::QuarantineFilter::Pending,
                10,
                0,
            )
            .expect("list")[0]
                .id
        });

        let status = send(&state, "POST", uri, Some(&cookie), &format!("id={id}")).await;
        assert!(status.is_success(), "{uri}: {status}");

        let entries = rows_of(&state, action);
        assert_eq!(entries.len(), 1, "{uri}: {:?}", rows(&state));
        assert_eq!(
            entries[0].target.as_deref(),
            Some(format!("quarantine:{id}").as_str())
        );
        assert!(entries[0].user_id.is_some(), "{uri}");
    }
}

#[tokio::test]
async fn blocking_a_species_picture_and_unblocking_it_are_recorded() {
    let (state, cookie) = station();
    let status = send(
        &state,
        "POST",
        "/admin/images/blacklist",
        Some(&cookie),
        "sci_name=Turdus+merula&url=https%3A%2F%2Fimg.example%2Fa.jpg%3Ftoken%3Dsecret",
    )
    .await;
    assert!(status.is_success(), "{status}");
    let added = rows_of(&state, "image.blacklist.add");
    assert_eq!(added.len(), 1, "{:?}", rows(&state));
    assert_eq!(added[0].target.as_deref(), Some("Turdus merula"));
    assert!(added[0].user_id.is_some());
    assert!(
        !format!("{:?}", added[0]).contains("secret"),
        "the URL, which can carry a token, is not recorded: {:?}",
        added[0]
    );

    let id: i64 = state.with_db(|c| {
        c.query_row("SELECT id FROM image_blacklist", [], |r| r.get(0))
            .expect("the entry")
    });
    let status = send(
        &state,
        "DELETE",
        &format!("/admin/images/blacklist/{id}"),
        Some(&cookie),
        "",
    )
    .await;
    assert!(status.is_success(), "{status}");
    let removed = rows_of(&state, "image.blacklist.remove");
    assert_eq!(removed.len(), 1, "{:?}", rows(&state));
    assert_eq!(
        removed[0].target.as_deref(),
        Some(format!("blacklist:{id}").as_str())
    );

    // Removing what is not there changes nothing, so records nothing.
    send(
        &state,
        "DELETE",
        "/admin/images/blacklist/9999",
        Some(&cookie),
        "",
    )
    .await;
    assert_eq!(rows_of(&state, "image.blacklist.remove").len(), 1);
}

#[tokio::test]
async fn pruning_the_notification_log_is_recorded_with_how_much_went() {
    let (state, cookie) = station();
    let status = send(
        &state,
        "DELETE",
        "/admin/notifications/prune",
        Some(&cookie),
        "",
    )
    .await;
    assert!(status.is_success(), "{status}");
    let entries = rows_of(&state, "notification.prune");
    assert_eq!(entries.len(), 1, "{:?}", rows(&state));
    assert!(entries[0].user_id.is_some());
    assert!(
        entries[0]
            .metadata
            .as_deref()
            .unwrap_or("")
            .contains("deleted=0"),
        "{:?}",
        entries[0].metadata
    );

    // A prune that failed deleted nothing and says so elsewhere; it is not a
    // prune in the log.
    let (state, cookie) = station();
    state
        .with_db(|c| c.execute_batch("DROP TABLE notification_log"))
        .expect("drop");
    send(
        &state,
        "DELETE",
        "/admin/notifications/prune",
        Some(&cookie),
        "",
    )
    .await;
    assert!(rows_of(&state, "notification.prune").is_empty());
}

#[tokio::test]
async fn deleting_a_backup_file_is_recorded_and_a_missing_one_is_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    let cookie = admin_session(&state);
    let backups = dir.path().join("backups");
    std::fs::create_dir_all(&backups).expect("mkdir");
    let name = "birds.db.backup.1733400000";
    std::fs::write(backups.join(name), b"not really a database").expect("write");

    let status = send(
        &state,
        "DELETE",
        &format!("/admin/system/backups/{name}"),
        Some(&cookie),
        "",
    )
    .await;
    assert!(status.is_success(), "{status}");
    assert!(!backups.join(name).exists(), "the file should be gone");
    let entries = rows_of(&state, "data.backup.delete");
    assert_eq!(entries.len(), 1, "{:?}", rows(&state));
    assert_eq!(entries[0].target.as_deref(), Some(name));
    assert!(entries[0].user_id.is_some());

    // The file is gone now; a second delete removes nothing and records nothing.
    send(
        &state,
        "DELETE",
        &format!("/admin/system/backups/{name}"),
        Some(&cookie),
        "",
    )
    .await;
    assert_eq!(rows_of(&state, "data.backup.delete").len(), 1);
}

// ── the vocabulary ──────────────────────────────────────────────────────

/// Every action string the production code writes.
///
/// `/admin/audit` filters with SQL `LIKE`, so an operator selects a family
/// with a prefix (`auth.%`, `species.%`). That only works while the names are
/// hierarchical and stable, and a name with one letter wrong would ship
/// silently: the row is written, the page renders it, and only the filter that
/// was supposed to catch it comes back empty.
///
/// This is the same lesson the station-health `CHECKS` table records: a set
/// that is only expressed as scattered call sites cannot be checked, so it is
/// written down once and compared against the source.
const ACTIONS: &[&str] = &[
    "account.password.set",
    "account.session.revoke",
    "account.session.revoke_others",
    "account.user.create",
    "account.user.delete",
    "audio.source.create",
    "audio.source.delete",
    "audio.source.listen_default",
    "audio.source.restart",
    "audio.source.update",
    "auth.login.fail",
    "auth.login.ok",
    "auth.login.throttled",
    "auth.logout",
    "classifier.install",
    "data.backup.delete",
    "data.backup.run",
    "data.database.restore",
    "data.detections.clear",
    "data.recordings.clear",
    "detection.comment.add",
    "detection.comment.delete",
    "detection.delete",
    "detection.lock",
    "detection.relabel",
    "detection.review",
    "detection.unlock",
    "image.blacklist.add",
    "image.blacklist.remove",
    "notification.prune",
    "quarantine.approve",
    "quarantine.delete",
    "quarantine.reject",
    "rule.create",
    "rule.delete",
    "rule.import",
    "rule.toggle",
    "settings.import_birdnet_pi",
    "settings.update",
    "species.clips.delete",
    "species.detections.delete",
    "species.exclude.add",
    "species.exclude.remove",
    "species.include.add",
    "species.include.remove",
    "species.interval.delete",
    "species.interval.set",
    "species.threshold.delete",
    "species.threshold.set",
    "species.whitelist.add",
    "species.whitelist.remove",
    "system.restart",
    "system.update.apply",
    "today.layout.change",
];

/// Action literals as they actually appear in the web crate's source.
///
/// Scanned rather than registered, because the call sites are handlers spread
/// across a dozen modules and a registry they had to opt into would simply be
/// the thing someone forgot.
fn actions_in_source() -> std::collections::BTreeSet<String> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("crates/birdnet-web/src");
    let mut found = std::collections::BTreeSet::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !line.contains("crate::audit::audit") {
                    continue;
                }
                // Window rather than line position: rustfmt puts the action on
                // its own line in a multi-line call and inline in a short one,
                // and a scanner that only understood one of those silently
                // missed four call sites when this was first written.
                //
                // Ten rather than seven because the action can itself be a
                // `match`, which rustfmt expands to one arm per line: the batch
                // endpoint's call in `routes/api_write.rs` puts four literals
                // at offsets 4..7, and at seven the last of them — the one that
                // deletes a detection — fell outside. Widening only ever finds
                // *more* literals, so it can strengthen the undocumented-action
                // assertion and never weaken it; checked at 7, 8, 9, 10, 12 and
                // 15 over the whole crate, the set found is identical (32) and
                // no unrelated string is mistaken for an action.
                for window_line in lines.iter().skip(i).take(10) {
                    for literal in window_line.split('"').skip(1).step_by(2) {
                        // Only dotted lowercase names. Targets are variables or
                        // `format!`s and metadata is `key=value`, so neither can
                        // be mistaken for an action.
                        // At least two non-empty dotted segments, so a bare
                        // `"."` — which the module doc's own prose contains —
                        // is not mistaken for an action name.
                        let segments: Vec<&str> = literal.split('.').collect();
                        if segments.len() >= 2
                            && segments.iter().all(|p| !p.is_empty())
                            && literal
                                .chars()
                                .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')
                        {
                            found.insert(literal.to_owned());
                        }
                    }
                }
            }
        }
    }
    found
}

#[test]
fn every_action_written_is_one_this_list_documents() {
    let found = actions_in_source();
    assert!(!found.is_empty(), "the scanner found no call sites at all");
    let known: std::collections::BTreeSet<String> =
        ACTIONS.iter().map(|s| (*s).to_owned()).collect();

    let undocumented: Vec<&String> = found.difference(&known).collect();
    assert!(
        undocumented.is_empty(),
        "these actions are written but not documented in ACTIONS — add them here \
         and to docs/book/admin/system.md: {undocumented:?}"
    );

    let unwritten: Vec<&String> = known.difference(&found).collect();
    assert!(
        unwritten.is_empty(),
        "these actions are documented but nothing writes them — the state this \
         whole change exists to fix: {unwritten:?}"
    );
}

#[test]
fn every_action_is_prefix_filterable() {
    // `/admin/audit`'s filter is SQL LIKE on a prefix, so a flat name is a
    // name nobody can select a family of.
    for action in ACTIONS {
        assert!(
            action.contains('.'),
            "{action} has no family prefix to filter on"
        );
        assert!(
            action.split('.').all(|part| !part.is_empty()),
            "{action} has an empty path segment"
        );
    }
}
