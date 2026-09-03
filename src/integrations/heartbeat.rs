//! Heartbeat / uptime-monitor ping client construction and its ping loop.
//!
//! # What this signal means, and what the other two mean
//!
//! A station 40 km away has three different ways of being wrong, and this
//! project answers them with three different signals. Conflating any two of
//! them makes both useless:
//!
//! * **"the box is gone"** — power cut, dead SD card, kernel panic, stolen Pi.
//!   Nothing running on the box can report this, because nothing is running.
//!   It can only be detected from outside, as the *absence* of a signal that
//!   should have arrived. That is this module: a fixed-cadence ping to an
//!   external monitor (Healthchecks.io, Uptime Kuma, a cron-monitor), whose
//!   grace period is the alarm.
//! * **"the box is alive but has stopped detecting"** — a dead microphone, a
//!   model that loads and returns nothing, a schedule that never opens.
//!   `integrations::deadman` answers this, and exports
//!   `birdnet_detection_silence_seconds`.
//! * **"the box is alive, detecting, and degrading"** — a filling disk, a
//!   source flapping, a microphone going deaf. `integrations::station_health`
//!   and `integrations::acoustic_health` answer this.
//!
//! # Why the ping is on a timer
//!
//! It used to be sent from inside the per-detection loop
//! (`src/daemon/processor.rs`), after every early `continue` — so a quiet night
//! sent nothing, and the absence of a ping meant "the box is dead **or** no
//! bird sang". Those cannot be told apart, which is fatal for the one signal
//! whose entire job is that distinction. A grace period wide enough not to
//! false-alarm on a December night at 55° N (16 hours of darkness, and longer
//! through a week of storms) is far too wide to notice a dead box; a grace
//! period tight enough to notice a dead box pages the operator every winter
//! night until they mute the channel — the same channel that carries the
//! deadman.
//!
//! So the ping now says one thing and says it on a schedule: *this process is
//! running and its runtime is scheduling work*. Whether it is hearing anything
//! is the deadman's question.

use std::sync::Arc;
use std::time::Duration;

use crate::cli::Cli;

/// Type alias for the heartbeat client handle.
pub type HeartbeatHandle = Arc<birdnet_integrations::heartbeat::HeartbeatClient>;

/// How often to ping the monitor.
///
/// Five minutes, matching the deadman, station-health and acoustic-health
/// loops, so a station has one polling cadence rather than four. It also sets
/// the floor on a useful grace period: a monitor configured below ~15 minutes
/// will false-alarm on one dropped 4G packet, and the manual says so.
const PING_EVERY: Duration = Duration::from_secs(300);

/// Create a heartbeat client from CLI flags and/or config file values.
///
/// Returns `None` if no heartbeat URL is configured.
pub fn create_heartbeat_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<HeartbeatHandle> {
    let url = cli
        .heartbeat_url
        .clone()
        .or_else(|| config?.get("HEARTBEAT_URL").map(String::from))?;

    match birdnet_integrations::heartbeat::HeartbeatClient::new(&url) {
        Ok(client) => {
            // The host, never the whole URL. A Healthchecks.io ping URL is
            // `https://hc-ping.com/<uuid>` and that UUID is a bearer
            // credential: anyone holding it can ping the monitor, which is
            // precisely how you make a dead station look alive, and on that
            // service it also carries a `/fail` sibling that can page the
            // operator at will. It reaches the support bundle through
            // `journal.log`, so it must not be logged in full.
            tracing::info!(monitor = %monitor_host(&url), "heartbeat monitoring enabled");
            Some(Arc::new(client))
        }
        Err(e) => {
            tracing::warn!(error = %e, "heartbeat client not created");
            None
        }
    }
}

/// The `scheme://host` of a ping URL, for logging. Everything after the host is
/// dropped, because that is where these services put the secret.
///
/// Falls back to `"(unparseable)"` rather than to the input: a value that did
/// not look like a URL is exactly the one most likely to be something else
/// pasted into the setting by mistake.
fn monitor_host(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "(unparseable)".to_string();
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    // Strip any userinfo, which is also a credential.
    let host = host.rsplit('@').next().unwrap_or(host);
    if host.is_empty() {
        return "(unparseable)".to_string();
    }
    format!("{scheme}://{host}")
}

/// Ping the monitor every [`PING_EVERY`] for as long as this process lives.
///
/// The first tick fires immediately, so a station that has just come back from
/// a power cut clears its monitor's alarm within seconds rather than within one
/// interval.
///
/// Failures use the same episode semantics as every other alerting loop here:
/// one `warn!` when pinging starts failing, one when it starts working again,
/// and `debug!` in between. A monitor URL that has been 404-ing for eleven
/// months is a silent hole in the only external liveness signal the station
/// has, and it used to log nothing above `debug`.
pub fn spawn_heartbeat(client: HeartbeatHandle) {
    spawn_heartbeat_every(client, PING_EVERY);
}

/// [`spawn_heartbeat`] with the cadence injected.
///
/// Exists so a test can prove the loop *repeats* — the property that
/// distinguishes this from the single per-detection ping it replaced — in
/// milliseconds rather than in five-minute steps.
fn spawn_heartbeat_every(client: HeartbeatHandle, every: Duration) {
    tokio::spawn(async move {
        tracing::info!(every_secs = every.as_secs(), "heartbeat pinger started");
        let mut failing = false;
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match client.ping().await {
                Ok(()) => {
                    if failing {
                        failing = false;
                        tracing::warn!("heartbeat ping succeeded again");
                    }
                }
                Err(e) => {
                    if failing {
                        tracing::debug!(error = %e, "heartbeat ping still failing");
                    } else {
                        failing = true;
                        tracing::warn!(
                            error = %e,
                            "heartbeat ping failed; the external monitor will treat this \
                             station as down until it succeeds"
                        );
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{create_heartbeat_client, monitor_host};
    use crate::integrations::test_support::{config_with, default_cli};

    #[test]
    fn heartbeat_is_none_without_url() {
        let cli = default_cli();
        assert!(create_heartbeat_client(&cli, None).is_none());
    }

    #[test]
    fn heartbeat_built_from_cli_url() {
        let mut cli = default_cli();
        cli.heartbeat_url = Some("https://heartbeat.example/ping".to_owned());
        assert!(create_heartbeat_client(&cli, None).is_some());
    }

    #[test]
    fn heartbeat_built_from_config_url() {
        let cli = default_cli();
        let cfg = config_with(&[("HEARTBEAT_URL", "https://hb.example/ping")]);
        assert!(create_heartbeat_client(&cli, Some(&cfg)).is_some());
    }

    /// The path is the credential on every service this setting is for, so it
    /// must not survive into a log line.
    #[test]
    fn only_the_host_of_a_ping_url_is_loggable() {
        assert_eq!(
            monitor_host("https://hc-ping.com/8f2b1c44-dead-beef-secret-uuid"),
            "https://hc-ping.com"
        );
        assert_eq!(
            monitor_host("https://kuma.example.net:3001/api/push/AbCdEf?status=up"),
            "https://kuma.example.net:3001"
        );
        // Userinfo is a credential too.
        assert_eq!(
            monitor_host("https://user:hunter2@mon.example/ping/xyz"),
            "https://mon.example"
        );
    }

    /// The counterpart: a redactor that returned a constant would pass the
    /// assertions above, and one that returned its input would pass nothing.
    #[test]
    fn the_host_redactor_keeps_the_host_and_drops_the_rest() {
        let secret = "8f2b1c44-dead-beef-secret-uuid";
        let out = monitor_host(&format!("https://hc-ping.com/{secret}"));
        assert!(!out.contains(secret), "the secret must not survive: {out}");
        assert!(out.contains("hc-ping.com"), "the host must survive: {out}");
        assert_ne!(
            monitor_host("https://a.example/x"),
            monitor_host("https://b.example/x"),
            "two different monitors must not log identically"
        );
    }

    #[test]
    fn a_value_that_is_not_a_url_is_not_echoed() {
        // Someone pasting a token into HEARTBEAT_URL by mistake must not have
        // it printed back at them in the journal.
        assert_eq!(monitor_host("hunter2"), "(unparseable)");
        assert_eq!(monitor_host("https://"), "(unparseable)");
    }
}

/// The behaviour that distinguishes a dead-man from a detection notification:
/// the ping arrives because time passed, not because a bird sang.
///
/// These drive the real [`spawn_heartbeat_every`] loop against a loopback
/// server, with no detection pipeline present at all — which is the whole
/// point. Against the previous code there was no loop to drive: the only
/// `ping()` call site in the workspace was inside
/// `src/daemon/processor.rs`'s per-detection branch, after every early
/// `continue`, so a station that heard nothing sent nothing.
#[cfg(test)]
mod ping_loop_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    /// A loopback server that answers `200 OK` and counts what it received.
    async fn counting_stub() -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let hits = Arc::new(AtomicUsize::new(0));
        let sink = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let sink = Arc::clone(&sink);
                tokio::spawn(async move {
                    let mut buf = [0_u8; 1024];
                    // One read is enough: a GET with no body arrives whole.
                    let _ = sock.read(&mut buf).await;
                    sink.fetch_add(1, Ordering::SeqCst);
                    let _ = sock
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                        .await;
                });
            }
        });
        (addr, hits)
    }

    /// Wait until `hits` reaches `want`, or give up after `budget`.
    async fn wait_for(hits: &AtomicUsize, want: usize, budget: Duration) -> usize {
        let deadline = tokio::time::Instant::now() + budget;
        while tokio::time::Instant::now() < deadline {
            if hits.load(Ordering::SeqCst) >= want {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        hits.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn a_ping_arrives_with_no_detection_having_happened() {
        let (addr, hits) = counting_stub().await;
        let client = Arc::new(
            birdnet_integrations::heartbeat::HeartbeatClient::new(&format!("http://{addr}/ping"))
                .expect("client"),
        );

        super::spawn_heartbeat_every(client, Duration::from_millis(50));

        // The first tick fires immediately, so a station coming back from a
        // power cut clears its monitor's alarm in seconds, not in one interval.
        let seen = wait_for(&hits, 1, Duration::from_secs(5)).await;
        assert!(
            seen >= 1,
            "the pinger must ping on its own; no detection occurred in this test"
        );
    }

    #[tokio::test]
    async fn the_ping_repeats_on_its_own_cadence() {
        let (addr, hits) = counting_stub().await;
        let client = Arc::new(
            birdnet_integrations::heartbeat::HeartbeatClient::new(&format!("http://{addr}/ping"))
                .expect("client"),
        );

        super::spawn_heartbeat_every(client, Duration::from_millis(50));

        // Three is the discrimination that matters: a one-shot ping at startup
        // would satisfy the test above and leave the monitor's grace period
        // firing an hour later on a perfectly healthy station.
        let seen = wait_for(&hits, 3, Duration::from_secs(10)).await;
        assert!(
            seen >= 3,
            "expected the pinger to keep pinging; saw {seen} ping(s)"
        );
    }

    /// A monitor that is refusing the ping must not take the station down with
    /// it: the loop keeps trying, so a transient 4G outage self-heals.
    #[tokio::test]
    async fn a_failing_monitor_does_not_stop_the_loop() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let hits = Arc::new(AtomicUsize::new(0));
        let sink = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let sink = Arc::clone(&sink);
                tokio::spawn(async move {
                    let mut buf = [0_u8; 1024];
                    let _ = sock.read(&mut buf).await;
                    sink.fetch_add(1, Ordering::SeqCst);
                    let _ = sock
                        .write_all(
                            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n",
                        )
                        .await;
                });
            }
        });

        let client = Arc::new(
            birdnet_integrations::heartbeat::HeartbeatClient::new(&format!("http://{addr}/ping"))
                .expect("client"),
        );
        super::spawn_heartbeat_every(client, Duration::from_millis(50));

        let seen = wait_for(&hits, 3, Duration::from_secs(10)).await;
        assert!(
            seen >= 3,
            "a 500 from the monitor must not end the loop; saw {seen} attempt(s)"
        );
    }
}
