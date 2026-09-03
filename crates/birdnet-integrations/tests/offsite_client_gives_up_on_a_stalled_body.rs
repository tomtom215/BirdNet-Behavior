//! A connection that stalls after its headers must not hang the station.
//!
//! # What was wrong
//!
//! `offsite::s3::client()` set `connect_timeout(30 s)` and nothing else, under
//! a comment that said so deliberately:
//!
//! ```text
//! crates/birdnet-integrations/src/offsite/s3.rs:42-48
//!   /// No overall request timeout: a station on a rural uplink can
//!   /// legitimately spend an hour on one upload, and a deadline that killed
//!   /// it would turn a slow link into a station with no offsite backups at
//!   /// all. A wedged connection is caught by this instead.
//! ```
//!
//! The first half of that reasoning is right and is preserved. The last
//! sentence is false. `connect_timeout` bounds the TCP connect and TLS
//! handshake only; a connection that *establishes* and then stalls part-way
//! through the exchange — the ordinary 4G failure, and the ordinary behaviour
//! of a middlebox that has lost the far side — is not caught by it at all.
//!
//! That mattered far beyond one missed upload. `run_offsite` is awaited
//! **inline** in `src/maintenance.rs`'s single sequential loop, so one wedged
//! socket stopped the daily `PRAGMA integrity_check`, `VACUUM`, the local
//! backup, clip retention, the per-species cap and log retention — for the life
//! of the process, with the `warn!` sitting on an error path that was never
//! reached. The station kept recording and quietly stopped maintaining itself.
//!
//! # What this gate holds
//!
//! The client the offsite path actually builds, pointed at a server that
//! completes the handshake, sends headers promising a body, sends one byte, and
//! then holds the socket open for ever. It must return — with an error — inside
//! a budget.
//!
//! Two halves, because neither alone is honest:
//!
//! 1. **The mechanism fires.** The stall test injects a two-second read
//!    timeout so it can watch the detector work without waiting out the
//!    production value. Against a client built the previous way — connect
//!    timeout only — it fails.
//! 2. **Production uses it.** `client()` is a one-line delegation to
//!    `client_with_timeouts(CONNECT_TIMEOUT, READ_TIMEOUT)`, and a test asserts
//!    `READ_TIMEOUT` is a bounded, plausible value. Without (1) that assertion
//!    would be a constant checking itself; without (2), (1) would prove only
//!    that reqwest has the feature.
//!
//! Observed failing against `connect_timeout`-only, with the real `client()`
//! and a 20-second budget: "the offsite client returned headers but then waited
//! past 20s for a body that never came" — exactly as it did in the field.

use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// How long the client gets before this test calls it wedged.
///
/// Comfortably above the read timeout the client should now carry, and
/// comfortably below "for ever", which is what the previous client did.
const BUDGET: Duration = Duration::from_secs(20);

/// Accept, promise a hundred bytes, send one, and then hold the socket.
///
/// This is what a 4G bearer that has lost its far side looks like from here:
/// the connection is established and the peer is not RST-ing, so nothing at the
/// transport layer will ever tell us it is dead.
async fn stalling_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0_u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nX")
                    .await;
                let _ = sock.flush().await;
                // Hold it open, sending nothing further, until the test ends.
                loop {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                }
            });
        }
    });
    (addr, handle)
}

/// The production read timeout must be bounded and plausible.
///
/// On its own this is a constant checking itself. Paired with the stall test
/// below — which drives the same constructor and watches the detector fire —
/// it is what connects that demonstration to the client the maintenance loop
/// actually builds.
#[test]
fn the_shipped_client_carries_a_bounded_read_timeout() {
    use birdnet_integrations::offsite::s3::{CONNECT_TIMEOUT, READ_TIMEOUT};
    assert!(
        READ_TIMEOUT >= Duration::from_secs(30),
        "shorter than the gap between the last byte of a PUT body and a slow \
         store's response, which would kill honest uploads"
    );
    assert!(
        READ_TIMEOUT <= Duration::from_secs(600),
        "a station must not sit on a dead socket for longer than this; the \
         maintenance loop is behind it"
    );
    assert!(
        CONNECT_TIMEOUT < READ_TIMEOUT,
        "a handshake that never completes should be given up on sooner than a \
         transfer that has merely gone quiet"
    );
}

#[tokio::test]
async fn the_offsite_client_gives_up_on_a_connection_that_stops_sending() {
    let (addr, server) = stalling_server().await;
    // Two seconds rather than the shipped two minutes, so this test takes
    // seconds. `client()` is a one-line delegation to this same constructor;
    // `the_shipped_client_carries_a_bounded_read_timeout` covers the value.
    let client = birdnet_integrations::offsite::s3::client_with_timeouts(
        Duration::from_secs(30),
        Duration::from_secs(2),
    )
    .expect("build the offsite client");

    let started = Instant::now();
    let outcome =
        tokio::time::timeout(BUDGET, client.get(format!("http://{addr}/probe")).send()).await;
    let elapsed = started.elapsed();
    server.abort();

    let inner = outcome.unwrap_or_else(|_| {
        panic!(
            "the offsite client was still waiting after {BUDGET:?} on a socket \
             that connected and then stopped sending. connect_timeout does not \
             bound this, and run_offsite is awaited inline in the maintenance \
             loop — so this is how a station stops backing up, checkpointing \
             and vacuuming for the rest of its life."
        )
    });

    // Reading the body is where the stall bites; `send()` may return the
    // headers. Either the send or the body read must give up inside the budget.
    match inner {
        Ok(resp) => {
            let body = tokio::time::timeout(BUDGET.saturating_sub(elapsed), resp.bytes()).await;
            let body = body.unwrap_or_else(|_| {
                panic!(
                    "the offsite client returned headers but then waited past \
                     {BUDGET:?} for a body that never came"
                )
            });
            assert!(
                body.is_err(),
                "a body that was never sent must be an error, not a short read \
                 the caller would treat as a completed upload"
            );
        }
        Err(e) => {
            assert!(
                e.is_timeout() || e.is_request() || e.is_body(),
                "expected a timeout-shaped failure, got {e}"
            );
        }
    }
}

/// The discrimination: a server that answers promptly must still work, so the
/// fix is a stall detector and not simply a shorter deadline.
///
/// A read timeout resets on every successful read, so a slow-but-progressing
/// rural uplink — the case the original comment was written to protect, and
/// which is still the right thing to protect — is unaffected. A total
/// `timeout()` would have broken it, which is why the fix is `read_timeout`.
#[tokio::test]
async fn a_server_that_answers_is_still_served() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0_u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi")
                    .await;
            });
        }
    });

    let client = birdnet_integrations::offsite::s3::client().expect("client");
    let resp = tokio::time::timeout(BUDGET, client.get(format!("http://{addr}/ok")).send())
        .await
        .expect("must not time out")
        .expect("must succeed");
    assert!(resp.status().is_success());
    let body = resp.bytes().await.expect("body");
    assert_eq!(&body[..], b"hi");
}

/// A transfer that is slow but *making progress* must not be killed.
///
/// This is the case the original comment protects — "a station on a rural
/// uplink can legitimately spend an hour on one upload" — and it is why the fix
/// bounds inactivity rather than total duration. The server here dribbles a
/// byte at a time with gaps shorter than the read timeout, taking far longer in
/// total than any single gap.
#[tokio::test]
async fn a_slow_but_progressing_transfer_is_not_killed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0_u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n")
                    .await;
                for _ in 0..8 {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    if sock.write_all(b"z").await.is_err() {
                        return;
                    }
                    let _ = sock.flush().await;
                }
            });
        }
    });

    let client = birdnet_integrations::offsite::s3::client().expect("client");
    let resp = tokio::time::timeout(BUDGET, client.get(format!("http://{addr}/slow")).send())
        .await
        .expect("must not time out")
        .expect("must succeed");
    let body = tokio::time::timeout(BUDGET, resp.bytes())
        .await
        .expect("must not time out")
        .expect("a transfer that keeps making progress must be allowed to finish");
    assert_eq!(&body[..], b"zzzzzzzz");
}
