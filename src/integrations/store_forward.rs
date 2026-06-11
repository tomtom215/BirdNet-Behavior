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
        loop {
            tick.tick().await;
            drain_cycle(&state, &client).await;
        }
    });
}

/// One replay pass: send due entries oldest-first, stop at the first
/// network failure (the uplink is evidently still down — hammering the rest
/// of the batch would only burn radio and battery), and refresh the
/// queue-depth gauge either way.
async fn drain_cycle(state: &AppState, client: &Client) {
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
            Ok(post) => match client.post_detection(&post).await {
                Ok(_) => Disposition::Delivered,
                Err(e) => Disposition::Failed(e.to_string()),
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
    let gauge_state = state.clone();
    let depth = tokio::task::spawn_blocking(move || {
        gauge_state.with_db(|conn| birdnet_db::outbound_queue::depth(conn, QUEUE_KIND).unwrap_or(0))
    })
    .await
    .unwrap_or(0);
    state.metrics().set_outbound_queue_depth(QUEUE_KIND, depth);
}

/// Outcome of one replay attempt.
enum Disposition {
    /// Upstream accepted it — remove from the queue.
    Delivered,
    /// Network/API failure — back off and keep it; stop this cycle.
    Failed(String),
    /// Permanently undeliverable (unparseable) — remove and warn.
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
            tracing::warn!(id, error = %error, "unparseable queued payload dropped");
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
