//! The MQTT presence session: one held-open connection carrying a last will.
//!
//! Home Assistant discovery registers a `binary_sensor` with
//! `device_class: connectivity` on `{prefix}/status`. Nothing published to
//! that topic, and nothing registered a will, so every station with discovery
//! enabled produced a "Station Status" entity that was permanently *unknown* —
//! and the one automation an unattended station exists to support, *notify me
//! when it goes offline*, could never fire.
//!
//! It could not be fixed on the detection path. A will is discarded by the
//! broker when the client sends DISCONNECT (MQTT 3.1.1 §3.14), and DISCONNECT
//! is how every stateless publish ends, so no amount of flag-setting there
//! would produce one. Presence needs a connection whose *unexpected* death is
//! the signal, which is what this loop holds open.
//!
//! ## What the operator sees
//!
//! | Station state | `{prefix}/status` | Published by |
//! |---|---|---|
//! | Running | `online` (retained) | this loop, on connect |
//! | Stopped deliberately | `offline` (retained) | this loop, on shutdown |
//! | Power cut, crash, cable pulled | `offline` (retained) | the **broker**, from the will |
//! | Broker itself down | last retained value | nobody — see below |
//!
//! The last row is the honest limit: a station cannot report on a broker that
//! is not there. `birdnet_mqtt_connected` is the signal for that case, and it
//! is why the gauge exists rather than being inferable from the topic.

use std::time::{Duration, Instant};

use birdnet_integrations::mqtt::{KEEPALIVE_SECS, MqttConfig, PresenceSession};
use birdnet_web::state::AppState;

use super::{MqttHandle, RepeatWarning};

/// How often the session is pinged.
///
/// Half the advertised keepalive, so one lost round trip still leaves time for
/// another before the broker's 1.5-period deadline. Pinging *at* the keepalive
/// interval races the broker's own timer and produces spurious will
/// publications on a link with any latency at all.
const PING_EVERY: Duration = Duration::from_secs(KEEPALIVE_SECS as u64 / 2);

/// How often the "detections today" count is republished.
///
/// The HA sensor is a count, not an event: it needs refreshing on a timer or
/// it shows whatever it last saw. Five minutes matches the station-health and
/// acoustic-health polls, so a station has one background cadence rather than
/// three.
const STATS_EVERY: Duration = Duration::from_secs(300);

/// Least time between two warnings that today's count cannot be read.
///
/// The count is read every [`STATS_EVERY`], so a database that stays locked
/// would otherwise log twelve lines an hour for as long as it lasts.
const STATS_WARN_EVERY: Duration = Duration::from_secs(3_600);

/// First reconnect delay after the session drops.
const RECONNECT_MIN: Duration = Duration::from_secs(15);

/// Longest reconnect delay.
///
/// A broker that has been down for an hour is not coming back in the next
/// fifteen seconds, and a station that keeps trying every fifteen seconds on a
/// metered link spends real money finding that out. Five minutes still
/// rediscovers a returning broker well inside the window an operator would
/// notice.
const RECONNECT_MAX: Duration = Duration::from_secs(300);

/// Suffix distinguishing the presence connection's client identifier.
///
/// This is not cosmetic. A broker that receives a CONNECT carrying a client
/// identifier already in use **must** disconnect the existing session
/// (§3.1.3.1). Sharing one identifier would therefore have every detection
/// publish kick the presence session off the broker, the presence loop
/// reconnect, the next detection kick it off again — a station flapping
/// between `online` and `offline` for as long as birds were singing, which is
/// precisely when an operator is least willing to trust the alert.
const PRESENCE_CLIENT_SUFFIX: &str = "-presence";

/// Derive the presence connection's config from the publish client's.
///
/// Everything is shared except the client identifier; see
/// [`PRESENCE_CLIENT_SUFFIX`].
#[must_use]
pub fn presence_config(base: &MqttConfig) -> MqttConfig {
    MqttConfig {
        client_id: format!("{}{PRESENCE_CLIENT_SUFFIX}", base.client_id),
        ..base.clone()
    }
}

/// The topic the "detections today" HA sensor reads.
fn stats_topic(config: &MqttConfig) -> String {
    format!("{}/stats/today", config.topic_prefix)
}

/// The station's local date, in the form the `detections` table stores.
fn local_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
        + birdnet_db::clock::local_utc_offset_secs();
    let c = birdnet_core::civil::civil_from_unix_secs(secs);
    format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
}

/// The next backoff delay, doubling up to [`RECONNECT_MAX`].
fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(RECONNECT_MAX)
}

/// Spawn the presence loop.
///
/// Does nothing unless MQTT is configured — the caller passes the same handle
/// the detection path uses, and `None` means no broker.
pub fn spawn_mqtt_presence(state: AppState, client: Option<MqttHandle>) {
    let Some(client) = client else { return };
    let config = presence_config(client.config());
    let stats_topic = stats_topic(&config);

    tokio::spawn(async move {
        tracing::info!(
            client_id = %config.client_id,
            status_topic = %config.status_topic(),
            keepalive_secs = KEEPALIVE_SECS,
            "MQTT presence session starting"
        );
        // Not-yet-connected is reported as down rather than left absent: the
        // station has a broker configured, so "we have not managed to reach
        // it" is a real answer and "no data" is not.
        state.metrics().set_mqtt_connected(false);

        let mut shutdown = state.subscribe_shutdown();
        let mut session: Option<PresenceSession> = None;
        let mut backoff = RECONNECT_MIN;
        let mut next_attempt = Instant::now();
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut stats_tick = tokio::time::interval(STATS_EVERY);
        stats_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut stats_warning = RepeatWarning::new(STATS_WARN_EVERY);

        loop {
            tokio::select! {
                res = shutdown.changed() => {
                    // A `watch` whose sender is gone means the runtime is
                    // going down under us; either way this loop is finished.
                    if res.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                _ = ping.tick() => {
                    session = tick_presence(&state, &config, session, &mut backoff, &mut next_attempt).await;
                }
                _ = stats_tick.tick() => {
                    session = publish_today(&state, &stats_topic, session, &mut stats_warning).await;
                }
            }
        }

        // A planned stop says so, rather than leaving Home Assistant to infer
        // it from a keepalive that stops arriving 45 seconds later.
        if let Some(open) = session {
            match tokio::task::spawn_blocking(move || open.shutdown()).await {
                Ok(Ok(())) => tracing::info!("MQTT presence session closed with an offline notice"),
                Ok(Err(e)) => tracing::warn!(error = %e, "MQTT offline notice failed to send"),
                Err(e) => tracing::warn!(error = %e, "MQTT shutdown task failed"),
            }
        }
        state.metrics().set_mqtt_connected(false);
    });
}

/// One keepalive tick: ping an open session, or try to open one.
async fn tick_presence(
    state: &AppState,
    config: &MqttConfig,
    session: Option<PresenceSession>,
    backoff: &mut Duration,
    next_attempt: &mut Instant,
) -> Option<PresenceSession> {
    if let Some(mut open) = session {
        let result = tokio::task::spawn_blocking(move || {
            let r = open.keepalive();
            (open, r)
        })
        .await;
        return match result {
            Ok((open, Ok(()))) => Some(open),
            Ok((_, Err(e))) => {
                // The session is gone, so the broker has already published
                // the will and Home Assistant already shows the station
                // offline. This log explains a transition the operator can
                // see, which is why it is a warning and not a debug line.
                tracing::warn!(error = %e, "MQTT presence session lost; reconnecting");
                state.metrics().set_mqtt_connected(false);
                state.metrics().set_mqtt_error(public_reason(&e));
                *backoff = RECONNECT_MIN;
                *next_attempt = Instant::now() + RECONNECT_MIN;
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "MQTT keepalive task failed");
                state.metrics().set_mqtt_connected(false);
                state
                    .metrics()
                    .set_mqtt_error("the MQTT task failed; see the station log");
                None
            }
        };
    }

    if Instant::now() < *next_attempt {
        return None;
    }
    let cfg = config.clone();
    match tokio::task::spawn_blocking(move || PresenceSession::connect(&cfg)).await {
        Ok(Ok(open)) => {
            tracing::info!("MQTT presence session connected; published online");
            state.metrics().set_mqtt_connected(true);
            *backoff = RECONNECT_MIN;
            Some(open)
        }
        Ok(Err(e)) => {
            // Debug, not warn: a broker that is down stays down, and this
            // fires every fifteen seconds at first. The state is carried by
            // `birdnet_mqtt_connected`, which does not scroll.
            tracing::debug!(error = %e, retry_in_secs = backoff.as_secs(), "MQTT presence connect failed");
            // Down, and since when (DD-22): a broker dead from boot never
            // reached the gauge before — it stayed at "not configured".
            state.metrics().set_mqtt_connected(false);
            state.metrics().set_mqtt_error(public_reason(&e));
            *next_attempt = Instant::now() + *backoff;
            *backoff = next_backoff(*backoff);
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "MQTT presence connect task failed");
            *next_attempt = Instant::now() + *backoff;
            *backoff = next_backoff(*backoff);
            None
        }
    }
}

/// The fixed phrase that stands for an MQTT failure on the public pages.
///
/// The error itself is in the station log (the callers log it); what is stored
/// for Station Health, which `/api/v2/health/conditions` serves to anyone who
/// can reach the station, is only this. A TLS failure names the certificate,
/// key and CA files by path, a connection failure the broker's host and port,
/// and neither belongs on a page an unauthenticated visitor can read.
const fn public_reason(error: &birdnet_integrations::mqtt::MqttError) -> &'static str {
    use birdnet_integrations::mqtt::MqttError;
    match error {
        MqttError::Connection(_) => "the broker could not be reached",
        MqttError::ConnAck(_) => "the broker refused the connection; check the credentials",
        MqttError::Io(_) => "the connection to the broker was lost",
        MqttError::Tls(_) => {
            "the TLS settings could not be used, or the broker's certificate was not accepted; see the station log"
        }
        MqttError::Protocol(_) => "the broker sent a reply the station did not expect",
        MqttError::Encode(_) | MqttError::Serialise(_) => {
            "a message could not be built; see the station log"
        }
        MqttError::NotConfigured => "MQTT is not configured",
    }
}

/// Publish today's detection count, on the presence connection.
///
/// Through the session rather than a fresh stateless publish so it is
/// acknowledged: this is the only surface that tells an operator the station
/// is still *doing* something, as opposed to still being powered on, and a
/// `QoS` 0 publish would report success into a broker that had gone away.
///
/// **A count that could not be read is not published.** The message is
/// *retained*, so `{"count":0}` sent on a failed read would sit on the broker
/// and read, in Home Assistant, as a station that has heard nothing today —
/// from a station that is hearing plenty and cannot reach its own database.
/// Skipping the publish leaves the last real count in place, which is stale
/// and says so by not moving, instead of a zero that looks measured. The
/// session itself is fine and is handed back.
async fn publish_today(
    state: &AppState,
    topic: &str,
    session: Option<PresenceSession>,
    warning: &mut RepeatWarning,
) -> Option<PresenceSession> {
    let mut open = session?;

    let db_state = state.clone();
    let read = tokio::task::spawn_blocking(move || {
        let date = local_date();
        db_state.with_db(|conn| {
            birdnet_db::sqlite::detection_count_for_date(conn, &date).map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|read| read);
    let count = match read {
        Ok(count) => {
            if warning.succeeded() {
                tracing::info!("MQTT daily-stats count can be read again");
            }
            count
        }
        Err(error) => {
            if let Some(passed_over) = warning.failed(Instant::now()) {
                tracing::warn!(
                    error,
                    passed_over,
                    "MQTT daily-stats not published: today's detection count could not be \
                     read (the last published count stays on the broker)"
                );
            }
            return Some(open);
        }
    };

    let payload = format!(r#"{{"count":{count}}}"#);
    let topic = topic.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        let r = open.publish(&topic, payload.as_bytes(), true);
        (open, r)
    })
    .await;

    match result {
        Ok((open, Ok(()))) => Some(open),
        Ok((_, Err(e))) => {
            tracing::warn!(error = %e, "MQTT daily-stats publish failed; reconnecting");
            state.metrics().set_mqtt_connected(false);
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "MQTT daily-stats task failed");
            state.metrics().set_mqtt_connected(false);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presence_session_does_not_share_the_publish_client_id() {
        // §3.1.3.1: a broker receiving a CONNECT whose client identifier is
        // already in use must disconnect the existing session. Sharing one
        // would have every detection publish kick the presence session off,
        // so the station would flap online/offline for as long as birds were
        // singing.
        let base = MqttConfig {
            client_id: "garden-station".to_owned(),
            ..MqttConfig::default()
        };
        let presence = presence_config(&base);
        assert_ne!(presence.client_id, base.client_id);
        assert_eq!(presence.client_id, "garden-station-presence");
    }

    #[test]
    fn the_presence_session_keeps_every_other_setting() {
        // Counterpart: only the identifier may differ. A presence connection
        // that quietly dropped the credentials or the TLS settings would fail
        // to connect on exactly the stations that need it most.
        let base = MqttConfig {
            host: "broker.local".to_owned(),
            port: 8883,
            client_id: "s".to_owned(),
            username: Some("u".to_owned()),
            password: Some("p".to_owned()),
            topic_prefix: "garden".to_owned(),
            tls: Some(birdnet_integrations::mqtt::TlsConfig::default()),
            ..MqttConfig::default()
        };
        let p = presence_config(&base);
        assert_eq!(p.host, base.host);
        assert_eq!(p.port, base.port);
        assert_eq!(p.username, base.username);
        assert_eq!(p.password, base.password);
        assert_eq!(p.topic_prefix, base.topic_prefix);
        assert!(p.tls.is_some());
        assert_eq!(p.status_topic(), "garden/status");
    }

    #[test]
    fn the_ping_interval_leaves_room_inside_the_brokers_deadline() {
        // The broker publishes the will after 1.5 keepalive periods of
        // silence. Pinging at exactly the keepalive interval races that timer
        // and produces spurious offline notices on any link with latency; the
        // gate is that one *missed* ping still leaves time for another.
        let keepalive = Duration::from_secs(u64::from(KEEPALIVE_SECS));
        let deadline = keepalive.mul_f32(1.5);
        assert!(
            PING_EVERY * 2 < deadline,
            "two ping intervals ({:?}) must fit inside the broker's deadline ({deadline:?})",
            PING_EVERY * 2
        );
    }

    #[test]
    fn backoff_grows_and_then_stops() {
        let mut d = RECONNECT_MIN;
        let mut seen = vec![d];
        for _ in 0..10 {
            d = next_backoff(d);
            seen.push(d);
        }
        assert!(
            seen.windows(2).all(|w| w[1] >= w[0]),
            "never shrinks: {seen:?}"
        );
        assert_eq!(d, RECONNECT_MAX, "and is capped: {seen:?}");
    }

    #[test]
    fn the_stats_topic_matches_what_discovery_advertises() {
        // discovery.rs builds `{prefix}/stats/today` for the "Detections
        // Today" sensor. Two independent format! calls agreeing today is not
        // a guarantee they agree tomorrow, which is what this pins.
        let cfg = MqttConfig {
            topic_prefix: "garden".to_owned(),
            ..MqttConfig::default()
        };
        assert_eq!(stats_topic(&cfg), "garden/stats/today");
    }

    // ── the stats publish, against a broker that records what it is sent ──

    /// What the stub broker was sent: `(topic, payload, retain)`.
    type Published = std::sync::Arc<std::sync::Mutex<Vec<(String, String, bool)>>>;

    /// A broker for one connection: answers CONNECT, acknowledges every
    /// `QoS` 1 PUBLISH after recording it, and answers pings.
    fn spawn_stub_broker() -> (u16, Published) {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub broker");
        let port = listener.local_addr().expect("addr").port();
        let seen: Published = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let _ = sock.set_read_timeout(Some(Duration::from_secs(10)));
            // One packet: the fixed header byte and the body it announces.
            let read = |sock: &mut std::net::TcpStream| -> Option<(u8, Vec<u8>)> {
                let mut first = [0_u8; 1];
                sock.read_exact(&mut first).ok()?;
                let (mut remaining, mut scale) = (0_usize, 1_usize);
                loop {
                    let mut b = [0_u8; 1];
                    sock.read_exact(&mut b).ok()?;
                    remaining += usize::from(b[0] & 0x7F) * scale;
                    if b[0] & 0x80 == 0 {
                        break;
                    }
                    scale *= 128;
                }
                let mut body = vec![0_u8; remaining];
                sock.read_exact(&mut body).ok()?;
                Some((first[0], body))
            };
            // CONNECT, then CONNACK (accepted).
            if read(&mut sock).is_none() || sock.write_all(&[0x20, 0x02, 0x00, 0x00]).is_err() {
                return;
            }
            while let Some((header, body)) = read(&mut sock) {
                match header & 0xF0 {
                    0x30 => {
                        let topic_len = usize::from(body[0]) << 8 | usize::from(body[1]);
                        let topic = String::from_utf8_lossy(&body[2..2 + topic_len]).into_owned();
                        let (id, payload_at) = if (header >> 1) & 0b11 > 0 {
                            ([body[2 + topic_len], body[3 + topic_len]], 4 + topic_len)
                        } else {
                            ([0, 0], 2 + topic_len)
                        };
                        let payload = String::from_utf8_lossy(&body[payload_at..]).into_owned();
                        if let Ok(mut log) = sink.lock() {
                            log.push((topic, payload, header & 1 == 1));
                        }
                        if (header >> 1) & 0b11 > 0
                            && sock.write_all(&[0x40, 0x02, id[0], id[1]]).is_err()
                        {
                            return;
                        }
                    }
                    0xC0 => {
                        let _ = sock.write_all(&[0xD0, 0x00]);
                    }
                    _ => return,
                }
            }
        });
        (port, seen)
    }

    /// A presence session connected to a fresh stub broker, and the config it
    /// used (for the stats topic).
    fn connected() -> (PresenceSession, Published, MqttConfig) {
        let (port, seen) = spawn_stub_broker();
        let config = MqttConfig {
            host: "127.0.0.1".to_owned(),
            port,
            topic_prefix: "garden".to_owned(),
            client_id: "station-presence".to_owned(),
            timeout_ms: 5_000,
            ..MqttConfig::default()
        };
        let session = PresenceSession::connect(&config).expect("presence session connects");
        (session, seen, config)
    }

    fn stats_publishes(seen: &Published, config: &MqttConfig) -> Vec<(String, bool)> {
        let topic = stats_topic(config);
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(t, _, _)| *t == topic)
            .map(|(_, payload, retain)| (payload.clone(), *retain))
            .collect()
    }

    /// The defect: a failed read became `0`, and the zero was published
    /// **retained**. Home Assistant then showed "0 detections today" for a
    /// station that was hearing birds and could not reach its own database.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_count_that_could_not_be_read_is_not_published_as_zero() {
        let state = crate::integrations::test_support::test_state();
        let today = local_date();
        state.with_db(|conn| {
            for time in ["06:00:00", "06:00:03"] {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                     VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9)",
                    rusqlite::params![today, time],
                )
                .unwrap();
            }
        });
        let (session, seen, config) = connected();
        let topic = stats_topic(&config);
        let mut warning = RepeatWarning::new(STATS_WARN_EVERY);

        // The count is published while it can be read...
        let session = publish_today(&state, &topic, Some(session), &mut warning).await;
        assert!(session.is_some(), "a healthy publish dropped the session");
        assert_eq!(
            stats_publishes(&seen, &config),
            vec![(r#"{"count":2}"#.to_owned(), true)]
        );

        // ...and nothing is published when it cannot.
        state.with_db(|conn| conn.execute_batch("DROP TABLE detections;").unwrap());
        let session = publish_today(&state, &topic, session, &mut warning).await;
        assert!(
            session.is_some(),
            "the connection is fine; only the count was unreadable"
        );
        assert_eq!(
            stats_publishes(&seen, &config),
            vec![(r#"{"count":2}"#.to_owned(), true)],
            "a count that could not be read was published"
        );
    }

    /// Counterpart: a real zero — a quiet morning — is a measurement, and is
    /// published. A fix that withheld every zero would pass the test above.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_real_zero_is_still_published() {
        let state = crate::integrations::test_support::test_state();
        let (session, seen, config) = connected();
        let topic = stats_topic(&config);
        let mut warning = RepeatWarning::new(STATS_WARN_EVERY);
        let session = publish_today(&state, &topic, Some(session), &mut warning).await;
        assert!(session.is_some());
        assert_eq!(
            stats_publishes(&seen, &config),
            vec![(r#"{"count":0}"#.to_owned(), true)]
        );
    }

    /// #6: the public reason names no file and no broker address. Each variant
    /// is given an error text that does, and none may come out.
    #[test]
    fn the_public_reason_carries_no_path_and_no_address() {
        use birdnet_integrations::mqtt::MqttError;
        let leaky = "/etc/birdnet/secret/client.key: no usable private key; broker 10.1.2.3:8883";
        let errors = [
            MqttError::Connection(leaky.to_owned()),
            MqttError::Io(std::io::Error::other(leaky)),
            MqttError::Encode(leaky.to_owned()),
            MqttError::Protocol(leaky.to_owned()),
            MqttError::Serialise(leaky.to_owned()),
            MqttError::Tls(leaky.to_owned()),
            MqttError::NotConfigured,
        ];
        for e in &errors {
            let phrase = super::public_reason(e);
            assert!(!phrase.is_empty());
            for secret in ["/etc", "client.key", "10.1.2.3", "8883", "birdnet/"] {
                assert!(!phrase.contains(secret), "{e:?} -> {phrase}");
            }
        }
        // The counterpart: the phrases are not all one, so the operator still
        // learns which kind of fault it is.
        let distinct: std::collections::HashSet<_> =
            errors.iter().map(super::public_reason).collect();
        assert!(distinct.len() >= 5, "{distinct:?}");
    }
}
