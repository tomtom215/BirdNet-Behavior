//! Native delivery for the notification services operators actually use.
//!
//! BirdNET-Pi shells out to the `apprise` Python CLI for every notification.
//! That works, but it makes a Python interpreter, a pip install and a
//! subprocess-per-detection part of the station's runtime, and it means a
//! misconfigured URL fails as an opaque non-zero exit status.
//!
//! This module keeps the **Apprise URL syntax** — which is what operators
//! already have written down — and delivers it in-process for the seven
//! schemes below. Anything else still goes to Apprise, so nobody's existing
//! configuration stops working:
//!
//! | Scheme | Service |
//! |---|---|
//! | `discord://` | Discord incoming webhook |
//! | `slack://` | Slack webhook or `chat.postMessage` |
//! | `tgram://` | Telegram bot API |
//! | `ntfy://`, `ntfys://` | ntfy (cloud or self-hosted) |
//! | `gotify://`, `gotifys://` | Gotify |
//! | `pover://`, `pushover://` | Pushover |
//! | `json://`, `jsons://` | generic JSON webhook |
//!
//! # Example
//!
//! ```rust
//! use birdnet_integrations::dispatch::{parse, Target};
//!
//! let t = parse("tgram://123456789:AAE_secret/-1001234567890/").unwrap();
//! assert_eq!(t.kind(), "telegram");
//! ```

mod parse;
mod plan;
mod send;

pub use parse::{NtfyAuth, ParseError, SlackAuth, Target, parse};
pub use plan::{Auth, Body, Expect, Message, Plan, Severity, plans};
pub use send::{SendError, send};

use std::time::Duration;

/// Total attempts (initial + retries) for one target before giving up.
const MAX_ATTEMPTS: u32 = 3;

/// A parsed configuration line, remembering how it was written.
#[derive(Debug, Clone)]
pub struct Route {
    /// Where to deliver.
    pub target: Target,
    /// Operator-facing label — the scheme and, where the URL names one, the
    /// host. Never the credential, so this is safe to log or show in the UI.
    pub label: String,
}

/// The result of reading a set of notification URLs.
#[derive(Debug, Default)]
pub struct Routes {
    /// URLs this crate delivers itself.
    pub native: Vec<Route>,
    /// Schemes seen that have no native sender, deduplicated. The caller falls
    /// back to Apprise for these and can name them in one warning.
    pub deferred: Vec<String>,
    /// Count of lines that were not URLs at all.
    pub unparseable: usize,
}

/// Read a set of notification URLs, one per line or comma-separated.
///
/// Blank lines and `#` comments are skipped, so an Apprise config file can be
/// passed through unchanged.
#[must_use]
pub fn routes(config: &str) -> Routes {
    let mut out = Routes::default();
    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        // An Apprise config file may write `tag=url`; take the URL half.
        let url = line.split_once('=').map_or(line, |(lhs, rhs)| {
            if lhs.contains("://") {
                line
            } else {
                rhs.trim()
            }
        });
        for piece in url.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match parse(piece) {
                Ok(target) => {
                    let label = label_for(&target);
                    out.native.push(Route { target, label });
                }
                Err(ParseError::UnsupportedScheme(s)) => {
                    if !out.deferred.contains(&s) {
                        out.deferred.push(s);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "ignoring a notification URL");
                    out.unparseable += 1;
                }
            }
        }
    }
    out
}

/// A credential-free description of a target, for logs and the settings UI.
fn label_for(target: &Target) -> String {
    let kind = target.kind();
    match target {
        Target::Ntfy { origin, topics, .. } => {
            format!("{kind} {origin} ({} topic(s))", topics.len())
        }
        Target::Gotify { origin, .. } => format!("{kind} {origin}"),
        Target::Json { endpoint, .. } => format!("{kind} {}", host_of(endpoint)),
        Target::Telegram { chat_ids, .. } => format!("{kind} ({} chat(s))", chat_ids.len()),
        Target::Discord { .. } | Target::Slack { .. } | Target::Pushover { .. } => kind.to_string(),
    }
}

/// The `scheme://host` prefix of a URL, dropping any path — which for a
/// webhook is where the secret lives.
fn host_of(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host = rest.split('/').next().unwrap_or(rest);
    if scheme.is_empty() {
        host.to_string()
    } else {
        format!("{scheme}://{host}")
    }
}

/// Deliver `msg` to `target`, retrying transient failures with jittered backoff.
///
/// Retries stop early on a non-retryable rejection: a 4xx means the URL or the
/// payload is wrong, and repeating it only spends the service's rate limit.
///
/// # Errors
///
/// Returns the last [`SendError`] if every attempt fails.
pub async fn send_with_retry(
    http: &reqwest::Client,
    target: &Target,
    msg: &Message,
) -> Result<(), SendError> {
    let mut last = None;
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            let delay = crate::retry::backoff_delay(attempt, crate::retry::jitter_frac());
            tracing::debug!(
                service = target.kind(),
                attempt = attempt + 1,
                delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                "retrying notification"
            );
            tokio::time::sleep(delay).await;
        }
        match send(http, target, msg).await {
            Ok(()) => return Ok(()),
            Err(e) if !e.is_retryable() => return Err(e),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| SendError::Transport {
        kind: target.kind(),
        reason: "no attempts made".to_string(),
    }))
}

/// Request timeout for native sends.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Build the HTTP client the native senders use.
///
/// # Errors
///
/// Returns the underlying [`reqwest::Error`] if the client cannot be built.
pub fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder().timeout(DEFAULT_TIMEOUT).build()
}
