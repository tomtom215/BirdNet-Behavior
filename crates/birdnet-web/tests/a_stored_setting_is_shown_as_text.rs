//! A setting is shown in its form exactly as it is stored — as text.
//!
//! The settings renderer put every stored value into `value="…"` (or a
//! `<textarea>`) without escaping. Two consequences, both from values a person
//! can legitimately type or an automation can legitimately `PUT`:
//!
//! * a site name containing `"` closed its attribute, and `"><script>…` became
//!   a script on the admin page — a route from the bearer API token (which may
//!   write settings) to whatever a signed-in admin can do;
//! * a password containing an entity (`p&lt;ss`) was shown as `p<ss`, and the
//!   next save of the form wrote `p<ss` back, silently changing it.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

#[tokio::test]
async fn a_stored_value_cannot_break_out_of_its_field() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        r#"INSERT INTO settings (key, value) VALUES
             ('site_name', 'Garden"><script>owned()</script>'),
             ('email_smtp_pass', 'p&lt;ss');"#,
    )
    .expect("seed settings");
    let resp = build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
    .oneshot(
        Request::builder()
            .uri("/admin/settings")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = String::from_utf8_lossy(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned();

    // Not `<script>owned()`: the CSP layer stamps a nonce onto every opening
    // script tag, injected ones included, so that literal never appears even
    // when the injection succeeded. The closing half cannot be rewritten.
    assert!(
        !html.contains("owned()</script>"),
        "a stored site name became a script on the settings page"
    );
    assert!(
        html.contains(r#"value="Garden&quot;&gt;&lt;script&gt;owned()&lt;/script&gt;""#),
        "the site name is not carried as escaped text"
    );
    assert!(
        html.contains(r#"value="p&amp;lt;ss""#),
        "the password would change on the next save: its entity was decoded"
    );
}
