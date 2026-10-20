//! Store-and-forward replay for `BirdWeather` uploads (the `outbound_queue`
//! table, migration 19).
//!
//! The detection path parks a payload here when a post fails after its
//! in-flight retries; this drainer replays the backlog once the uplink
//! returns. `BirdWeather` is the one channel where late delivery is correct
//! — an append-only community-science record whose payloads carry their own
//! timestamps — so a station that spent the weekend offline backfills
//! Monday morning instead of losing the data. MQTT and Apprise/email stay
//! fire-and-forget by design (live telemetry / look-now alerts).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use birdnet_integrations::birdweather::{Client, DetectionPost, QUEUE_KIND};
use birdnet_web::state::AppState;

use super::RepeatWarning;

/// Least time between two warnings that the queue depth cannot be read. The
/// cycle runs every [`DRAIN_TICK`], so a database that stays locked would
/// otherwise log a line a minute.
const DEPTH_WARN_EVERY: Duration = Duration::from_secs(3_600);

/// How often the drainer looks for due entries. Listing an empty/not-due
/// queue is one indexed `SELECT`, so a tight-ish cadence costs nothing and
/// rediscovers a returning uplink quickly.
const DRAIN_TICK: Duration = Duration::from_secs(60);

/// Replays attempted per cycle. Bounds the burst when connectivity returns
/// after a long outage so the backlog trickles out instead of slamming the
/// `BirdWeather` API (and a Pi's freshly re-associated Wi-Fi) all at once.
const DRAIN_BATCH: u32 = 25;

/// Pause between replays inside one cycle — same gentleness rationale.
const DRAIN_SPACING: Duration = Duration::from_millis(200);

/// Current Unix time in seconds (0 if the clock is somehow before the epoch).
pub fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Spawn the background drainer. Returns immediately; the task runs for the
/// life of the runtime and exits with it (it holds no resources that need a
/// graceful stop — every cycle's state lives in the database).
pub fn spawn_birdweather_drainer(state: AppState, client: Client) {
    tokio::spawn(async move {
        tracing::info!("BirdWeather store-and-forward drainer started");
        let mut tick = tokio::time::interval(DRAIN_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut depth_warning = RepeatWarning::new(DEPTH_WARN_EVERY);
        loop {
            tick.tick().await;
            drain_cycle(&state, &client, &mut depth_warning).await;
        }
    });
}

/// One replay pass: send due entries oldest-first, stop at the first
/// network failure (the uplink is evidently still down — hammering the rest
/// of the batch would only burn radio and battery), and refresh the
/// queue-depth gauge either way.
async fn drain_cycle(state: &AppState, client: &Client, warning: &mut RepeatWarning) {
    let now = unix_now_secs();
    let queue_state = state.clone();
    let due = tokio::task::spawn_blocking(move || {
        queue_state.with_db(|conn| {
            birdnet_db::outbound_queue::due(conn, QUEUE_KIND, now, DRAIN_BATCH)
                .map_err(|e| e.to_string())
        })
    })
    .await;

    let items = match due {
        Ok(Ok(items)) => items,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "outbound queue read failed");
            Vec::new()
        }
        Err(e) => {
            tracing::warn!(error = %e, "outbound queue read task failed");
            Vec::new()
        }
    };

    let mut replayed = 0_u32;
    for item in items {
        let disposition = match serde_json::from_str::<DetectionPost>(&item.payload) {
            // Parked by a version that uploaded at 0, 0 when a station had no
            // position. Replaying it would put the detection on the public map
            // in the Gulf of Guinea, once a position is set and the client
            // exists again; there is no position to correct it with.
            Ok(post) => match parked_position_problem(&post) {
                Some(why) => Disposition::Poison(why),
                None => match client.post_detection(&post).await {
                    Ok(_) => Disposition::Delivered,
                    // Refused for its content: the identical replay cannot
                    // succeed, and treating it as an outage re-posted it 48
                    // times over two days, ending every cycle it headed
                    // (INT13).
                    Err(e) if e.is_permanent() => Disposition::Poison(e.to_string()),
                    Err(e) => Disposition::Failed(e.to_string()),
                },
            },
            // A payload that no longer deserializes (schema drift across an
            // upgrade) can never succeed: drop it rather than retry forever.
            Err(e) => Disposition::Poison(e.to_string()),
        };

        let stop = apply_disposition(state, &item, disposition).await;
        if stop {
            break;
        }
        replayed += 1;
        tokio::time::sleep(DRAIN_SPACING).await;
    }

    if replayed > 0 {
        tracing::info!(replayed, "BirdWeather backlog replayed");
    }

    // Refresh the gauge after every pass so /metrics and the system page
    // track the backlog through outages and recovery alike.
    //
    // A depth that could not be read is **not** set to zero. This used to
    // `unwrap_or(0)`, so a locked database exported "nothing is waiting" for a
    // backlog that was exactly what an operator would be looking at the gauge
    // for. The series keeps the last depth that was read (and does not appear
    // at all if none ever was), and the failure is logged, throttled.
    let gauge_state = state.clone();
    let depth = tokio::task::spawn_blocking(move || {
        gauge_state.with_db(|conn| {
            birdnet_db::outbound_queue::depth(conn, QUEUE_KIND).map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|depth| depth);
    match depth {
        Ok(depth) => {
            if warning.succeeded() {
                tracing::info!("outbound queue depth can be read again");
            }
            state.metrics().set_outbound_queue_depth(QUEUE_KIND, depth);
        }
        Err(error) => {
            if let Some(passed_over) = warning.failed(std::time::Instant::now()) {
                tracing::warn!(
                    error,
                    passed_over,
                    "outbound queue depth could not be read; the queue-depth metric keeps its \
                     last value"
                );
            }
        }
    }
}

/// Why a parked upload must not be replayed: it was queued with a position
/// `BirdWeather` must not be given (see
/// [`birdnet_integrations::birdweather::check_location`]).
fn parked_position_problem(post: &DetectionPost) -> Option<String> {
    birdnet_integrations::birdweather::check_location(Some(post.lat), Some(post.lon))
        .err()
        .map(|problem| {
            format!("parked with a station position it must not be published at: {problem}")
        })
}

/// Outcome of one replay attempt.
enum Disposition {
    /// Upstream accepted it — remove from the queue.
    Delivered,
    /// Network/API failure — back off and keep it; stop this cycle.
    Failed(String),
    /// Permanently undeliverable (unparseable, or refused for its content) —
    /// remove and warn.
    Poison(String),
}

/// Persist one replay outcome. Returns `true` when the cycle should stop
/// (the uplink is still down).
async fn apply_disposition(
    state: &AppState,
    item: &birdnet_db::outbound_queue::QueuedItem,
    disposition: Disposition,
) -> bool {
    let id = item.id;
    let now = unix_now_secs();
    match disposition {
        Disposition::Delivered => {
            let state = state.clone();
            let _ = tokio::task::spawn_blocking(move || {
                state.with_db(|conn| {
                    if let Err(e) = birdnet_db::outbound_queue::delete(conn, id) {
                        tracing::warn!(error = %e, id, "failed to dequeue replayed payload");
                    }
                });
            })
            .await;
            false
        }
        Disposition::Failed(error) => {
            tracing::debug!(id, error = %error, "BirdWeather replay failed; will retry");
            let state = state.clone();
            let _ = tokio::task::spawn_blocking(move || {
                state.with_db(|conn| {
                    match birdnet_db::outbound_queue::mark_failure(conn, id, &error, now) {
                        Ok(true) => tracing::warn!(
                            id,
                            "BirdWeather payload exhausted its replay attempts; dropped"
                        ),
                        Ok(false) => {}
                        Err(e) => tracing::warn!(error = %e, id, "failed to record replay failure"),
                    }
                });
            })
            .await;
            true
        }
        Disposition::Poison(error) => {
            tracing::warn!(id, error = %error, "undeliverable queued payload dropped");
            let state = state.clone();
            let _ = tokio::task::spawn_blocking(move || {
                state.with_db(|conn| {
                    let _ = birdnet_db::outbound_queue::delete(conn, id);
                });
            })
            .await;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::test_support::test_state;

    /// A stand-in for `app.birdweather.com` on loopback: answers every request
    /// with success and records each body. `std::net` rather than an HTTP
    /// framework, like `tests/store_forward_e2e.rs`'s stub.
    fn spawn_stub() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead as _, BufReader, Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let port = listener.local_addr().expect("stub addr").port();
        let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&bodies);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut length = 0_usize;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0_u8; length];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                if let Ok(mut log) = sink.lock() {
                    log.push(String::from_utf8_lossy(&body).into_owned());
                }
                let json = r#"{"success":true}"#;
                let _ = reader.into_inner().write_all(
                    format!(
                        "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{json}",
                        json.len()
                    )
                    .as_bytes(),
                );
            }
        });
        (port, bodies)
    }

    fn park(state: &AppState, species: &str, lat: f64, lon: f64) {
        let payload = serde_json::json!({
            "timestamp": "2026-06-10T06:00:00Z",
            "common_name": species,
            "scientific_name": species,
            "confidence": 0.9,
            "lat": lat,
            "lon": lon,
        });
        state.with_ingest_db(|conn| {
            birdnet_db::outbound_queue::enqueue(conn, QUEUE_KIND, &payload.to_string(), 0)
                .expect("enqueue");
        });
    }

    fn queued(state: &AppState) -> u64 {
        state.with_db(|conn| birdnet_db::outbound_queue::depth(conn, QUEUE_KIND).expect("depth"))
    }

    /// Uploads parked by a version that posted at 0, 0 for a station with no
    /// position are not replayed once the station has a client again: the
    /// position is a lie and the map is public. A parked upload with a real
    /// position is, in the same cycle — a gate that refused everything would
    /// otherwise pass.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_upload_parked_at_null_island_is_dropped_not_replayed() {
        let state = test_state();
        let (port, bodies) = spawn_stub();
        let client = Client::new("tok", 51.0, -0.1)
            .expect("client")
            .with_base_url(&format!("http://127.0.0.1:{port}/api/v1"));
        park(&state, "Pica pica", 0.0, 0.0);
        park(&state, "Turdus merula", 51.0, -0.1);
        park(&state, "Erithacus rubecula", 200.0, 3.0);
        assert_eq!(queued(&state), 3);

        drain_cycle(&state, &client, &mut RepeatWarning::new(DEPTH_WARN_EVERY)).await;

        let sent = bodies.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "replayed: {sent:?}");
        assert!(sent[0].contains("Turdus merula"), "{}", sent[0]);
        assert_eq!(queued(&state), 0, "the unpublishable ones stay parked");
    }

    /// The exported depth of the `BirdWeather` queue, `None` if the series is
    /// absent.
    fn exported_depth(state: &AppState) -> Option<u64> {
        state
            .metrics()
            .snapshot()
            .outbound_queue
            .into_iter()
            .find(|(kind, _)| kind == QUEUE_KIND)
            .map(|(_, depth)| depth)
    }

    fn idle_client() -> Client {
        // Nothing is due in these tests, so the endpoint is never contacted.
        Client::new("tok", 51.0, -0.1)
            .expect("client")
            .with_base_url("http://127.0.0.1:9/api/v1")
    }

    /// The defect: the depth read was `unwrap_or(0)`, so a database that could
    /// not be read exported "nothing is waiting" — for the backlog the gauge
    /// exists to show. The series keeps the last depth that was read, and is
    /// absent if none ever was.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_queue_depth_that_could_not_be_read_is_not_exported_as_zero() {
        // Never measured, and unreadable: the series must not appear.
        let state = test_state();
        state.with_db(|conn| conn.execute_batch("DROP TABLE outbound_queue;").unwrap());
        let mut warning = RepeatWarning::new(DEPTH_WARN_EVERY);
        drain_cycle(&state, &idle_client(), &mut warning).await;
        assert_eq!(
            exported_depth(&state),
            None,
            "an unmeasured queue was exported as a depth"
        );

        // Measured at five, then unreadable: five is still the last truth.
        let state = test_state();
        state.metrics().set_outbound_queue_depth(QUEUE_KIND, 5);
        state.with_db(|conn| conn.execute_batch("DROP TABLE outbound_queue;").unwrap());
        drain_cycle(&state, &idle_client(), &mut warning).await;
        assert_eq!(
            exported_depth(&state),
            Some(5),
            "a failed read overwrote the last depth that was read"
        );
    }

    /// Counterpart: a queue that is read and is empty exports a real zero, and
    /// a readable backlog is exported as it is. A fix that never set the gauge
    /// would pass the test above.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_queue_that_reads_empty_exports_zero_and_a_backlog_exports_its_depth() {
        let state = test_state();
        let mut warning = RepeatWarning::new(DEPTH_WARN_EVERY);
        drain_cycle(&state, &idle_client(), &mut warning).await;
        assert_eq!(exported_depth(&state), Some(0));

        // Parked, but not due for an hour: stays queued, and is counted.
        state.with_ingest_db(|conn| {
            birdnet_db::outbound_queue::enqueue(conn, QUEUE_KIND, "{}", unix_now_secs() + 3_600)
                .unwrap();
        });
        drain_cycle(&state, &idle_client(), &mut warning).await;
        assert_eq!(exported_depth(&state), Some(1));
    }

    #[test]
    fn a_parked_position_is_judged_as_a_new_one_is() {
        let post = |lat: f64, lon: f64| DetectionPost {
            timestamp: "2026-06-10T06:00:00Z".to_owned(),
            common_name: "x".to_owned(),
            scientific_name: "x".to_owned(),
            confidence: 0.9,
            lat,
            lon,
            soundscape: None,
            algorithm: None,
        };
        assert!(parked_position_problem(&post(0.0, 0.0)).is_some());
        assert!(parked_position_problem(&post(f64::NAN, 1.0)).is_some());
        assert!(parked_position_problem(&post(51.0, -0.1)).is_none());
        assert!(parked_position_problem(&post(0.0, 25.0)).is_none());
    }
}
