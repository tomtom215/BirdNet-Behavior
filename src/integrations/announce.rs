//! Delivery of operational alerts, retried until one actually lands.
//!
//! # Why this is not just a `send`
//!
//! The three alert loops — [`super::deadman`], [`super::station_health`],
//! [`super::acoustic_health`] — all follow the same discipline: one message
//! per episode, one recovery notice, nothing in between. A notifier that
//! re-fired every five minutes while the operator slept would train them to
//! ignore it, which costs more than the alert is worth.
//!
//! That discipline was implemented as a latch set when the alert was *sent*,
//! and `send` reported success for a notification that had reached nobody: the
//! rate limiter and the circuit breaker each drop a destination without
//! failing, and a send where every destination was dropped returned `Ok(())`.
//! So the deadman crossing its threshold during a dawn chorus — when the
//! detection rate limit is exactly what is exhausted — latched its episode on
//! an alert that never left the box, and `transition()` then returned
//! `Transition::None` for as long as the silence lasted. The station was
//! silent, the operator was never told, and the only trace was a `debug!` line
//! the default filter drops.
//!
//! Splitting the two halves fixes it without spamming:
//!
//! * the **log** still happens once per episode, in the loop, where the state
//!   machine decides that something changed;
//! * the **push** is parked here and retried on every subsequent tick until it
//!   is delivered.
//!
//! Keying the outbox by episode also makes supersession free: a recovery
//! notice queued while the onset alert is still undelivered replaces it, so an
//! operator whose uplink was down for an hour is not told about a fault that
//! has already cleared.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use birdnet_db::notifications::NotifStatus;
use birdnet_integrations::apprise::NotifyType;
use birdnet_web::state::AppState;

use super::AppriseHandle;

/// How long an alert may wait before its body says it was held up.
///
/// Below this, the delay is shorter than any of the loops' own poll intervals
/// and saying so would be noise. Above it, an operator reading "no detections
/// for 25 hours" needs to know they are reading it two hours late.
const LATE_AFTER: Duration = Duration::from_secs(600);

/// One operational alert waiting to be delivered.
#[derive(Debug, Clone)]
pub(super) struct Alert {
    /// Notification title.
    title: String,
    /// Notification body, as written when the episode was observed.
    body: String,
    /// Severity.
    kind: NotifyType,
    /// When the episode was observed — not when this attempt is made.
    raised: Instant,
}

impl Alert {
    /// An alert raised now.
    pub(super) fn new(title: impl Into<String>, body: impl Into<String>, kind: NotifyType) -> Self {
        Self::raised_at(title, body, kind, Instant::now())
    }

    /// The same, with the raise time supplied, so the late-delivery note is
    /// testable without waiting ten minutes.
    pub(super) fn raised_at(
        title: impl Into<String>,
        body: impl Into<String>,
        kind: NotifyType,
        raised: Instant,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            kind,
            raised,
        }
    }

    /// The title, for a caller that needs to check what is queued.
    pub(super) fn title(&self) -> &str {
        &self.title
    }

    /// The body to send at `now`.
    ///
    /// An alert that has been retried for a while carries how long it waited,
    /// because everything else in it is written in the present tense and would
    /// otherwise be read as current.
    fn body_at(&self, now: Instant) -> String {
        let waited = now.saturating_duration_since(self.raised);
        if waited < LATE_AFTER {
            return self.body.clone();
        }
        let minutes = waited.as_secs() / 60;
        format!(
            "{}\n\n(Raised {minutes} minutes ago; earlier attempts to send this \
             did not reach a destination.)",
            self.body
        )
    }
}

/// Alerts that have been logged and are waiting to be delivered.
///
/// One entry per episode key: a station-health condition, an audio source, or
/// `()` for the single-episode deadman. Queueing under a key that is already
/// waiting replaces it, which is how a recovery supersedes an undelivered
/// onset.
#[derive(Debug)]
pub(super) struct Outbox<K> {
    /// key → (the alert, how many delivery attempts have failed).
    pending: BTreeMap<K, (Alert, u32)>,
}

impl<K: Ord + Clone> Default for Outbox<K> {
    fn default() -> Self {
        Self {
            pending: BTreeMap::new(),
        }
    }
}

impl<K: Ord + Clone> Outbox<K> {
    /// An empty outbox.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Queue `alert` under `key`, replacing anything still waiting there.
    pub(super) fn queue(&mut self, key: K, alert: Alert) {
        self.pending.insert(key, (alert, 0));
    }

    /// Everything waiting, in key order, with the number of attempts that have
    /// already failed for each.
    pub(super) fn waiting(&self) -> Vec<(K, Alert, u32)> {
        self.pending
            .iter()
            .map(|(k, (a, n))| (k.clone(), a.clone(), *n))
            .collect()
    }

    /// Record the outcome of one delivery attempt.
    ///
    /// A delivered alert leaves the outbox; an undelivered one stays, with its
    /// attempt count raised, and is tried again at the next tick.
    pub(super) fn settle(&mut self, key: &K, delivered: bool) {
        if delivered {
            self.pending.remove(key);
        } else if let Some(entry) = self.pending.get_mut(key) {
            entry.1 = entry.1.saturating_add(1);
        }
    }
}

/// Try to deliver everything waiting, and record what did not go.
///
/// With no notifier configured this delivers everything: the loud log line the
/// loop already wrote *is* the delivery on such a station, and parking alerts
/// for a notifier that will never exist would retry for ever and re-log for
/// ever.
pub(super) async fn flush<K: Ord + Clone + std::fmt::Debug>(
    outbox: &mut Outbox<K>,
    apprise: Option<&AppriseHandle>,
    state: &AppState,
) {
    let metrics = state.metrics();
    let Some(handle) = apprise else {
        outbox.pending.clear();
        return;
    };
    for (key, alert, failed) in outbox.waiting() {
        let now = Instant::now();
        let outcome = handle
            .lock()
            .await
            .send_operational_alert(&alert.title, &alert.body_at(now), alert.kind)
            .await;
        match outcome {
            Ok(()) => {
                if failed > 0 {
                    tracing::info!(
                        episode = ?key,
                        after_attempts = failed + 1,
                        "an alert that had not been delivered has now gone out"
                    );
                }
                record(state, NotifStatus::Sent, alert.title(), None);
                outbox.settle(&key, true);
            }
            Err(e) => {
                metrics.inc_notification_dropped(e.drop_reason());
                // Loud on the first failure, quiet on the retries: the episode
                // itself has already been logged by the loop, and this line
                // repeats every poll for as long as the notifier is down.
                if failed == 0 {
                    tracing::warn!(
                        episode = ?key,
                        alert = %alert.title(),
                        error = %e,
                        "an alert about this station was not delivered; it will \
                         be retried at every poll until it is"
                    );
                    // Once, on the first failure. The retry runs every poll,
                    // so a notifier down for a day would otherwise write ~288
                    // rows for one alert and bury the log it exists to be.
                    record(
                        state,
                        NotifStatus::Queued,
                        alert.title(),
                        Some(&e.to_string()),
                    );
                } else {
                    tracing::debug!(
                        episode = ?key,
                        alert = %alert.title(),
                        failed_attempts = failed + 1,
                        error = %e,
                        "an alert about this station is still undelivered"
                    );
                }
                outbox.settle(&key, false);
            }
        }
    }
}

/// Write one `notification_log` row for an alert about the station.
///
/// # Why this exists
///
/// The notification log contained **every robin and no deadman**: only the
/// detection path called `record_notification`, so an operator who suspected
/// they had missed an alert had no record to consult. The three alerting loops
/// all deliver through [`flush`], so one call site here covers all of them —
/// which is the point of having moved delivery into the outbox.
///
/// The species columns stay `None`. An alert about a failing backup is not
/// about a bird, and filling them with placeholders would make the log's
/// species filter answer wrongly rather than not at all.
///
/// [`NotifStatus::Queued`] rather than `Failed` for an undelivered alert: its
/// own doc comment describes this exact situation — "accepted by this station
/// but not yet delivered", parked for replay — and an operator looking at a
/// wall of red needs "not there yet" to be distinguishable from "lost" before
/// they go and climb a hill.
fn record(state: &AppState, status: NotifStatus, title: &str, error: Option<&str>) {
    let row = birdnet_db::notifications::NotifRecord {
        channel: CHANNEL,
        species_com_name: None,
        species_sci_name: None,
        confidence: None,
        detection_date: None,
        detection_time: None,
        status,
        message: Some(title),
        error,
    };
    if let Err(e) = state.with_db(|conn| birdnet_db::notifications::log_notification(conn, &row)) {
        // Debug, and never a `warn!`: this is the *logging* of a delivery
        // failure, and a second warning about the log would double every
        // genuine one.
        tracing::debug!(error = %e, "could not record an operational alert in the notification log");
    }
}

/// The `notification_log.channel` value every alert about the station carries.
///
/// One value rather than one per loop, so `channel = 'alert'` selects the
/// operational history and `channel = 'apprise'` the bird traffic. Splitting it
/// three ways would mean an operator had to know the loops existed to query
/// them.
pub(super) const CHANNEL: &str = "alert";

#[cfg(test)]
mod tests {
    use super::{Alert, LATE_AFTER, Outbox};
    use birdnet_integrations::apprise::NotifyType;
    use std::time::{Duration, Instant};

    fn alert(body: &str) -> Alert {
        Alert::new("Station has gone quiet", body, NotifyType::Warning)
    }

    #[test]
    fn an_undelivered_alert_stays_and_is_offered_again() {
        // The whole point. Before this, the episode was latched on the attempt
        // and the alert was never offered a second time.
        let mut out: Outbox<()> = Outbox::new();
        out.queue((), alert("no detections for 25 hours"));
        for expected_failures in 0..5 {
            let waiting = out.waiting();
            assert_eq!(waiting.len(), 1, "the alert was dropped after a failure");
            assert_eq!(waiting[0].2, expected_failures);
            out.settle(&(), false);
        }
    }

    #[test]
    fn a_delivered_alert_is_not_offered_again() {
        // The counterpart: an outbox that never cleared would re-send the same
        // alert every five minutes for ever, which is the noise the episode
        // discipline exists to prevent.
        let mut out: Outbox<()> = Outbox::new();
        out.queue((), alert("no detections for 25 hours"));
        out.settle(&(), true);
        assert!(out.waiting().is_empty());
    }

    #[test]
    fn a_recovery_supersedes_an_onset_that_never_went_out() {
        // An operator whose uplink was down for an hour must not be told about
        // a fault that cleared while it was down.
        let mut out: Outbox<String> = Outbox::new();
        out.queue("disk".to_owned(), alert("disk is 92% full"));
        out.settle(&"disk".to_owned(), false);
        out.queue(
            "disk".to_owned(),
            Alert::new(
                "Station health recovered",
                "Resolved: disk",
                NotifyType::Info,
            ),
        );

        let waiting = out.waiting();
        assert_eq!(waiting.len(), 1, "the two must not both be queued");
        assert_eq!(waiting[0].1.title, "Station health recovered");
        assert_eq!(
            waiting[0].2, 0,
            "the replacement starts its own attempt count"
        );
    }

    #[test]
    fn episodes_do_not_supersede_each_other() {
        // The discrimination for the gate above: replacement is per key. A
        // single-slot outbox would satisfy it and lose every condition but the
        // most recent.
        let mut out: Outbox<String> = Outbox::new();
        out.queue("disk".to_owned(), alert("disk is 92% full"));
        out.queue("thermal".to_owned(), alert("SoC is at 84 C"));
        assert_eq!(out.waiting().len(), 2);
        out.settle(&"disk".to_owned(), true);
        let waiting = out.waiting();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].0, "thermal");
    }

    #[test]
    fn settling_a_key_that_is_not_waiting_is_harmless() {
        let mut out: Outbox<String> = Outbox::new();
        out.settle(&"nothing".to_owned(), false);
        out.settle(&"nothing".to_owned(), true);
        assert!(out.waiting().is_empty());
    }

    #[test]
    fn a_promptly_delivered_alert_reads_exactly_as_written() {
        const INSIDE: Duration = Duration::from_secs(60);
        const _: () = assert!(
            LATE_AFTER.as_secs() > INSIDE.as_secs(),
            "the probe time must be inside the window it is probing"
        );
        let raised = Instant::now();
        let a = Alert::raised_at(
            "t",
            "no detections for 25 hours",
            NotifyType::Warning,
            raised,
        );
        assert_eq!(a.body_at(raised + INSIDE), "no detections for 25 hours");
    }

    #[test]
    fn an_alert_held_back_says_how_long_it_waited() {
        // Everything in these bodies is present tense. An operator reading
        // "no detections for 25 hours" two hours late would otherwise take it
        // for the current state.
        let raised = Instant::now();
        let a = Alert::raised_at(
            "t",
            "no detections for 25 hours",
            NotifyType::Warning,
            raised,
        );
        let body = a.body_at(raised + Duration::from_secs(2 * 3600));
        assert!(body.starts_with("no detections for 25 hours"));
        assert!(body.contains("Raised 120 minutes ago"), "{body}");
    }

    // ── the notification log ────────────────────────────────────────────

    use super::{CHANNEL, flush};
    use birdnet_db::notifications::{NotifStatus, recent_notifications};
    use birdnet_web::state::AppState;
    use std::sync::Arc;

    /// A station with an empty notification log.
    fn station() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("birds.db");
        birdnet_db::sqlite::open_or_create(&db).expect("open");
        let state = AppState::new(db).expect("state");
        (dir, state)
    }

    /// `(channel, status, message)` for every logged row.
    ///
    /// `status` is compared as the string the column stores rather than as a
    /// `NotifStatus`, because that is what a query against this table sees —
    /// and the mapping from one to the other is the thing worth pinning.
    fn logged(state: &AppState) -> Vec<(String, String, String)> {
        state
            .with_db(|conn| recent_notifications(conn, 100, 0))
            .expect("read the log")
            .into_iter()
            .map(|r| (r.channel, r.status, r.message.unwrap_or_default()))
            .collect()
    }

    /// A notifier with one native destination pointed at `addr`.
    fn notifier(addr: &str) -> crate::integrations::AppriseHandle {
        let route = birdnet_integrations::dispatch::parse(&format!("json://{addr}/hook"))
            .expect("a json:// route parses");
        let client = birdnet_integrations::apprise::Client::new_cli_only(
            std::path::PathBuf::from("/nonexistent"),
            birdnet_integrations::apprise::NotifyConfig::default(),
        )
        .expect("client")
        .with_native_routes(
            vec![birdnet_integrations::dispatch::Route {
                target: route,
                label: "json".to_owned(),
            }],
            false,
        );
        Arc::new(tokio::sync::Mutex::new(client))
    }

    /// Accept one connection and answer `204`, then stop.
    async fn accepting_stub() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
                    let mut buf = [0_u8; 4096];
                    let _ = sock.read(&mut buf).await;
                    let _ = sock
                        .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                        .await;
                    let _ = sock.flush().await;
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn a_delivered_alert_reaches_the_notification_log() {
        // The defect: the log contained every robin and no deadman, so an
        // operator who suspected they had missed an alert had nothing to
        // consult.
        let (_d, state) = station();
        let apprise = notifier(&accepting_stub().await);
        let mut outbox: Outbox<()> = Outbox::new();
        outbox.queue(
            (),
            Alert::new(
                "Station has gone quiet",
                "no detections",
                NotifyType::Warning,
            ),
        );

        flush(&mut outbox, Some(&apprise), &state).await;

        let rows = logged(&state);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].0, CHANNEL, "one channel for every station alert");
        assert_eq!(rows[0].1, NotifStatus::Sent.to_string());
        assert_eq!(rows[0].2, "Station has gone quiet");
    }

    #[tokio::test]
    async fn an_undelivered_alert_is_logged_as_queued_not_failed() {
        // `Queued`'s own doc comment describes this exactly: accepted by the
        // station, parked for replay. An operator looking at a wall of red
        // needs "not there yet" to be distinguishable from "lost" before they
        // go and climb a hill.
        let (_d, state) = station();
        // Nothing is listening on this port, so every send fails.
        let apprise = notifier("127.0.0.1:1");
        let mut outbox: Outbox<()> = Outbox::new();
        outbox.queue(
            (),
            Alert::new("Disk almost full", "94 %", NotifyType::Warning),
        );

        flush(&mut outbox, Some(&apprise), &state).await;

        let rows = logged(&state);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].1, NotifStatus::Queued.to_string(), "{rows:?}");
        assert_eq!(rows[0].2, "Disk almost full");
    }

    #[tokio::test]
    async fn a_retried_alert_is_logged_once_not_once_per_poll() {
        // The retry runs every five-minute poll. A notifier down for a day
        // would write ~288 rows for one alert and bury the log it exists to
        // be.
        let (_d, state) = station();
        let apprise = notifier("127.0.0.1:1");
        let mut outbox: Outbox<()> = Outbox::new();
        outbox.queue(
            (),
            Alert::new("Disk almost full", "94 %", NotifyType::Warning),
        );

        for _ in 0..4 {
            flush(&mut outbox, Some(&apprise), &state).await;
        }

        let queued = logged(&state)
            .into_iter()
            .filter(|(_, s, _)| *s == NotifStatus::Queued.to_string())
            .count();
        assert_eq!(queued, 1, "four polls, one row: {:?}", logged(&state));
    }

    #[tokio::test]
    async fn no_notifier_configured_writes_nothing() {
        // The discrimination. A station with no notifier has nothing to
        // report about delivery, and a row there would say a send was
        // attempted when none was.
        let (_d, state) = station();
        let mut outbox: Outbox<()> = Outbox::new();
        outbox.queue((), Alert::new("t", "b", NotifyType::Warning));

        flush(&mut outbox, None, &state).await;

        assert!(logged(&state).is_empty(), "{:?}", logged(&state));
    }

    #[tokio::test]
    async fn an_alert_carries_no_species_columns() {
        // An alert about a failing backup is not about a bird. Filling these
        // with placeholders would make the Notification Center's species
        // filter answer wrongly rather than not at all.
        let (_d, state) = station();
        let apprise = notifier(&accepting_stub().await);
        let mut outbox: Outbox<()> = Outbox::new();
        outbox.queue((), Alert::new("Backup failed", "b", NotifyType::Warning));

        flush(&mut outbox, Some(&apprise), &state).await;

        let rows = state
            .with_db(|conn| recent_notifications(conn, 10, 0))
            .expect("read");
        let row = rows.first().expect("a row");
        assert_eq!(row.species_com_name, None);
        assert_eq!(row.species_sci_name, None);
        assert_eq!(row.confidence, None);
    }
}
