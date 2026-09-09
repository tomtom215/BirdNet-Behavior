//! The station-health conditions as last evaluated, for anyone who asks
//! (OP-4).
//!
//! The conditions the notifier pushes — a dead microphone, a full disk, a
//! failing backup, a lost data volume — were push-only: `evaluate` was private
//! and its sole caller the notifier, so an operator who missed a push could
//! not ask "what is wrong right now?". The notifier now publishes what it
//! found here on every poll, and `/api/v2/health/conditions` answers from it,
//! saying when it was evaluated so a reader knows how old the answer is.

use serde::Serialize;

/// One thing wrong with the station, as the notifier would say it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Condition {
    /// Stable identity (`disk`, `source:cam1`, `maintenance-failed:backup`).
    pub key: String,
    /// Short title.
    pub title: String,
    /// What is wrong, and what to do about it.
    pub body: String,
}

/// What the last evaluation found, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ConditionsSnapshot {
    /// Seconds since the Unix epoch at which `conditions` was evaluated;
    /// `None` until the notifier has run once (or when it is disabled).
    pub evaluated_at: Option<u64>,
    /// Everything wrong at that moment; empty when nothing was.
    pub conditions: Vec<Condition>,
}

impl ConditionsSnapshot {
    /// A snapshot taken now.
    #[must_use]
    pub fn now(conditions: Vec<Condition>) -> Self {
        Self {
            evaluated_at: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            ),
            conditions,
        }
    }
}
