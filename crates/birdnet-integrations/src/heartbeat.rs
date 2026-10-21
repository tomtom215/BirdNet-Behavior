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
#[derive(Clone)]
pub struct HeartbeatClient {
    /// The URL to ping.
    url: String,
    /// HTTP client.
    client: reqwest::Client,
}

impl std::fmt::Debug for HeartbeatClient {
    /// Hand-written because a heartbeat URL carries its check's key in the
    /// path: only the scheme and host are printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeartbeatClient")
            .field(
                "url",
                &birdnet_core::config::redact::redact_value(&self.url),
            )
            .finish_non_exhaustive()
    }
}

impl HeartbeatClient {
    /// Create a new heartbeat client.
    ///
    /// # Errors
    ///
    /// Returns `HeartbeatError::Http` if the HTTP client cannot be built.
    pub fn new(url: &str) -> Result<Self, HeartbeatError> {
        // The guarded builder: the address is typed by an operator (or imported
        // from another station's `birdnet.conf`), and it is requested on a
        // schedule for as long as the station runs. A link-local or cloud
        // metadata address is refused, by name resolution and on every
        // redirect hop; a LAN or loopback monitor (Uptime Kuma next door) is
        // not. No proxy is used, as for webhooks: through one the proxy would
        // resolve the name and the guard would see nothing.
        let client = crate::egress_guard::guarded_client_builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| HeartbeatError::Http(crate::http_error_text(e)))?;

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
        // A literal host never reaches the guarded resolver, so it is checked
        // here. An unparseable URL is left for the request to report.
        if let Ok(parsed) = reqwest::Url::parse(&self.url) {
            crate::egress_guard::check_url(&parsed)
                .map_err(|refused| HeartbeatError::Http(refused.to_string()))?;
        }
        let resp = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|e| HeartbeatError::Http(crate::http_error_text(e)))?;

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

    /// A heartbeat address typed (or imported) as a cloud metadata address is
    /// not requested.
    #[tokio::test]
    async fn a_metadata_address_is_refused_not_pinged() {
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://[fe80::1]/x",
            "http://0xa9.0xfe.0xa9.0xfe/x",
        ] {
            let err = HeartbeatClient::new(url)
                .unwrap()
                .ping()
                .await
                .expect_err("must be refused");
            assert!(
                err.to_string().contains("outbound-address guard"),
                "{url}: {err}"
            );
        }
    }

    /// The counterpart: a monitor on the same box or LAN is pinged, so the
    /// guard is not a blanket refusal.
    #[tokio::test]
    async fn a_loopback_monitor_is_pinged() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{Read as _, Write as _};
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 1024];
            let n = sock.read(&mut buf).unwrap();
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        HeartbeatClient::new(&format!("http://127.0.0.1:{port}/ping/key-123"))
            .unwrap()
            .ping()
            .await
            .expect("a loopback monitor answers");
        assert!(server.join().unwrap().starts_with("GET /ping/key-123 "));
    }

    #[test]
    fn heartbeat_error_display() {
        let err = HeartbeatError::Http("connection refused".to_string());
        assert_eq!(err.to_string(), "heartbeat HTTP error: connection refused");

        let err = HeartbeatError::Server("404".to_string());
        assert_eq!(err.to_string(), "heartbeat server error: 404");
    }
}
