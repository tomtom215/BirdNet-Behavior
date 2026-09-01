//! Alert-rule webhook dispatch: the request-shape decision (pure) and the
//! network send (the only non-pure step).

use birdnet_db::alert_rules::WebhookAuth;

/// The wire-level shape of an outbound webhook request: method, body, and
/// content-type. Built by [`build_webhook_spec`] from operator-supplied
/// rule config, then handed to [`dispatch_webhook`] for the actual send.
///
/// Returned as a value (rather than wired into `reqwest::RequestBuilder`
/// directly) so the request shape can be tested without building a
/// reqwest client or hitting the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WebhookSpec {
    /// HTTP verb. `Get` carries no body; `Post` carries a JSON body.
    pub method: WebhookMethod,
    /// JSON body sent with `Post`. Defaults to `"{}"` when the operator
    /// supplies no body template — matching the historical behaviour
    /// expected by alert-rule sinks that require valid JSON.
    pub body: String,
}

/// Webhook HTTP method picked from the operator's alert-rule config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WebhookMethod {
    /// `GET` — used when the operator's rule has `method = "GET"`
    /// (case-insensitive). The body is ignored.
    Get,
    /// `POST` — the default for everything else. Carries the JSON body.
    Post,
}

/// Pure helper: build the [`WebhookSpec`] for an alert-rule webhook.
///
/// The previous inline form in `dispatch_webhook` produced a chain of
/// mutants on the method-comparison + body-default that no test could
/// catch without running an HTTP server. Returning a comparable value
/// makes the dispatch decision unit-testable: tests assert each cell of
/// the (method, body-present) decision matrix.
///
/// Method is case-insensitively matched: `"get"`, `"Get"`, and `"GET"`
/// all produce [`WebhookMethod::Get`]. Anything else (including
/// nonsense like `"PATCH"`) falls through to `Post` because the
/// alert-rule schema documents only `GET` and `POST` and we want the
/// safe default for misconfigured rules.
#[must_use]
pub(super) fn build_webhook_spec(method: &str, body: Option<&str>) -> WebhookSpec {
    if method.eq_ignore_ascii_case("GET") {
        WebhookSpec {
            method: WebhookMethod::Get,
            body: String::new(),
        }
    } else {
        WebhookSpec {
            method: WebhookMethod::Post,
            body: body.map_or_else(|| "{}".to_owned(), str::to_owned),
        }
    }
}

/// Error type for the webhook dispatcher.
///
/// Returning a typed error (rather than `()` plus tracing-only diagnostics)
/// has two benefits:
/// * The function's body-replacement cargo-mutants become unviable —
///   `Result<u16, WebhookError>` can't be substituted with `()`.
/// * The caller can react to specific failure modes if it ever wants
///   to (today it just logs, but the surface is there).
#[derive(Debug)]
pub(super) enum WebhookError {
    /// Building the reqwest client failed (TLS init, system DNS, etc.).
    ClientBuild(String),
    /// The request was sent but the network or the remote rejected it.
    Send(String),
}

impl std::fmt::Display for WebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClientBuild(e) => write!(f, "failed to build HTTP client for webhook: {e}"),
            Self::Send(e) => write!(f, "webhook dispatch failed: {e}"),
        }
    }
}

impl std::error::Error for WebhookError {}

/// Fire an alert-rule webhook request and return the HTTP status on success.
///
/// `dispatch_webhook` is the only non-pure step in the alert-rule dispatch
/// chain: every decision *about* the request is delegated to
/// [`build_webhook_spec`], which is unit-tested. This function's body is
/// dominated by the network call; returning `Result<u16, WebhookError>`
/// makes the body-replacement cargo-mutants unviable.
pub(super) async fn dispatch_webhook(
    url: &str,
    method: &str,
    body: Option<&str>,
    auth: Option<&WebhookAuth>,
) -> Result<u16, WebhookError> {
    let spec = build_webhook_spec(method, body);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| WebhookError::ClientBuild(e.to_string()))?;

    let mut request = match spec.method {
        WebhookMethod::Get => client.get(url),
        WebhookMethod::Post => client
            .post(url)
            .header("Content-Type", "application/json")
            .body(spec.body),
    };

    request = match auth {
        Some(WebhookAuth::Bearer(token)) => request.bearer_auth(token),
        Some(WebhookAuth::Basic { user, password }) => request.basic_auth(user, Some(password)),
        Some(WebhookAuth::Header { name, value }) => request.header(name, value),
        None => request,
    };

    request
        .send()
        .await
        .map(|resp| resp.status().as_u16())
        // `reqwest::Error` appends the URL it failed on. A rule's webhook URL
        // is frequently the credential itself — a Slack or Discord hook, a
        // Home Assistant `?token=` — so it must not reach the log through an
        // error message.
        .map_err(|e| WebhookError::Send(e.without_url().to_string()))
}

/// A webhook URL with everything after the host removed.
///
/// The path and query of a webhook URL are where the secret lives, and the
/// dispatch log line names the rule and the target on every failure.
#[must_use]
pub(super) fn redact_url(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    // Userinfo is a credential too: `https://user:pass@host/...`.
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    if scheme.is_empty() {
        host.to_string()
    } else {
        format!("{scheme}://{host}/…")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WebhookAuth, WebhookError, WebhookMethod, build_webhook_spec, dispatch_webhook, redact_url,
    };

    // ── build_webhook_spec ──────────────────────────────────────────────
    //
    // The four cells of the decision matrix:
    //   (method ∈ {GET, POST}) × (body ∈ {Some, None})
    // The pure helper makes every cell observable in a unit test.

    #[test]
    fn build_webhook_spec_get_ignores_body() {
        // GET → method=Get, body empty (per the contract docstring).
        let s = build_webhook_spec("GET", Some("{\"hello\": \"world\"}"));
        assert_eq!(s.method, WebhookMethod::Get);
        assert_eq!(s.body, "");
        // Case insensitivity:
        let s2 = build_webhook_spec("get", None);
        assert_eq!(s2.method, WebhookMethod::Get);
        let s3 = build_webhook_spec("Get", None);
        assert_eq!(s3.method, WebhookMethod::Get);
    }

    #[test]
    fn build_webhook_spec_post_uses_supplied_body() {
        let s = build_webhook_spec("POST", Some("{\"k\": 1}"));
        assert_eq!(s.method, WebhookMethod::Post);
        assert_eq!(s.body, "{\"k\": 1}");
    }

    #[test]
    fn build_webhook_spec_post_defaults_body_to_empty_object() {
        // No body supplied: default to "{}" so the recipient sees valid
        // JSON. Pins the contract for alert-rule sinks that require
        // application/json bodies.
        let s = build_webhook_spec("POST", None);
        assert_eq!(s.method, WebhookMethod::Post);
        assert_eq!(s.body, "{}");
    }

    #[test]
    fn build_webhook_spec_unknown_method_falls_back_to_post() {
        // Operator misconfigures method as "PATCH"? Fall back to POST
        // because the safe default is "send something with a body" rather
        // than "send a GET with no body".
        let s = build_webhook_spec("PATCH", Some("{\"a\":1}"));
        assert_eq!(s.method, WebhookMethod::Post);
        assert_eq!(s.body, "{\"a\":1}");
    }

    // ── dispatch_webhook ────────────────────────────────────────────────
    //
    // The function is async and makes a real network call, so the test
    // exercises a deliberately-failing URL (TEST-NET-2, RFC 5737) and
    // asserts the Err arm fires. This is enough to catch:
    //   - "replace dispatch_webhook -> Result<u16, WebhookError> with Ok(0)" — would
    //     produce Ok(0), the test asserts Err.
    //   - "replace … with Err(WebhookError::ClientBuild(\"xyzzy\".into()))" — would
    //     produce the wrong error variant, the test asserts the Send
    //     variant (because client builds fine on a healthy CI host).
    //   - "replace dispatch_webhook -> Result<u16, WebhookError> with ()" — unviable
    //     by return type, no longer counted as missed.

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_webhook_returns_send_error_on_unreachable() {
        // TEST-NET-2 (198.51.100.0/24) — RFC 5737 reserved range, will
        // not route. The 10-second timeout caps the test runtime.
        let r = dispatch_webhook("http://198.51.100.1:1/", "POST", None, None).await;
        assert!(r.is_err(), "expected Err on unreachable host, got {r:?}");
        // Confirm it's the Send variant, not ClientBuild — the client
        // builds fine; only the network call fails.
        match r {
            Err(WebhookError::Send(_)) => {}
            Err(WebhookError::ClientBuild(e)) => {
                panic!("expected Send error, got ClientBuild({e})")
            }
            Ok(s) => panic!("expected Err, got Ok({s})"),
        }
    }

    #[test]
    fn webhook_error_display_distinguishes_variants() {
        // Pin the Display impls so the log messages from the call site
        // remain searchable. Catches "delete fmt arm" mutations.
        assert!(
            WebhookError::ClientBuild("tls-init failed".into())
                .to_string()
                .contains("client")
        );
        assert!(
            WebhookError::Send("connect refused".into())
                .to_string()
                .contains("dispatch")
        );
    }

    // ── authentication reaches the wire ─────────────────────────────────
    //
    // `build_webhook_spec` is pure and says nothing about headers, so the only
    // way to know a credential is actually sent is to read the bytes. These
    // use a one-shot blocking stub on an ephemeral port rather than a mock:
    // what is being checked is the request as a server sees it.

    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// Serve exactly one request and return everything up to the body.
    fn stub_once() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut buf = vec![0_u8; 8192];
            let n = sock.read(&mut buf).unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
            let _ = sock.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
            let _ = sock.flush();
        });
        (addr, rx)
    }

    /// The headers of the single request `auth` produces, lowercased by name.
    async fn headers_for(auth: Option<&WebhookAuth>) -> Vec<(String, String)> {
        let (addr, rx) = stub_once();
        let status = dispatch_webhook(&format!("http://{addr}/hook"), "POST", None, auth)
            .await
            .expect("dispatched");
        assert_eq!(status, 204);
        let raw = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("request");
        raw.lines()
            .skip(1)
            .take_while(|l| !l.is_empty())
            .filter_map(|l| l.split_once(':'))
            .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect()
    }

    /// One header value by (lowercase) name.
    fn header_of<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn a_bearer_credential_reaches_the_server() {
        let headers = headers_for(Some(&WebhookAuth::Bearer("tok_ABC123".into()))).await;
        assert_eq!(
            header_of(&headers, "authorization"),
            Some("Bearer tok_ABC123")
        );
    }

    #[tokio::test]
    async fn a_basic_credential_reaches_the_server_base64_encoded() {
        let headers = headers_for(Some(&WebhookAuth::Basic {
            user: "ada".into(),
            password: "hunter2".into(),
        }))
        .await;
        // base64("ada:hunter2")
        assert_eq!(
            header_of(&headers, "authorization"),
            Some("Basic YWRhOmh1bnRlcjI=")
        );
    }

    #[tokio::test]
    async fn a_custom_header_credential_reaches_the_server_under_its_own_name() {
        let headers = headers_for(Some(&WebhookAuth::Header {
            name: "X-API-Key".into(),
            value: "k_XYZ".into(),
        }))
        .await;
        assert_eq!(header_of(&headers, "x-api-key"), Some("k_XYZ"));
        assert_eq!(
            header_of(&headers, "authorization"),
            None,
            "a custom header must not also set Authorization"
        );
    }

    #[tokio::test]
    async fn a_rule_without_auth_sends_no_authorization_header() {
        // Counterpart: an empty or defaulted credential header is not the same
        // as none. Some servers reject a malformed `Authorization` outright,
        // and others count it as a failed login attempt and lock the account.
        let headers = headers_for(None).await;
        assert_eq!(header_of(&headers, "authorization"), None);
        // ...and the request is otherwise unchanged.
        assert_eq!(
            header_of(&headers, "content-type"),
            Some("application/json")
        );
    }

    // ── the URL is a credential too ─────────────────────────────────────

    #[test]
    fn redacting_a_url_keeps_the_host_and_drops_everything_after_it() {
        // A Slack or Discord webhook URL *is* the credential, and Home
        // Assistant's is `?token=`. The dispatch log names the target on every
        // failure, which on a broken rule is every detection.
        let cases = [
            (
                "https://hooks.slack.com/services/T0/B0/SUPERSECRET",
                "https://hooks.slack.com/…",
            ),
            (
                "https://ha.lan:8123/api/webhook/SUPERSECRET?token=SUPERSECRET",
                "https://ha.lan:8123/…",
            ),
            (
                "https://ada:SUPERSECRET@hooks.example.com/x",
                "https://hooks.example.com/…",
            ),
            ("http://198.51.100.1:1/", "http://198.51.100.1:1/…"),
        ];
        for (url, expected) in cases {
            let redacted = redact_url(url);
            assert_eq!(redacted, expected, "{url}");
            assert!(!redacted.contains("SUPERSECRET"), "{redacted}");
        }
    }

    #[tokio::test]
    async fn a_failed_dispatch_does_not_name_the_url_it_failed_on() {
        // `reqwest::Error` appends ` for url (...)` in `Display`, so the error
        // itself leaks the whole webhook — including the path the credential
        // lives in — even when the caller only logs `{e}`.
        let e = dispatch_webhook(
            "http://198.51.100.1:1/hook/SUPERSECRETPATH",
            "POST",
            None,
            None,
        )
        .await
        .expect_err("TEST-NET-2 port 1 cannot connect");
        let rendered = format!("{e} {e:?}");
        assert!(!rendered.contains("SUPERSECRETPATH"), "{rendered}");
    }
}
