//! Operational alerting: the conditions that end a season, other than silence.
//!
//! # Why this exists
//!
//! Before this module the station could notify a human about exactly three
//! things: a species detection, the weekly report, and the detection deadman
//! ("nothing at all has been heard for N hours"). The deadman is the important
//! one and it closes the biggest hole — every component gauge green while the
//! station detects nothing.
//!
//! It does not close the others. Each of these reached the journal and the
//! Prometheus endpoint and nowhere else:
//!
//! * one of several microphones dead while the rest keep working, so the
//!   deadman stays quiet and the station has been recording on two of three
//!   channels for a month;
//! * the disk purger deleting recordings every day to stay under its threshold,
//!   quietly discarding the audio a researcher would want to re-examine;
//! * a failing integrity check or a backup that has not completed in weeks —
//!   the two things standing between a corrupt database and a lost season;
//! * the analytics database quarantined and rebuilt, which empties every
//!   behavioural dashboard until someone notices;
//! * a Pi sitting at thermal-throttle temperature in a sealed enclosure in
//!   July, losing inference throughput and shortening the SD card's life.
//!
//! On a station nobody logs into and no Prometheus scrapes, the journal is a
//! diary written for nobody. **The instrumentation was never the gap — the
//! notifier was wired to one condition.**
//!
//! # Episode semantics
//!
//! Deliberately identical to [`super::deadman`]: one alert when a condition
//! starts, one recovery notice when it ends, and *nothing* in between. A
//! notifier that re-fires every poll while the operator is asleep trains them to
//! ignore it, and an ignored alert is worse than no alert because it is
//! believed to be working.
//!
//! Each condition keeps its own episode state, so a hot afternoon does not
//! suppress a disk alert.

use std::collections::BTreeMap;
use std::time::Duration;

use birdnet_web::state::AppState;

use super::AppriseHandle;

/// How often the conditions are re-measured.
///
/// Five minutes matches the deadman. Every check is either an indexed `SELECT`,
/// a `statvfs`, or a sysfs read, so the cost is negligible next to inference —
/// and none of these conditions changes faster than that.
const POLL_EVERY: Duration = Duration::from_secs(300);

/// Consecutive polls a condition must persist before it is worth waking
/// someone.
///
/// At [`POLL_EVERY`] this is fifteen minutes of *continuous* fault. Everything
/// here is momentarily noisy in normal operation: the capture supervisor
/// reconnects a USB mic that re-enumerated or an RTSP camera that rebooted, disk
/// usage spikes while a clip is extracted and drops again when the purger runs,
/// and a CPU touches its throttle point during a model load. Alerting on a
/// single sample would fill an operator's phone with faults that fixed
/// themselves, which is the fastest way to teach someone to ignore the channel.
///
/// Fifteen minutes is comfortably longer than any of those and far shorter than
/// a dawn chorus.
const REQUIRED_CONSECUTIVE_POLLS: u32 = 3;

/// Minutes of continuous fault before an alert, for use in alert copy.
const DEBOUNCE_MINUTES: u64 = (POLL_EVERY.as_secs() * REQUIRED_CONSECUTIVE_POLLS as u64) / 60;

/// Disk usage at which recordings are being purged to make room.
///
/// Matches the capture layer's own default purge threshold. Above this the
/// station is still working, but it is discarding audio to stay alive — which
/// is exactly the kind of degradation an operator would want to hear about
/// while there is still time to fit a bigger card.
const DISK_ALERT_PERCENT: f64 = 90.0;

/// CPU temperature at which a Raspberry Pi begins throttling.
///
/// The soft limit is 80 °C on Pi 4/5 (hard limit 85 °C). Alerting at the soft
/// limit gives an operator the chance to add a fan or vent before throughput
/// starts dropping.
const THERMAL_ALERT_C: f32 = 80.0;

/// How long since a scheduled maintenance job last completed before it is
/// treated as failing.
///
/// The backup + VACUUM job runs weekly and the integrity check daily, so three
/// weeks is several missed runs — unambiguous rather than a single blip, and
/// still early enough to act on.
const MAINTENANCE_STALE_SECS: i64 = 21 * 24 * 3600;

/// A condition that is either healthy or not, with a human-readable reason.
///
/// The `key` is the episode identity: the same key across polls is the same
/// episode, which is what stops a per-source alert from re-firing under a
/// different name each time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Condition {
    /// Stable identity for episode tracking (e.g. `source:cam1`).
    pub key: String,
    /// Short alert title.
    pub title: String,
    /// What is wrong, and what to do about it.
    pub body: String,
}

/// Decide which conditions start an episode and which end one.
///
/// Pure, so the episode policy is testable without a station. `alerted` is the
/// set of keys currently in an episode; `streak` counts how many consecutive
/// polls each currently-faulty key has been seen for and is updated in place;
/// `now` is what this poll found. Returns `(newly_broken, newly_recovered)`.
///
/// A condition must persist for [`REQUIRED_CONSECUTIVE_POLLS`] before it
/// alerts — see that constant for why a single sample is not enough. Recovery
/// is immediate and keyed on *absence* rather than on a "healthy" signal, so a
/// condition that stops being measurable (a temperature sensor that disappears)
/// closes its episode instead of alerting forever on a reading nobody can take.
fn transitions(
    alerted: &BTreeMap<String, String>,
    streak: &mut BTreeMap<String, u32>,
    now: &[Condition],
) -> (Vec<Condition>, Vec<String>) {
    // Drop streaks for anything that is no longer faulty, so a fault that comes
    // and goes has to earn its alert from scratch rather than accumulating
    // credit across unrelated episodes.
    streak.retain(|k, _| now.iter().any(|c| &c.key == k));

    let mut broken = Vec::new();
    for condition in now {
        let seen = streak.entry(condition.key.clone()).or_insert(0);
        *seen = seen.saturating_add(1);
        if *seen >= REQUIRED_CONSECUTIVE_POLLS && !alerted.contains_key(&condition.key) {
            broken.push(condition.clone());
        }
    }

    let recovered: Vec<String> = alerted
        .keys()
        .filter(|k| !now.iter().any(|c| &c.key == *k))
        .cloned()
        .collect();
    (broken, recovered)
}

/// Everything currently wrong with the station, as of this poll.
///
/// Runs on a blocking thread: it touches `SQLite`, `statvfs` and sysfs.
fn evaluate(state: &AppState) -> Vec<Condition> {
    let mut out = Vec::new();
    check_sources(state, &mut out);
    check_disk(state, &mut out);
    check_thermal(&mut out);
    check_maintenance(state, &mut out);
    out
}

/// Any configured audio source whose gauge has been down.
///
/// Reads the supervisor's own `birdnet_audio_source_up` gauge — the same signal
/// the Station Health page shows — rather than probing the device, so this
/// cannot itself disturb capture.
fn check_sources(state: &AppState, out: &mut Vec<Condition>) {
    let sources = state.with_db(|conn| {
        use birdnet_db::audio_sources::AudioSourceStore;
        AudioSourceStore::list(conn).unwrap_or_default()
    });
    let enabled: Vec<(String, Option<bool>)> = sources
        .iter()
        .filter(|s| s.disabled_at.is_none())
        .map(|s| (s.id.clone(), state.metrics().source_up(&s.id)))
        .collect();
    out.extend(source_conditions(&enabled));
}

/// The source policy, separated from reading the gauges so it can be tested.
///
/// `enabled` is every enabled source paired with its liveness gauge: `Some(true)`
/// up, `Some(false)` down, `None` not yet reported.
fn source_conditions(enabled: &[(String, Option<bool>)]) -> Vec<Condition> {
    // A single-source station that is fully down is the deadman's territory: it
    // will notice, with better wording, once detections stop. Alerting here as
    // well would double-notify one fault, and two notifications for one fault is
    // how an operator learns to mute the channel. This check exists for the case
    // the deadman structurally cannot see — some sources up, some down.
    if enabled.len() < 2 {
        return Vec::new();
    }
    enabled
        .iter()
        .filter(|(_, up)| *up == Some(false))
        .map(|(id, _)| Condition {
            key: format!("source:{id}"),
            title: format!("Audio source down: {id}"),
            body: format!(
                "The audio source '{id}' has been down for over {DEBOUNCE_MINUTES} minutes \
                 while other sources keep recording, so detections are still arriving and \
                 nothing else will report this. Check the cable, the USB port or the \
                 camera's power, then Admin → Audio."
            ),
        })
        .collect()
}

/// Disk usage high enough that recordings are being purged.
fn check_disk(state: &AppState, out: &mut Vec<Condition>) {
    let dir = state
        .db_path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        );
    let Ok(usage) = birdnet_core::audio::capture::disk_usage(&dir) else {
        return;
    };
    out.extend(disk_condition(usage.used_percent()));
}

/// The disk policy, separated from `statvfs` so it can be tested.
fn disk_condition(used: f64) -> Option<Condition> {
    (used >= DISK_ALERT_PERCENT).then(|| Condition {
        key: "disk".to_owned(),
        title: "Disk nearly full — recordings are being deleted".to_owned(),
        body: format!(
            "The data disk is {used:.0}% full, so the station is purging older recordings to \
             keep going. Detections are still being recorded and the database is safe, but \
             the audio behind older detections is being discarded. Free space, fit a larger \
             card, or lower the clip retention in Admin → Recording."
        ),
    })
}

/// A CPU sitting at or above the throttling threshold.
fn check_thermal(out: &mut Vec<Condition>) {
    out.extend(thermal_condition(
        birdnet_web::system_info::cpu_temperature(),
    ));
}

/// The thermal policy, separated from the sysfs read so it can be tested.
///
/// `None` in means no reading was available, which must produce no condition
/// rather than a default — a station whose sensor is missing is not a station
/// that is cool, and it is certainly not one that is hot.
fn thermal_condition(temp: Option<f32>) -> Option<Condition> {
    let temp = temp?;
    (temp >= THERMAL_ALERT_C).then(|| Condition {
        key: "thermal".to_owned(),
        title: format!("Station running hot: {temp:.0} °C"),
        body: format!(
            "CPU temperature is {temp:.0} °C, at or above the throttling threshold of \
             {THERMAL_ALERT_C:.0} °C. The board will slow itself down to stay safe, which \
             costs inference throughput and shortens the life of the storage. In a sealed \
             enclosure, add a vent or a fan."
        ),
    })
}

/// A scheduled maintenance job that has not completed in far too long.
fn check_maintenance(state: &AppState, out: &mut Vec<Condition>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    for (job, label) in [
        (birdnet_db::sqlite::JOB_BACKUP_VACUUM, "backup"),
        (birdnet_db::sqlite::JOB_INTEGRITY_CHECK, "integrity check"),
    ] {
        let last =
            state.with_db(|conn| birdnet_db::sqlite::last_run_unix(conn, job).ok().flatten());
        // `None` means "never run", which on a station that has just been
        // installed is normal and on one that has been up for weeks is not.
        // The maintenance loop records a run on its first tick, so a persistent
        // `None` is itself the signal — but only once the station is old enough
        // for that to mean something, which the `days` arithmetic below gives
        // for free (a fresh install has no stale timestamp to compare).
        out.extend(maintenance_condition(job, label, last, now));
    }
}

/// The maintenance policy, separated from the query so it can be tested.
///
/// `last_run` of `None` means "never recorded", which produces no condition:
/// on a fresh install that is normal, and there is no timestamp to measure
/// staleness against. The maintenance loop records a run on its first tick, so
/// a station that has been up long enough to be stale will have one.
fn maintenance_condition(
    job: &str,
    label: &str,
    last_run: Option<i64>,
    now: i64,
) -> Option<Condition> {
    let age = now.saturating_sub(last_run?);
    (age > MAINTENANCE_STALE_SECS).then(|| {
        let days = age / 86_400;
        Condition {
            key: format!("maintenance:{job}"),
            title: format!("Scheduled {label} has not run for {days} days"),
            body: format!(
                "The station's {label} last completed {days} days ago. This is the job that \
                 stands between a corrupted database and a lost season, so it is worth \
                 looking at the journal for why it is failing: \
                 journalctl -u birdnet-behavior | grep -i {label}"
            ),
        }
    })
}

/// Spawn the station-health notifier.
///
/// `enabled == false` still spawns nothing; the conditions are cheap but there
/// is no reason to evaluate them for a station whose operator has said they do
/// not want them.
pub fn spawn_station_health(state: AppState, apprise: Option<AppriseHandle>, enabled: bool) {
    if !enabled {
        tracing::info!("station-health alerts disabled");
        return;
    }
    tokio::spawn(async move {
        tracing::info!(
            poll_secs = POLL_EVERY.as_secs(),
            "station-health notifier started"
        );
        // key -> title, so a recovery notice can name what recovered.
        let mut alerted: BTreeMap<String, String> = BTreeMap::new();
        // How many consecutive polls each currently-faulty key has been seen
        // for; see REQUIRED_CONSECUTIVE_POLLS.
        let mut streak: BTreeMap<String, u32> = BTreeMap::new();
        let mut tick = tokio::time::interval(POLL_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;

            let probe_state = state.clone();
            let Ok(current) = tokio::task::spawn_blocking(move || evaluate(&probe_state)).await
            else {
                continue;
            };

            let (broken, recovered) = transitions(&alerted, &mut streak, &current);
            for condition in broken {
                tracing::warn!(
                    key = %condition.key,
                    "STATION HEALTH — {}",
                    condition.title
                );
                notify(
                    apprise.as_ref(),
                    &condition.title,
                    &condition.body,
                    birdnet_integrations::apprise::NotifyType::Warning,
                )
                .await;
                alerted.insert(condition.key, condition.title);
            }
            for key in recovered {
                let title = alerted.remove(&key).unwrap_or_else(|| key.clone());
                tracing::info!(key = %key, "station health recovered: {title}");
                notify(
                    apprise.as_ref(),
                    "Station health recovered",
                    &format!("Resolved: {title}"),
                    birdnet_integrations::apprise::NotifyType::Info,
                )
                .await;
            }
        }
    });
}

/// Best-effort Apprise delivery; the WARN log is the guaranteed signal.
async fn notify(
    apprise: Option<&AppriseHandle>,
    title: &str,
    body: &str,
    kind: birdnet_integrations::apprise::NotifyType,
) {
    let Some(handle) = apprise else { return };
    let mut client = handle.lock().await;
    if let Err(e) = client.send_notification(title, body, kind).await {
        tracing::warn!(error = %e, "station-health notification failed to send");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond(key: &str) -> Condition {
        Condition {
            key: key.to_owned(),
            title: format!("t:{key}"),
            body: format!("b:{key}"),
        }
    }

    fn alerted(keys: &[&str]) -> BTreeMap<String, String> {
        keys.iter()
            .map(|k| ((*k).to_owned(), format!("t:{k}")))
            .collect()
    }

    /// The literal `2` in `a_transient_fault_never_alerts` assumes the debounce
    /// is longer than two polls. A compile-time assertion rather than a runtime
    /// one, so shortening the constant fails the build instead of silently
    /// making that test vacuous.
    const _: () = assert!(REQUIRED_CONSECUTIVE_POLLS > 2);

    /// Drive `polls` consecutive identical polls and return what alerted.
    fn after_polls(polls: u32, conditions: &[Condition]) -> Vec<String> {
        let alerted_set = alerted(&[]);
        let mut streak = BTreeMap::new();
        let mut fired = Vec::new();
        for _ in 0..polls {
            let (broken, _) = transitions(&alerted_set, &mut streak, conditions);
            fired.extend(broken.into_iter().map(|c| c.key));
        }
        fired
    }

    #[test]
    fn a_healthy_station_says_nothing() {
        let mut streak = BTreeMap::new();
        let (broken, recovered) = transitions(&alerted(&[]), &mut streak, &[]);
        assert!(broken.is_empty());
        assert!(recovered.is_empty());
    }

    /// A fault that fixes itself within the debounce window never alerts.
    ///
    /// This is the difference between a channel an operator reads and one they
    /// mute: a USB mic re-enumerating, a disk spike during clip extraction, a
    /// CPU touching its throttle point during a model load are all normal and
    /// all self-healing.
    #[test]
    fn a_transient_fault_never_alerts() {
        // Two polls, spelled as a literal rather than as
        // `REQUIRED_CONSECUTIVE_POLLS - 1`. Deriving it from the constant makes
        // the test parameterised by the very thing it is checking: at
        // `REQUIRED_CONSECUTIVE_POLLS = 1` the expression is 0, the loop runs
        // zero times, and the assertion passes for a station with no debounce
        // at all. Observed — this test stayed green through exactly that.
        assert!(
            after_polls(2, &[cond("disk")]).is_empty(),
            "a fault present for two polls must stay silent"
        );
    }

    /// …and a persistent one does, exactly once.
    #[test]
    fn a_persistent_fault_alerts_once_at_the_threshold() {
        // With `alerted` held empty (as `after_polls` does), a missing
        // once-only rule would fire on every poll past the threshold.
        let fired = after_polls(REQUIRED_CONSECUTIVE_POLLS, &[cond("disk")]);
        assert_eq!(
            fired,
            vec!["disk".to_owned()],
            "must alert on exactly the poll that crosses the threshold"
        );
    }

    /// The property that makes an alert worth reading: it fires once.
    #[test]
    fn an_ongoing_condition_does_not_re_fire() {
        let mut streak: BTreeMap<String, u32> =
            [("disk".to_owned(), REQUIRED_CONSECUTIVE_POLLS)].into();
        let (broken, recovered) = transitions(&alerted(&["disk"]), &mut streak, &[cond("disk")]);
        assert!(
            broken.is_empty(),
            "still broken on the next poll must not re-fire — an alert that \
             repeats every five minutes while the operator sleeps is one they \
             learn to ignore"
        );
        assert!(recovered.is_empty());
    }

    #[test]
    fn a_resolved_condition_announces_recovery_exactly_once() {
        let mut streak = BTreeMap::new();
        let (_, recovered) = transitions(&alerted(&["disk"]), &mut streak, &[]);
        assert_eq!(recovered, vec!["disk".to_owned()]);
        let (broken, recovered) = transitions(&alerted(&[]), &mut streak, &[]);
        assert!(broken.is_empty() && recovered.is_empty());
    }

    /// A fault that flickers must restart its streak rather than accumulate
    /// credit across separate episodes.
    ///
    /// Without the `retain`, two isolated one-poll blips a day apart would add
    /// up to an alert about a station that is fine.
    #[test]
    fn a_flickering_fault_restarts_its_streak() {
        let alerted_set = alerted(&[]);
        let mut streak = BTreeMap::new();
        for _ in 0..(REQUIRED_CONSECUTIVE_POLLS - 1) {
            transitions(&alerted_set, &mut streak, &[cond("disk")]);
        }
        // One healthy poll in between.
        transitions(&alerted_set, &mut streak, &[]);
        let (broken, _) = transitions(&alerted_set, &mut streak, &[cond("disk")]);
        assert!(
            broken.is_empty(),
            "the streak must restart after a healthy poll, not resume"
        );
    }

    /// Each condition keeps its own episode, so one fault cannot mask another.
    ///
    /// The counterpart to the once-only rule: a single shared "already alerted"
    /// flag would satisfy every test above and silently swallow a disk alert
    /// raised during a hot afternoon.
    #[test]
    fn conditions_do_not_mask_each_other() {
        let mut streak: BTreeMap<String, u32> = [
            ("thermal".to_owned(), REQUIRED_CONSECUTIVE_POLLS),
            ("disk".to_owned(), REQUIRED_CONSECUTIVE_POLLS - 1),
        ]
        .into();
        let (broken, recovered) = transitions(
            &alerted(&["thermal"]),
            &mut streak,
            &[cond("thermal"), cond("disk")],
        );
        assert_eq!(
            broken.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
            vec!["disk"],
            "a new fault must alert even while another is ongoing"
        );
        assert!(recovered.is_empty(), "the ongoing one has not recovered");
    }

    #[test]
    fn several_sources_down_are_several_episodes() {
        let fired = after_polls(
            REQUIRED_CONSECUTIVE_POLLS,
            &[cond("source:cam1"), cond("source:cam2")],
        );
        assert_eq!(fired.len(), 2, "each source is its own episode");
    }

    /// A condition that stops being measurable closes its episode rather than
    /// alerting forever on a reading nobody can take.
    #[test]
    fn an_unmeasurable_condition_recovers_rather_than_sticking() {
        let mut streak = BTreeMap::new();
        let (_, recovered) = transitions(&alerted(&["thermal"]), &mut streak, &[]);
        assert_eq!(recovered, vec!["thermal".to_owned()]);
    }

    // -----------------------------------------------------------------------
    // The conditions themselves
    // -----------------------------------------------------------------------
    //
    // The episode machinery above decides *when* to speak. These decide *what
    // counts as wrong*, which is where the judgement is, and none of it was
    // covered — the coverage report on the PR is what pointed that out.

    fn up(id: &str) -> (String, Option<bool>) {
        (id.to_owned(), Some(true))
    }
    fn down(id: &str) -> (String, Option<bool>) {
        (id.to_owned(), Some(false))
    }

    /// A single-source station that is down is the deadman's job, not this one.
    ///
    /// This is the rule that keeps one fault from producing two notifications,
    /// and it is exactly the kind of deliberate omission a later edit
    /// "simplifies" away. Two notifications for one fault is how an operator
    /// learns to mute the channel.
    #[test]
    fn a_lone_source_going_down_is_left_to_the_deadman() {
        assert!(source_conditions(&[down("local")]).is_empty());
        assert!(source_conditions(&[]).is_empty());
    }

    /// …but one of several is precisely what the deadman cannot see.
    #[test]
    fn one_of_several_sources_down_is_reported() {
        let got = source_conditions(&[up("cam1"), down("cam2"), up("cam3")]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].key, "source:cam2");
        assert!(got[0].body.contains("other sources keep recording"));
    }

    /// A source that has not reported yet is not a source that is down.
    ///
    /// `None` is the state at startup and in web-only mode. Treating it as down
    /// would alert every station every time it restarts.
    #[test]
    fn an_unreported_source_is_not_treated_as_down() {
        let got = source_conditions(&[up("cam1"), ("cam2".to_owned(), None)]);
        assert!(got.is_empty(), "unknown is not down: {got:?}");
    }

    #[test]
    fn every_source_down_is_reported_separately() {
        let got = source_conditions(&[down("cam1"), down("cam2")]);
        assert_eq!(got.len(), 2, "each source is its own episode");
    }

    /// The disk threshold fires at the boundary, not past it.
    #[test]
    fn disk_alerts_at_the_threshold_and_not_below() {
        assert!(disk_condition(DISK_ALERT_PERCENT - 0.1).is_none());
        assert!(disk_condition(DISK_ALERT_PERCENT).is_some());
        assert!(disk_condition(99.0).is_some());
        // A healthy station must be silent, or the alert means nothing.
        assert!(disk_condition(42.0).is_none());
    }

    /// A missing temperature sensor is not a cool station.
    ///
    /// The `Option` must produce no condition rather than defaulting: several
    /// boards expose no thermal zone at all, and a default either alerts them
    /// forever or hides a real overheat.
    #[test]
    fn thermal_needs_a_reading_and_respects_the_threshold() {
        assert!(thermal_condition(None).is_none());
        assert!(thermal_condition(Some(THERMAL_ALERT_C - 0.1)).is_none());
        assert!(thermal_condition(Some(THERMAL_ALERT_C)).is_some());
        let hot = thermal_condition(Some(85.0)).expect("85 C is hot");
        assert!(
            hot.title.contains("85"),
            "the reading is in the title: {}",
            hot.title
        );
    }

    /// A job that has never run produces no condition.
    ///
    /// On a fresh install "never" is normal and there is no timestamp to
    /// measure staleness against. Alerting here would greet every new operator
    /// with a fault on day one.
    #[test]
    fn maintenance_never_run_is_not_a_fault() {
        assert!(maintenance_condition("backup_vacuum", "backup", None, 1_800_000_000).is_none());
    }

    #[test]
    fn maintenance_alerts_only_once_genuinely_stale() {
        let now = 1_800_000_000_i64;
        // One day old: the backup job runs weekly, so this is healthy.
        assert!(maintenance_condition("j", "backup", Some(now - 86_400), now).is_none());
        // Exactly at the threshold is not yet past it.
        assert!(
            maintenance_condition("j", "backup", Some(now - MAINTENANCE_STALE_SECS), now).is_none()
        );
        let stale = maintenance_condition("j", "backup", Some(now - 30 * 86_400), now)
            .expect("30 days is several missed weekly runs");
        assert!(stale.title.contains("30 days"), "{}", stale.title);
        assert!(stale.body.contains("journalctl"), "the fix is actionable");
    }

    /// A clock that jumped backwards must not manufacture a fault.
    ///
    /// An RTC-less Pi that boots before NTP can briefly report a `now` earlier
    /// than a recorded run. `saturating_sub` makes that zero rather than a
    /// huge unsigned age, which would otherwise alert on a healthy station
    /// every cold boot.
    #[test]
    fn a_backwards_clock_does_not_manufacture_staleness() {
        let now = 1_000_000_i64;
        assert!(maintenance_condition("j", "backup", Some(now + 86_400), now).is_none());
    }

    /// The debounce copy must not outrun the debounce.
    ///
    /// The first draft of this module told operators a source had "been down
    /// for more than 15 minutes" while alerting on the very first poll. The
    /// constant and the sentence now come from the same place.
    #[test]
    fn the_alert_copy_matches_the_actual_debounce() {
        assert_eq!(
            DEBOUNCE_MINUTES,
            (POLL_EVERY.as_secs() * u64::from(REQUIRED_CONSECUTIVE_POLLS)) / 60
        );
        assert_eq!(DEBOUNCE_MINUTES, 15);
    }
}
