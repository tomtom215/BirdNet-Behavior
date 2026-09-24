//! The live sockets answer this station's own pages, and only so many at once.
//!
//! # The defect
//!
//! `/api/v2/ws/detections` and `/api/v2/ws/spectrogram` upgraded any request.
//! A WebSocket is not subject to the same-origin policy or to CORS, and the
//! CSRF guard only looks at state-changing methods — a handshake is a `GET`.
//! So on an open station (the default) any web page someone in the house
//! visited could open both sockets and read every detection as it happened and
//! the live spectrogram of the garden microphone, which `GET /api/v2/detections`
//! does not allow across origins. And nothing bounded how many sockets stayed
//! open: every spectrogram frame is copied once per client.
//!
//! # What is guarded
//!
//! A handshake whose `Origin` is another site is refused with `403`; the
//! station's own origin and a client that sends none (a script, `websocat`)
//! still connect. Past the cap, a handshake is refused with `503`, and closing
//! one socket makes room for the next.
//!
//! `oneshot` cannot upgrade a connection, so these tests serve the router on a
//! real socket and speak the handshake by hand.

use std::net::SocketAddr;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

const SOCKETS: [&str; 2] = ["/api/v2/ws/detections", "/api/v2/ws/spectrogram"];

fn open_station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn serve(state: AppState) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = build_router(state).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    addr
}

/// Open a connection, send a WebSocket handshake, and return the status code
/// with the still-open stream (an accepted socket stays accepted while it is
/// held).
async fn handshake(addr: SocketAddr, path: &str, origin: Option<&str>) -> (u16, TcpStream) {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let origin = origin.map_or_else(String::new, |o| format!("Origin: {o}\r\n"));
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{origin}\r\n"
    );
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = stream.read(&mut byte).await.expect("read");
        assert!(n > 0, "the server closed before answering: {head:?}");
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head);
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("no status line in {head:?}"));
    (status, stream)
}

#[tokio::test]
async fn another_website_cannot_open_the_live_sockets() {
    let addr = serve(open_station()).await;
    for path in SOCKETS {
        let (status, _) = handshake(addr, path, Some("http://evil.example")).await;
        assert_eq!(
            status, 403,
            "{path}: a cross-site page opened the live socket"
        );

        // Counterparts: the station's own page, and a client with no Origin.
        let (status, _) = handshake(addr, path, Some(&format!("http://{addr}"))).await;
        assert_eq!(status, 101, "{path}: the station's own page was refused");
        let (status, _) = handshake(addr, path, None).await;
        assert_eq!(status, 101, "{path}: a non-browser client was refused");
    }
}

#[tokio::test]
async fn the_live_sockets_are_capped_and_a_closed_one_makes_room() {
    let addr = serve(open_station().with_live_socket_limit(2)).await;
    for path in SOCKETS {
        let (first, held) = handshake(addr, path, None).await;
        let (second, _also_held) = handshake(addr, path, None).await;
        assert_eq!(
            (first, second),
            (101, 101),
            "{path}: precondition — two fit"
        );
        let (third, _) = handshake(addr, path, None).await;
        assert_eq!(
            third, 503,
            "{path}: a third socket was admitted past the cap"
        );

        // Closing one frees its slot once the server sees the close.
        drop(held);
        let mut admitted = None;
        for _ in 0..40 {
            let (status, stream) = handshake(addr, path, None).await;
            if status == 101 {
                admitted = Some(stream);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(
            admitted.is_some(),
            "{path}: a closed socket never gave its slot back"
        );
    }
}
