//! Executes the requests [`super::plan`] describes.
//!
//! # Keeping credentials out of the log
//!
//! For Discord, Slack webhooks and Telegram the credential *is* part of the
//! request URL. `reqwest::Error` carries the URL it failed on and appends it
//! to its `Display` output, so a bare `{e}` in a `tracing::warn!` publishes a
//! working Discord webhook to the operator's journal. Every transport error
//! here therefore goes through [`reqwest::Error::without_url`] first, and the
//! gate `a_transport_error_never_carries_the_url` holds it.

use std::fmt;

use super::parse::Target;
use super::plan::{Auth, Body, Expect, Message, Plan, plans};

/// Why a native send failed.
#[derive(Debug)]
pub enum SendError {
    /// The request never completed (DNS, TCP, TLS, timeout).
    ///
    /// The message has had the URL stripped: see the module docs.
    Transport {
        /// Service name, e.g. `"discord"`.
        kind: &'static str,
        /// Reason, with no URL in it.
        reason: String,
    },
    /// The service answered, and said no.
    Rejected {
        /// Service name, e.g. `"discord"`.
        kind: &'static str,
        /// HTTP status, or the 2xx status that accompanied `{"ok": false}`.
        status: u16,
        /// Truncated response detail.
        detail: String,
    },
}

impl SendError {
    /// Service name, safe to log or use as a metric label.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Transport { kind, .. } | Self::Rejected { kind, .. } => kind,
        }
    }

    /// Whether retrying this send could plausibly succeed.
    ///
    /// Transport failures, 5xx and 429 are worth another attempt; any other
    /// 4xx means the URL or the payload is wrong and repeating it only spends
    /// the service's rate limit. A `{"ok": false}` body is reported with the
    /// 2xx status that carried it, so it lands here as non-retryable — which
    /// is right: Telegram answers `200` with `chat not found` indefinitely.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Transport { .. } => true,
            Self::Rejected { status, .. } => *status >= 500 || *status == 429,
        }
    }
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport { kind, reason } => write!(f, "{kind} request failed: {reason}"),
            Self::Rejected {
                kind,
                status,
                detail,
            } => write!(f, "{kind} rejected the message (HTTP {status}): {detail}"),
        }
    }
}

impl std::error::Error for SendError {}

/// Longest response body quoted back in an error.
const DETAIL_LIMIT: usize = 200;

/// Describe a transport failure without disclosing the URL it happened on.
///
/// `Error::without_url` removes the ` for url (...)` clause `Display` appends,
/// but on its own that leaves an unhelpful "error sending request", so the
/// source chain — which carries the actual cause and no URL — is appended.
fn transport(kind: &'static str, e: reqwest::Error) -> SendError {
    let e = e.without_url();
    let mut reason = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        use std::fmt::Write as _;
        let _ = write!(reason, ": {cause}");
        source = cause.source();
    }
    SendError::Transport { kind, reason }
}

/// Clip a response body to something a log line can carry.
fn truncate(s: &str) -> String {
    let trimmed = s.trim();
    match trimmed.char_indices().nth(DETAIL_LIMIT) {
        Some((idx, _)) => format!("{}…", &trimmed[..idx]),
        None => trimmed.to_string(),
    }
}

/// Decide whether a completed response counts as delivered.
///
/// Separate from the request so the `{"ok": false}` cases can be tested
/// without a socket.
pub(super) fn judge(
    kind: &'static str,
    expect: Expect,
    status: u16,
    text: &str,
) -> Result<(), SendError> {
    let reject = || SendError::Rejected {
        kind,
        status,
        detail: truncate(text),
    };
    if !(200..300).contains(&status) {
        return Err(reject());
    }
    if expect == Expect::Status {
        return Ok(());
    }
    // Slack's legacy webhook replies with the bare string `ok`; the Web API
    // and Telegram reply with JSON carrying an `ok` boolean.
    if text.trim() == "ok" {
        return Ok(());
    }
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
    if parsed.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(());
    }
    Err(reject())
}

/// Issue one planned request.
async fn execute(http: &reqwest::Client, plan: &Plan) -> Result<(), SendError> {
    let mut req = http.post(&plan.url);
    req = match &plan.body {
        Body::Json(v) => req.json(v),
        Body::Form(kv) => req.form(kv),
    };
    for (name, value) in &plan.headers {
        req = req.header(*name, value);
    }
    req = match &plan.auth {
        Some(Auth::Bearer(t)) => req.bearer_auth(t),
        Some(Auth::Basic { user, password }) => req.basic_auth(user, password.as_ref()),
        None => req,
    };

    let resp = req.send().await.map_err(|e| transport(plan.kind, e))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    judge(plan.kind, plan.expect, status, &text)
}

/// Deliver `msg` to `target`.
///
/// A target naming several topics or chats produces several requests; all are
/// attempted and the first failure is what is returned, so one stale chat id
/// does not silence the others.
///
/// # Errors
///
/// Returns [`SendError`] if a request fails or the service rejects it.
pub async fn send(http: &reqwest::Client, target: &Target, msg: &Message) -> Result<(), SendError> {
    let mut first_error = None;
    for plan in plans(target, msg) {
        if let Err(e) = execute(http, &plan).await
            && first_error.is_none()
        {
            first_error = Some(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::{Expect, judge};

    #[test]
    fn a_two_hundred_with_ok_false_is_a_rejection() {
        // Slack and Telegram report a bad token, a missing channel and a
        // deleted chat all as `200 OK` with `{"ok": false}`. Judging on status
        // alone would count every one of those as a delivered notification and
        // the operator would never learn the channel is misconfigured.
        let err = judge(
            "slack",
            Expect::OkField,
            200,
            r#"{"ok":false,"error":"channel_not_found"}"#,
        )
        .expect_err("must not count as delivered");
        assert!(err.to_string().contains("channel_not_found"), "{err}");
        assert!(!err.is_retryable(), "a missing channel will not reappear");
    }

    #[test]
    fn a_two_hundred_with_ok_true_is_a_delivery() {
        // Counterpart: without this the rule "OkField always rejects" would
        // also satisfy the gate above, and nothing would ever send.
        assert!(judge("slack", Expect::OkField, 200, r#"{"ok":true}"#).is_ok());
        assert!(
            judge(
                "telegram",
                Expect::OkField,
                200,
                r#"{"ok":true,"result":{}}"#
            )
            .is_ok()
        );
    }

    #[test]
    fn slacks_legacy_webhook_answers_with_the_bare_word_ok() {
        // The incoming-webhook endpoint replies `ok`, not JSON, so a
        // JSON-only check would report every successful post as a failure.
        assert!(judge("slack", Expect::OkField, 200, "ok").is_ok());
        assert!(judge("slack", Expect::OkField, 200, "ok\n").is_ok());
        // ...but not any other plain-text body.
        assert!(judge("slack", Expect::OkField, 200, "invalid_payload").is_err());
    }

    #[test]
    fn a_status_only_service_is_not_asked_about_its_body() {
        // Discord answers 204 with an empty body and ntfy answers 200 with a
        // JSON message object that has no `ok` field at all.
        assert!(judge("discord", Expect::Status, 204, "").is_ok());
        assert!(
            judge(
                "ntfy",
                Expect::Status,
                200,
                r#"{"id":"x","topic":"garden"}"#
            )
            .is_ok()
        );
    }

    #[test]
    fn a_long_error_body_is_clipped_rather_than_logged_whole() {
        // An HTML error page from a misconfigured reverse proxy is tens of
        // kilobytes; a station's journal should not carry it once per detection.
        let huge = "x".repeat(10_000);
        let err = judge("gotify", Expect::Status, 500, &huge).expect_err("rejected");
        assert!(err.to_string().len() < 400, "{}", err.to_string().len());
        assert!(err.to_string().ends_with('…'));
    }

    #[test]
    fn clipping_counts_characters_not_bytes() {
        // Slicing a multi-byte string at a byte offset panics, and species
        // names and server messages are routinely non-ASCII.
        //
        // The leading `x` is load-bearing: `é` is two bytes, so a plain run of
        // them puts a char boundary on every even byte and a byte-offset slice
        // at `DETAIL_LIMIT` would land safely by luck. One ASCII byte in front
        // shifts the run odd, so `&s[..DETAIL_LIMIT]` cuts a character in half.
        let huge = format!("x{}", "é".repeat(10_000));
        assert!(
            !huge.is_char_boundary(super::DETAIL_LIMIT),
            "no longer a trap"
        );
        let err = judge("gotify", Expect::Status, 500, &huge).expect_err("rejected");
        assert!(err.to_string().contains('é'));
    }
}
