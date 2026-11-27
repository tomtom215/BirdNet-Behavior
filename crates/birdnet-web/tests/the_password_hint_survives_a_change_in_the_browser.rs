//! The sign-in page's "Forgotten the password?" hint (N25-14).
//!
//! It gave one way back, `sudo grep '^CADDY_PWD'`, which prints the password
//! the installer wrote. After a change on the Accounts page the file still
//! holds that old value and the station ignores it, so a locked-out owner who
//! followed the hint was refused again, with no next step. The hint must say
//! when the file's value is stale and give a reset that works then.
//!
//! Read from the rendered `<details>`, not the whole document: the template's
//! own comment ships to the browser and quotes the hint.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

async fn login_page() -> String {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let res = birdnet_web::server::build_router(state)
        .oneshot(
            Request::get("/login")
                .header("host", "localhost")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

fn hint(html: &str) -> &str {
    let start = html
        .find(r#"<details class="login-card__hint">"#)
        .expect("the hint is on the page");
    let end = start + html[start..].find("</details>").expect("closed");
    &html[start..end]
}

#[tokio::test]
async fn the_hint_does_not_offer_a_stale_password_as_the_only_way_back() {
    let html = login_page().await;
    let hint = hint(&html);
    assert!(
        hint.contains("changed in the browser"),
        "the hint must say the file's value is stale after a change in the browser:\n{hint}"
    );
    assert!(
        hint.contains("install.sh") && hint.contains("reset-password"),
        "the hint must give a reset that works after such a change:\n{hint}"
    );
    assert!(
        hint.contains(".env"),
        "and the container's equivalent:\n{hint}"
    );
}

/// The counterpart: what the hint already got right stays. The file and the
/// command for a station whose password was never changed, and the username.
#[tokio::test]
async fn the_hint_still_names_the_file_the_command_and_the_user() {
    let html = login_page().await;
    let hint = hint(&html);
    assert!(hint.contains("sudo grep '^CADDY_PWD' /etc/birdnet/birdnet.conf"));
    assert!(hint.contains("<code>admin</code>"));
}
