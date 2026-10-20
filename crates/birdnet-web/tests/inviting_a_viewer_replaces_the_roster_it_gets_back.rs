//! Inviting a viewer swaps the roster in place of the roster, not after it.
//!
//! `POST /admin/accounts/users` answers with the whole user list ("Re-render the
//! full user list so the new row appears"), and the form that posts it asked
//! htmx to append that answer to `#user-list` (`hx-swap="beforeend"`). So every
//! invitation left the page showing the existing users twice — measured in
//! Chromium against the fixture: one row before, three after, "Administrator,
//! Administrator, Jess" — until the page was reloaded. The Remove button beside
//! it swaps `innerHTML`, which is what a whole-roster answer needs.
//!
//! The browser is where the duplication shows, but what decides it is a pair of
//! facts about the server, and this holds both: the answer is the whole roster,
//! and the form replaces the list with it.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_invite_form_replaces_the_list_with_the_whole_roster_it_is_answered_with() {
    let app = station();

    let (status, page) = send(
        &app,
        Request::builder()
            .uri("/station/access")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let at = page
        .find(r#"<form hx-post="/admin/accounts/users""#)
        .expect("the Access tab has the invite form");
    let rest = &page[at..];
    let form_tag = &rest[..rest.find('>').expect("the form tag closes")];
    assert!(
        form_tag.contains(r##"hx-target="#user-list""##),
        "the invite form no longer targets the roster: {form_tag}"
    );
    assert!(
        form_tag.contains(r#"hx-swap="innerHTML""#),
        "the invite form appends a whole roster to the roster it already shows: {form_tag}"
    );

    // The other half: what comes back is the whole roster, the seed admin
    // included, so appending it would show that row twice.
    let (_, answer) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/admin/accounts/users")
            .header("host", "localhost")
            .header("origin", "http://localhost")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(
                "username=newbie&label=Jess&password=correct-horse-battery",
            ))
            .unwrap(),
    )
    .await;
    assert!(
        answer.contains(">Administrator<") && answer.contains(">Jess<"),
        "the answer to an invitation is the whole roster: {answer}"
    );
}
