//! The station's own diagnostics, reachable from a browser (`OP-1`).
//!
//! `--doctor` and `--support-bundle` live in the binary crate: they read the
//! CLI, probe the audio device listing, run the deep integrity check and shell
//! out to `journalctl`. None of that belongs in this crate, and this crate
//! cannot depend on the binary. What it can do is hold two closures the binary
//! hands it at start-up — the same shape as [`crate::notifier::Notifier`] —
//! and route to them. Before this seam existed the entire diagnostic apparatus
//! was reachable only by someone who could already SSH in, on a product whose
//! whole premise is a station nobody can log into.
//!
//! The closures are the binary's, so what they do is the binary's decision:
//! the doctor is run read-only (`--fix` is never implied by a `GET`), and the
//! bundle is the CLI's bundle, redaction included. This module only promises
//! that a process which installed no hooks answers "not available here" rather
//! than pretending.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

type DoctorFn = dyn Fn() -> String + Send + Sync;
type BundleFn = dyn Fn(&Path) -> Result<(), String> + Send + Sync;

/// The two diagnostic entry points the binary exposes to the web layer.
#[derive(Clone)]
pub struct Diagnostics {
    doctor_json: Arc<DoctorFn>,
    support_bundle: Arc<BundleFn>,
}

impl fmt::Debug for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Diagnostics").finish_non_exhaustive()
    }
}

impl Diagnostics {
    /// Wire the two hooks.
    ///
    /// `doctor_json` renders the same JSON document `--doctor-json` prints;
    /// `support_bundle` writes the same archive `--support-bundle` writes, to
    /// the path it is given. Both block, so the routes call them from
    /// `spawn_blocking`.
    pub fn new(
        doctor_json: impl Fn() -> String + Send + Sync + 'static,
        support_bundle: impl Fn(&Path) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            doctor_json: Arc::new(doctor_json),
            support_bundle: Arc::new(support_bundle),
        }
    }

    /// Run every doctor check and return the JSON report.
    #[must_use]
    pub fn doctor_json(&self) -> String {
        (self.doctor_json)()
    }

    /// Write the support bundle to `dest`.
    ///
    /// # Errors
    ///
    /// Whatever the binary's bundle writer reports — it could not stage or
    /// could not archive.
    pub fn write_support_bundle(&self, dest: &Path) -> Result<(), String> {
        (self.support_bundle)(dest)
    }
}
