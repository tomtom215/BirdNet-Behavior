//! A session row must describe the *visitor's* device, not the reverse proxy's.
//!
//! `sessions` has carried `user_agent` and `ip_hash` columns since the accounts
//! work landed, and every call site passed `None` for both. `/admin/accounts`
//! therefore listed every session as "Unknown device" with nothing to tell one
//! from another, which is precisely the question that page exists to answer
//! ("is one of these not mine?").
//!
//! Filling them is only worth anything if the address is resolved correctly.
//! Taking the peer address — the obvious implementation — records the *proxy*
//! on every proxied station: one identical hash for every session, which looks
//! like an answer and is not. So these two gates are a pair, and the second is
//! the one with teeth.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A station with a real admin password, so the login path is exercised rather
/// than the fresh-station bypass.
fn state_with_admin(password: &str) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    let hash = birdnet_db::accounts::hash_password(password).expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        rusqlite::params![hash],
    )
    .expect("set admin password");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// POST /login through the shipped router, with the given forwarded header.
async fn login_through_proxy(state: &AppState, xff: Option<&str>) -> StatusCode {
    let app = birdnet_web::server::build_router(state.clone());
    let mut b = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("user-agent", "ProbeBrowser/1.0")
        .header("origin", "http://localhost")
        .header("host", "localhost");
    if let Some(v) = xff {
        b = b.header("x-forwarded-for", v);
    }
    let req = b
        .body(Body::from("username=admin&password=hunter2correct"))
        .expect("build request");
    app.oneshot(req).await.expect("router responds").status()
}

fn stored_sessions(state: &AppState) -> Vec<(Option<String>, Option<String>)> {
    state
        .with_db(
            |conn| -> rusqlite::Result<Vec<(Option<String>, Option<String>)>> {
                let mut stmt = conn.prepare("SELECT user_agent, ip_hash FROM sessions")?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            },
        )
        .expect("read sessions")
}

/// Gate 1 — the columns are populated at all.
///
/// This one is satisfied by *any* implementation that writes something, the
/// peer-address one included. It is here because without it a regression that
/// reverts to `None` would leave gate 2 vacuously true (no rows to compare).
#[tokio::test]
async fn a_login_records_the_device_it_came_from() {
    let state = state_with_admin("hunter2correct");
    let status = login_through_proxy(&state, None).await;
    assert_eq!(
        status,
        StatusCode::SEE_OTHER,
        "login should redirect on success"
    );

    let rows = stored_sessions(&state);
    assert_eq!(rows.len(), 1, "exactly one session should exist");
    assert_eq!(
        rows[0].0.as_deref(),
        Some("ProbeBrowser/1.0"),
        "the user agent must be recorded, not left NULL"
    );
    assert!(
        rows[0].1.is_some(),
        "the client address hash must be recorded, not left NULL"
    );
}

/// Gate 2 — and it records the *visitor*.
///
/// Two logins arrive through the same proxy from two different visitors. If
/// the fingerprint came from the peer address, both rows would carry the same
/// hash and the accounts page could never distinguish them.
#[tokio::test]
async fn two_visitors_through_one_proxy_get_different_fingerprints() {
    let state = state_with_admin("hunter2correct");
    login_through_proxy(&state, Some("198.51.100.7")).await;
    login_through_proxy(&state, Some("198.51.100.8")).await;

    let rows = stored_sessions(&state);
    assert_eq!(rows.len(), 2, "two logins should produce two sessions");
    let a = rows[0].1.clone().expect("first session has an ip hash");
    let b = rows[1].1.clone().expect("second session has an ip hash");
    assert_ne!(
        a, b,
        "both sessions carry the same address hash, so the fingerprint was \
         taken from the proxy rather than from the visitor"
    );
}

/// Gate 3 — the counterpart, or gate 2 is satisfied by a random value.
///
/// The same visitor twice must produce the *same* hash, otherwise the column
/// is noise and "was this session mine?" is still unanswerable.
#[tokio::test]
async fn the_same_visitor_twice_gets_the_same_fingerprint() {
    let state = state_with_admin("hunter2correct");
    login_through_proxy(&state, Some("198.51.100.7")).await;
    login_through_proxy(&state, Some("198.51.100.7")).await;

    let rows = stored_sessions(&state);
    assert_eq!(rows.len(), 2);
    // `Some(x) == Some(x)` is the claim; `None == None` would satisfy the
    // comparison while proving nothing, and a revert to writing NULL is
    // exactly the regression this file guards. Assert presence first.
    assert!(
        rows[0].1.is_some() && rows[1].1.is_some(),
        "both sessions must carry a hash for the stability claim to mean anything"
    );
    assert_eq!(
        rows[0].1, rows[1].1,
        "one visitor must hash to one value, or the column carries no signal"
    );
}

/// Gate 4 — a forged header from an untrusted peer changes nothing.
///
/// `oneshot` inserts no `ConnectInfo`, so the peer defaults to loopback, which
/// *is* trusted; this gate therefore drives the resolver directly with an
/// untrusted peer rather than through the router. Without it, an attacker
/// reaching a directly-exposed station could write any fingerprint they liked
/// into the operator's session list.
#[test]
fn a_forged_header_from_an_untrusted_peer_does_not_move_the_fingerprint() {
    use axum::http::{HeaderMap, HeaderName, HeaderValue};
    use birdnet_web::client_ip::TrustedProxies;
    use birdnet_web::session::hash_client_ip;

    let trusted = TrustedProxies::default();
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static("198.51.100.7"),
    );
    let peer = "203.0.113.5".parse().unwrap();

    assert_eq!(
        hash_client_ip(trusted.client_ip(&headers, peer)),
        hash_client_ip(peer),
        "the fingerprint must come from the peer when the peer is not a trusted proxy"
    );
    assert_ne!(
        hash_client_ip(trusted.client_ip(&headers, peer)),
        hash_client_ip("198.51.100.7".parse().unwrap()),
    );
}
