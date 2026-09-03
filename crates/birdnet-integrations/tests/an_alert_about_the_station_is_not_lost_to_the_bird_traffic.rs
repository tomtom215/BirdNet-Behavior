//! A notification that reached nobody must not be reported as sent, and an
//! alert about the station must not lose a race with a blackbird.
//!
//! # What was wrong
//!
//! Two defects, and they compounded.
//!
//! 1. `send_notification_with_image` ended with
//!
//!    ```text
//!    return match (delivered, first_error) {
//!        (0, Some(e)) => Err(e),
//!        _ => Ok(()),
//!    };
//!    ```
//!
//!    `(0, None)` is the *fully skipped* case: every destination refused by
//!    the rate limiter or the circuit breaker, no send attempted, no error to
//!    report. It returned `Ok(())`. Nothing had left the station.
//!
//! 2. Every destination was admitted through the same
//!    [`birdnet_integrations::dispatch::Gate::admit`], whose token bucket is
//!    sized for detections. A dawn chorus drains it.
//!
//! Together: the deadman crosses its threshold at 06:00 during the chorus,
//! `notify()` gets `Ok(())` from a send that never happened, the loop sets
//! `alerted = true`, and the episode is latched. `transition()` then returns
//! `Transition::None` for as long as the silence lasts, so the alert is never
//! attempted again — for the life of the process. The same held for every
//! station-health condition and every stream fault.
//!
//! The skip itself was logged at `debug`, and the default filter puts
//! `birdnet_integrations` at `info`, so there was no evidence either.
//!
//! # What this gate holds
//!
//! Against a real local HTTP destination, driven through the real
//! `Client::with_native_routes`:
//!
//! 1. a send that every destination skipped is an error, not `Ok(())`;
//! 2. an operational alert goes out when the detection bucket is empty;
//! 3. an operational alert is *still* suppressed by an open circuit — the
//!    discrimination, since a priority path that ignored both guards would
//!    satisfy (2) and hammer a retired webhook;
//! 4. an ordinary notification to a working destination still goes, which is
//!    what stops (1) from being satisfied by a client that always errors.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use birdnet_integrations::apprise::{AppriseError, Client, NotifyConfig, NotifyType};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// A destination that counts what it is asked to deliver.
struct Destination {
    /// `json://host:port/` — Apprise's generic JSON POST, over plain HTTP.
    url: String,
    /// Requests that actually arrived.
    seen: Arc<AtomicUsize>,
}

/// Stand up a local destination answering `status` to every POST.
///
/// `404` is deliberate for the "dead endpoint" case: `SendError::is_retryable`
/// treats a non-429 4xx as final, so each send fails on its first attempt with
/// no backoff and the test runs in milliseconds rather than minutes.
async fn destination(status: u16) -> Destination {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                let mut buf = [0_u8; 8192];
                // One read is enough: the bodies here are a few hundred bytes
                // and arrive in the same segment as the headers.
                let _ = sock.read(&mut buf).await;
                counter.fetch_add(1, Ordering::SeqCst);
                let reason = if status == 200 { "OK" } else { "Not Found" };
                let _ = sock
                    .write_all(
                        format!(
                            "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\n\
                             Connection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Destination {
        url: format!("json://{addr}/"),
        seen,
    }
}

/// The client the station builds for a native-URL-only configuration, pointed
/// at `dest` and rate-limited to `rate_per_minute`.
fn client(dest: &Destination, rate_per_minute: u32) -> Client {
    let parsed = birdnet_integrations::dispatch::routes(&dest.url);
    assert_eq!(
        parsed.native.len(),
        1,
        "the fixture URL must parse to exactly one native route: {}",
        dest.url
    );
    Client::new_cli_only(
        std::path::PathBuf::new(),
        NotifyConfig {
            min_confidence: 0.0,
            species_watchlist: Vec::new(),
            species_notify_exclude: Vec::new(),
            cooldown: std::time::Duration::ZERO,
            per_species_cooldown: std::collections::HashMap::new(),
            rate_per_minute,
        },
    )
    .expect("build the client")
    .with_native_routes(parsed.native, false)
}

#[tokio::test]
async fn a_notification_no_destination_accepted_is_not_reported_as_sent() {
    let dest = destination(200).await;
    // One send a minute: the second is refused by the bucket, with no error
    // from any destination — exactly the `(0, None)` that returned `Ok(())`.
    let mut c = client(&dest, 1);

    c.send_notification("Bird", "one", NotifyType::Info)
        .await
        .expect("the first send must succeed");
    assert_eq!(dest.seen.load(Ordering::SeqCst), 1);

    let outcome = c.send_notification("Bird", "two", NotifyType::Info).await;
    assert_eq!(
        dest.seen.load(Ordering::SeqCst),
        1,
        "the second send must not have reached the destination, or this test \
         is not exercising the skip path"
    );
    match outcome {
        Err(AppriseError::AllDestinationsSkipped {
            circuit_open,
            rate_limited,
        }) => {
            assert_eq!(circuit_open, 0);
            assert_eq!(rate_limited, 1);
        }
        Err(other) => panic!("expected the skip error, got {other}"),
        Ok(()) => panic!(
            "a notification that reached nobody was reported as sent. This is \
             what latched an alert episode on an alert that never left the box."
        ),
    }
}

#[tokio::test]
async fn an_alert_about_the_station_outranks_the_bird_traffic() {
    let dest = destination(200).await;
    let mut c = client(&dest, 1);

    // Spend the minute's budget on a detection, as a dawn chorus does.
    c.send_notification("Bird", "one", NotifyType::Info)
        .await
        .expect("the first send must succeed");
    assert!(
        c.send_notification("Bird", "two", NotifyType::Info)
            .await
            .is_err(),
        "the bucket must be empty, or the alert below is not being tested \
         against an exhausted limit"
    );

    c.send_operational_alert(
        "Station has gone quiet",
        "No bird detections for 25 hours.",
        NotifyType::Warning,
    )
    .await
    .expect("an alert about the station must not be dropped by the detection limit");

    assert_eq!(
        dest.seen.load(Ordering::SeqCst),
        2,
        "the operational alert did not reach the destination"
    );
}

#[tokio::test]
async fn an_alert_about_the_station_is_still_suppressed_by_a_dead_destination() {
    // The discrimination. A priority path that simply ignored both guards
    // would pass the gate above and spend an attempt on a retired webhook
    // every time a condition was re-evaluated — and it is the retries, not
    // the sends, that get an address banned.
    let dest = destination(404).await;
    let mut c = client(&dest, 0); // rate limit disabled: only the breaker

    for i in 0..3 {
        assert!(
            c.send_notification("Bird", "x", NotifyType::Info)
                .await
                .is_err(),
            "send {i} to a 404 destination must fail"
        );
    }
    assert_eq!(
        dest.seen.load(Ordering::SeqCst),
        3,
        "three failures are what open the circuit"
    );

    match c
        .send_operational_alert("Station has gone quiet", "b", NotifyType::Warning)
        .await
    {
        Err(AppriseError::AllDestinationsSkipped {
            circuit_open,
            rate_limited,
        }) => {
            assert_eq!(circuit_open, 1);
            assert_eq!(rate_limited, 0);
        }
        Err(other) => panic!("expected the skip error, got {other}"),
        Ok(()) => panic!("an alert that was suppressed was reported as sent"),
    }
    assert_eq!(
        dest.seen.load(Ordering::SeqCst),
        3,
        "the open circuit must not have been forced"
    );
}

#[tokio::test]
async fn an_ordinary_notification_to_a_working_destination_still_goes() {
    // The counterpart to the first gate: a client that returned an error
    // whenever `delivered == 0` for any reason would satisfy it and silence
    // every station on earth.
    let dest = destination(200).await;
    let mut c = client(&dest, 60);
    for i in 0..5 {
        c.send_notification("Bird", "x", NotifyType::Info)
            .await
            .unwrap_or_else(|e| panic!("send {i} failed: {e}"));
    }
    assert_eq!(dest.seen.load(Ordering::SeqCst), 5);
}

/// The client's lifetime skip counters must move, because they are what
/// `birdnet_notifications_dropped_total` is derived from.
#[tokio::test]
async fn the_skip_counters_record_what_was_dropped() {
    let dest = destination(200).await;
    let mut c = client(&dest, 1);
    assert_eq!(c.skip_counts(), (0, 0));
    c.send_notification("Bird", "one", NotifyType::Info)
        .await
        .expect("first send");
    for _ in 0..4 {
        assert!(
            c.send_notification("Bird", "more", NotifyType::Info)
                .await
                .is_err()
        );
    }
    assert_eq!(
        c.skip_counts(),
        (0, 4),
        "four rate-limited sends must be counted as such"
    );
}
