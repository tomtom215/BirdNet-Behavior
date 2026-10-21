//! The reads `tests/a_failed_read_is_not_defaulted_into_a_fact.rs` found, and the
//! pages that printed a default for each.
//!
//! The root crate's sweep lists every place a read error is turned into a value
//! inside the closure that made the read. Reading the 225 hits, these were the
//! ones where the value then said something to the operator that the station had
//! not measured:
//!
//! * the species page's "Best detection" said "No recording captured for this
//!   species yet" for a bird whose recordings the database could not list;
//! * "No per-species thresholds configured" for a list it could not read, which
//!   invites adding a row that a failed read made look absent;
//! * the backup panel said a scheduled backup had never run;
//! * a blacklisted image was served because the blacklist could not be asked;
//! * the live source picker said no source was configured; `/stream` said "no such
//!   audio source" (404) or "no audio source configured" (503) for a table it could
//!   not read;
//! * the Today pill said "recording" because the freshness read had failed;
//! * the BirdWeather test said "token not configured" for a settings table it could
//!   not read;
//! * the sign-in said "Incorrect username or password" for a users table it could
//!   not read;
//! * the Migration cards said "No arrivals yet" and "Quiet week";
//! * a share link said the sighting did not exist.
//!
//! Each test drives a station whose table has gone, beside the working station
//! with the same data and the empty station, because the discrimination matters
//! as much as the alarm: a station that truly has nothing must still say so.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A station with `statements` run against it after it is seeded.
fn station(statements: &[&str], seed: impl FnOnce(&rusqlite::Connection)) -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    seed(&conn);
    for sql in statements {
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("could not break the station with {sql:?}: {e}"));
    }
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

const fn nothing(_: &rusqlite::Connection) {}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    form: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let (status, _, body) = send(app, "GET", uri, "").await;
    (status, body)
}

fn one_clip(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-05-01', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.55, 'b.wav')",
        [],
    )
    .expect("seed detection");
}

/// A detection whose clip retention has reclaimed: the row is kept, the audio is not.
fn one_reclaimed_clip(conn: &rusqlite::Connection) {
    one_clip(conn);
    conn.execute(
        "UPDATE detections SET Clip_Pruned_At = '2026-05-02 00:00:00'",
        [],
    )
    .expect("reclaim");
}

// ── the species page's best recording ────────────────────────────────────

#[tokio::test]
async fn the_species_hero_does_not_say_no_recording_when_it_could_not_look() {
    let uri = "/pages/species-hero?name=Eurasian+Blackbird";

    let (status, body) = get(&station(&["DROP TABLE detections"], one_clip), uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.contains("No recording captured"),
        "a failed read was reported as a species with no recording: {body}"
    );
    assert!(
        !body.contains("clips reclaimed"),
        "a failed read was reported as reclaimed clips: {body}"
    );
    assert!(body.contains("couldn't load"), "{body}");

    // Counterparts: the same station working says each of its three true things.
    let (_, body) = get(&station(&[], one_clip), uri).await;
    assert!(body.contains("b.wav"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(
        body.contains("No recording captured for this species yet"),
        "{body}"
    );
    assert!(!body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], one_reclaimed_clip), uri).await;
    assert!(body.contains("reclaimed"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");
}

// ── per-species thresholds ──────────────────────────────────────────────────

fn one_threshold(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO species_thresholds (sci_name, confidence_threshold) VALUES ('Turdus merula', 0.8)",
        [],
    )
    .expect("seed threshold");
}

/// A threshold row whose `created_at` no reader can decode: the write of a
/// different species goes through, and the read that follows it does not.
fn a_threshold_the_list_cannot_read(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO species_thresholds (sci_name, confidence_threshold, created_at)
         VALUES ('Unreadable bird', 0.5, X'ff')",
        [],
    )
    .expect("seed unreadable threshold");
}

#[tokio::test]
async fn the_thresholds_list_does_not_say_none_configured_when_it_could_not_look() {
    let uri = "/admin/species/thresholds";
    let (_, body) = get(&station(&["DROP TABLE species_thresholds"], nothing), uri).await;
    assert!(
        !body.contains("No per-species thresholds configured"),
        "a failed read was reported as an empty list: {body}"
    );
    assert!(body.contains("couldn't load"), "{body}");

    // Counterparts: a station with none says so, and one with a row lists it.
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(
        body.contains("No per-species thresholds configured"),
        "{body}"
    );
    assert!(!body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], one_threshold), uri).await;
    assert!(body.contains("Turdus merula"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");
}

/// The write went through; the list that follows it could not be read. The
/// answer must not be the list re-rendered as "none configured", under a
/// success the operator then trusts.
#[tokio::test]
async fn a_threshold_saved_but_not_redrawn_says_so() {
    let app = station(&[], a_threshold_the_list_cannot_read);
    let (status, _, body) = send(
        &app,
        "POST",
        "/admin/species/thresholds/set",
        "sci_name=Parus+major&threshold=0.6",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.contains("No per-species thresholds configured"),
        "a failed re-read was drawn as an empty list: {body}"
    );
    assert!(body.contains("couldn't load"), "{body}");
}

// ── the backup panel ────────────────────────────────────────────────────────

#[tokio::test]
async fn the_backup_panel_does_not_say_never_run_when_it_could_not_look() {
    let uri = "/station/data";
    let (_, body) = get(&station(&["DROP TABLE maintenance_runs"], nothing), uri).await;
    assert!(
        !body.contains("first automatic backup due shortly"),
        "a failed read was reported as a backup that has never run: {body}"
    );
    assert!(body.contains("could not be read"), "{body}");

    // Counterparts: a station that has never run it says so; one that has says when.
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(
        body.contains("first automatic backup due shortly"),
        "{body}"
    );
    assert!(!body.contains("could not be read"), "{body}");
    let ran = |conn: &rusqlite::Connection| {
        conn.execute(
            "INSERT INTO maintenance_runs (job, last_run_unix) VALUES (?1, strftime('%s','now'))",
            [birdnet_db::sqlite::JOB_BACKUP_VACUUM],
        )
        .expect("record a run");
    };
    let (_, body) = get(&station(&[], ran), uri).await;
    assert!(body.contains("scheduled backup"), "{body}");
    assert!(
        !body.contains("first automatic backup due shortly"),
        "{body}"
    );
}

// ── the live source picker and /stream ──────────────────────────────────────

fn one_source(conn: &rusqlite::Connection) {
    use birdnet_db::audio_sources::{AudioSourceStore as _, NewAudioSource, SourceKind};
    let new = NewAudioSource::defaults("src_a", SourceKind::UsbAlsa, "plughw:CARD=PRO,DEV=0");
    conn.insert(&new).expect("seed source");
}

#[tokio::test]
async fn the_live_source_picker_does_not_say_no_source_when_it_could_not_look() {
    let uri = "/recordings?view=live";
    let (_, body) = get(&station(&["DROP TABLE audio_sources"], nothing), uri).await;
    assert!(
        !body.contains("no source yet"),
        "a failed read was reported as a station with no microphone: {body}"
    );
    assert!(body.contains("couldn't read the sources"), "{body}");

    // Counterparts: no microphone says so; one microphone is offered.
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(body.contains("no source yet"), "{body}");
    assert!(!body.contains("couldn't read the sources"), "{body}");
    let (_, body) = get(&station(&[], one_source), uri).await;
    assert!(body.contains(r#"value="src_a""#), "{body}");
    assert!(!body.contains("couldn't read the sources"), "{body}");
}

#[tokio::test]
async fn the_stream_does_not_say_no_source_when_it_could_not_look() {
    let broken = || station(&["DROP TABLE audio_sources"], nothing);

    // No `?source_id=`: "no audio source configured" is a statement about the
    // station's configuration.
    let (status, body) = get(&broken(), "/stream").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        !body.contains("no audio source configured"),
        "a failed read was reported as an unconfigured station: {body}"
    );
    assert!(body.contains("could not be read"), "{body}");

    // A named source: 404 says the source does not exist.
    let (status, body) = get(&broken(), "/stream?source_id=src_a").await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a failed read was reported as a source that does not exist: {body}"
    );
    assert!(!body.contains("no such audio source"), "{body}");

    // Counterparts: a station with none, and a name that is not there.
    let (status, body) = get(&station(&[], nothing), "/stream").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.contains("no audio source configured"), "{body}");
    let (status, body) = get(&station(&[], one_source), "/stream?source_id=nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body.contains("no such audio source"), "{body}");
}

// ── the Today recording pill, the outage banner and the health line ─────────

/// A station whose supervisor has not published a gauge yet but whose
/// detection daemon is running: the first seconds after a start, where the
/// pill falls back on the detection-freshness read.
fn just_started(conn: &rusqlite::Connection) {
    one_source(conn);
}

fn station_just_started(statements: &[&str]) -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    just_started(&conn);
    for sql in statements {
        conn.execute_batch(sql).expect("break the station");
    }
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    state
        .detection_status_flag()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    build_router(state)
}

#[tokio::test]
async fn the_recording_pill_is_not_green_when_the_freshness_read_failed() {
    for uri in ["/pages/today-pills", "/pages/station-health-line"] {
        let (_, body) = get(&station_just_started(&["DROP TABLE detections"]), uri).await;
        assert!(
            !body.contains("bnb-dot live"),
            "{uri}: a pill with nothing to go on was drawn as recording: {body}"
        );
        assert!(
            body.contains("recording state unknown"),
            "{uri}: the pill does not say it could not look: {body}"
        );

        // Counterpart: the same station, working, has nothing contradicting
        // the gauge-less start and keeps the pill it always had.
        let (_, body) = get(&station_just_started(&[]), uri).await;
        assert!(body.contains("bnb-dot live"), "{uri}: {body}");
        assert!(!body.contains("recording state unknown"), "{uri}: {body}");
    }
}

#[tokio::test]
async fn the_outage_banner_does_not_vanish_when_the_freshness_read_failed() {
    let uri = "/pages/today-nudge";
    let (_, body) = get(&station_just_started(&["DROP TABLE detections"]), uri).await;
    assert!(
        body.contains("couldn't load"),
        "a strip that is absent when nothing is wrong was absent because it could not look: {body:?}"
    );

    // Counterpart: a working station with nothing wrong is silent.
    let (_, body) = get(&station_just_started(&[]), uri).await;
    assert!(body.trim().is_empty(), "{body:?}");
}

// ── the BirdWeather test ────────────────────────────────────────────────────

#[tokio::test]
async fn the_birdweather_test_does_not_say_no_token_when_it_could_not_look() {
    let uri = "/admin/notifications/test/birdweather";
    let (_, _, body) = send(&station(&["DROP TABLE settings"], nothing), "POST", uri, "").await;
    assert!(
        !body.contains("token not configured"),
        "a failed read was reported as a missing token: {body}"
    );
    assert!(body.contains("could not be read"), "{body}");

    // Counterpart: a settings table that reads, with no token in it.
    let (_, _, body) = send(&station(&[], nothing), "POST", uri, "").await;
    assert!(body.contains("token not configured"), "{body}");
    assert!(!body.contains("could not be read"), "{body}");
}

// ── the rule test button ────────────────────────────────────────────────────

#[tokio::test]
async fn testing_a_rule_that_could_not_be_read_is_not_a_rule_that_is_gone() {
    let uri = "/admin/rules/1/test";
    let (status, _, body) = send(
        &station(&["DROP TABLE alert_rules"], nothing),
        "POST",
        uri,
        "",
    )
    .await;
    // htmx never swaps a 4xx/5xx body, so a failed button press has to arrive as
    // a 200 carrying the failure; and "404" would say the rule does not exist.
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("could not be read"), "{body}");

    // Counterpart: a rule that is not there is not there.
    let (status, _, _) = send(&station(&[], nothing), "POST", uri, "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── signing in ──────────────────────────────────────────────────────────────

fn a_known_admin(conn: &rusqlite::Connection) {
    let hash = birdnet_db::accounts::hash_password(&format!("pw-{}-sweep", std::process::id()))
        .expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [hash],
    )
    .expect("set the admin password");
}

fn login_form(user: &str) -> String {
    format!(
        "username={user}&password=not-the-password-{}",
        std::process::id()
    )
}

#[tokio::test]
async fn a_users_table_the_login_could_not_read_is_not_a_wrong_password() {
    let (status, headers, body) = send(
        &station(&["DROP TABLE users"], nothing),
        "POST",
        "/login",
        &login_form("admin"),
    )
    .await;
    let location = headers
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(status.is_redirection(), "{status}: {body}");
    assert!(
        !location.contains("error=1"),
        "a failed read was reported as an incorrect password: {location}"
    );

    // The page that redirect lands on says what happened, and does not blame
    // the password.
    let page = get(
        &station(&[], nothing),
        location.trim_start_matches("http://localhost"),
    )
    .await
    .1;
    assert!(page.contains("could not look up"), "{page}");
    assert!(!page.contains("Incorrect username or password"), "{page}");

    // Counterparts: a wrong password for an account that exists, and a name that
    // does not, are both the credentials error.
    for user in ["admin", "nobody"] {
        let (_, headers, body) = send(
            &station(&[], a_known_admin),
            "POST",
            "/login",
            &login_form(user),
        )
        .await;
        let location = headers
            .get("location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(location.contains("error=1"), "{user}: {location} {body}");
    }
}

// ── the Migration cards ─────────────────────────────────────────────────────

fn heard_today(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES (date('now','localtime'), '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'b.wav')",
        [],
    )
    .expect("seed detection");
}

#[tokio::test]
async fn the_migration_cards_do_not_say_quiet_when_they_could_not_look() {
    for (kind, quiet) in [("arrived", "No arrivals yet"), ("peaking", "Quiet week")] {
        let uri = format!("/pages/migration-card?kind={kind}");
        let (_, body) = get(&station(&["DROP TABLE detections"], heard_today), &uri).await;
        assert!(
            !body.contains(quiet),
            "{kind}: a failed read was reported as {quiet:?}: {body}"
        );
        assert!(body.contains("couldn't load"), "{kind}: {body}");

        // Counterparts: nothing heard is quiet; a bird heard is named.
        let (_, body) = get(&station(&[], nothing), &uri).await;
        assert!(body.contains(quiet), "{kind}: {body}");
        assert!(!body.contains("couldn't load"), "{kind}: {body}");
        let (_, body) = get(&station(&[], heard_today), &uri).await;
        assert!(body.contains("Eurasian Blackbird"), "{kind}: {body}");
    }
}

// ── share links ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_share_link_does_not_say_the_clip_is_gone_when_the_station_could_not_look() {
    let token =
        birdnet_web::routes::share::issue_token_for("2026-05-01", "06:10:00", "Eurasian Blackbird");
    let uri = format!("/r/{token}");
    let (status, body) = get(&station(&["DROP TABLE detections"], one_clip), &uri).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a failed read was reported as a link that never existed: {body}"
    );
    assert!(!body.contains("This clip is gone"), "{body}");

    // Counterparts: the clip is shown; and a sighting that is not there is gone.
    let (status, body) = get(&station(&[], one_clip), &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = get(&station(&[], nothing), &uri).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body.contains("This clip is gone"), "{body}");
}

// ── inviting a viewer ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_blank_invite_label_is_no_label() {
    let pw = format!("viewer-pass-{}", std::process::id());
    for (label, stored) in [
        ("", None),
        ("+++", None),
        ("Bob+Smith", Some("Bob Smith")),
        ("++Bob++", Some("Bob")),
    ] {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        let app = build_router(state.clone());
        let (status, _, body) = send(
            &app,
            "POST",
            "/admin/accounts/users",
            &format!("username=bob&password={pw}&label={label}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let got: Option<String> = state.with_read_db(|conn| {
            conn.query_row("SELECT label FROM users WHERE username = 'bob'", [], |r| {
                r.get(0)
            })
            .expect("the invited user exists")
        });
        assert_eq!(
            got.as_deref(),
            stored,
            "label {label:?} was stored as {got:?}"
        );
    }
}

// ── a route nothing linked to ───────────────────────────────────────────────

/// `/pages/quarantine-pending-count` drew a count badge for a nav entry that was
/// never built: the Quarantine page already shows "Pending Review" in its stats
/// card, and Today's review strip carries the same number. Nothing in the tree
/// requested it, so it is not served.
#[tokio::test]
async fn the_pending_count_partial_nothing_linked_to_is_not_served() {
    let (status, _) = get(&station(&[], nothing), "/pages/quarantine-pending-count").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The page it was meant for still says how many are waiting.
    let (_, body) = get(&station(&[], nothing), "/pages/quarantine-stats").await;
    assert!(body.contains("Pending Review"), "{body}");
}
