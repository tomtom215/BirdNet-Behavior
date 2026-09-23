//! A viewer account is read-only. That must not mean "reads every credential".
//!
//! The admin gate only checked the role on unsafe methods, so every `/admin`
//! GET was open to a viewer — including the settings forms, which printed the
//! SMTP password, the BirdWeather token and every notification URL into
//! `value="…"`; the audio sources list, which printed each RTSP address with
//! its camera password; and the backup, full-backup and support-bundle
//! downloads, which hand over the whole database with every password hash in
//! it. The settings *API* has always masked the same values.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_db::accounts::{Role, UserStore as _};
use birdnet_db::audio_sources::{AudioSourceStore as _, NewAudioSource, SourceKind};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const SMTP_PASS: &str = "smtp-secret-9f3a";
const BW_TOKEN: &str = "bw-token-77c1";
const CAM_PASS: &str = "campass-41d2";

fn station() -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let admin_hash = birdnet_db::accounts::hash_password("admin-password-1").expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&admin_hash],
    )
    .expect("admin password");
    let viewer_hash = birdnet_db::accounts::hash_password("viewer-password-1").expect("hash");
    conn.create_user("viewer", &viewer_hash, Role::Viewer, None)
        .expect("viewer");
    conn.execute_batch(&format!(
        "INSERT INTO settings (key, value) VALUES
           ('email_smtp_pass', '{SMTP_PASS}'),
           ('birdweather_token', '{BW_TOKEN}');"
    ))
    .expect("settings");
    conn.insert(&NewAudioSource::defaults(
        "cam1",
        SourceKind::Rtsp,
        format!("rtsp://viewer:{CAM_PASS}@camera.local/stream"),
    ))
    .expect("rtsp source");
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

async fn sign_in(app: &axum::Router, user: &str, password: &str) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("origin", "http://localhost")
                .header("host", "localhost")
                .body(Body::from(format!("username={user}&password={password}")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::SEE_OTHER,
        "{user} could not sign in"
    );
    res.headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

async fn get(app: &axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::COOKIE, cookie)
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

#[tokio::test]
async fn a_viewer_sees_the_forms_with_every_credential_masked() {
    let app = station();
    let viewer = sign_in(&app, "viewer", "viewer-password-1").await;
    for uri in ["/admin/settings", "/station/alerts", "/station/capture"] {
        let (status, html) = get(&app, uri, &viewer).await;
        assert_eq!(status, StatusCode::OK, "{uri} is still readable");
        for secret in [SMTP_PASS, BW_TOKEN, CAM_PASS] {
            assert!(
                !html.contains(secret),
                "{uri} showed a viewer the credential {secret}"
            );
        }
    }
    // Counterpart: the admin, who types these in, still sees them.
    let admin = sign_in(&app, "admin", "admin-password-1").await;
    let (_, html) = get(&app, "/admin/settings", &admin).await;
    assert!(html.contains(SMTP_PASS) && html.contains(BW_TOKEN));
    let (_, html) = get(&app, "/station/capture", &admin).await;
    assert!(html.contains(CAM_PASS));
}

#[tokio::test]
async fn a_viewer_cannot_download_the_database_or_the_support_bundle() {
    let app = station();
    let viewer = sign_in(&app, "viewer", "viewer-password-1").await;
    for uri in [
        "/admin/system/backup/full",
        "/admin/system/backups/birds.db.backup.1",
        "/admin/support-bundle",
        "/admin/rules/export",
        "/admin/audio/sources/cam1",
        "/admin/audio/sources/cam1/edit",
    ] {
        let (status, _) = get(&app, uri, &viewer).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a viewer may GET {uri}");
    }
    // Counterpart: the status pill the capture tab polls stays readable.
    let (status, _) = get(&app, "/admin/audio/sources/cam1/probe", &viewer).await;
    assert_eq!(status, StatusCode::OK);
}
