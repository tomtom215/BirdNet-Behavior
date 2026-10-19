//! A burst of reads that scan the whole history cannot take every pooled reader.
//!
//! Moving the feeds, exports and insights off the writer lock put them on the
//! pool, and a pool is four connections. Against the real binary with 300 000
//! detections, sixteen concurrent heavy reads then kept `GET /api/v2/health`,
//! which the container `HEALTHCHECK` polls, waiting 22 s: every reader was busy
//! with a scan. `run_blocking_heavy` (and `cached_fragment`, and the handlers
//! that take `heavy_read_permit`) admit half the pool at a time, so the rest is
//! free for the cheap requests.
//!
//! The scan is stood in for by holding a reader for 600 ms. A cheap read that
//! arrives 100 ms into twelve of them must be answered at once when they went
//! through the gate. The counterpart sends the same twelve through plain
//! `run_blocking`, which has no gate, and shows the cheap read then waits for a
//! reader: the mechanism the gate exists to prevent, so "answered at once"
//! cannot pass by the pool being larger than the test thinks.

use std::time::{Duration, Instant};

use birdnet_web::state::AppState;

const SCAN: Duration = Duration::from_millis(600);

fn busy(state: &AppState) {
    state.with_read_db(|conn| {
        conn.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)).ok();
        std::thread::sleep(SCAN);
    });
}

/// How long a cheap read takes when it arrives 100 ms into twelve scans.
async fn cheap_read_during_twelve_scans(state: &AppState, gated: bool) -> Duration {
    let mut scans = Vec::new();
    for _ in 0..12 {
        let state = state.clone();
        scans.push(tokio::spawn(async move {
            if gated {
                state.run_blocking_heavy(busy).await;
            } else {
                state.run_blocking(busy).await;
            }
        }));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    state
        .run_blocking(|s| {
            s.with_read_db(|conn| conn.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)))
        })
        .await
        .expect("the cheap read");
    let took = started.elapsed();
    for scan in scans {
        scan.await.expect("scan");
    }
    took
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cheap_read_is_not_queued_behind_a_burst_of_heavy_ones() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    assert!(
        state.reader_count() >= 2,
        "precondition: a pool with readers to spare"
    );

    let gated = cheap_read_during_twelve_scans(&state, true).await;
    assert!(
        gated < Duration::from_millis(250),
        "the cheap read waited {gated:?} behind gated heavy reads"
    );

    // Counterpart: no gate, same load. The cheap read has to wait for a reader.
    let ungated = cheap_read_during_twelve_scans(&state, false).await;
    assert!(
        ungated > Duration::from_millis(300),
        "without the gate the cheap read took only {ungated:?}: the test no longer shows the \
         pool filling up, so its other half proves nothing"
    );
}
