//! How the operator's reset reaches the tracker.
//!
//! The tracker lives in the detection event processor's thread and is the
//! authority for what the station has learned (see [`super::DynamicThresholds`]);
//! the database table is only its copy. A reset made from the web crate that
//! deleted rows alone would be undone by the processor's next write, which
//! replaces the whole table from memory. So the web side leaves a request here,
//! and the processor applies it to the tracker before it judges the next
//! detection, then writes the result back.
//!
//! A queue rather than a shared lock on the tracker: the processor takes the
//! tracker by `&mut` on its hot path and nothing else should be able to contend
//! with it. The handle also carries the configuration the processor runs with,
//! because that is set by environment variable and config file, not by the
//! settings table, so the web crate has no other way to say what the feature is
//! doing.

use std::sync::Mutex;

use super::DynamicThresholdConfig;

/// One operator reset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reset {
    /// Forget one species, by scientific name.
    Species(String),
    /// Forget every species.
    All,
}

/// What the web crate may know about, and ask of, the running tracker.
#[derive(Debug)]
pub struct DynamicThresholdHandle {
    config: DynamicThresholdConfig,
    pending: Mutex<Vec<Reset>>,
}

impl DynamicThresholdHandle {
    /// A handle for a tracker running with `config`.
    #[must_use]
    pub const fn new(config: DynamicThresholdConfig) -> Self {
        Self {
            config,
            pending: Mutex::new(Vec::new()),
        }
    }

    /// The configuration the tracker runs with.
    #[must_use]
    pub const fn config(&self) -> DynamicThresholdConfig {
        self.config
    }

    /// Leave a reset for the processor to apply before its next decision.
    pub fn request(&self, reset: Reset) {
        self.lock().push(reset);
    }

    /// Take every reset left since the last call, oldest first.
    #[must_use]
    pub fn drain(&self) -> Vec<Reset> {
        std::mem::take(&mut *self.lock())
    }

    /// A poisoned lock still holds a valid `Vec`; a reset must not be lost to
    /// an unrelated panic.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Reset>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
