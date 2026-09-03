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

use super::MqttHandle;

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
                    session = publish_today(&state, &stats_topic, session).await;
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
                *backoff = RECONNECT_MIN;
                *next_attempt = Instant::now() + RECONNECT_MIN;
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "MQTT keepalive task failed");
                state.metrics().set_mqtt_connected(false);
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

/// Publish today's detection count, on the presence connection.
///
/// Through the session rather than a fresh stateless publish so it is
/// acknowledged: this is the only surface that tells an operator the station
/// is still *doing* something, as opposed to still being powered on, and a
/// `QoS` 0 publish would report success into a broker that had gone away.
async fn publish_today(
    state: &AppState,
    topic: &str,
    session: Option<PresenceSession>,
) -> Option<PresenceSession> {
    let mut open = session?;

    let db_state = state.clone();
    let count = tokio::task::spawn_blocking(move || {
        let date = local_date();
        db_state.with_db(|conn| {
            birdnet_db::sqlite::detection_count_for_date(conn, &date).unwrap_or_else(|e| {
                tracing::debug!(error = %e, "MQTT daily-stats query failed");
                0
            })
        })
    })
    .await
    .unwrap_or(0);

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
}
