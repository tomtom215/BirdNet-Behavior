//! Hooks the binary hands the web layer so "Test all channels" can exercise
//! the integrations the binary owns (DD-24).
//!
//! `POST /admin/notifications/test` answered "All configured channels
//! passed" in ten milliseconds with the MQTT broker dead: it tested push and
//! BirdWeather, both skipped on a station without them, and called the empty
//! run a pass. MQTT and email are built from the CLI and the config file by
//! the binary, out of the web crate's reach, so the binary supplies a probe
//! for each it configured; a probe that is absent is reported as "not
//! configured (skipped)", and a run in which every channel was skipped says
//! that nothing was tested.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// What a probe answers: a one-line success description, or the error.
pub type ProbeOutcome = Result<String, String>;

/// A probe's future.
pub type ProbeFuture = Pin<Box<dyn Future<Output = ProbeOutcome> + Send>>;

/// One channel's test: sends something real and reports what happened.
pub type Probe = Arc<dyn Fn() -> ProbeFuture + Send + Sync>;

/// The probes the binary configured.
#[derive(Clone, Default)]
pub struct NotificationProbes {
    /// Publishes a test message to the MQTT broker, when MQTT is configured.
    pub mqtt: Option<Probe>,
    /// Sends a test email, when email is configured.
    pub email: Option<Probe>,
}

impl std::fmt::Debug for NotificationProbes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationProbes")
            .field("mqtt", &self.mqtt.is_some())
            .field("email", &self.email.is_some())
            .finish()
    }
}

impl NotificationProbes {
    /// A probe that answers `outcome` — for tests of the page that runs them.
    #[must_use]
    pub fn fixed(outcome: ProbeOutcome) -> Probe {
        Arc::new(move || {
            let outcome = outcome.clone();
            Box::pin(async move { outcome })
        })
    }
}
