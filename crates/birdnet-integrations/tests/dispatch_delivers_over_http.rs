//! The executor is checked against a stub server, not just against the plan.
//!
//! `ntfy://` and `json://` are the two schemes whose host comes from the URL,
//! so they are the only ones that can be pointed at a socket in a test. What
//! they prove holds for the rest: [`birdnet_integrations::dispatch::send`]
//! issues the planned method, URL, headers, auth and body, and reads the
//! response the way the plan says to.

use std::sync::{Arc, Mutex};

use birdnet_integrations::dispatch::{Message, Severity, parse, send};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One request as the stub server saw it on the wire.
#[derive(Debug, Clone, Default)]
struct Seen {
    /// Request line, e.g. `POST /message HTTP/1.1`.
    line: String,
    /// Headers, names lowercased.
    headers: Vec<(String, String)>,
    /// Request body.
    body: String,
}

impl Seen {
    /// Header value by (lowercase) name.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A one-shot HTTP server that answers `status`/`body` and records what it got.
struct Stub {
    /// `host:port` to build URLs from.
    addr: String,
    /// Requests received so far.
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Stub {
    /// Bind on an ephemeral port and answer every request identically.
    async fn start(status: u16, reply: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);

        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let sink = Arc::clone(&sink);
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    // Read until the body named by Content-Length is complete.
                    loop {
                        let mut chunk = [0_u8; 1024];
                        let Ok(n) = sock.read(&mut chunk).await else {
                            return;
                        };
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some((head, body)) = text.split_once("\r\n\r\n") {
                            let want: usize = head
                                .lines()
                                .find_map(|l| {
                                    let (n, v) = l.split_once(':')?;
                                    (n.eq_ignore_ascii_case("content-length"))
                                        .then(|| v.trim().parse().ok())?
                                })
                                .unwrap_or(0);
                            if body.len() >= want {
                                let mut lines = head.lines();
                                let line = lines.next().unwrap_or_default().to_string();
                                let headers = lines
                                    .filter_map(|l| l.split_once(':'))
                                    .map(|(n, v)| {
                                        (n.trim().to_ascii_lowercase(), v.trim().to_string())
                                    })
                                    .collect();
                                sink.lock().unwrap().push(Seen {
                                    line,
                                    headers,
                                    body: body.to_string(),
                                });
                                break;
                            }
                        }
                    }
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.flush().await;
                });
            }
        });

        Self { addr, seen }
    }

    /// Everything received so far.
    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// A detection-shaped message.
fn msg() -> Message {
    Message {
        title: "Bird Detection: Tawny Owl".to_string(),
        body: "Strix aluco".to_string(),
        severity: Severity::Warning,
        image_url: None,
    }
}

/// The shared client, with the same timeout production uses.
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("client")
}

#[tokio::test]
async fn a_send_puts_the_bearer_token_on_the_wire() {
    // `ntfy://` and `json://` are the schemes whose host is configurable, so
    // this uses ntfy's bearer token to prove the auth reaches the socket.
    let stub = Stub::start(200, "{}").await;
    let target = parse(&format!("ntfy://tk_SECRET123@{}/garden", stub.addr)).unwrap();
    send(&client(), &target, &msg()).await.expect("delivered");

    let reqs = stub.requests();
    assert_eq!(reqs.len(), 1);
    assert!(
        reqs[0].line.starts_with("POST / HTTP/1.1"),
        "{}",
        reqs[0].line
    );
    assert_eq!(reqs[0].header("authorization"), Some("Bearer tk_SECRET123"));
    assert_eq!(
        reqs[0].header("content-type"),
        Some("application/json"),
        "ntfy's JSON publish form needs the JSON content type"
    );
    let body: serde_json::Value = serde_json::from_str(&reqs[0].body).expect("json body");
    assert_eq!(body["topic"], "garden");
    assert_eq!(body["title"], "Bird Detection: Tawny Owl");
}

#[tokio::test]
async fn several_topics_become_several_requests() {
    let stub = Stub::start(200, "{}").await;
    let target = parse(&format!("ntfy://{}/garden/rare/owls", stub.addr)).unwrap();
    send(&client(), &target, &msg()).await.expect("delivered");

    let topics: Vec<String> = stub
        .requests()
        .iter()
        .map(|r| {
            serde_json::from_str::<serde_json::Value>(&r.body).unwrap()["topic"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(topics, ["garden", "rare", "owls"]);
}

#[tokio::test]
async fn a_basic_auth_json_webhook_sends_the_credential_it_was_given() {
    let stub = Stub::start(204, "").await;
    let target = parse(&format!("json://ada:hunter2@{}/hook", stub.addr)).unwrap();
    send(&client(), &target, &msg()).await.expect("delivered");

    let reqs = stub.requests();
    assert!(
        reqs[0].line.starts_with("POST /hook HTTP/1.1"),
        "{}",
        reqs[0].line
    );
    // base64("ada:hunter2")
    assert_eq!(
        reqs[0].header("authorization"),
        Some("Basic YWRhOmh1bnRlcjI=")
    );
    let body: serde_json::Value = serde_json::from_str(&reqs[0].body).unwrap();
    assert_eq!(body["type"], "warning");
}

#[tokio::test]
async fn a_rejected_send_reports_the_status_and_the_reason() {
    let stub = Stub::start(403, "topic is reserved").await;
    let target = parse(&format!("ntfy://{}/garden", stub.addr)).unwrap();
    let err = send(&client(), &target, &msg())
        .await
        .expect_err("rejected");

    assert_eq!(err.kind(), "ntfy");
    assert!(err.to_string().contains("403"), "{err}");
    assert!(err.to_string().contains("topic is reserved"), "{err}");
    assert!(
        !err.is_retryable(),
        "a 403 will not become a 200 on a retry"
    );
}

#[tokio::test]
async fn a_server_error_is_retryable_and_a_client_error_is_not() {
    // Counterpart to the gate above: without this, `is_retryable` returning a
    // constant `false` would still pass, and a station would give up on the
    // first blip from a self-hosted server that was merely restarting.
    for (status, retryable) in [
        (500, true),
        (502, true),
        (429, true),
        (400, false),
        (404, false),
    ] {
        let stub = Stub::start(status, "no").await;
        let target = parse(&format!("ntfy://{}/garden", stub.addr)).unwrap();
        let err = send(&client(), &target, &msg())
            .await
            .expect_err("rejected");
        assert_eq!(err.is_retryable(), retryable, "HTTP {status}");
    }
}

#[tokio::test]
async fn a_transport_error_never_carries_the_url() {
    // `reqwest::Error` embeds the URL it failed on and prints it from
    // `Display`. For Discord and Telegram that URL *is* the credential, so a
    // connection failure logged as `{e}` would publish a working webhook to
    // the journal and into every support bundle taken afterwards.
    //
    // Port 1 is reserved and nothing listens on it, so this is a real
    // connection failure rather than a simulated one.
    let target = parse("json://127.0.0.1:1/SUPERSECRETWEBHOOKPATH").unwrap();
    let err = send(&client(), &target, &msg()).await.expect_err("refused");

    let rendered = format!("{err} {err:?}");
    assert!(
        !rendered.contains("SUPERSECRETWEBHOOKPATH"),
        "the URL reached the error: {rendered}"
    );
    assert!(err.is_retryable(), "a refused connection is worth retrying");
    // ...and it still says something useful about what went wrong.
    assert!(
        rendered.to_lowercase().contains("connect") || rendered.to_lowercase().contains("refused"),
        "unhelpfully vague: {rendered}"
    );
}

#[tokio::test]
async fn the_notification_client_delivers_through_its_native_routes() {
    // End-to-end through the type the daemon actually holds: a detection that
    // clears the confidence threshold reaches the destination with no Apprise
    // server, no config file and no subprocess.
    use birdnet_integrations::apprise::{Client, NotifyConfig};
    use birdnet_integrations::dispatch::routes;

    let stub = Stub::start(200, "{}").await;
    let parsed = routes(&format!("ntfy://{}/garden", stub.addr));
    assert_eq!(parsed.native.len(), 1, "the stub URL must parse natively");

    let mut client = Client::new_cli_only(
        std::path::PathBuf::new(),
        NotifyConfig {
            min_confidence: 0.5,
            ..NotifyConfig::default()
        },
    )
    .expect("client")
    .with_native_routes(parsed.native, false);

    assert!(client.should_notify("Strix aluco", 0.91));
    client
        .notify_detection("Strix aluco", 0.91, "2026-09-01", "03:14:00")
        .await
        .expect("delivered");

    let reqs = stub.requests();
    assert_eq!(reqs.len(), 1, "exactly one send, not a duplicate");
    let body: serde_json::Value = serde_json::from_str(&reqs[0].body).unwrap();
    assert_eq!(body["topic"], "garden");
    assert!(
        body["message"].as_str().unwrap().contains("Strix aluco"),
        "{}",
        reqs[0].body
    );
}

#[tokio::test]
async fn a_failing_native_route_is_reported_rather_than_swallowed() {
    // Counterpart: without this, a client that never actually sent would still
    // pass the gate above if `send_notification` returned `Ok(())`
    // unconditionally — which is what it did before native routes existed and
    // no destination was configured.
    use birdnet_integrations::apprise::{Client, NotifyConfig};
    use birdnet_integrations::dispatch::routes;

    let stub = Stub::start(404, "no such topic").await;
    let parsed = routes(&format!("ntfy://{}/garden", stub.addr));
    let mut client = Client::new_cli_only(std::path::PathBuf::new(), NotifyConfig::default())
        .expect("client")
        .with_native_routes(parsed.native, false);

    let err = client
        .notify_detection("Strix aluco", 0.91, "2026-09-01", "03:14:00")
        .await
        .expect_err("the only destination refused the message");
    assert!(err.to_string().contains("404"), "{err}");
}

// ---------------------------------------------------------------------------
// The delivery guards, through the client the daemon holds
// ---------------------------------------------------------------------------

/// A client with one native route pointed at `stub`, and no rate limit.
fn client_for(addr: &str, rate_per_minute: u32) -> birdnet_integrations::apprise::Client {
    use birdnet_integrations::apprise::{Client, NotifyConfig};
    use birdnet_integrations::dispatch::routes;

    let parsed = routes(&format!("ntfy://{addr}/garden"));
    assert_eq!(parsed.native.len(), 1);
    Client::new_cli_only(
        std::path::PathBuf::new(),
        NotifyConfig {
            rate_per_minute,
            ..NotifyConfig::default()
        },
    )
    .expect("client")
    .with_native_routes(parsed.native, false)
}

#[tokio::test]
async fn a_dead_destination_stops_being_retried_per_detection() {
    // A retired webhook answers 404 to every request forever. Without a
    // breaker the station spends `MAX_ATTEMPTS` requests on it for every
    // detection, all day. The count on the wire is the whole point, so this
    // gate counts requests rather than inspecting state.
    let stub = Stub::start(404, "gone").await;
    let mut client = client_for(&stub.addr, 0);

    for _ in 0..20 {
        let _ = client
            .send_notification("t", "b", birdnet_integrations::apprise::NotifyType::Info)
            .await;
    }

    let attempts = stub.requests().len();
    assert!(
        attempts <= 4,
        "20 notifications to a dead destination made {attempts} requests; \
         the circuit should have opened after the third failure"
    );
    let (open, limited) = client.skip_counts();
    assert!(open >= 16, "only {open} sends were skipped");
    assert_eq!(limited, 0, "nothing here is rate limited");
}

#[tokio::test]
async fn a_healthy_destination_is_never_tripped() {
    // Counterpart: a breaker that opened regardless would also pass the gate
    // above, and would silently stop a working station from notifying.
    let stub = Stub::start(200, "{}").await;
    let mut client = client_for(&stub.addr, 0);

    for _ in 0..20 {
        client
            .send_notification("t", "b", birdnet_integrations::apprise::NotifyType::Info)
            .await
            .expect("delivered");
    }

    assert_eq!(stub.requests().len(), 20);
    assert_eq!(client.skip_counts(), (0, 0));
}

#[tokio::test]
async fn a_rate_limit_caps_what_reaches_a_working_destination() {
    // Pushover allows ten thousand messages a *month*. The cap is not about
    // our load, it is about not being cut off by the service.
    let stub = Stub::start(200, "{}").await;
    let mut client = client_for(&stub.addr, 5);

    for _ in 0..20 {
        let _ = client
            .send_notification("t", "b", birdnet_integrations::apprise::NotifyType::Info)
            .await;
    }

    assert_eq!(
        stub.requests().len(),
        5,
        "the rate limit did not bound what reached the destination"
    );
    let (open, limited) = client.skip_counts();
    assert_eq!(limited, 15);
    assert_eq!(open, 0, "a rate limit is not a destination failure");
}

#[tokio::test]
async fn being_rate_limited_never_opens_the_circuit() {
    // Counterpart to the gate above. Feeding a rate-limited send into the
    // breaker would make a busy morning look like an outage and suppress
    // notifications long after the burst had passed.
    let stub = Stub::start(200, "{}").await;
    let mut client = client_for(&stub.addr, 2);

    for _ in 0..10 {
        let _ = client
            .send_notification("t", "b", birdnet_integrations::apprise::NotifyType::Info)
            .await;
    }
    let (open, limited) = client.skip_counts();
    assert_eq!(open, 0, "the circuit opened on rate-limited sends");
    assert_eq!(limited, 8);
}
