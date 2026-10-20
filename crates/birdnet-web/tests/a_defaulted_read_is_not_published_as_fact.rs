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

use axum::body::Body;
use axum::http::{Request, StatusCode};
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
