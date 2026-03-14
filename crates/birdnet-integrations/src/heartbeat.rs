//! Heartbeat client for uptime monitoring.
//!
//! Pings a configured URL after each analysis cycle, compatible with
//! monitoring services like Uptime Kuma, Healthchecks.io, and similar.

use std::fmt;
use std::time::Duration;

/// Default request timeout for heartbeat pings.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors from the heartbeat client.
#[derive(Debug)]
pub enum HeartbeatError {
    /// HTTP request failed.
    Http(String),
    /// Server returned a non-success status.
    Server(String),
}

impl fmt::Display for HeartbeatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(msg) => write!(f, "heartbeat HTTP error: {msg}"),
            Self::Server(msg) => write!(f, "heartbeat server error: {msg}"),
        }
    }
}

impl std::error::Error for HeartbeatError {}

/// A simple heartbeat client that pings a URL to indicate liveness.
///
/// Used with uptime monitoring services (Uptime Kuma, Healthchecks.io, etc.)
/// to confirm that the detection pipeline is running.
#[derive(Debug, Clone)]
pub struct HeartbeatClient {
    /// The URL to ping.
    url: String,
    /// HTTP client.
    client: reqwest::Client,
}

impl HeartbeatClient {
    /// Create a new heartbeat client.
    ///
    /// # Errors
    ///
    /// Returns `HeartbeatError::Http` if the HTTP client cannot be built.
    pub fn new(url: &str) -> Result<Self, HeartbeatError> {
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| HeartbeatError::Http(e.to_string()))?;

        Ok(Self {
            url: url.to_string(),
            client,
        })
    }

    /// Send a heartbeat ping (HTTP GET to the configured URL).
    ///
    /// # Errors
    ///
    /// Returns `HeartbeatError` on network or server failure.
    pub async fn ping(&self) -> Result<(), HeartbeatError> {
        let resp = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|e| HeartbeatError::Http(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            Err(HeartbeatError::Server(format!(
                "heartbeat ping returned {status}"
            )))
        }
    }

    /// Get the configured URL.
    pub fn url(&self) -> &str {
        &self.url
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_client_stores_url() {
        let client = HeartbeatClient::new("https://hc-ping.com/abc-123").unwrap();
        assert_eq!(client.url(), "https://hc-ping.com/abc-123");
    }

    #[test]
    fn heartbeat_error_display() {
        let err = HeartbeatError::Http("connection refused".to_string());
        assert_eq!(err.to_string(), "heartbeat HTTP error: connection refused");

        let err = HeartbeatError::Server("404".to_string());
        assert_eq!(err.to_string(), "heartbeat server error: 404");
    }
}
