//! Integration tests for page rendering (HTML pages and HTMX partials).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::{Connection, params};
use tower::ServiceExt;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// Create a test `AppState` with an in-memory database and sample data.
fn test_state() -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    // Apply the full migration chain — hand-coded CREATE TABLE in
    // test fixtures drifts the moment a migration adds a column.
    // See ADR-16 "Anti-patterns this standard exists to prevent /
    // Hand-coded schema in test fixtures duplicating the migration".
    birdnet_db::migration::migrate(&conn).unwrap();

    let records = [
        (
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.87,
        ),
        (
            "2026-03-12",
            "06:35:00",
            "Erithacus rubecula",
            "European Robin",
            0.92,
        ),
        (
            "2026-03-12",
            "06:45:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.78,
        ),
        ("2026-03-12", "07:00:00", "Parus major", "Great Tit", 0.81),
        (
            "2026-03-11",
            "18:00:00",
            "Cyanistes caeruleus",
            "Eurasian Blue Tit",
            0.75,
        ),
    ];

    for (date, time, sci, com, conf) in &records {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![date, time, sci, com, conf],
        )
        .unwrap();
    }

    // One species heard *today*, distinct from every historical one above.
    //
    // The fixture used to be entirely historical, which is how
    // `htmx_top_species_partial_returns_list` came to assert that the card
    // headed "Today · Top species" showed a bird detected in March: the partial
    // read the dateless `species_summary` rollup, so a fixed past date and
    // "today" were the same answer. They are not the same answer any more, and
    // the difference is what that test now pins.
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES (date('now','localtime'), '05:15:00', 'Sylvia atricapilla', 'Eurasian Blackcap', 0.83)",
        [],
    )
    .unwrap();

    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

fn app() -> axum::Router {
    let state = test_state();
    build_router(state)
}

#[tokio::test]
async fn dashboard_page_returns_html() {
    let app = app();

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("BirdNet-Behavior"));
    assert!(html.contains("htmx.min.js"));
    assert!(html.contains("/static/css/app.css"));
    assert!(html.contains("Detections as they happen"));
    assert!(html.contains("Top species"));
}

#[tokio::test]
async fn species_page_returns_html() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/species")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // The v3-spine Species home: the page-head headline, the view switcher,
    // and the server-rendered List table.
    assert!(html.contains("Who you've heard"));
    assert!(html.contains("sp-seg"));
    assert!(html.contains("sp-table"));
}

#[tokio::test]
async fn htmx_stats_partial_returns_html() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("Detections"));
    assert!(html.contains("Species"));
    assert!(html.contains("stat-tile"));
    assert!(html.contains('5')); // total detections from test data
    assert!(html.contains('4')); // unique species from test data
}

#[tokio::test]
async fn htmx_detections_partial_returns_table() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/detections")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("feed-row"));
    assert!(html.contains("Eurasian Blackbird"));
    assert!(html.contains("European Robin"));
}

#[tokio::test]
async fn htmx_top_species_partial_returns_list() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/top-species")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // The card is headed "Today · Top species", so it must carry the species
    // heard today and *not* the ones the fixture recorded in March. This
    // assertion used to read `contains("Eurasian Blackbird")` — a bird detected
    // on 2026-03-12 — because the partial read the dateless `species_summary`
    // rollup and answered with all-time totals under a heading that said today.
    assert!(
        html.contains("Eurasian Blackcap"),
        "the Today card must show what was heard today"
    );
    assert!(
        !html.contains("Eurasian Blackbird"),
        "and must not show March's commonest species: {html}"
    );
    // v3 spine: the rail's top-species rows use the x-top treatment
    // (banding code under the name) instead of the old list-row.
    assert!(html.contains("x-top"));
    assert!(html.contains("bnb-avatar"));
}

/// The badge is the answer a non-technical operator gets to "is my station
/// working?", on every page, refreshed every 30 s. It used to mean only
/// "SQLite is not corrupt", so it read green with a dead microphone. These pin
/// both halves of the new contract.
#[tokio::test]
async fn htmx_health_badge_flags_a_station_with_no_microphone() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/pages/health-badge")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // The fixture station has detections but no configured capture source, so
    // "Healthy" would be a false reassurance.
    assert!(html.contains("No microphone"), "{html}");
    assert!(html.contains(r#"data-health="warn""#), "{html}");
    assert!(!html.contains("Healthy"));
}

#[tokio::test]
async fn htmx_health_badge_returns_healthy_for_a_capturing_station() {
    let state = test_state();
    let new_source = birdnet_db::audio_sources::NewAudioSource::defaults(
        "src_ok",
        birdnet_db::audio_sources::SourceKind::UsbAlsa,
        "plughw:CARD=PRO,DEV=0",
    );
    state.with_db(|c| {
        birdnet_db::audio_sources::AudioSourceStore::insert(c, &new_source).unwrap();
    });
    // The supervisor publishes this gauge; the badge reads the same one the
    // Capture tab's status pill does.
    state.metrics().set_source_up("src_ok", true);

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/pages/health-badge")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // The badge grades three inputs — database, capture, disk — and this test
    // controls only the first two. The third is the *host's* filesystem, and
    // the badge correctly reports "Disk full" above 90 %, so asserting
    // "Healthy" unconditionally made this fail on any full build machine. It
    // did, here, after a scale probe filled the volume.
    //
    // So assert the two signals this test actually sets, and tolerate exactly
    // one thing: a disk warning, named. Anything else — a database error, a
    // capture problem, an ungraded badge — still fails, which is what the test
    // is for. Widening it to "any warn" would have hidden the mic-down case
    // this file's sibling test exists to catch.
    let disk_warning = html.contains(r#"data-health="warn""#) && html.contains("Disk full");
    assert!(
        (html.contains("Healthy") && html.contains(r#"data-health="ok""#)) || disk_warning,
        "expected a healthy badge, or a disk warning on a full build host; got {html}"
    );
    assert!(
        !html.contains("Mic down") && !html.contains("No microphone"),
        "a source publishing an up gauge must not read as down: {html}"
    );
    assert!(
        !html.contains(r#"data-health="err""#),
        "nothing here should grade as an error: {html}"
    );
}

#[tokio::test]
async fn analytics_page_returns_html() {
    let app = app();

    // The behavioral-analytics surface lives on the Patterns home's
    // "Behavior" tab since the v3 spine (the old /analytics redirects there).
    let response = app
        .oneshot(
            Request::builder()
                .uri("/patterns?tab=behavior")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("Behavioral Analytics"));
    // Behavior tab is a defined-in-place masonry since the Patterns reskin:
    // each card carries a sentence-case eyebrow + a plain-English headline.
    assert!(html.contains("Activity sessions"));
    assert!(html.contains("Bursts of singing"));
    assert!(html.contains("Species retention"));
    assert!(html.contains("Who keeps coming back"));
    // The v0.8.0 dawn-sequence card (sequence_count + window_funnel_events).
    assert!(html.contains("Dawn sequence"));
    assert!(html.contains("The morning running order"));
}

#[tokio::test]
async fn htmx_hourly_chart_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/hourly-chart")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // Should return either SVG chart or "no detections" message
    assert!(html.contains("<svg") || html.contains("No detections"));
}

#[tokio::test]
async fn htmx_daily_chart_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/daily-chart")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // Should return either SVG chart or "no data" message
    assert!(html.contains("<svg") || html.contains("No detection data"));
}

#[tokio::test]
async fn htmx_analytics_status_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/analytics-status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("Analytics Engine"));
}

#[tokio::test]
async fn htmx_analytics_config_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/analytics-config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("Version"));
    assert!(html.contains("SQLite Database"));
}

#[tokio::test]
async fn htmx_analytics_dawn_sequence_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/analytics-dawn-sequence")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // No DuckDB in the test fixture, so the partial reports the graceful
    // "analytics unavailable" fragment with a 200 (HTMX swaps it in place).
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // With no DuckDB configured the partial returns the graceful unavailable
    // fragment, which names the feature ("Dawn sequence requires DuckDB
    // analytics…"). The populated card is screenshot-verified separately.
    assert!(html.contains("Dawn sequence"));
}

#[tokio::test]
async fn htmx_cooccurrence_matrix_partial() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/cooccurrence-matrix?days=3650")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    // Either a rendered matrix or the graceful "not enough data" message.
    assert!(html.contains("<svg") || html.contains("Not enough data"));
}

#[tokio::test]
async fn htmx_activity_streamgraph_partial() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/activity-streamgraph?days=3650")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("<svg") || html.contains("Not enough data"));
}

#[tokio::test]
async fn htmx_dawn_chorus_partial() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/dawn-chorus")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    // All-time top species exist in the fixture, so the polar renders.
    assert!(html.contains("<svg") || html.contains("Not enough data"));
}

#[tokio::test]
async fn htmx_life_accumulation_partial() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/life-accumulation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("<svg") || html.contains("Not enough data"));
}

#[tokio::test]
async fn htmx_seasonal_phenology_partial() {
    // The heatmap page's embedded ridgeline lives at /pages/seasonal-phenology;
    // the dedicated /migration page owns /pages/migration-ridgeline (tested in
    // all_redesigned_pages_render_ok).
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/seasonal-phenology")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("<svg") || html.contains("Not enough data"));
}

#[tokio::test]
async fn htmx_confidence_chart_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/confidence-chart")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // Should contain SVG chart (test data has detections with various confidence levels)
    assert!(html.contains("<svg"));
}

#[tokio::test]
async fn all_redesigned_pages_render_ok() {
    // Every primary page + every new visualization partial must render
    // without a server error against a seeded database.
    let routes = [
        "/",
        "/onboarding",
        "/species",
        // The Species home's three views (List/Photos/Life list).
        "/species?view=photos",
        "/species?view=lifelist",
        "/species?view=list&filter=week",
        // The v3-spine homes, every tab.
        "/patterns",
        "/patterns?tab=dawn",
        "/patterns?tab=migration",
        "/patterns?tab=together",
        "/patterns?tab=trends",
        "/patterns?tab=behavior",
        "/reports",
        "/reports?tab=year",
        "/reports?tab=history",
        "/station",
        // The gated Station management tabs (open-admin bypass in tests).
        "/station/capture",
        "/station/alerts",
        "/station/data",
        "/station/settings",
        "/station/access",
        "/recordings",
        "/recordings?view=clips",
        "/recordings?view=live",
        "/notifications",
        "/quarantine",
        "/kiosk",
        "/pages/today-daystrip",
        "/pages/cooccurrence-matrix",
        "/pages/acoustic-network",
        "/pages/activity-streamgraph",
        "/pages/dawn-chorus",
        "/pages/seasonal-phenology",
        "/pages/migration-ridgeline",
        "/pages/migration-stats",
        "/pages/migration-diversity",
        "/pages/dawn-polar",
        "/pages/dawn-list",
        "/pages/analytics-dawn-sequence",
        "/pages/life-accumulation",
        // The Health-detail / full-form fallback admin pages still render
        // through the admin shell; the eight folded management pages now
        // redirect to their Station tab (covered by the admin-nav parity test
        // `folded_pages_redirect_to_their_station_tab`).
        "/admin/overview",
        "/admin/system",
        "/admin/settings",
    ];
    for route in routes {
        let app = app();
        let response = app
            .oneshot(Request::builder().uri(route).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "route {route} did not return 200"
        );
    }

    // The live-audio paths (`/listen`, `/livestream`, `/live`) are retired
    // duplicates that now permanently redirect into the Recordings home's Live
    // view; they are covered by `legacy_routes_redirect_to_their_homes` and
    // intentionally excluded from the 200 list above.
}

/// Every pre-spine route that folded into a v3 home must permanently
/// redirect to it through the REAL built router (middleware included) —
/// "never 404 a veteran's bookmark". The unit test on the redirect module
/// checks the table; this checks nothing else swallowed the routes.
#[tokio::test]
async fn legacy_routes_redirect_to_their_homes() {
    for (old, new) in birdnet_web::routes::redirects::LEGACY_ROUTES {
        let response = app()
            .oneshot(Request::builder().uri(*old).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::PERMANENT_REDIRECT,
            "{old} should permanently redirect"
        );
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
            Some(*new),
            "{old} should land on {new}"
        );
    }
}

/// Unmatched paths under `/api/` must 404 with JSON, not the branded HTML
/// page — API consumers are scripts, and an HTML body hides the failure.
#[tokio::test]
async fn unknown_api_path_returns_json_404() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/definitely-not-a-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        content_type.starts_with("application/json"),
        "API 404 should be JSON, got content-type: {content_type}"
    );

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "not found");
    assert_eq!(json["path"], "/api/v2/definitely-not-a-route");
}

/// Unmatched page URLs keep the friendly branded HTML 404.
#[tokio::test]
async fn unknown_page_path_returns_html_404() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/definitely-not-a-page")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("That page flew off"));
}
