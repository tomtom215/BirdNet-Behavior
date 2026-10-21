//! A read that failed is not published as a read that found nothing — the API
//! and the data-quality panels.
//!
//! Each surface here defaulted a failed query inside its `with_db` closure and
//! then printed the default as if the station had measured it:
//!
//! * `/api/v2/species/tracking` answered `200` with `"species": []` — a
//!   dashboard polling it would record a day on which nothing notable
//!   happened;
//! * `/api/v2/soundlevel` answered `200` "no sound level observations yet", or
//!   a spectrum with no bands and `samples: 0`;
//! * the data-quality panels drew "No detections in database.", "No data for
//!   the last 30 days.", a histogram of six zeros and "Database quality looks
//!   good!" for a database they had not been able to read.
//!
//! Each is driven against a station whose table has gone, with the working
//! station beside it, because the discrimination matters as much as the alarm:
//! a station that has truly heard nothing must still say so.
//!
//! The second half is the sites an earlier sweep classified and left: the
//! Today pills and review nudge, the merged-history note, the Recordings grid,
//! the live feed's badges, the microphone-health panel,
//! and the two admin writes that report their outcome honestly or not at all.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_db::accounts::UserStore as _;
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station(drop: &[&str], seed: impl FnOnce(&rusqlite::Connection)) -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    seed(&conn);
    for table in drop {
        conn.execute_batch(&format!("DROP TABLE {table};"))
            .expect("drop");
    }
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

const fn nothing(_: &rusqlite::Connection) {}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn one_detection(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-05-01', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.55, 'b.wav')",
        [],
    )
    .expect("seed detection");
}

fn one_sound_level(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO sound_level_broadband
             (date, hour, source, samples, a_power_sum, z_power_sum, calibration_db)
         VALUES ('2026-05-01', 6, 'mic', 10, 1.0, 1.0, 0.0)",
        [],
    )
    .expect("seed broadband");
    conn.execute(
        "INSERT INTO sound_levels
             (date, hour, source, band_hz, samples, mean_power_sum, min_db, max_db)
         VALUES ('2026-05-01', 6, 'mic', 1000.0, 10, 1.0, -60.0, -40.0)",
        [],
    )
    .expect("seed band");
}

// ── the API ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn species_tracking_that_could_not_read_is_an_error_not_an_empty_day() {
    let uri = "/api/v2/species/tracking?date=2026-05-01";
    let (status, body) = get(&station(&["detections"], nothing), uri).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(body.contains("\"error\""), "{body}");
    assert!(
        !body.contains("\"species\":[]"),
        "an empty list was published for a day that could not be read: {body}"
    );

    // Counterparts: a day with a bird, and a day with none, are both 200.
    let (status, body) = get(&station(&[], one_detection), uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Turdus merula"), "{body}");
    let (status, body) = get(&station(&[], nothing), uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"species\":[]"), "{body}");
}

#[tokio::test]
async fn soundlevel_that_could_not_read_is_an_error_not_no_observations() {
    let uri = "/api/v2/soundlevel";
    let (status, body) = get(&station(&["sound_level_broadband"], nothing), uri).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        !body.contains("no sound level observations yet"),
        "a failed read was reported as an empty station: {body}"
    );

    // The broadband read works and the band read does not: a spectrum with no
    // bands and `samples: 0` is the same lie, one query later.
    let (status, body) = get(&station(&["sound_levels"], one_sound_level), uri).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(!body.contains("\"samples\":0"), "{body}");

    // Counterparts: a station that has measured nothing says so; one that has
    // measured something reports it.
    let (status, body) = get(&station(&[], nothing), uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("no sound level observations yet"), "{body}");
    let (status, body) = get(&station(&[], one_sound_level), uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"samples\":10"), "{body}");
}

// ── the data-quality panels ─────────────────────────────────────────────

#[tokio::test]
async fn the_quality_summary_does_not_say_empty_when_it_could_not_look() {
    let uri = "/admin/quality/summary";
    let (_, body) = get(&station(&["detections"], nothing), uri).await;
    assert!(!body.contains("No detections in database"), "{body}");
    assert!(body.contains("couldn't load"), "{body}");

    // The discrimination: `quality_summary` itself errors on an empty
    // station (`SUM` over no rows is NULL), so an empty one has to be told
    // apart from a broken one by a count, or the first-run page would be
    // reported as a fault.
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(body.contains("No detections in database"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], one_detection), uri).await;
    assert!(body.contains("Total Detections"), "{body}");
}

#[tokio::test]
async fn the_quality_trend_does_not_say_empty_when_it_could_not_look() {
    let uri = "/admin/quality/trend";
    let (_, body) = get(&station(&["detections"], nothing), uri).await;
    assert!(!body.contains("No data for the last 30 days"), "{body}");
    assert!(body.contains("couldn't load"), "{body}");

    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(body.contains("No data for the last 30 days"), "{body}");
}

/// The whole panel set, as the Station Data tab draws it: no section may print
/// the all-clear or an empty-state sentence for a database it could not read.
#[tokio::test]
async fn the_data_tab_quality_panels_never_print_an_all_clear_they_did_not_measure() {
    let uri = "/station/data";
    let (_, broken) = get(&station(&["detections"], nothing), uri).await;
    for claim in [
        "No detections in database",
        "No data for the last 30 days",
        "No data yet.",
        "No detections in the last 30 days",
        "Database quality looks good",
        "No species with avg confidence",
    ] {
        assert!(
            !broken.contains(claim),
            "the quality panels said {claim:?} about a database they could not read"
        );
    }
    assert!(
        broken.contains("couldn't load"),
        "the failure is not shown anywhere"
    );

    // And the counterpart, so the list above is known to be the page's real
    // wording: a working empty station prints them.
    let (_, empty) = get(&station(&[], nothing), uri).await;
    for claim in [
        "No detections in database",
        "No data for the last 30 days",
        "No data yet.",
        "No detections in the last 30 days",
        "Database quality looks good",
    ] {
        assert!(
            empty.contains(claim),
            "a working empty station no longer says {claim:?}"
        );
    }
}

// ── the admin pages ─────────────────────────────────────────────────────
//
// The same defect on the pages an operator acts from. An empty list here is
// not just wrong, it invites the wrong action: "No alert rules defined" invites
// recreating the rules, an empty blacklist invites adding the entry again.

#[tokio::test]
async fn the_species_storage_table_does_not_say_no_detections_when_it_could_not_look() {
    for uri in ["/admin/species/manage", "/admin/species/manage/partial"] {
        let (_, body) = get(&station(&["detections"], nothing), uri).await;
        assert!(!body.contains("No detections yet"), "{uri}: {body}");
        assert!(body.contains("couldn't load"), "{uri}: {body}");
        let (_, body) = get(&station(&[], nothing), uri).await;
        assert!(body.contains("No detections yet"), "{uri}: {body}");
    }
}

#[tokio::test]
async fn measuring_a_species_that_could_not_be_listed_is_not_zero_bytes() {
    let app = station(&["detections"], nothing);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/species/manage/measure")
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("sci_name=Turdus+merula"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let reswap = res
        .headers()
        .get("hx-reswap")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(
        !body.contains("0 B across 0 clip"),
        "a failed read was reported as an empty species: {body}"
    );
    assert_eq!(reswap.as_deref(), Some("none"), "{body}");
}

#[tokio::test]
async fn the_filter_test_does_not_preview_an_empty_station_when_it_could_not_look() {
    let uri = "/admin/species/test";
    // The species list comes from the maintained `species_summary`.
    let (_, body) = get(&station(&["species_summary"], nothing), uri).await;
    assert!(body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(!body.contains("couldn't load"), "{body}");
}

#[tokio::test]
async fn the_rule_lists_do_not_say_no_rules_when_they_could_not_look() {
    let (_, body) = get(&station(&["alert_rules"], nothing), "/admin/rules/list").await;
    assert!(!body.contains("No alert rules defined"), "{body}");
    assert!(body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], nothing), "/admin/rules/list").await;
    assert!(body.contains("No alert rules defined"), "{body}");

    let uri = "/admin/metric-rules/list";
    let (_, body) = get(&station(&["metric_rules"], nothing), uri).await;
    assert!(!body.contains("No rules on the station"), "{body}");
    assert!(body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(body.contains("No rules on the station"), "{body}");
}

/// The Station Alerts tab draws the same log as the partial the earlier suite
/// covers, through a different function, and kept the default.
#[tokio::test]
async fn the_alerts_tab_does_not_say_nothing_was_sent_when_it_could_not_look() {
    let uri = "/station/alerts";
    let (_, body) = get(&station(&["notification_log"], nothing), uri).await;
    assert!(!body.contains("No notifications yet"), "{body}");
    assert!(
        body.contains("couldn't load the notification history"),
        "{body}"
    );
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(body.contains("No notifications yet"), "{body}");
    assert!(
        !body.contains("couldn't load the notification history"),
        "{body}"
    );
}

#[tokio::test]
async fn the_image_blacklist_does_not_say_no_entries_when_it_could_not_look() {
    let uri = "/admin/images";
    let (_, body) = get(&station(&["image_blacklist"], nothing), uri).await;
    assert!(!body.contains("0 entries blacklisted"), "{body}");
    assert!(body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(body.contains("0 entries blacklisted"), "{body}");
}

#[tokio::test]
async fn the_import_history_does_not_say_nothing_was_imported_when_it_could_not_look() {
    let uri = "/admin/migrate/batches";
    let (_, body) = get(&station(&["import_batches"], nothing), uri).await;
    assert!(!body.contains("No histories have been imported"), "{body}");
    assert!(body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(body.contains("No histories have been imported"), "{body}");

    // A batch whose row count cannot be read is not a batch of zero rows.
    let seed = |c: &rusqlite::Connection| {
        c.execute(
            "INSERT INTO import_batches (source_kind, row_count) VALUES ('birdnet-pi', 40)",
            [],
        )
        .expect("batch");
    };
    let (_, body) = get(&station(&["detections"], seed), uri).await;
    assert!(body.contains("couldn't load"), "{body}");
}

// ── the second sweep ────────────────────────────────────────────────────────
//
// Sites an earlier pass classified and left. Each one printed a default for a
// read that had failed, and the default was not neutral: it decided which
// banner showed, which pill was drawn, whether a "Show more" button existed, or
// what an operator was told about a write that had in fact gone through.

/// A station with `statements` run against it after it is seeded — a dropped
/// table, a dropped view, a dropped column — so one named read refuses and the
/// rest still work. Returns the state as well, for the tests that must look at
/// what a write left behind.
fn station_sql(
    statements: &[&str],
    seed: impl FnOnce(&rusqlite::Connection),
) -> (axum::Router, AppState) {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    seed(&conn);
    for sql in statements {
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("could not break the station with {sql:?}: {e}"));
    }
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    (build_router(state.clone()), state)
}

/// One request, with the headers the page's own forms send.
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

fn one_pending_review(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO quarantine (date, time, sci_name, com_name, confidence, reason)
         VALUES ('2026-05-01', '06:10:00', 'Upupa epops', 'Eurasian Hoopoe', 0.61, 'manual')",
        [],
    )
    .expect("seed quarantine");
}

// ── Today: the review nudge and the recording pills ─────────────────────────

/// The strip under the hero is "absent entirely when nothing waits", so a
/// failed count that fell to zero hid the one prompt that asks the operator to
/// act — on exactly the station whose queue they could not otherwise see.
#[tokio::test]
async fn the_review_nudge_does_not_vanish_when_the_queue_could_not_be_read() {
    let uri = "/pages/today-nudge";
    let (_, body) = get(&station(&["quarantine"], nothing), uri).await;
    assert!(body.contains("couldn't load"), "{body}");
    assert!(
        !body.contains("waiting for your eye"),
        "a failed read was reported as a queue: {body}"
    );

    // Counterparts: sightings waiting are announced, and a station with none
    // waiting says nothing at all — a fault note there would be a false alarm.
    let (_, body) = get(&station(&[], one_pending_review), uri).await;
    assert!(body.contains("waiting for your eye"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(body.trim().is_empty(), "{body}");
}

/// `NoSource` says "no microphone configured" in the colour of a fault. A
/// station whose source list could not be read does not know that, and it was
/// the failed read, not the station, that had no microphone.
#[tokio::test]
async fn the_recording_pills_do_not_say_no_microphone_when_the_sources_could_not_be_read() {
    for (uri, claim) in [
        ("/pages/today-pills", "no microphone configured"),
        ("/pages/station-health-line", "no microphone"),
    ] {
        let (_, body) = get(&station(&["audio_sources"], nothing), uri).await;
        assert!(
            !body.contains(claim),
            "{uri} said {claim:?} about a source list it could not read: {body}"
        );
        assert!(body.contains("couldn't read"), "{uri}: {body}");

        // Counterparts: a station with none configured says so; one with a
        // source does not.
        let (_, body) = get(&station(&[], nothing), uri).await;
        assert!(body.contains(claim), "{uri}: {body}");
        assert!(!body.contains("couldn't read"), "{uri}: {body}");
        let with_source = |conn: &rusqlite::Connection| {
            use birdnet_db::audio_sources::{AudioSourceStore as _, NewAudioSource, SourceKind};
            conn.insert(&NewAudioSource::defaults(
                "mic0",
                SourceKind::Rtsp,
                "rtsp://127.0.0.1:1/dead",
            ))
            .expect("seed source");
        };
        let (_, body) = get(&station(&[], with_source), uri).await;
        assert!(!body.contains(claim), "{uri}: {body}");
        assert!(!body.contains("couldn't read"), "{uri}: {body}");
    }
}

// ── the merged-history note ─────────────────────────────────────────────────

fn a_different_site_import(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO import_batches (source_kind, source_label, distance_km, row_count)
         VALUES ('birdnet-pi', 'Coastal site', 341.0, 1)",
        [],
    )
    .expect("seed batch");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, import_batch_id)
         VALUES ('2026-05-01', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.55, 1)",
        [],
    )
    .expect("seed imported detection");
}

fn only_the_batch(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO import_batches (source_kind, source_label, distance_km, row_count)
         VALUES ('birdnet-pi', 'Coastal site', 341.0, 1)",
        [],
    )
    .expect("seed batch");
}

/// The note is the caveat on every analytic page that its hour-of-day and
/// location readings describe more than one place. Silence is how it says the
/// history is one place, so a failed read must not produce it.
#[tokio::test]
async fn the_merged_history_note_does_not_go_quiet_when_the_imports_could_not_be_read() {
    let uri = "/pages/provenance-note";
    // The batch list refuses…
    let (_, body) = get(&station(&["import_batches"], nothing), uri).await;
    assert!(body.contains("couldn't load"), "{body}");
    // …and, separately, the count of imported rows does.
    let (_, body) = get(&station(&["detections"], only_the_batch), uri).await;
    assert!(body.contains("couldn't load"), "{body}");

    // Counterparts: a different site is named; a station that never imported
    // says nothing, and does not say it could not look.
    let (_, body) = get(&station(&[], a_different_site_import), uri).await;
    assert!(body.contains("Merged history"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(body.trim().is_empty(), "{body}");
}

// ── Recordings ──────────────────────────────────────────────────────────────

fn one_locked_clip(conn: &rusqlite::Connection) {
    one_detection(conn);
    conn.execute("UPDATE detections SET is_locked = 1", [])
        .expect("lock");
}

/// A locked clip drawn as unlocked says "Lock — protect from auto-purge" about
/// a keeper, and the clip's total decides whether "Show more" exists at all.
/// None of the grid's lookups may fall back to a default and draw rows on it.
#[tokio::test]
async fn the_clips_grid_does_not_draw_rows_on_lookups_it_could_not_make() {
    for uri in ["/recordings", "/pages/recordings-clips"] {
        // The lock set refuses (it reads a column the row query does not).
        let (_, body) = get(
            &station_sql(
                &["DROP INDEX idx_detections_locked; ALTER TABLE detections DROP COLUMN is_locked"],
                one_locked_clip,
            )
            .0,
            uri,
        )
        .await;
        assert!(body.contains("couldn't load"), "{uri}: {body}");
        assert!(
            !body.contains("Lock — protect from auto-purge"),
            "{uri}: a clip whose lock could not be read was drawn as unlocked: {body}"
        );

        // The first-seen map refuses (it reads the analytic view).
        let (_, body) = get(
            &station_sql(&["DROP VIEW detections_analytic"], one_detection).0,
            uri,
        )
        .await;
        assert!(body.contains("couldn't load"), "{uri}: {body}");

        // Counterparts: the same station, working.
        let (_, body) = get(&station(&[], one_locked_clip), uri).await;
        assert!(
            body.contains("Locked — click to allow auto-purge"),
            "{uri}: {body}"
        );
        assert!(!body.contains("couldn't load"), "{uri}: {body}");
        let (_, body) = get(&station(&[], one_detection), uri).await;
        assert!(
            body.contains("Lock — protect from auto-purge"),
            "{uri}: {body}"
        );
        let (_, body) = get(&station(&[], nothing), uri).await;
        assert!(body.contains("No saved clips yet"), "{uri}: {body}");
        assert!(!body.contains("couldn't load"), "{uri}: {body}");
    }
}

// ── the live feed and the day's best clips ──────────────────────────────────

/// A `BLOB` in a `TEXT` column: SQLite stores it, and a reader that asks for a
/// `String` refuses it. It is a row a scan must read and a page need not.
fn a_row_nothing_can_read_back(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES ('1999-01-01', '06:00:00', X'ff', 'Unreadable', 0.9)",
        [],
    )
    .expect("seed unreadable row");
}

fn a_clip_from_today(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES (date('now', 'localtime'), '06:10:00', 'Turdus merula', 'Eurasian Blackbird',
                 0.92, 'b.wav')",
        [],
    )
    .expect("seed today's clip");
}

/// "first ever" is the badge an operator waits for. Without the first-seen map
/// it is simply absent, which reads as *this was not a first* — so the feed and
/// the best-clips rail fail as a whole rather than draw rows without it.
#[tokio::test]
async fn the_feed_does_not_draw_rows_without_the_badge_it_could_not_compute() {
    let uri = "/pages/detections";
    let (_, body) = get(
        &station_sql(&["DROP VIEW detections_analytic"], one_detection).0,
        uri,
    )
    .await;
    assert!(body.contains("couldn't load the live feed"), "{body}");
    assert!(!body.contains("Eurasian Blackbird"), "{body}");

    let (_, body) = get(&station(&[], one_detection), uri).await;
    assert!(body.contains("Eurasian Blackbird"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");

    // The best-clips rail reads the same view for its rows, so only a row the
    // map must scan and the rail does not can tell the two reads apart.
    let uri = "/pages/best-detections";
    let both = |conn: &rusqlite::Connection| {
        a_clip_from_today(conn);
        a_row_nothing_can_read_back(conn);
    };
    let (_, body) = get(&station(&[], both), uri).await;
    assert!(
        body.contains("couldn't load today&#x27;s best recordings"),
        "{body}"
    );
    assert!(!body.contains("Eurasian Blackbird"), "{body}");
    let (_, body) = get(&station(&[], a_clip_from_today), uri).await;
    assert!(body.contains("Eurasian Blackbird"), "{body}");
    assert!(!body.contains("couldn't load"), "{body}");
}

// ── Station health: the microphone panel ────────────────────────────────────

fn a_noise_floor_sample(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO audio_levels
             (date, hour, source, samples, noise_floor_sum_dbfs, noise_floor_min_dbfs,
              snr_sum_db, flatness_sum)
         VALUES (date('now', 'localtime'), 6, 'mic0', 10, -500.0, -60.0, 100.0, 5.0)",
        [],
    )
    .expect("seed noise floor");
}

/// The panel is silent until the sampler has run, so a station whose noise-floor
/// history cannot be read looked like a station an hour old. The one panel that
/// exists to say a microphone has gone deaf cannot vanish without saying why.
#[tokio::test]
async fn the_microphone_panel_does_not_vanish_when_its_history_could_not_be_read() {
    let uri = "/station";
    let (_, body) = get(&station(&["audio_levels"], nothing), uri).await;
    assert!(
        body.contains("couldn't load the microphone noise-floor history"),
        "{body}"
    );
    assert!(!body.contains("Microphone health"), "{body}");

    // Counterparts: samples are shown; a station that has none yet is silent
    // about it, and does not claim a fault.
    let (_, body) = get(&station(&[], a_noise_floor_sample), uri).await;
    assert!(body.contains("Microphone health"), "{body}");
    assert!(!body.contains("noise-floor history"), "{body}");
    let (_, body) = get(&station(&[], nothing), uri).await;
    assert!(!body.contains("Microphone health"), "{body}");
    assert!(!body.contains("noise-floor history"), "{body}");
}

// ── admin writes that went through ──────────────────────────────────────────

/// A viewer whose `label` no reader can decode: `create_user` finds only the
/// row it inserted, `list_users` scans them all.
fn a_user_the_list_cannot_read(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO users (username, pwd_argon2, role, label)
         VALUES ('unreadable', 'x', 'viewer', X'ff')",
        [],
    )
    .expect("seed unreadable user");
}

fn a_viewer_to_remove(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO users (username, pwd_argon2, role) VALUES ('victim', 'x', 'viewer')",
        [],
    )
    .expect("seed viewer");
}

/// The account was made and the list could not be read back. The answer was the
/// empty roster — "No users — the admin row is created by the schema
/// migration" — swapped over a list that had users in it, under a toast that
/// said the invitation had worked.
#[tokio::test]
async fn an_invitation_that_worked_is_reported_even_when_the_list_cannot_be_redrawn() {
    let form = "username=newbie&label=Jess&password=correct-horse-battery";

    let (app, state) = station_sql(&[], a_user_the_list_cannot_read);
    let (status, headers, body) = send(&app, "POST", "/admin/accounts/users", form).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Invited newbie"), "{body}");
    assert!(
        !body.contains("No users"),
        "the roster was swapped for an empty one: {body}"
    );
    assert_eq!(
        headers.get("hx-reswap").and_then(|v| v.to_str().ok()),
        Some("none"),
        "the list on the page must be left as it was: {body}"
    );
    assert!(
        state.with_db(|c| c.find_user_by_name("newbie")).is_ok(),
        "the account was not created, so the toast above is the wrong half of this test"
    );

    // Counterpart: a working roster is redrawn with the new row, and nothing
    // tells the browser to leave it alone.
    let (app, _) = station_sql(&[], nothing);
    let (_, headers, body) = send(&app, "POST", "/admin/accounts/users", form).await;
    assert!(body.contains("Invited newbie"), "{body}");
    assert!(body.contains("user-name\">Jess"), "{body}");
    assert!(headers.get("hx-reswap").is_none(), "{body}");
}

#[tokio::test]
async fn a_removal_that_worked_is_reported_even_when_the_list_cannot_be_redrawn() {
    let both = |conn: &rusqlite::Connection| {
        a_viewer_to_remove(conn);
        a_user_the_list_cannot_read(conn);
    };
    let (app, state) = station_sql(&[], both);
    let id = state
        .with_db(|c| c.find_user_by_name("victim"))
        .expect("victim")
        .id;
    let uri = format!("/admin/accounts/users/{id}");
    let (status, headers, body) = send(&app, "DELETE", &uri, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("User removed"), "{body}");
    assert!(!body.contains("No users"), "{body}");
    assert_eq!(
        headers.get("hx-reswap").and_then(|v| v.to_str().ok()),
        Some("none"),
        "{body}"
    );
    assert!(
        state.with_db(|c| c.find_user_by_name("victim")).is_err(),
        "the account is still there"
    );

    let (app, state) = station_sql(&[], a_viewer_to_remove);
    let id = state
        .with_db(|c| c.find_user_by_name("victim"))
        .expect("victim")
        .id;
    let (_, headers, body) = send(&app, "DELETE", &format!("/admin/accounts/users/{id}"), "").await;
    assert!(body.contains("User removed"), "{body}");
    assert!(!body.contains("victim"), "{body}");
    assert!(headers.get("hx-reswap").is_none(), "{body}");
}

/// A refused approval used to put the database's own words in a toast — "Approve
/// failed: SQLite error: no such table: …" — to someone who has not seen a table
/// since school. The siblings (reject, delete) already say what is certain.
/// What is certain here is a stronger thing, and the test holds it: the
/// approval is one transaction, so a refusal left the sighting waiting and put
/// nothing in the records.
#[tokio::test]
async fn a_refused_approval_says_so_without_the_databases_words() {
    let (app, state) = station_sql(&["DROP TABLE detection_reviews"], one_pending_review);
    let (status, _, body) = send(&app, "POST", "/pages/quarantine-approve", "id=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for raw in ["no such table", "SQLite", "detection_reviews"] {
        assert!(
            !body.contains(raw),
            "the toast quotes the database ({raw:?}): {body}"
        );
    }
    assert!(body.contains("Not approved"), "{body}");
    let (reviewed, records): (i64, i64) = state.with_db(|c| {
        (
            c.query_row("SELECT reviewed FROM quarantine WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap(),
            c.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
                .unwrap(),
        )
    });
    assert_eq!(
        (reviewed, records),
        (0, 0),
        "'Not approved' is only true if the refusal left no half-approval behind"
    );

    // Counterpart: the same click on a working station is "Approved.".
    let (app, state) = station_sql(&[], one_pending_review);
    let (_, _, body) = send(&app, "POST", "/pages/quarantine-approve", "id=1").await;
    assert!(body.contains("Approved."), "{body}");
    assert!(!body.contains("Not approved"), "{body}");
    let records: i64 = state.with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap()
    });
    assert_eq!(records, 1);
}

/// A session that outlives its owner's attention: the open window's own sentinel
/// id, so `revoke_others` keeps it, with a `user_agent` no reader can decode.
fn a_session_the_list_cannot_read(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO sessions (id, user_id, expires_at, user_agent)
         VALUES ('open-bypass', 1, datetime('now', '+1 day'), X'ff')",
        [],
    )
    .expect("seed unreadable session");
}

/// Signing a session out is a write that worked; the list that would show it is
/// a read that may not. The answer was a single dash swapped over the list.
#[tokio::test]
async fn a_session_signed_out_is_reported_even_when_the_list_cannot_be_redrawn() {
    let both = |conn: &rusqlite::Connection| {
        a_session_the_list_cannot_read(conn);
        conn.execute(
            "INSERT INTO sessions (id, user_id, expires_at) VALUES ('other-device', 1, datetime('now', '+1 day'))",
            [],
        )
        .expect("seed a session");
    };
    let (app, state) = station_sql(&[], both);
    let (status, headers, body) =
        send(&app, "DELETE", "/admin/accounts/sessions/other-device", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Session signed out"), "{body}");
    assert!(
        !body.contains("account-sessions__empty"),
        "the list was swapped for a dash: {body}"
    );
    assert_eq!(
        headers.get("hx-reswap").and_then(|v| v.to_str().ok()),
        Some("none"),
        "{body}"
    );
    let left: i64 = state.with_db(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'other-device'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    });
    assert_eq!(
        left, 0,
        "the session is still there, so the toast is the wrong half of this test"
    );

    // Counterpart: a readable list is redrawn without the signed-out session.
    let (app, _) = station_sql(&[], |c| {
        c.execute(
            "INSERT INTO sessions (id, user_id, expires_at) VALUES ('other-device', 1, datetime('now', '+1 day'))",
            [],
        )
        .expect("seed a session");
    });
    let (_, headers, body) =
        send(&app, "DELETE", "/admin/accounts/sessions/other-device", "").await;
    assert!(body.contains("Session signed out"), "{body}");
    assert!(headers.get("hx-reswap").is_none(), "{body}");
    assert!(
        !body.contains("other-device") && !body.contains("vice"),
        "{body}"
    );
}

#[tokio::test]
async fn other_devices_signed_out_are_reported_even_when_the_list_cannot_be_redrawn() {
    let both = |conn: &rusqlite::Connection| {
        a_session_the_list_cannot_read(conn);
        conn.execute(
            "INSERT INTO sessions (id, user_id, expires_at) VALUES ('other-device', 1, datetime('now', '+1 day'))",
            [],
        )
        .expect("seed a session");
    };
    let (app, state) = station_sql(&[], both);
    let (status, headers, body) =
        send(&app, "POST", "/admin/accounts/sessions/revoke-others", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Signed out 1 other device"), "{body}");
    assert!(!body.contains("account-sessions__empty"), "{body}");
    assert_eq!(
        headers.get("hx-reswap").and_then(|v| v.to_str().ok()),
        Some("none"),
        "{body}"
    );
    let left: i64 = state.with_db(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'other-device'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    });
    assert_eq!(left, 0);
}
