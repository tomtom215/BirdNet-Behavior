//! The notifier the station's alert loops deliver through, shared with the web
//! layer so "Test notifications" can use it.
//!
//! # Why the web layer needs a handle at all
//!
//! `OB-9`. The admin page's Apprise test built a **fresh** `reqwest::Client`
//! and `POST`ed `{apprise_url}/notify` itself. That exercised none of the
//! machinery that decides whether an alert about the station actually leaves
//! the box: not the native (`ntfy://`, `discord://`, …) routes delivered
//! in-process by [`birdnet_integrations::dispatch`], not the `apprise` CLI
//! fallback, not the per-destination circuit breaker, and not the rate limiter
//! whose exhaustion during a dawn chorus is what `OB-5` was about. A green
//! "test notification sent" therefore said nothing about the deadman alert.
//!
//! Worse, the button was *disabled* for the configuration most stations have:
//! it keyed off the `apprise_url` setting — an Apprise **API server** — so a
//! station configured only with native notification URLs saw "Not configured"
//! and a dead button while its alerts worked fine.
//!
//! Holding the same [`birdnet_integrations::apprise::Client`] the alert loops
//! hold fixes both: the test is the same call `announce::flush` makes, against
//! the same guards, and the page can say which destinations the running
//! station actually resolved instead of guessing from a settings row.
//!
//! # Why the destination summary is a snapshot
//!
//! The client lives behind a `tokio::sync::Mutex`, so reading it needs an
//! `await`; the pages that render the test card are synchronous. The routes,
//! though, are fixed once — [`birdnet_integrations::apprise::Client::with_native_routes`]
//! is a construction-time builder — so the labels, the CLI fallback flag and
//! the server URL are copied out once at wiring time and never go stale. Only
//! the *sending* takes the lock. What does change at runtime — an open
//! circuit, an exhausted bucket — is deliberately not summarised here: it is
//! what pressing the button reports.

use std::sync::Arc;

use birdnet_integrations::apprise::Client;

/// A shared, lockable Apprise client — the type the binary's alert loops hold.
pub type ClientHandle = Arc<tokio::sync::Mutex<Client>>;

/// The station's notifier, plus what it resolved to at startup.
#[derive(Debug, Clone)]
pub struct Notifier {
    /// The client every operational alert is delivered through.
    client: ClientHandle,
    /// Credential-free labels for the natively delivered destinations, as
    /// [`Client::native_labels`] reports them. Safe to render.
    destinations: Arc<[String]>,
    /// Whether the `apprise` CLI would be invoked for a configured config file.
    apprise_cli: bool,
    /// Whether an Apprise API server URL was resolved.
    apprise_server: bool,
}

impl Notifier {
    /// Snapshot `client`'s resolved destinations and keep the handle.
    ///
    /// Takes the lock once, at wiring time, before the handle is shared.
    pub async fn attach(client: ClientHandle) -> Self {
        let (destinations, apprise_cli, apprise_server) = {
            let guard = client.lock().await;
            (
                guard
                    .native_labels()
                    .into_iter()
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
                    .into(),
                guard.needs_apprise_cli(),
                !guard.url().is_empty(),
            )
        };
        Self {
            client,
            destinations,
            apprise_cli,
            apprise_server,
        }
    }

    /// Build one directly from its parts, for tests and callers that already
    /// know what the client resolved.
    #[must_use]
    pub fn from_parts(
        client: ClientHandle,
        destinations: Vec<String>,
        apprise_cli: bool,
        apprise_server: bool,
    ) -> Self {
        Self {
            client,
            destinations: destinations.into(),
            apprise_cli,
            apprise_server,
        }
    }

    /// The client itself — the same handle `announce::flush` locks.
    #[must_use]
    pub const fn client(&self) -> &ClientHandle {
        &self.client
    }

    /// Credential-free labels for the natively delivered destinations.
    #[must_use]
    pub fn destinations(&self) -> &[String] {
        &self.destinations
    }

    /// Whether the `apprise` CLI would be invoked for a configured config file.
    #[must_use]
    pub const fn apprise_cli(&self) -> bool {
        self.apprise_cli
    }

    /// Whether an Apprise API server URL was resolved.
    #[must_use]
    pub const fn apprise_server(&self) -> bool {
        self.apprise_server
    }
}
