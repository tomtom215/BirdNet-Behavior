//! The HTTPS listener, over a real socket.
//!
//! `tls.rs`'s unit tests cover certificate material — what is minted, what is
//! reused, what is refused. They say nothing about whether the accept loop
//! actually serves anything, and that loop is hand-written precisely because
//! `axum::serve` has no seam for a TLS acceptor. Two things it has to do that
//! `axum::serve` does for free are asserted here rather than reasoned about:
//! the handshake completes against the certificate the station generated for
//! itself, and `ConnectInfo` reaches the handlers (without it the per-IP rate
//! limiter degrades to one global bucket, silently).

use rustls_pki_types::pem::PemObject as _;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::ConnectInfo;
use axum::routing::get;
use birdnet_web::tls::{self, TlsMode, TlsSettings};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

/// A router that reports the peer address the transport gave it.
fn echo_peer_router() -> Router {
    Router::new().route(
        "/whoami",
        get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.to_string() }),
    )
}

/// Bind an ephemeral port and start the TLS listener over `app`.
///
/// Returns the bound address, a handle that stops the server when sent on, and
/// the directory holding the generated CA (kept alive by the caller).
async fn start_tls(
    app: Router,
) -> (
    SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().expect("state dir");
    let settings = TlsSettings {
        mode: TlsMode::SelfSigned,
        state_dir: dir.path().to_path_buf(),
        hostnames: vec!["localhost".to_string()],
        ..TlsSettings::default()
    };
    let (config, _resolver) = tls::server_config(&settings)
        .expect("build server config")
        .expect("self-signed mode produces a config");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();

    let handle = tokio::spawn(async move {
        tls::serve(listener, config, app, async {
            let _ = stop_rx.await;
        })
        .await
        .expect("serve");
    });

    (addr, stop_tx, dir, handle)
}

/// A rustls client that trusts only the CA in `state_dir`.
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

/// Issue one HTTP/1.1 GET over TLS and return the whole response as text.
///
/// Hand-rolled rather than reaching for a client crate: the request is three
/// lines, and `Connection: close` means the response is exactly "read to EOF",
/// so there is no parsing to get wrong.
async fn https_get(addr: SocketAddr, state_dir: &std::path::Path, path: &str) -> String {
    let connector = tokio_rustls::TlsConnector::from(client_config(state_dir));
    let tcp = TcpStream::connect(addr).await.expect("connect");
    let name = rustls_pki_types::ServerName::try_from("localhost").expect("server name");
    let mut tls = connector.connect(name, tcp).await.expect("handshake");

    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    tls.write_all(req.as_bytes()).await.expect("write request");
    let mut buf = Vec::new();
    // A clean close arrives as EOF; some stacks send close_notify and then RST,
    // which surfaces as an error after the body is already in `buf`.
    let _ = tls.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).into_owned()
}

/// Issue one plain-HTTP GET and return the whole response as text.
async fn http_get(addr: SocketAddr, path: &str, host: &str) -> String {
    let mut tcp = TcpStream::connect(addr).await.expect("connect");
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    tcp.write_all(req.as_bytes()).await.expect("write request");
    let mut buf = Vec::new();
    let _ = tcp.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn serves_https_and_hands_handlers_the_client_address() {
    let (addr, stop, dir, handle) = start_tls(echo_peer_router()).await;

    let response = https_get(addr, dir.path(), "/whoami").await;
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "expected a 200 over TLS, got:\n{response}"
    );

    // The body is the peer address the handler saw. It must be a real
    // 127.0.0.1 address with the client's own ephemeral port — not a default,
    // and not the server's own address.
    let body = response
        .rsplit("\r\n\r\n")
        .next()
        .unwrap_or_default()
        .trim();
    let peer: SocketAddr = body
        .parse()
        .unwrap_or_else(|e| panic!("handler returned {body:?}, which is not an address: {e}"));
    assert_eq!(peer.ip().to_string(), "127.0.0.1", "peer was {peer}");
    assert_ne!(
        peer.port(),
        addr.port(),
        "the handler was given the listener's own port, so ConnectInfo is not \
         carrying the client — the per-IP rate limiter would collapse to one bucket"
    );

    let _ = stop.send(());
    handle.await.expect("server task");
}

#[tokio::test]
async fn a_plain_http_probe_does_not_take_the_listener_down() {
    let (addr, stop, dir, handle) = start_tls(echo_peer_router()).await;

    // A monitoring check, a port scanner, or an operator who typed http://.
    // The handshake fails; the listener must not.
    let garbage = http_get(addr, "/whoami", "localhost").await;
    assert!(
        !garbage.starts_with("HTTP/1.1 200"),
        "plain HTTP should not be served on the TLS port, got:\n{garbage}"
    );

    let response = https_get(addr, dir.path(), "/whoami").await;
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "a failed handshake killed the listener; the next real client got:\n{response}"
    );

    let _ = stop.send(());
    handle.await.expect("server task");
}

#[tokio::test]
async fn shutdown_stops_accepting_and_returns() {
    let (addr, stop, dir, handle) = start_tls(echo_peer_router()).await;
    assert!(
        https_get(addr, dir.path(), "/whoami")
            .await
            .contains("200 OK")
    );

    let _ = stop.send(());
    // If `serve` never returned, the harness would hang here rather than fail,
    // so bound it: a drain that takes a second on an idle listener is a bug.
    tokio::time::timeout(std::time::Duration::from_secs(5), handle)
        .await
        .expect("serve did not return within 5s of the shutdown signal")
        .expect("server task");

    let refused = TcpStream::connect(addr).await;
    // The port may linger in TIME_WAIT, so a successful connect is not proof of
    // a leak — but a completed TLS request would be.
    if refused.is_ok() {
        let after = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            https_get(addr, dir.path(), "/whoami"),
        )
        .await
        .unwrap_or_default();
        assert!(
            !after.contains("200 OK"),
            "the listener kept serving after shutdown returned"
        );
    }
}

#[tokio::test]
async fn redirect_listener_sends_308_to_the_https_origin() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        tls::serve_redirect(listener, 8503, async {
            let _ = stop_rx.await;
        })
        .await
        .expect("redirect server");
    });

    let response = http_get(addr, "/today?filter=rare", "pi.local:8502").await;
    assert!(
        response.starts_with("HTTP/1.1 308"),
        "expected 308 (which preserves the method, so a POSTed settings form is \
         not silently downgraded to a GET), got:\n{response}"
    );
    assert!(
        response
            .to_ascii_lowercase()
            .contains("location: https://pi.local:8503/today?filter=rare"),
        "the redirect must keep the path, the query and the requested host:\n{response}"
    );

    let _ = stop_tx.send(());
    handle.await.expect("redirect task");
}

#[tokio::test]
async fn redirect_refuses_a_host_header_it_would_be_unsafe_to_reflect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        tls::serve_redirect(listener, 8503, async {
            let _ = stop_rx.await;
        })
        .await
        .expect("redirect server");
    });

    // A `Host` carrying a path turns the Location header into an open redirect
    // to somebody else's site.
    let response = http_get(addr, "/", "evil.example/@attacker.test").await;
    assert!(
        response.starts_with("HTTP/1.1 400"),
        "a Host header that is not a hostname must be refused, not reflected \
         into Location:\n{response}"
    );

    let _ = stop_tx.send(());
    handle.await.expect("redirect task");
}
