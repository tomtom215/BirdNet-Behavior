//! A page title is text. Whatever a URL puts in it must not become markup.
//!
//! The layout filled `<title>{{title}}</title>` with the title as given, and
//! `/species/detail` passes the `?name=` query straight through as the title.
//! So `?name=</title><script>…</script>` closed the title and opened a script
//! — on a public route, no sign-in needed — and the CSP did not stop it:
//! `inject_script_nonce` stamps the per-request nonce onto *every* opening
//! `<script>` in the response, the injected one included. An admin who
//! followed such a link ran the attacker's script same-origin, where the CSRF
//! check passes: a password change is one `fetch` away.
//!
//! The counterpart holds the other side of the fix: titles are escaped once,
//! at the layout, so a caller that used to pre-escape (the detection detail
//! page) must not produce `&amp;#x27;` for a name with an apostrophe.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

fn state() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES ('2026-05-01', '06:00:00', 'Accipiter cooperii', 'Cooper''s Hawk', 0.9)",
        [],
    )
    .expect("seed");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(uri: &str) -> String {
    let resp = build_router(state())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{uri}");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&body).into_owned()
}

/// The `<title>` element as served, from its opening tag to the first `</title>`.
fn title_of(html: &str) -> &str {
    let start = html.find("<title>").expect("a <title>");
    let end = html[start..].find("</title>").expect("a </title>") + start;
    &html[start + "<title>".len()..end]
}

#[tokio::test]
async fn a_name_in_the_url_cannot_close_the_title() {
    let html = get("/species/detail?name=%3C%2Ftitle%3E%3Cscript%3Eowned()%3C%2Fscript%3E").await;
    assert!(
        !html.contains("<script>owned()") && !html.contains("owned()</script>"),
        "the query string became a script element in the page"
    );
    assert_eq!(
        title_of(&html),
        "&lt;/title&gt;&lt;script&gt;owned()&lt;/script&gt; · BirdNet-Behavior",
        "the title must carry the name as escaped text"
    );
}

#[tokio::test]
async fn an_apostrophe_in_a_title_is_escaped_exactly_once() {
    let species = get("/species/detail?name=Cooper%27s%20Hawk").await;
    assert_eq!(title_of(&species), "Cooper&#x27;s Hawk · BirdNet-Behavior");

    let detail =
        get("/detections/detail?date=2026-05-01&time=06:00:00&name=Cooper%27s%20Hawk").await;
    let title = title_of(&detail);
    assert!(
        title.starts_with("Cooper&#x27;s Hawk · "),
        "the detection page's title is double-escaped or missing the name: {title}"
    );
}
