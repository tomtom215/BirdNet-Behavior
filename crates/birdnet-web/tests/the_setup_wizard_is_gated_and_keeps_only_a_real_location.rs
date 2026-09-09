//! The first-run wizard, on the two stations the installer actually produces.
//!
//! `ON-6`: the bare-metal installer generates an admin password and, on a
//! headless install, writes no location, so the very first page load is the
//! wizard. Its page was public while its save was behind the admin gate, so
//! Finish answered a bare `401 Sign in required.` whose `Location` led, after
//! signing in, to `405 GET /onboarding/save` — every answer gone. The page now
//! lives behind the same gate as its save: an open station still shows it,
//! a password-protected one asks for the password first.
//!
//! And the save guarded its fields only with `is_empty()`, so
//! `latitude=999&longitude=abc&timezone=Mars/Olympus` was persisted, the
//! settings overlay dropped the pair silently at the next start, and the
//! doctor said "no latitude/longitude set" while the wizard showed 999. Half a
//! pair was written too, so two runs could leave the station at a point
//! nobody typed.
//!
//! Observed failing against the shipped tree: `GET /onboarding` on the
//! password station answered `200`, and the settings table held
//! `latitude=999`, `longitude=abc`, `timezone=Mars/Olympus` after the POST.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

use birdnet_db::accounts::{self, UserStore};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

fn station(password: Option<&str>) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    if let Some(pwd) = password {
        state.with_db(|conn| {
            let admin = conn.find_user_by_name("admin").expect("seed admin");
            let hash = accounts::hash_password(pwd).expect("hash");
            conn.set_password(admin.id, &hash).expect("set");
        });
    }
    (dir, state)
}

async fn get(state: &AppState, path: &str) -> (StatusCode, Option<String>, String) {
    let resp = build_router(state.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let location = resp
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

async fn post_form(state: &AppState, path: &str, body: &str) -> (StatusCode, Option<String>) {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let location = resp
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned());
    (resp.status(), location)
}

/// As [`post_form`], also returning the `Set-Cookie` the response carried.
async fn post_form_with_cookie(
    state: &AppState,
    path: &str,
    body: &str,
) -> (StatusCode, Option<String>, Option<String>) {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::HOST, "localhost")
                .header(header::ORIGIN, "http://localhost")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let header_str = |name: header::HeaderName| {
        resp.headers()
            .get(name)
            .map(|v| v.to_str().unwrap().to_owned())
    };
    (
        resp.status(),
        header_str(header::LOCATION),
        header_str(header::SET_COOKIE),
    )
}

/// `GET path` carrying `cookie` (the raw `Set-Cookie` value's first pair).
async fn get_with_cookie(
    state: &AppState,
    path: &str,
    cookie: &str,
) -> (StatusCode, Option<String>) {
    let pair = cookie.split(';').next().unwrap_or_default().to_owned();
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(path)
                .header(header::COOKIE, pair)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let location = resp
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned());
    (resp.status(), location)
}

fn setting(state: &AppState, key: &str) -> Option<String> {
    state.with_db(|conn| {
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    })
}

#[tokio::test]
async fn a_password_protected_station_asks_for_the_password_before_the_wizard() {
    let (_dir, state) = station(Some("probe-pass-1"));

    let (status, location, _) = get(&state, "/onboarding").await;
    assert_eq!(
        status,
        StatusCode::SEE_OTHER,
        "the wizard page was served without a login"
    );
    assert_eq!(location.as_deref(), Some("/login?next=/onboarding"));

    // The address the old 401 sent people to: a redirect to sign in, never a 405.
    let (status, location, _) = get(&state, "/onboarding/save").await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{location:?}");
    assert_eq!(location.as_deref(), Some("/login?next=/onboarding/save"));

    // And the save itself is still refused: the gate is on both halves.
    let (status, _) = post_form(&state, "/onboarding/save", "latitude=1&longitude=2").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(setting(&state, "latitude"), None);
}

/// The counterpart: a station with no password is the fresh Docker run and the
/// operator who cleared it, and it must keep showing the wizard to anyone.
#[tokio::test]
async fn an_open_station_still_shows_the_wizard_to_everyone() {
    let (_dir, state) = station(None);
    let (status, _, body) = get(&state, "/onboarding").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Set up your station"), "{body}");

    // A stale tab, or a bookmark of the save URL, lands back on the wizard.
    let (status, location, _) = get(&state, "/onboarding/save").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/onboarding"));
}

#[tokio::test]
async fn the_wizard_keeps_a_real_location_and_nothing_else() {
    let (_dir, state) = station(None);

    // Out of range, not a number, not a zone: nothing is persisted, and the
    // request still completes so the operator is not stranded.
    let (status, location) = post_form(
        &state,
        "/onboarding/save",
        "latitude=999&longitude=abc&timezone=Mars%2FOlympus",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{location:?}");
    assert_eq!(
        setting(&state, "latitude"),
        None,
        "an out-of-range latitude was stored"
    );
    assert_eq!(
        setting(&state, "longitude"),
        None,
        "a non-numeric longitude was stored"
    );
    assert_eq!(
        setting(&state, "timezone"),
        None,
        "a zone that does not exist was stored"
    );

    // Half a pair is no location at all.
    post_form(&state, "/onboarding/save", "latitude=51.48&longitude=").await;
    assert_eq!(
        setting(&state, "latitude"),
        None,
        "half a coordinate pair was stored"
    );

    // The counterpart: a real answer is kept, including the EU decimal comma
    // the settings form already accepts, and a real zone.
    post_form(
        &state,
        "/onboarding/save",
        "latitude=51%2C48&longitude=-0.13&timezone=Europe%2FLondon",
    )
    .await;
    assert_eq!(setting(&state, "latitude").as_deref(), Some("51.48"));
    assert_eq!(setting(&state, "longitude").as_deref(), Some("-0.13"));
    assert_eq!(
        setting(&state, "timezone").as_deref(),
        Some("Europe/London")
    );
    assert_eq!(
        setting(&state, "onboarding_complete").as_deref(),
        Some("true")
    );
}

/// DD-14: an open station's wizard asks for the admin password first, and the
/// browser that finishes setup with one owns the station — it leaves with a
/// session, and everybody else meets the login page from then on. Before
/// this, the six steps never asked, and `/admin/*` stayed open until the
/// operator found a form labelled "Reset password" on their own.
#[tokio::test]
async fn an_open_station_asks_for_a_password_and_the_first_browser_owns_it() {
    let (_dir, state) = station(None);

    let (status, _, body) = get(&state, "/onboarding").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("First, the admin password"), "{body}");
    assert!(body.contains(r#"name="password_confirm""#), "{body}");

    // Anyone can reach an admin page right now — that is the condition.
    let (status, _, _) = get(&state, "/admin/settings").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an open station serves /admin/settings"
    );

    let (status, location, cookie) = post_form_with_cookie(
        &state,
        "/onboarding/save",
        "password=correct-horse-battery&password_confirm=correct-horse-battery&latitude=51.48&longitude=-0.13",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{location:?}");
    assert_eq!(location.as_deref(), Some("/"));
    let cookie = cookie.expect("the browser that set the password leaves signed in");
    assert!(cookie.starts_with("bnb-session="), "{cookie}");
    assert_eq!(setting(&state, "latitude").as_deref(), Some("51.48"));

    // Everybody else now meets the login page; the one that set it does not.
    let (status, location, _) = get(&state, "/admin/settings").await;
    assert_eq!(status, StatusCode::SEE_OTHER, "the station is gated now");
    assert_eq!(location.as_deref(), Some("/login?next=/admin/settings"));
    let (status, _) = get_with_cookie(&state, "/admin/settings", &cookie).await;
    assert_eq!(status, StatusCode::OK, "the setting browser is signed in");

    // And the wizard no longer asks, because the answer is on file.
    let (status, _) = get_with_cookie(&state, "/onboarding", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, body) = {
        let pair = cookie.split(';').next().unwrap().to_owned();
        let resp = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/onboarding")
                    .header(header::COOKIE, pair)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            None::<String>,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    };
    assert!(!body.contains("First, the admin password"), "{body}");
}

/// A password that does not match its confirmation, or is too short, saves
/// nothing at all and sends the operator back to the step — not a configured,
/// still-open station with no explanation.
#[tokio::test]
async fn a_refused_password_saves_nothing_and_says_why() {
    let (_dir, state) = station(None);
    for body in [
        "password=correct-horse-battery&password_confirm=different-one-here&latitude=51.48&longitude=-0.13",
        "password=short&password_confirm=short&latitude=51.48&longitude=-0.13",
    ] {
        let (status, location, cookie) =
            post_form_with_cookie(&state, "/onboarding/save", body).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{body}");
        assert_eq!(
            location.as_deref(),
            Some("/onboarding?error=password"),
            "{body}"
        );
        assert!(cookie.is_none(), "{body}");
        assert_eq!(setting(&state, "latitude"), None, "{body}");
        assert_eq!(setting(&state, "onboarding_complete"), None, "{body}");
        let (status, _, _) = get(&state, "/admin/settings").await;
        assert_eq!(status, StatusCode::OK, "still open: nothing was set");
    }
    let (_, _, page) = get(&state, "/onboarding?error=password").await;
    assert!(page.contains("did not match"), "{page}");

    // Blank means "not now": setup completes, the station stays open, no cookie.
    let (status, location, cookie) =
        post_form_with_cookie(&state, "/onboarding/save", "latitude=51.48&longitude=-0.13").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/"));
    assert!(cookie.is_none());
    assert_eq!(setting(&state, "latitude").as_deref(), Some("51.48"));
}
