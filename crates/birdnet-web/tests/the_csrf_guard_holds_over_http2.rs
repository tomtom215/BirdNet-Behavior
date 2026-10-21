//! The same-origin guard holds on HTTP/2, where there is no `Host` header.
//!
//! # What was wrong
//!
//! `is_same_origin` compared an `Origin`/`Referer` against the request's
//! `Host` *header* and returned `true` when there was none ("nothing we can
//! verify"). Over HTTP/2 there never is one: the authority travels in
//! `:authority`, which hyper puts on the URI, and the HTTPS listener offers
//! `h2` through ALPN. So on the one transport every modern browser prefers,
//! the CSRF guard was a no-op.
//!
//! The open-admin bypass read the authority properly (`request_authority`
//! falls back to the URI), so on a station with no admin password and TLS on,
//! a page on `evil.example` could auto-submit a form to
//! `https://<station>/admin/accounts/users` with `Origin: https://evil.example`
//! and get a synthesised admin who creates an account.
//!
//! # What this gate holds
//!
//! Every request here is a real HTTP/2 request over the real TLS listener,
//! built with the `h2` crate so that *no* `Host` header is sent — a hand-built
//! `oneshot` request with a `Host` would never have shown it.
//!
//! * a cross-origin POST is refused (403) and nothing is written;
//! * the counterparts: a same-origin POST still works, a client that sends
//!   neither `Origin` nor `Referer` (curl, a script) still works, a bearer API
//!   call with a foreign `Origin` is not stopped by this guard, and HTTP/1.1
//!   behaves as it did.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{Request, StatusCode};
use birdnet_web::api_token::ApiToken;
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use birdnet_web::tls::{self, TlsMode, TlsSettings};
use rustls_pki_types::pem::PemObject as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
/// Same length as `TOKEN`, different value.
const WRONG: &str = "fedcba9876543210fedcba9876543210";
const NEW_USER: &str = "username=mallory&password=correct-horse-battery&label=";

struct Station {
    addr: SocketAddr,
    db: std::path::PathBuf,
    ca_dir: tempfile::TempDir,
    _db_dir: tempfile::TempDir,
    stop: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

/// An open station (no admin password) with the write API enabled, behind the
/// real TLS listener.
async fn station() -> Station {
    let db_dir = tempfile::tempdir().expect("db dir");
    let db = db_dir.path().join("birds.db");
    let state = AppState::new(db.clone())
        .expect("state")
        .with_api_token(ApiToken::new(TOKEN).expect("long enough"));
    let app = build_router(state);

    let ca_dir = tempfile::tempdir().expect("state dir");
    let settings = TlsSettings {
        mode: TlsMode::SelfSigned,
        state_dir: ca_dir.path().to_path_buf(),
        hostnames: vec!["localhost".to_string()],
        ..TlsSettings::default()
    };
    let (config, _resolver) = tls::server_config(&settings)
        .expect("server config")
        .expect("self-signed config");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (stop, stop_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        tls::serve(listener, config, app, async {
            let _ = stop_rx.await;
        })
        .await
        .expect("serve");
    });
    Station {
        addr,
        db,
        ca_dir,
        _db_dir: db_dir,
        stop,
        handle,
    }
}

impl Station {
    fn client(&self, alpn: &[&[u8]]) -> tokio_rustls::TlsConnector {
        let pem = std::fs::read(tls::ca_certificate_path(self.ca_dir.path())).expect("CA");
        let mut roots = rustls::RootCertStore::empty();
        for cert in rustls_pki_types::CertificateDer::pem_slice_iter(&pem) {
            roots.add(cert.expect("parse CA")).expect("trust CA");
        }
        let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
        config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
        tokio_rustls::TlsConnector::from(Arc::new(config))
    }

    async fn connect(&self, alpn: &[&[u8]]) -> tokio_rustls::client::TlsStream<TcpStream> {
        let tcp = TcpStream::connect(self.addr).await.expect("connect");
        let name = rustls_pki_types::ServerName::try_from("localhost").expect("name");
        self.client(alpn).connect(name, tcp).await.expect("tls")
    }

    /// One HTTP/2 POST. The authority is in the URI (`:authority`) and **no**
    /// `Host` header is sent, as a browser does.
    async fn h2_post(&self, path: &str, headers: &[(&str, &str)], body: &str) -> StatusCode {
        let authority = format!("localhost:{}", self.addr.port());
        self.h2_post_to(&authority, path, headers, body).await
    }

    /// The same, naming `authority` in `:authority` (the TLS connection still
    /// goes to this station, as a DNS-rebound name would).
    async fn h2_post_to(
        &self,
        authority: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> StatusCode {
        let tls = self.connect(&[b"h2"]).await;
        assert_eq!(
            tls.get_ref().1.alpn_protocol(),
            Some(&b"h2"[..]),
            "the listener did not negotiate h2, so this is not an HTTP/2 test"
        );
        let (send, conn) = h2::client::handshake(tls).await.expect("h2 handshake");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let mut send = send.ready().await.expect("ready");
        let mut req = Request::builder()
            .method("POST")
            .uri(format!("https://{authority}{path}"))
            .header("content-type", "application/x-www-form-urlencoded");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let (response, mut stream) = send
            .send_request(req.body(()).expect("request"), false)
            .expect("send");
        stream
            .send_data(axum::body::Bytes::copy_from_slice(body.as_bytes()), true)
            .expect("body");
        response.await.expect("response").status()
    }

    /// One HTTP/1.1 POST over the same TLS listener, with a `Host` header.
    async fn h1_post(&self, path: &str, headers: &[(&str, &str)], body: &str) -> String {
        let mut tls = self.connect(&[b"http/1.1"]).await;
        let mut req = format!(
            "POST {path} HTTP/1.1\r\nHost: localhost:{}\r\n\
             Content-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: {}\r\nConnection: close\r\n",
            self.addr.port(),
            body.len()
        );
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        req.push_str(body);
        tls.write_all(req.as_bytes()).await.expect("write");
        let mut buf = Vec::new();
        let _ = tls.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// Whether the account the forged form would create exists.
    fn mallory_exists(&self) -> bool {
        let conn = rusqlite::Connection::open(&self.db).expect("open db");
        conn.query_row(
            "SELECT COUNT(*) FROM users WHERE username = 'mallory'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .expect("count")
            > 0
    }

    async fn finish(self) {
        let _ = self.stop.send(());
        self.handle.await.expect("server task");
    }
}

const ACCOUNTS: &str = "/admin/accounts/users";

#[tokio::test]
async fn a_cross_origin_post_over_http2_is_refused_and_writes_nothing() {
    let st = station().await;
    let status = st
        .h2_post(ACCOUNTS, &[("origin", "https://evil.example")], NEW_USER)
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an HTTP/2 form post from another website reached the admin handler"
    );
    assert!(
        !st.mallory_exists(),
        "the forged request created an account even though it answered {status}"
    );
    st.finish().await;
}

#[tokio::test]
async fn a_cross_site_referer_over_http2_is_refused_too() {
    let st = station().await;
    let status = st
        .h2_post(
            ACCOUNTS,
            &[("referer", "https://evil.example/page")],
            NEW_USER,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!st.mallory_exists());
    st.finish().await;
}

#[tokio::test]
async fn a_forged_login_over_http2_mints_no_session() {
    // `POST /login` on an open station hands out an admin cookie for any
    // credentials; it is behind the same guard.
    let st = station().await;
    let status = st
        .h2_post(
            "/login",
            &[("origin", "https://evil.example")],
            "username=x&password=y",
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    st.finish().await;
}

#[tokio::test]
async fn a_rebound_name_over_http2_cannot_mint_an_open_station_cookie() {
    // No Origin at all, so the CSRF guard has nothing to object to; what stops
    // this is the open-admin name check, which `POST /login` used to make
    // against the `Host` header only — absent over HTTP/2, so always passing.
    let st = station().await;
    let status = st
        .h2_post_to(
            "rebind.evil.example",
            "/login",
            &[],
            "username=x&password=y",
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an open station minted an admin session for a rebindable name over HTTP/2"
    );
    // Counterpart: the same request by an address the owner uses is let in.
    let ok = st
        .h2_post_to("192.168.1.20:8503", "/login", &[], "username=x&password=y")
        .await;
    assert!(ok.is_redirection(), "got {ok}");
    st.finish().await;
}

#[tokio::test]
async fn the_stations_own_page_over_http2_still_works() {
    let st = station().await;
    let own = format!("https://localhost:{}", st.addr.port());
    let status = st.h2_post(ACCOUNTS, &[("origin", &own)], NEW_USER).await;
    assert!(
        status.is_success(),
        "a same-origin HTTP/2 form post was refused: {status}"
    );
    assert!(st.mallory_exists(), "the same-origin post wrote nothing");
    st.finish().await;
}

#[tokio::test]
async fn a_client_that_sends_no_origin_over_http2_still_works() {
    // curl --http2, a script, Home Assistant: no Origin, no Referer. A remote
    // site cannot make a browser send such a request, so nothing to refuse.
    let st = station().await;
    let status = st.h2_post(ACCOUNTS, &[], NEW_USER).await;
    assert!(status.is_success(), "got {status}");
    assert!(st.mallory_exists());
    st.finish().await;
}

#[tokio::test]
async fn a_bearer_call_over_http2_is_not_stopped_by_the_origin_guard() {
    // The exemption is for the write API with a bearer credential. The wrong
    // credential must be answered by the bearer gate (401), not by CSRF (403).
    let st = station().await;
    let bearer = format!("Bearer {WRONG}");
    let status = st
        .h2_post(
            "/api/v2/dynamic-thresholds/reset",
            &[
                ("origin", "https://evil.example"),
                ("authorization", &bearer),
            ],
            "{}",
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    st.finish().await;
}

#[tokio::test]
async fn http_1_1_is_unchanged() {
    let st = station().await;
    let port = st.addr.port();

    let cross = st
        .h1_post(ACCOUNTS, &[("Origin", "https://evil.example")], NEW_USER)
        .await;
    assert!(cross.starts_with("HTTP/1.1 403"), "{cross}");
    assert!(!st.mallory_exists());

    let own = format!("https://localhost:{port}");
    let same = st.h1_post(ACCOUNTS, &[("Origin", &own)], NEW_USER).await;
    assert!(same.starts_with("HTTP/1.1 200"), "{same}");
    assert!(st.mallory_exists());
    st.finish().await;
}
