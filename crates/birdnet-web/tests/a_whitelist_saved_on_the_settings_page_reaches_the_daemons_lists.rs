//! The species whitelist can be set where a BirdNET-Pi user looks for it, and
//! the daemon reads what was saved.
//!
//! BirdNET-Pi's `whitelist_species_list.txt` exempts a species from the
//! occurrence threshold and from nothing else. A station moved here from
//! BirdNET-Pi had no way to say so, and the one bird it listed there (a rare
//! visitor the range model keeps holding back) stopped arriving.
//!
//! Three links, each of which can be dropped without the others noticing, and
//! so each is held: the Settings page renders the control; posting it stores
//! the row; and `configured_species_lists`, the function the detection daemon
//! refreshes through, returns it. The counterpart asserts the other two lists
//! are untouched, so "whitelist saved" cannot pass by a handler that writes
//! every list.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::routes::admin::species::handler::configured_species_lists;
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, String) {
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn a_whitelist_posted_on_the_settings_page_is_what_the_daemon_reads() {
    let (_dir, state) = station();

    let (status, page) = send(
        &state,
        Request::builder()
            .uri("/admin/settings")
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains(r#"name="species_whitelist""#),
        "the Settings page has no whitelist control"
    );

    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs([("species_whitelist", " Snowy Owl , Great Grey Owl ")])
        .finish();
    let (status, _) = send(
        &state,
        Request::builder()
            .method("POST")
            .uri("/admin/settings")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "POST /admin/settings");

    let lists = configured_species_lists(&state);
    assert_eq!(
        lists.whitelist,
        vec!["Snowy Owl".to_string(), "Great Grey Owl".to_string()],
        "the whitelist the daemon refreshes through is not what was saved"
    );
    assert!(lists.include.is_empty(), "the include list was written too");
    assert!(lists.exclude.is_empty(), "the exclude list was written too");

    // And the page shows it back.
    let (_, page) = send(
        &state,
        Request::builder()
            .uri("/admin/settings")
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert!(
        page.contains("Snowy Owl"),
        "the saved whitelist is not shown"
    );
}

async fn post_form(state: &AppState, uri: &str, body: &str) -> (StatusCode, String) {
    send(
        state,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body.to_owned()))
            .expect("request"),
    )
    .await
}

#[tokio::test]
async fn the_species_card_adds_to_and_removes_from_the_whitelist_alone() {
    let (_dir, state) = station();

    let (status, partial) =
        post_form(&state, "/admin/species/whitelist/add", "name=Snowy+Owl").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        partial.contains("Snowy Owl"),
        "the card does not show the new entry"
    );
    let lists = configured_species_lists(&state);
    assert_eq!(lists.whitelist, vec!["Snowy Owl".to_string()]);
    assert!(lists.include.is_empty() && lists.exclude.is_empty());

    // The other lists' buttons are untouched by it: excluding a different bird
    // leaves the whitelist as it was.
    post_form(&state, "/admin/species/exclude/add", "name=House+Sparrow").await;
    let lists = configured_species_lists(&state);
    assert_eq!(lists.whitelist, vec!["Snowy Owl".to_string()]);
    assert_eq!(lists.exclude, vec!["House Sparrow".to_string()]);

    let (status, partial) =
        post_form(&state, "/admin/species/whitelist/remove", "name=Snowy+Owl").await;
    assert_eq!(status, StatusCode::OK);
    assert!(configured_species_lists(&state).whitelist.is_empty());
    assert!(
        partial.contains("House Sparrow"),
        "removing a whitelist entry must not clear the exclusion list's card"
    );
    assert_eq!(
        configured_species_lists(&state).exclude,
        vec!["House Sparrow".to_string()]
    );
}
