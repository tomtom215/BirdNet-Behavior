//! The settings page never shows defaults for settings it could not read.
//!
//! `load_all_settings` defaulted a failed read to an empty map. The page then
//! rendered every field at its default as though that were the station's
//! configuration; the save compared the submission against that empty map, so
//! every field — render-time defaults included — counted as changed and would
//! be written over the operator's real configuration; and the JSON API
//! reported `{}` as the settings.

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// A `settings` view in place of the table: `CREATE TABLE IF NOT EXISTS`
/// leaves it alone, and reading it as the settings table fails.
fn unreadable() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch("DROP TABLE settings; CREATE VIEW settings AS SELECT 1 AS nope;")
        .expect("break the settings table");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(state: &AppState, uri: &str) -> (u16, String) {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn an_unreadable_settings_table_is_not_a_page_of_defaults() {
    let st = unreadable();
    let (_, page) = get(&st, "/admin/settings").await;
    assert!(
        !page.contains(r#"name="site_name""#),
        "the form was rendered from defaults"
    );
    assert!(
        page.contains("could not be read"),
        "said nothing: {}",
        &page[..page.len().min(400)]
    );

    for tab in ["/station/capture", "/station/alerts", "/station/settings"] {
        let (_, page) = get(&st, tab).await;
        assert!(
            page.contains("could not be read"),
            "{tab} rendered defaults"
        );
    }

    // The notice's own way out must lead somewhere.
    assert!(page.contains(r#"href="/admin/doctor""#));
    let (doctor, _) = get(&st, "/admin/doctor").await;
    assert_eq!(doctor, 200, "the notice links to a page that is not there");

    let (status, json) = get(&st, "/api/v2/settings").await;
    assert_ne!(status, 200, "the API reported the settings as {json}");
}

/// The counterpart: a healthy station renders its form.
#[tokio::test]
async fn a_readable_station_renders_its_form() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let st = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let (status, page) = get(&st, "/admin/settings").await;
    assert_eq!(status, 200);
    assert!(page.contains(r#"name="site_name""#));
}
