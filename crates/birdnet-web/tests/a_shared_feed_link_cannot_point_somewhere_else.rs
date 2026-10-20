//! A feed address someone passes you cannot make the feed link somewhere else.
//!
//! `/feeds/rare.rss?base=…` let the caller choose the host that every item link
//! in the feed points at, and the only check was that the value looked like a
//! URL. `…/feeds/rare.rss?base=https://evil.example` is then a feed on this
//! station's own domain whose every "Heard at 06:10 — listen" link goes to
//! someone else's page, and a feed address is exactly what people paste into a
//! reader, a chat or an email. The override stays — it is how a reverse proxy
//! with a path prefix gets its links right — but only for an address the
//! request itself or the operator has already named.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    // A first-ever, high-confidence detection: rare by every definition.
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-09-08', '06:10:00', 'Tichodroma muraria', 'Wallcreeper', 0.97, 'w.wav')",
        [],
    )
    .expect("seed");
    birdnet_web::server::build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

async fn feed(app: &axum::Router, uri: &str, host: &str) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{uri}");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&body).into_owned()
}

/// What the links are when nothing overrides them.
fn default_base() -> String {
    std::env::var("BNB_BASE_URL").unwrap_or_else(|_| "http://localhost:8502".to_owned())
}

const FEEDS: [&str; 2] = ["/feeds/rare.rss", "/feeds/rare.ics"];

#[tokio::test]
async fn a_base_naming_another_host_is_ignored() {
    let app = station();
    for path in FEEDS {
        for hostile in [
            "https://evil.example",
            "https://evil.example/birdnet",
            // Looks like this station, is not.
            "https://station.local.evil.example",
            "http://station.local@evil.example",
        ] {
            let body = feed(&app, &format!("{path}?base={hostile}"), "station.local").await;
            assert!(
                !body.contains("evil.example"),
                "{path}?base={hostile} put another host in the feed:\n{body}"
            );
            assert!(
                body.contains(&format!("{}/detections/detail", default_base())),
                "{path}: links should fall back to the configured base:\n{body}"
            );
        }
    }
}

/// The counterpart: the override still works for the host that was asked for, so
/// a reverse proxy with a path prefix can still have its links right.
#[tokio::test]
async fn a_base_naming_the_host_that_was_asked_is_honoured() {
    let app = station();
    for path in FEEDS {
        let body = feed(
            &app,
            &format!("{path}?base=https://Station.Local:8443/birdnet"),
            "station.local:8443",
        )
        .await;
        assert!(
            body.contains("https://Station.Local:8443/birdnet/detections/detail"),
            "{path}: the matching base was not used:\n{body}"
        );
    }
}

/// A line break in a species name must not start a new calendar property.
#[tokio::test]
async fn a_carriage_return_in_a_name_cannot_start_a_new_ical_line() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-09-08', '06:10:00', 'Tichodroma muraria', ?1, 0.97, 'w.wav')",
        ["Wallcreeper\rATTENDEE:mailto:evil@example.com"],
    )
    .expect("seed");
    let app = birdnet_web::server::build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ));
    let body = feed(&app, "/feeds/rare.ics", "station.local").await;
    assert!(
        !body.replace("\r\n", "").contains('\r'),
        "a lone CR survived into the calendar:\n{body:?}"
    );
    assert!(
        !body.split("\r\n").any(|l| l.starts_with("ATTENDEE")),
        "a name started its own property:\n{body:?}"
    );
}
