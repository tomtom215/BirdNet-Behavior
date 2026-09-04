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
//! One delivered notification per episode was still the wrong side of the
//! trade for a fault that lasts (`OB-16`): nothing re-armed an episode but a
//! process restart, so a station that went quiet in April was announced once
//! and then never mentioned again. An open episode is now re-announced on
//! [`super::reminder`]'s widening schedule — 24 h, 72 h, then weekly — with
//! the *current* silence figure rather than the one it opened with, because
//! "no detections for 25 hours" read four months later is worse than saying
//! nothing.
//!
//! A silent *night* is normal; the default threshold is sized for "a full
//! day with zero detections", which for a working garden station means
//! something is broken. Stations in genuinely sparse habitats raise
//! `--deadman-hours` (or `DEADMAN_HOURS`) accordingly.

use std::time::Duration;

use birdnet_web::state::AppState;

use super::AppriseHandle;
use super::announce::{Alert, Outbox};
use super::reminder::{Reminders, still_broken};

/// How often freshness is re-measured. One indexed `SELECT` per pass.
const POLL_EVERY: Duration = Duration::from_secs(300);

/// Default quiet threshold when neither CLI nor config sets one.
pub const DEFAULT_DEADMAN_HOURS: u32 = 24;

/// What a freshness measurement means for the alert state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Transition {
    /// Crossed the threshold: fire the alert and remember it.
    ///
    /// `ever_detected` is false when the station has never produced a detection
    /// at all, in which case `silent_hours` is how long it has been *listening*
    /// rather than how long since the last detection. The two faults read very
    /// differently to an operator and must not share a sentence.
    Alert {
        silent_hours: u64,
        ever_detected: bool,
    },
    /// Still over the threshold, and already announced.
    ///
    /// Its own variant rather than [`Transition::None`] so the loop can offer
    /// a reminder without re-deriving "is it still broken?" from the threshold
    /// a second time. *When* to remind is a clock question and belongs to
    /// [`super::reminder`]; *whether there is anything to remind about* is this
    /// function's, and was previously indistinguishable from a healthy station.
    StillBroken {
        silent_hours: u64,
        ever_detected: bool,
    },
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
    recorded_secs: Option<u64>,
    threshold_secs: u64,
) -> Transition {
    match silence_secs {
        Some(secs) if secs >= threshold_secs => {
            if already_alerted {
                Transition::StillBroken {
                    silent_hours: secs / 3600,
                    ever_detected: true,
                }
            } else {
                Transition::Alert {
                    silent_hours: secs / 3600,
                    ever_detected: true,
                }
            }
        }
        Some(_) if already_alerted => Transition::Recovered,
        // Fresh and un-alerted: nothing to say.
        Some(_) => Transition::None,
        // No detection has ever been recorded, so there is no "last detection"
        // to measure silence from. That is not the same as healthy, and it used
        // to be treated as such — one arm covering both "brand-new station" and
        // "never worked", with a comment about first boot and no time bound. A
        // station whose microphone, gain, confidence threshold or occurrence
        // filter was wrong on the day it was installed detected nothing on day
        // one and nothing on day three hundred, and the watchdog that exists to
        // prove the audio-to-insert chain is alive stayed silent for the year.
        // The one case where the chain was never alive was the one it exempted.
        //
        // Recording effort is the reference that separates them: how long the
        // station has been *listening*, from `recording_effort`, which the
        // effort recorder credits while capture runs. A new station has little
        // and stays quiet; one that has listened past the threshold and heard
        // nothing is broken. Effort rather than process uptime, which a restart
        // resets, and rather than wall-clock age, which counts the hours the
        // station was switched off.
        //
        // Both queries fail closed to `None`, so a database error cannot
        // manufacture an alert — it lands back on "cannot tell", which is the
        // honest answer.
        None => match recorded_secs {
            Some(recorded) if recorded >= threshold_secs => {
                let silent_hours = recorded / 3600;
                if already_alerted {
                    Transition::StillBroken {
                        silent_hours,
                        ever_detected: false,
                    }
                } else {
                    Transition::Alert {
                        silent_hours,
                        ever_detected: false,
                    }
                }
            }
            _ => Transition::None,
        },
    }
}

/// What the operator is told when the station has gone quiet.
///
/// One function, because the onset alert and every reminder must describe the
/// same fault — and because the figures in it are re-measured at each poll,
/// so a reminder carries what is true now rather than what was true in April.
fn quiet_body(silent_hours: u64, threshold_hours: u32, ever_detected: bool) -> String {
    if ever_detected {
        format!(
            "No bird detections for {silent_hours} hours (threshold {threshold_hours} h). \
             The process and audio sources may still look healthy — check microphone \
             placement/foam, per-source gain, and recent log lines on the station."
        )
    } else {
        // A station that has never detected anything is a different fault from
        // one that has gone quiet, and the operator's first move is different
        // too: this one was never working, so the thing to check is the
        // configuration they entered, not the weather or the microphone foam.
        format!(
            "This station has recorded {silent_hours} hours of audio and has never \
             detected a single bird (threshold {threshold_hours} h). That is a setup \
             fault rather than a quiet season — check the microphone and its channel, \
             the gain, the confidence threshold, the species occurrence filter, and \
             that the model and labels downloaded completely."
        )
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
        // `Some` while an episode is open, carrying its re-notification clock.
        // A bool here is what made "announced once, ever" the whole policy.
        let mut episode: Option<Reminders> = None;
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

            // How long the station has been listening. Only consulted when
            // `silence` is `None` — see `transition` — so this is one extra
            // indexed aggregate per five-minute poll on the path that would
            // otherwise say nothing at all.
            let effort_state = state.clone();
            let recorded = tokio::task::spawn_blocking(move || {
                effort_state.with_db(|conn| {
                    birdnet_db::sqlite::total_recording_seconds(conn).unwrap_or_else(|e| {
                        tracing::debug!(error = %e, "deadman effort query failed");
                        None
                    })
                })
            })
            .await
            .unwrap_or(None);
            let now = std::time::Instant::now();
            match transition(episode.is_some(), silence, recorded, threshold_secs) {
                Transition::Alert {
                    silent_hours,
                    ever_detected,
                } => {
                    episode = Some(Reminders::opened_at(now));
                    tracing::warn!(
                        silent_hours,
                        threshold_hours,
                        "DETECTION DEADMAN — no detections recorded for over the configured \
                         threshold; check microphone/stream health, gain, and the model"
                    );
                    outbox.queue(
                        (),
                        Alert::new(
                            if ever_detected {
                                "Station has gone quiet"
                            } else {
                                "Station has never detected anything"
                            },
                            quiet_body(silent_hours, threshold_hours, ever_detected),
                            birdnet_integrations::apprise::NotifyType::Warning,
                        ),
                    );
                }
                Transition::StillBroken {
                    silent_hours,
                    ever_detected,
                } => {
                    // The episode is open and was announced. Say so again only
                    // when the widening schedule says it is time.
                    if let Some(open_for) = episode.as_mut().and_then(|r| r.due(now)) {
                        tracing::warn!(
                            silent_hours,
                            threshold_hours,
                            "DETECTION DEADMAN — still quiet; re-announcing"
                        );
                        outbox.queue(
                            (),
                            Alert::new(
                                "Station is still quiet",
                                still_broken(
                                    &quiet_body(silent_hours, threshold_hours, ever_detected),
                                    open_for,
                                ),
                                birdnet_integrations::apprise::NotifyType::Warning,
                            ),
                        );
                    }
                }
                Transition::Recovered => {
                    episode = None;
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
            super::announce::flush(&mut outbox, apprise.as_ref(), &state).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 3600;

    /// A station that has never detected anything, and has been listening long
    /// enough that this cannot be explained by the install being new.
    ///
    /// `seconds_since_last_detection` returns `None` for an empty `detections`
    /// table, and the deadman used to fold that into "nothing to say" with a
    /// comment about brand-new stations not alarming on first boot. The comment
    /// was right about the first hour and had no time bound, so a station whose
    /// microphone, gain, confidence threshold or occurrence filter was wrong
    /// from the day it was installed detected nothing on day one and nothing on
    /// day three hundred, and the watchdog whose entire purpose is "prove the
    /// audio to insert chain is alive" stayed silent for the whole year. The
    /// one case where the chain was *never* alive was the one case it exempted.
    #[test]
    fn a_station_that_has_never_detected_alerts_once_it_has_listened_long_enough() {
        assert_eq!(
            transition(false, None, Some(25 * HOUR), 24 * HOUR),
            Transition::Alert {
                silent_hours: 25,
                ever_detected: false
            },
            "25 hours of recording and not one detection is a fault, not a quiet season"
        );
        assert_eq!(
            transition(true, None, Some(30 * HOUR), 24 * HOUR),
            Transition::StillBroken {
                silent_hours: 30,
                ever_detected: false
            },
            "and it stays a fault, with the figure re-measured"
        );
    }

    /// The counterpart, and the reason the fix is not "alert whenever freshness
    /// is unknown". A station installed an hour ago has no detections and no
    /// history, and must not page anyone.
    #[test]
    fn a_new_station_that_has_not_listened_long_enough_stays_quiet() {
        assert_eq!(
            transition(false, None, Some(HOUR), 24 * HOUR),
            Transition::None,
            "one hour into an install is not evidence of anything"
        );
        assert_eq!(
            transition(false, None, Some(0), 24 * HOUR),
            Transition::None
        );
    }

    /// The other half of the counterpart: freshness unknown *and* effort
    /// unknown means the station cannot answer the question, which is not the
    /// same as answering it badly. Both queries fail closed to `None`, so a
    /// database error must not manufacture an alert.
    #[test]
    fn unmeasurable_effort_still_says_nothing() {
        assert_eq!(transition(false, None, None, 24 * HOUR), Transition::None);
        assert_eq!(transition(true, None, None, 24 * HOUR), Transition::None);
    }

    /// A station that has detected before is judged on silence, never on
    /// effort — otherwise a station that has recorded for a year would alarm
    /// permanently the moment it went quiet for an hour.
    #[test]
    fn effort_does_not_override_a_measurable_silence() {
        assert_eq!(
            transition(false, Some(HOUR), Some(10_000 * HOUR), 24 * HOUR),
            Transition::None,
            "a fresh detection wins over any amount of recording history"
        );
        assert_eq!(
            transition(true, Some(HOUR), Some(10_000 * HOUR), 24 * HOUR),
            Transition::Recovered
        );
    }

    /// The two faults must not share a sentence: the operator's first move is
    /// different for each.
    #[test]
    fn the_two_faults_read_differently() {
        let quiet = quiet_body(25, 24, true);
        let never = quiet_body(25, 24, false);
        assert_ne!(quiet, never);
        assert!(
            quiet.contains("No bird detections for 25 hours"),
            "the gone-quiet body is unchanged: {quiet}"
        );
        assert!(
            never.contains("never detected a single bird"),
            "the never-detected body must say so plainly: {never}"
        );
        assert!(
            never.contains("setup fault"),
            "and must point at the configuration, not the weather: {never}"
        );
    }

    /// Renamed from `unknown_freshness_never_alerts`, which stopped being true
    /// and had been the whole defect written down as a passing test: unknown
    /// freshness now alerts when effort says the station has been listening.
    /// What remains true is the narrower claim in the new name.
    #[test]
    fn unknown_freshness_with_unknown_effort_never_alerts() {
        assert_eq!(transition(false, None, None, 24 * HOUR), Transition::None);
        assert_eq!(transition(true, None, None, 24 * HOUR), Transition::None);
    }

    #[test]
    fn fresh_station_stays_quiet() {
        assert_eq!(
            transition(false, Some(HOUR), None, 24 * HOUR),
            Transition::None
        );
    }

    #[test]
    fn crossing_threshold_alerts_once_and_then_reports_still_broken() {
        let first = transition(false, Some(25 * HOUR), None, 24 * HOUR);
        assert_eq!(
            first,
            Transition::Alert {
                silent_hours: 25,
                ever_detected: true
            }
        );
        // Still silent on the next poll. Not `Alert` — no re-fire while the
        // operator sleeps — and not `None` either: the loop needs to know the
        // episode is still open so `reminder::Reminders` can decide whether
        // 24 h have passed. This returned `None` until OB-16, which is why
        // nothing but a restart ever re-armed an episode.
        assert_eq!(
            transition(true, Some(26 * HOUR), None, 24 * HOUR),
            Transition::StillBroken {
                silent_hours: 26,
                ever_detected: true
            }
        );
    }

    #[test]
    fn a_healthy_station_is_not_still_broken() {
        // The counterpart. `StillBroken` must mean "the fault is still there",
        // not "we alerted once"; a version that returned it whenever
        // `already_alerted` was set would remind for ever about a fault that
        // had cleared, and `recovery_announces_and_rearms` would not notice.
        assert_eq!(
            transition(true, Some(HOUR), None, 24 * HOUR),
            Transition::Recovered
        );
        assert_eq!(transition(true, None, None, 24 * HOUR), Transition::None);
    }

    #[test]
    fn boundary_is_inclusive() {
        // Exactly at the threshold counts as silent — pins `>=` vs `>`.
        assert_eq!(
            transition(false, Some(24 * HOUR), None, 24 * HOUR),
            Transition::Alert {
                silent_hours: 24,
                ever_detected: true
            }
        );
    }

    #[test]
    fn recovery_announces_and_rearms() {
        assert_eq!(
            transition(true, Some(HOUR), None, 24 * HOUR),
            Transition::Recovered
        );
        // After re-arming, a new silence episode alerts again.
        assert_eq!(
            transition(false, Some(30 * HOUR), None, 24 * HOUR),
            Transition::Alert {
                silent_hours: 30,
                ever_detected: true
            }
        );
    }
}
