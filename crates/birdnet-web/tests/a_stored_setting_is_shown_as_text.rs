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
             ('station_name', 'n&lt;ame'),
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
    // The entity case, on a field whose value is still echoed: a decoded entity
    // would be written back by the next save as a different value.
    assert!(
        html.contains(r#"value="n&amp;lt;ame""#),
        "the name would change on the next save: its entity was decoded"
    );
    // A password is no longer echoed at all (see
    // `a_credential_is_never_printed_in_clear`): the box holds the placeholder,
    // and a save of it keeps the stored `p&lt;ss` byte for byte.
    assert!(
        !html.contains("p&amp;lt;ss") && !html.contains("p&lt;ss"),
        "the saved password was printed"
    );
    assert!(html.contains(r#"name="email_smtp_pass" type="password" value="***REDACTED***""#));
}

/// `/admin/settings` for a station holding `seed` in its settings table.
async fn settings_page(seed: &str) -> String {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(seed).expect("seed settings");
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
    String::from_utf8_lossy(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned()
}

/// From `needle` to the first `end` after it: the region one print site writes.
fn region<'a>(html: &'a str, needle: &str, end: &str) -> &'a str {
    let start = html
        .find(needle)
        .unwrap_or_else(|| panic!("the page has no {needle}"));
    let rest = &html[start..];
    &rest[..rest
        .find(end)
        .unwrap_or_else(|| panic!("no {end} after {needle}"))]
}

/// The same property as above, asked with a probe nothing downstream rewrites,
/// and of the region each value is printed into.
///
/// `seal::redeem` turns any `<script` the server did not seal into
/// `&lt;script` in the finished body, so a test that types a `<script>` and looks
/// for it cannot tell a page that escaped from a page that did not. The test
/// above still stands because it also asks about the closing tag and the whole
/// escaped attribute, neither of which the response pass can forge; it depends on
/// that, though, and a payload without a closing tag would pass it by default.
/// `<i>` is not touched by anything, so what is read back here is the page's
/// own escaping, in an attribute and in element content.
#[tokio::test]
async fn a_stored_tag_is_text_in_an_attribute_and_in_a_textarea() {
    let html = settings_page(
        r#"INSERT INTO settings (key, value) VALUES
             ('station_name', 'Plot"><i>probe</i>'),
             ('notify_body_template', '</textarea><i>probe</i>');"#,
    )
    .await;

    let input = region(&html, r#"<input id="station_name""#, ">");
    assert!(
        input.contains(r#"value="Plot&quot;&gt;&lt;i&gt;probe&lt;/i&gt;""#),
        "the station name is not carried as escaped text in its attribute: {input}"
    );

    let area = region(
        &html,
        r#"<textarea id="notify_body_template""#,
        "</textarea>",
    );
    let content = area.rsplit_once('>').expect("the textarea's opening tag").1;
    assert_eq!(
        content, "&lt;/textarea&gt;&lt;i&gt;probe&lt;/i&gt;",
        "the template closed its own textarea or was not shown as text: {area}"
    );

    assert!(
        !html.contains("<i>probe"),
        "a stored tag reached the settings page as markup"
    );
}
