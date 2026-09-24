//! A full backup is not started by another website.
//!
//! `GET /admin/system/backup/full` snapshots the database and tars every
//! recording into a scratch file beside it. It is a GET, so the CSRF check
//! does not see it, and on a station with no password `/admin` is open: any
//! page a household member visited could carry
//! `<img src="http://192.168.1.10:8502/admin/system/backup/full">`, and each
//! load cost the station a full archive's worth of disk and CPU.
//!
//! Browsers mark such a request `Sec-Fetch-Site: cross-site`; the download
//! link on the station's own page is `same-origin`. The counterpart holds that
//! the station's own link still downloads.

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

fn open_station(dir: &std::path::Path) -> AppState {
    let db = dir.join("birds.db");
    let conn = rusqlite::Connection::open(&db).expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    AppState::from_connection(conn, db)
}

async fn backup(state: &AppState, site: &str) -> u16 {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/admin/system/backup/full")
                .header("host", "localhost")
                .header("sec-fetch-site", site)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    resp.status().as_u16()
}

#[tokio::test]
async fn a_cross_site_request_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let st = open_station(dir.path());
    assert_eq!(backup(&st, "cross-site").await, 403);
}

#[tokio::test]
async fn the_stations_own_link_still_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let st = open_station(dir.path());
    assert_eq!(backup(&st, "same-origin").await, 200);
}
