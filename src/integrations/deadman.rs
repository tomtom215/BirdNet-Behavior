//! Detection deadman: end-to-end "is the station actually detecting?"
//! watchdog.
//!
//! Every component gauge can be green — process up, sources up, disk fine —
//! while the station detects nothing (clogged mic foam, gain knocked to
//! zero, a model/labels mismatch after a bad update). The only signal that
//! proves the whole audio → capture → inference → insert chain is alive is
//! a recent detection row. This task measures that freshness on a fixed
//! cadence and:
//!
//! * publishes it as the `birdnet_detection_silence_seconds` gauge (the
//!   Prometheus-side alert hook) and to the system page / health surface;
//! * after a configurable quiet threshold (default 24 h; `0` disables),
//!   logs a loud warning and — when Apprise is configured — sends one
//!   notification per silence episode, with a recovery notice when
//!   detections resume. One per episode, because a notifier that re-fires
//!   every poll while the operator is asleep trains them to ignore it.
//!
//! One *delivered* notification per episode. The loud log line happens once,
//! when the state machine says something changed; the push is parked in
//! [`super::announce::Outbox`] and retried at every poll until it lands. Before
//! that split, the episode was latched on the send *attempt*, and a send where
//! every destination had been dropped by the rate limiter returned `Ok(())` —
//! so a deadman that fired during a dawn chorus, when the detection rate limit
//! is exactly what is exhausted, was never announced at all.
//!
//! A silent *night* is normal; the default threshold is sized for "a full
//! day with zero detections", which for a working garden station means
//! something is broken. Stations in genuinely sparse habitats raise
//! `--deadman-hours` (or `DEADMAN_HOURS`) accordingly.

use std::time::Duration;

use birdnet_web::state::AppState;

use super::AppriseHandle;
use super::announce::{Alert, Outbox};

/// How often freshness is re-measured. One indexed `SELECT` per pass.
const POLL_EVERY: Duration = Duration::from_secs(300);

/// Default quiet threshold when neither CLI nor config sets one.
pub const DEFAULT_DEADMAN_HOURS: u32 = 24;

/// What a freshness measurement means for the alert state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Transition {
    /// Crossed the threshold: fire the alert and remember it.
    Alert { silent_hours: u64 },
    /// Detections resumed after an alert: announce recovery, re-arm.
    Recovered,
    /// No state change.
    None,
}

/// Pure alert-state decision: separates the "when do we speak" policy from
/// the I/O so the episode semantics (one alert per silence, recovery
/// notice, unknown-freshness does nothing) are unit-testable.
const fn transition(
    already_alerted: bool,
    silence_secs: Option<u64>,
    threshold_secs: u64,
) -> Transition {
    match silence_secs {
        Some(secs) if secs >= threshold_secs => {
            if already_alerted {
                Transition::None
            } else {
                Transition::Alert {
                    silent_hours: secs / 3600,
                }
            }
        }
        Some(_) if already_alerted => Transition::Recovered,
        // Fresh and un-alerted: nothing to say. `None` freshness (no
        // detections ever / unmeasurable) lands here too — a brand-new
        // station must not alarm on first boot; the gauge stays "unknown"
        // and the operator has the onboarding flow instead.
        _ => Transition::None,
    }
}

/// Spawn the deadman task. `threshold_hours == 0` still spawns it — the
/// freshness gauge is valuable on its own — but never alerts.
pub fn spawn_detection_deadman(
    state: AppState,
    apprise: Option<AppriseHandle>,
    threshold_hours: u32,
) {
    tokio::spawn(async move {
        tracing::info!(
            threshold_hours,
            alerts = threshold_hours > 0,
            "detection deadman started"
        );
        let threshold_secs = u64::from(threshold_hours) * 3600;
        let mut alerted = false;
        // Undelivered alerts, keyed by `()`: the deadman has one episode at a
        // time, so a recovery notice queued while the onset is still stuck
        // replaces it rather than following it.
        let mut outbox: Outbox<()> = Outbox::new();
        let mut tick = tokio::time::interval(POLL_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;

            let db_state = state.clone();
            let silence = tokio::task::spawn_blocking(move || {
                db_state.with_db(|conn| {
                    birdnet_db::sqlite::seconds_since_last_detection(conn).unwrap_or_else(|e| {
                        tracing::debug!(error = %e, "deadman freshness query failed");
                        None
                    })
                })
            })
            .await
            .unwrap_or(None);

            state.metrics().set_detection_silence_secs(silence);

            if threshold_secs == 0 {
                continue;
            }
            match transition(alerted, silence, threshold_secs) {
                Transition::Alert { silent_hours } => {
                    alerted = true;
                    tracing::warn!(
                        silent_hours,
                        threshold_hours,
                        "DETECTION DEADMAN — no detections recorded for over the configured \
                         threshold; check microphone/stream health, gain, and the model"
                    );
                    outbox.queue(
                        (),
                        Alert::new(
                            "Station has gone quiet",
                            format!(
                                "No bird detections for {silent_hours} hours (threshold \
                                 {threshold_hours} h). The process and audio sources may still \
                                 look healthy — check microphone placement/foam, per-source \
                                 gain, and recent log lines on the station."
                            ),
                            birdnet_integrations::apprise::NotifyType::Warning,
                        ),
                    );
                }
                Transition::Recovered => {
                    alerted = false;
                    tracing::info!("detection deadman recovered — detections are flowing again");
                    outbox.queue(
                        (),
                        Alert::new(
                            "Station is detecting again",
                            "Detections have resumed after the quiet period.",
                            birdnet_integrations::apprise::NotifyType::Info,
                        ),
                    );
                }
                Transition::None => {}
            }

            // Anything the state machine raised — this tick or an earlier one
            // that never got out — is offered again here.
            super::announce::flush(&mut outbox, apprise.as_ref(), &state.metrics()).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 3600;

    #[test]
    fn unknown_freshness_never_alerts() {
        assert_eq!(transition(false, None, 24 * HOUR), Transition::None);
        assert_eq!(transition(true, None, 24 * HOUR), Transition::None);
    }

    #[test]
    fn fresh_station_stays_quiet() {
        assert_eq!(transition(false, Some(HOUR), 24 * HOUR), Transition::None);
    }

    #[test]
    fn crossing_threshold_alerts_exactly_once() {
        let first = transition(false, Some(25 * HOUR), 24 * HOUR);
        assert_eq!(first, Transition::Alert { silent_hours: 25 });
        // Still silent on the next poll: no re-fire while the operator sleeps.
        assert_eq!(
            transition(true, Some(26 * HOUR), 24 * HOUR),
            Transition::None
        );
    }

    #[test]
    fn boundary_is_inclusive() {
        // Exactly at the threshold counts as silent — pins `>=` vs `>`.
        assert_eq!(
            transition(false, Some(24 * HOUR), 24 * HOUR),
            Transition::Alert { silent_hours: 24 }
        );
    }

    #[test]
    fn recovery_announces_and_rearms() {
        assert_eq!(
            transition(true, Some(HOUR), 24 * HOUR),
            Transition::Recovered
        );
        // After re-arming, a new silence episode alerts again.
        assert_eq!(
            transition(false, Some(30 * HOUR), 24 * HOUR),
            Transition::Alert { silent_hours: 30 }
        );
    }
}
