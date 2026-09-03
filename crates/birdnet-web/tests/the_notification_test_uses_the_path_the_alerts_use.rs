//! "Test notifications" must exercise the path an alert about the station
//! takes, and must be usable by the stations that have one.
//!
//! # What was wrong (`OB-9`)
//!
//! Two defects in one button.
//!
//! 1. **It tested a path nothing else uses.** The handler built a fresh
//!    `reqwest::Client` and `POST`ed `{apprise_url}/notify` itself:
//!
//!    ```text
//!    let client = reqwest::Client::builder()...build()?;
//!    let url = format!("{}/notify", apprise_url.trim_end_matches('/'));
//!    client.post(&url).json(&body).send().await
//!    ```
//!
//!    That is not how an alert is delivered. `announce::flush` locks the
//!    shared `apprise::Client` and calls `send_operational_alert`, which walks
//!    the native routes delivered in-process, falls back to the `apprise` CLI
//!    for a config file, and passes every destination through a circuit
//!    breaker and a rate limiter first. None of that was under test, so a
//!    green "test notification sent" said nothing about whether the deadman
//!    alert would leave the box — which is exactly what `OB-5` turned out to
//!    be about.
//!
//! 2. **It was disabled for the configuration most stations have.** The button
//!    keyed off the `apprise_url` *setting* — an Apprise **API server**. A
//!    station configured only with native notification URLs (`ntfy://`,
//!    `discord://`, …) saw "Not configured" and a dead button while its alerts
//!    worked fine.
//!
//! # What this gate holds
//!
//! Against a real local HTTP destination, reached through the real
//! `Client::with_native_routes` and the real admin router:
//!
//! 1. pressing the button on a native-routes-only station delivers to that
//!    destination — the shipped handler reaches nothing, because no
//!    `apprise_url` is set;
//! 2. the button is *enabled* on such a station, and the page names what it
//!    resolved;
//! 3. a station with no notifier at all still gets a disabled button, and
//!    pressing it anyway is reported as a failure — the counterpart that stops
//!    (2) being satisfied by "always enabled", and the one gate here that must
//!    stay **green** against the shipped code;
//! 4. a destination whose circuit is open is **reported** rather than
//!    bypassed. This is the discrimination: a "fix" that read the notifier's
//!    routes and then built its own client to send them would pass (1) and
//!    (2) and fail this, because the guards live in the shared client;
//! 5. structurally, the module builds exactly one HTTP client — BirdWeather's.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_integrations::apprise::{Client, NotifyConfig, NotifyType};
use birdnet_web::notifier::Notifier;
use birdnet_web::state::AppState;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tower::ServiceExt as _;

/// A destination that counts what it is asked to deliver.
struct Destination {
    /// `json://host:port/` — Apprise's generic JSON POST, over plain HTTP.
    url: String,
    /// Requests that actually arrived.
    seen: Arc<AtomicUsize>,
}

/// Stand up a local destination answering `status` to every POST.
///
/// `404` for the dead-endpoint case: `SendError::is_retryable` treats a
/// non-429 4xx as final, so each send fails on its first attempt with no
/// backoff and the test runs in milliseconds rather than minutes.
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

/// The client a native-URL-only station builds, pointed at `dest`.
///
/// No Apprise server URL and no usable config file: `url()` is empty and
/// `needs_apprise_cli()` is false, so the *only* thing that can make this
/// station's push channel "configured" is the native route.
fn native_only_client(dest: &Destination) -> Client {
    let parsed = birdnet_integrations::dispatch::routes(&dest.url);
    assert_eq!(
        parsed.native.len(),
        1,
        "the fixture URL must parse to exactly one native route: {}",
        dest.url
    );
    Client::new_cli_only(
        PathBuf::new(),
        NotifyConfig {
            min_confidence: 0.0,
            species_watchlist: Vec::new(),
            species_notify_exclude: Vec::new(),
            cooldown: std::time::Duration::ZERO,
            per_species_cooldown: std::collections::HashMap::new(),
            // The detection rate limit off, so nothing here is measuring it.
            // `an_alert_about_the_station_is_not_lost_to_the_bird_traffic`
            // owns that discrimination.
            rate_per_minute: 0,
        },
    )
    .expect("build the client")
    .with_native_routes(parsed.native, false)
}

/// A station whose state carries `client` as its notifier.
async fn station_with(client: Client) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    let handle = Arc::new(tokio::sync::Mutex::new(client));
    let state = state.with_notifier(Notifier::attach(handle).await);
    (dir, state)
}

/// A station with no notifier at all.
fn station_without_notifier() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

/// Drive the real router — auth middleware included — and return the body.
async fn call(state: &AppState, method: &str, uri: &str) -> (StatusCode, String) {
    let app = birdnet_web::server::build_router(state.clone());
    let res = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("read body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The push channel's `<form>`, so an assertion about *its* button cannot be
/// satisfied (or broken) by the BirdWeather card next to it.
fn push_form(body: &str) -> &str {
    let start = body
        .find(r#"hx-post="/admin/notifications/test/apprise""#)
        .unwrap_or_else(|| panic!("the page has no push-test form:\n{body}"));
    let rest = &body[start..];
    let end = rest.find("</form>").expect("the form closes");
    &rest[..end]
}

/// Pressing the button delivers to the destination the station resolved.
#[tokio::test]
async fn the_button_sends_through_the_stations_own_notifier() {
    let dest = destination(200).await;
    let (_dir, state) = station_with(native_only_client(&dest)).await;

    let (status, body) = call(&state, "POST", "/admin/notifications/test/apprise").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        dest.seen.load(Ordering::SeqCst),
        1,
        "the test notification never reached the station's own destination — \
         which is the whole of OB-9: the button tested a path nothing else \
         uses. Body:\n{body}"
    );
    assert!(
        body.contains("result-banner ok"),
        "a delivered test must be reported as delivered:\n{body}"
    );
}

/// The button is enabled, and the page says what it will send to.
#[tokio::test]
async fn a_native_route_only_station_gets_a_live_button() {
    let dest = destination(200).await;
    let (_dir, state) = station_with(native_only_client(&dest)).await;

    let (status, body) = call(&state, "GET", "/admin/notifications/test").await;
    assert_eq!(status, StatusCode::OK);

    let form = push_form(&body);
    assert!(
        !form.contains("disabled>"),
        "a station with a resolved native route must not get a dead button — \
         this is the half of OB-9 that hit the configuration most stations \
         have. Form:\n{form}"
    );
    assert!(
        body.contains("1 destination(s)"),
        "the page must say what it resolved:\n{body}"
    );
    assert!(
        body.contains("json http://127.0.0.1"),
        "the page must name the destination it resolved:\n{body}"
    );
}

/// The counterpart: no notifier is still a disabled button, and pressing it
/// anyway is reported as a failure.
///
/// Without this, "enable it whenever any route resolved" could be satisfied by
/// enabling it always, and an operator on a station that can notify nobody
/// would press a button that silently does nothing.
///
/// Deliberately behavioural only. The wording the page uses is asserted by
/// `disabled_buttons_explain_why` next to the renderer; putting it here too
/// would make this counterpart fail against the shipped code for a copy
/// reason, and a counterpart that goes red both ways discriminates nothing.
#[tokio::test]
async fn a_station_that_resolved_nothing_still_says_so() {
    let (_dir, state) = station_without_notifier();

    let (status, body) = call(&state, "GET", "/admin/notifications/test").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        push_form(&body).contains("disabled>"),
        "a station with no destination must not offer a live button:\n{body}"
    );

    // And pressing it anyway — the form is the only thing stopping that — is
    // reported as a failure, not as a send.
    let (_, posted) = call(&state, "POST", "/admin/notifications/test/apprise").await;
    assert!(
        posted.contains("result-banner err"),
        "a test with nowhere to go must not be reported as sent:\n{posted}"
    );
}

/// The discrimination: the shared client's guards apply to the test too.
///
/// A "fix" that read the notifier's routes and then sent them with a client of
/// its own would pass both gates above and fail this one, because the circuit
/// breaker's state lives in the shared client. It would also be the exact
/// mistake `OB-9` describes, made again.
#[tokio::test]
async fn an_open_circuit_is_reported_rather_than_bypassed() {
    let dest = destination(404).await;
    let mut client = native_only_client(&dest);

    // Three failures are what open the circuit. Driven through the client
    // before it is shared, as a run of failing detections would.
    for i in 0..3 {
        assert!(
            client
                .send_notification("Bird", "x", NotifyType::Info)
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

    let (_dir, state) = station_with(client).await;
    let (status, body) = call(&state, "POST", "/admin/notifications/test/apprise").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        dest.seen.load(Ordering::SeqCst),
        3,
        "the open circuit must not have been forced — a test that bypasses the \
         breaker is testing something the alerts do not do:\n{body}"
    );
    assert!(
        body.contains("result-banner err"),
        "a suppressed test must not be reported as sent:\n{body}"
    );
    assert!(
        body.contains("open circuit"),
        "the operator must be told *why* nothing was sent:\n{body}"
    );
}

/// Structurally: one HTTP client in this module, and it is BirdWeather's.
///
/// The behavioural gates above are about one handler. This is about the shape
/// of the module: `send_apprise_test` built a second `reqwest::Client`, and a
/// future edit that reintroduces one for push would put the test back on a
/// path the alerts do not use without failing anything else here.
#[test]
fn the_module_builds_exactly_one_http_client() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes/admin/notification_test.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
    // Strip line comments so prose about the old code — including this
    // finding's own write-up — is not mistaken for the code.
    let code: String = source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        code.matches("reqwest::Client::builder()").count(),
        1,
        "exactly one HTTP client belongs in this module — BirdWeather's, whose \
         token is read per request. Push goes through the station's own \
         notifier, guards and all."
    );
    assert!(
        code.contains("send_operational_alert"),
        "the push test must make the call `announce::flush` makes"
    );
    assert!(
        code.contains("state.notifier()"),
        "the push test must use the handle the alert loops hold"
    );
}
