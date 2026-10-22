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

/// Built at run time, not written as a literal: CodeQL's
/// `rust/hard-coded-cryptographic-value` follows a string literal into any
/// parameter called `password` (see `tests/no_literal_reaches_a_credential_parameter.rs`).
fn admin_password() -> String {
    format!("admin-password-{}", std::process::id())
}

fn viewer_password() -> String {
    format!("viewer-password-{}", std::process::id())
}

fn station() -> axum::Router {
    build_router(station_state())
}

fn station_state() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let admin_hash = birdnet_db::accounts::hash_password(&admin_password()).expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&admin_hash],
    )
    .expect("admin password");
    let viewer_hash = birdnet_db::accounts::hash_password(&viewer_password()).expect("hash");
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
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
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
    let viewer = sign_in(&app, "viewer", &viewer_password()).await;
    for uri in ["/admin/settings", "/station/alerts", "/station/capture"] {
        let (status, html) = get(&app, uri, &viewer).await;
        assert_eq!(status, StatusCode::OK, "{uri} is still readable");
        for canary in [SMTP_PASS, BW_TOKEN, CAM_PASS] {
            assert!(
                !html.contains(canary),
                "{uri} showed a viewer the credential {canary}"
            );
        }
    }
    // Counterpart: the admin, who types these in, still gets the forms and the
    // list. Nobody is shown a credential in clear any more (the administrator's
    // pages are held to that by `a_credential_is_never_printed_in_clear`), so
    // what the admin sees in place of them is the saved-value placeholder and
    // the camera's address with only its password hidden.
    let admin = sign_in(&app, "admin", &admin_password()).await;
    let (status, html) = get(&app, "/admin/settings", &admin).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !html.contains(SMTP_PASS) && !html.contains(BW_TOKEN),
        "the administrator's settings form printed a credential"
    );
    assert!(html.contains(r#"name="email_smtp_pass" type="password" value="***REDACTED***""#));
    let (_, html) = get(&app, "/station/capture", &admin).await;
    assert!(
        !html.contains(CAM_PASS),
        "the capture tab printed the camera password"
    );
    assert!(html.contains("rtsp://viewer:***REDACTED***@camera.local/stream"));
}

#[tokio::test]
async fn a_viewer_cannot_download_the_database_or_the_support_bundle() {
    let app = station();
    let viewer = sign_in(&app, "viewer", &viewer_password()).await;
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

/// `GET /admin/migrate/conf/lines` is the BirdNET-Pi import's download of the
/// station's `birdnet.conf` keys, verbatim (the Flickr key among them). It was
/// reachable by a viewer: the comment on it said "behind admin RBAC", and the
/// RBAC gate refuses only the paths its deny list names.
///
/// The token is not what is under test here (a bad one answers 404 to an
/// administrator, which proves the request reached the handler); the deny list
/// is, and it answers before the handler runs.
#[tokio::test]
async fn a_viewer_cannot_download_the_conf_lines_the_import_offers() {
    let app = station();
    let viewer = sign_in(&app, "viewer", &viewer_password()).await;
    let admin = sign_in(&app, "admin", &admin_password()).await;
    let uri = "/admin/migrate/conf/lines?token=1";
    let (status, _) = get(&app, uri, &viewer).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a viewer may GET {uri}");
    // Counterpart: the administrator is not refused by the gate; the handler
    // runs and finds no such token.
    let (status, _) = get(&app, uri, &admin).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the admin reaches the handler"
    );
}

/// The doctor prints the MQTT broker's host and the certificate and CA paths;
/// the boot id is the process id and its start time in nanoseconds. A read-only
/// account has no use for either (it cannot press the restart button the boot
/// id exists for).
#[tokio::test]
async fn a_viewer_cannot_read_the_diagnostics_or_the_process_id() {
    let app = station();
    let viewer = sign_in(&app, "viewer", &viewer_password()).await;
    let admin = sign_in(&app, "admin", &admin_password()).await;
    for uri in [
        "/admin/doctor",
        "/admin/doctor.json",
        "/admin/system/boot-id",
    ] {
        let (status, _) = get(&app, uri, &viewer).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a viewer may GET {uri}");
        let (status, _) = get(&app, uri, &admin).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "the admin was refused {uri}");
    }
    let (status, body) = get(&app, "/admin/system/boot-id", &admin).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains('-'), "a boot id: {body:?}");
}

/// The live log carries whatever a subsystem wrote while it ran: paths, host
/// names, addresses, the names of accounts that signed in. None of it goes
/// through the redaction the forms use, so a viewer is refused the stream and
/// its page, and is not offered a link to either.
#[tokio::test]
async fn a_viewer_cannot_read_the_live_log_and_is_not_offered_it() {
    let app = station();
    let viewer = sign_in(&app, "viewer", &viewer_password()).await;
    let admin = sign_in(&app, "admin", &admin_password()).await;
    for uri in ["/admin/system/logs", "/admin/system/logs/page"] {
        let (status, _) = get(&app, uri, &viewer).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a viewer may GET {uri}");
    }
    // Counterpart: the administrator reaches both (the stream answers at once
    // with its headers; the body is not read, it never ends).
    for uri in ["/admin/system/logs", "/admin/system/logs/page"] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::COOKIE, &admin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "the admin was refused {uri}");
    }
    // The pages that link to the viewer page offer the link to the admin
    // and not to a viewer, who would be sent to a refusal.
    let link = r#"href="/admin/system/logs/page""#;
    for uri in ["/admin/overview", "/admin/system", "/station/data"] {
        let (status, html) = get(&app, uri, &viewer).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(
            !html.contains(link),
            "{uri} offered a viewer a link to the log, which refuses them"
        );
        let (_, html) = get(&app, uri, &admin).await;
        assert!(html.contains(link), "{uri}: the admin lost the log link");
    }
}

/// The classifier panel is on the Capture tab, which a viewer may open. A
/// configured model that is missing prints its path; the viewer's copy does not.
#[tokio::test]
async fn the_classifier_panel_shows_a_viewer_no_file_path() {
    let state = station_state();
    let missing = format!("/srv/secret-models-{}/nope.onnx", std::process::id());
    state
        .classifier_setup()
        .configure(birdnet_web::classifier_setup::Layout {
            model_dir: std::path::PathBuf::from("/srv/secret-models"),
            model: Some(std::path::PathBuf::from(&missing)),
            ..Default::default()
        });
    let app = build_router(state);
    let viewer = sign_in(&app, "viewer", &viewer_password()).await;
    let admin = sign_in(&app, "admin", &admin_password()).await;
    for uri in ["/admin/classifier/panel", "/station/capture"] {
        let (status, html) = get(&app, uri, &viewer).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(
            html.contains("did not start with it"),
            "{uri}: the panel still says what is wrong, without the path"
        );
        assert!(
            !html.contains(&missing) && !html.contains("secret-models"),
            "{uri} showed a viewer where the station keeps its model"
        );
        // Counterpart: the administrator, who has to put the file there, sees it.
        let (_, html) = get(&app, uri, &admin).await;
        assert!(
            html.contains(&missing),
            "{uri}: the admin's panel lost the path"
        );
    }
}

/// The Diagnostics page answers a viewer `403` (above), so a viewer's admin nav
/// and the classifier panel do not offer a link to it. A link that only ever
/// fails reads as a broken station.
#[tokio::test]
async fn a_viewer_is_not_offered_the_diagnostics_link() {
    let state = station_state();
    let model = tempfile::NamedTempFile::new().expect("a model file that exists");
    state
        .classifier_setup()
        .configure(birdnet_web::classifier_setup::Layout {
            model_dir: model.path().parent().expect("dir").to_path_buf(),
            model: Some(model.path().to_path_buf()),
            ..Default::default()
        });
    let app = build_router(state);
    let viewer = sign_in(&app, "viewer", &viewer_password()).await;
    let admin = sign_in(&app, "admin", &admin_password()).await;
    let link = r#"href="/admin/doctor""#;
    for uri in [
        "/admin/overview",
        "/admin/system",
        "/admin/settings",
        "/admin/classifier/panel",
        "/station/capture",
    ] {
        let (status, html) = get(&app, uri, &viewer).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(
            !html.contains(link),
            "{uri} offered a viewer a link to a page that refuses them"
        );
        // Counterpart: the administrator keeps it, wherever it was offered.
        let (_, html) = get(&app, uri, &admin).await;
        assert!(
            html.contains(link),
            "{uri}: the administrator lost the Diagnostics link"
        );
    }
    // The rest of the nav is untouched for a viewer: only the refused entry goes.
    let (_, html) = get(&app, "/admin/overview", &viewer).await;
    assert!(
        html.contains(r#"href="/admin/system""#) && html.contains(r#"href="/admin/audio""#),
        "a viewer's nav lost entries it may open"
    );
    // And the engine's own page keeps telling the viewer something is wrong.
    let (_, html) = get(&app, "/admin/classifier/panel", &viewer).await;
    assert!(html.contains("did not start with it"), "{html}");
}

/// A CPU profile (`G-24`) is the station's stacks, and taking one slows it:
/// an administrator's to ask for, not a viewer's or a passer-by's.
#[cfg(feature = "profiling")]
#[tokio::test]
async fn only_an_administrator_can_take_a_cpu_profile() {
    let app = station();
    let uri = "/admin/debug/pprof/profile?seconds=1&format=folded";
    let (status, _) = get(&app, uri, "").await;
    assert_ne!(status, StatusCode::OK, "signed out");
    let viewer = sign_in(&app, "viewer", &viewer_password()).await;
    let (status, _) = get(&app, uri, &viewer).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a viewer");

    let admin = sign_in(&app, "admin", &admin_password()).await;
    let (status, _) = get(&app, "/admin/debug/pprof/profile?seconds=0", &admin).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "zero seconds");
    // Something to sample: the profile counts CPU time, so an idle test
    // process would answer an empty (and correct) profile.
    let busy = std::thread::spawn(|| {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut x = 0_u64;
        while std::time::Instant::now() < until {
            x = std::hint::black_box(x.wrapping_add(1));
        }
    });
    let (status, body) = get(&app, uri, &admin).await;
    busy.join().expect("busy thread");
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.is_empty(), "the busy thread was sampled");
    assert!(
        body.lines().all(|l| l
            .rsplit_once(' ')
            .is_some_and(|(_, n)| n.parse::<u64>().is_ok())),
        "folded stacks, each with its count: {body}"
    );
}
