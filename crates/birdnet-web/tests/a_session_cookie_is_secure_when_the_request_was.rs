//! The session cookie is `Secure` when the request it answers reached the
//! station over HTTPS — decided per request, from the connection.
//!
//! `Secure` was decided from one place: the scheme of `BNB_PUBLIC_URL`. A
//! station started with `--tls-mode` and no such variable, or one behind a TLS
//! reverse proxy that was never told its public name, therefore issued a cookie
//! without `Secure` on a connection that was plainly encrypted — and a cookie
//! is not scoped to a port, so the same cookie would also be sent in clear to
//! the station's plain-HTTP listener. The opposite setting had the opposite
//! trap: an `https://` public URL made every login `Secure`, including one from
//! `http://192.168.1.5:8502` on the LAN, where the browser drops a `Secure`
//! cookie and the sign-in loops without saying why.
//!
//! The rule is now the connection's: the TLS listener's own requests, or a
//! trusted proxy's `X-Forwarded-Proto` / `Forwarded: proto=`. A header from a
//! peer that is not a trusted proxy is ignored, the same rule that decides
//! whose `X-Forwarded-For` is believed. (`BNB_PUBLIC_URL` as a fallback for a
//! proxy that sends neither header is covered by the unit tests in
//! `request_scheme.rs`: the environment is process-wide, and these tests run in
//! parallel.)

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, header};
use rustls_pki_types::pem::PemObject as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt as _;

use birdnet_web::state::AppState;
use birdnet_web::tls::{self, TlsMode, TlsSettings};

/// The seed admin's password, built per process: the `CodeQL` query on
/// hard-coded cryptographic values follows a literal or a `const` into any
/// parameter called `password` (CLAUDE.md); `format!` over a runtime value is
/// not one.
fn password() -> String {
    format!("a-real-password-{}", std::process::id())
}

fn state() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let hash = birdnet_db::accounts::hash_password(&password()).expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&hash],
    )
    .expect("give the seed admin a real password");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// `POST /login` (or `/logout`) from `peer`, with `extra` headers, over the
/// plain-HTTP path; returns the `Set-Cookie` value.
async fn cookie_via(path: &str, peer: &str, extra: &[(&str, &str)]) -> String {
    let app = birdnet_web::server::build_router(state());
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", "http://localhost")
        .header("host", "localhost");
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let mut req = req
        .body(Body::from(format!(
            "username=admin&password={}",
            password()
        )))
        .expect("request");
    let peer: SocketAddr = format!("{peer}:40000").parse().unwrap();
    req.extensions_mut().insert(ConnectInfo(peer));
    let res = app.oneshot(req).await.expect("router responds");
    res.headers()
        .get(header::SET_COOKIE)
        .unwrap_or_else(|| panic!("{path} set no cookie: {:?}", res.status()))
        .to_str()
        .expect("ascii")
        .to_owned()
}

fn is_secure(cookie: &str) -> bool {
    cookie.split(';').any(|a| a.trim() == "Secure")
}

// ── behind a proxy ──────────────────────────────────────────────────────

/// The baseline, and the counterpart to everything below: a plain-HTTP login
/// from the LAN gets a cookie the browser will send back over plain HTTP.
#[tokio::test]
async fn a_plain_http_login_gets_a_cookie_that_works_over_http() {
    let cookie = cookie_via("/login", "192.168.1.20", &[]).await;
    assert!(
        !is_secure(&cookie),
        "a Secure cookie over HTTP loops: {cookie}"
    );
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"));
}

#[tokio::test]
async fn a_trusted_proxy_that_says_https_gets_a_secure_cookie() {
    for (name, value) in [
        ("x-forwarded-proto", "https"),
        ("x-forwarded-proto", "HTTPS"),
        // The browser-facing hop is the leftmost: a chain whose later hop was
        // plain HTTP still began on HTTPS.
        ("x-forwarded-proto", "https, http"),
        (
            "forwarded",
            "for=203.0.113.5;proto=https;host=birds.example.com",
        ),
    ] {
        let cookie = cookie_via("/login", "127.0.0.1", &[(name, value)]).await;
        assert!(is_secure(&cookie), "{name}: {value} was ignored: {cookie}");
    }
}

#[tokio::test]
async fn a_proxy_that_says_http_does_not() {
    for (name, value) in [
        ("x-forwarded-proto", "http"),
        ("forwarded", "for=203.0.113.5;proto=http"),
    ] {
        let cookie = cookie_via("/login", "127.0.0.1", &[(name, value)]).await;
        assert!(
            !is_secure(&cookie),
            "{name}: {value} made it Secure: {cookie}"
        );
    }
}

/// The header is the sender's own words: from a peer that is not a trusted
/// proxy it is ignored, as `X-Forwarded-For` is. (Believing it would only hurt
/// the sender — their browser would drop the cookie — but a rule that asks
/// "who said so?" for one forwarded header and not another is the kind that
/// gets copied wrongly.)
#[tokio::test]
async fn a_stranger_cannot_make_the_cookie_secure_by_asking() {
    let cookie = cookie_via("/login", "203.0.113.9", &[("x-forwarded-proto", "https")]).await;
    assert!(!is_secure(&cookie), "{cookie}");
}

/// Signing out clears the cookie with the attributes it was set with; a browser
/// matches on name, domain and path but a `Secure` mismatch is how a clear
/// stops working on some of them.
#[tokio::test]
async fn the_clearing_cookie_follows_the_same_rule() {
    let secure = cookie_via("/logout", "127.0.0.1", &[("x-forwarded-proto", "https")]).await;
    assert!(
        secure.contains("Max-Age=0") && is_secure(&secure),
        "{secure}"
    );
    let plain = cookie_via("/logout", "127.0.0.1", &[]).await;
    assert!(plain.contains("Max-Age=0") && !is_secure(&plain), "{plain}");
}

// ── on the station's own TLS listener ───────────────────────────────────

/// A rustls client that trusts only the CA the station generated.
fn client_config(state_dir: &std::path::Path) -> Arc<rustls::ClientConfig> {
    let pem = std::fs::read(tls::ca_certificate_path(state_dir)).expect("read CA");
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pki_types::CertificateDer::pem_slice_iter(&pem) {
        roots.add(cert.expect("parse CA")).expect("trust CA");
    }
    Arc::new(
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth(),
    )
}

/// `--tls-mode` without `BNB_PUBLIC_URL`: the request is encrypted, so the
/// cookie is `Secure`, whatever headers it does or does not carry.
#[tokio::test]
async fn a_login_on_the_tls_listener_gets_a_secure_cookie() {
    let dir = tempfile::tempdir().expect("state dir");
    let settings = TlsSettings {
        mode: TlsMode::SelfSigned,
        state_dir: dir.path().to_path_buf(),
        hostnames: vec!["localhost".to_string()],
        ..TlsSettings::default()
    };
    let (config, _resolver) = tls::server_config(&settings)
        .expect("server config")
        .expect("self-signed mode produces a config");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let app = birdnet_web::server::build_router(state());
    let server = tokio::spawn(async move {
        tls::serve(listener, config, app, async {
            let _ = stop_rx.await;
        })
        .await
        .expect("serve");
    });

    let connector = tokio_rustls::TlsConnector::from(client_config(dir.path()));
    let tcp = TcpStream::connect(addr).await.expect("connect");
    let name = rustls_pki_types::ServerName::try_from("localhost").expect("server name");
    let mut tls_stream = connector.connect(name, tcp).await.expect("handshake");
    let body = format!("username=admin&password={}", password());
    let req = format!(
        "POST /login HTTP/1.1\r\nHost: localhost\r\nOrigin: https://localhost\r\n\
         Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    tls_stream
        .write_all(req.as_bytes())
        .await
        .expect("write request");
    let mut buf = Vec::new();
    let _ = tls_stream.read_to_end(&mut buf).await;
    let response = String::from_utf8_lossy(&buf).into_owned();

    let _ = stop_tx.send(());
    server.await.expect("server task");

    let cookie = response
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .unwrap_or_else(|| panic!("the login set no cookie:\n{response}"));
    assert!(
        is_secure(cookie),
        "an encrypted login got a cookie that would also travel in clear: {cookie}"
    );
}
