//! A page render must not be able to hold the detection writer up.
//!
//! # What this is defending
//!
//! `AppState` used to hold a single `Mutex<Connection>`, and every path took it
//! — each page, the 30-second health-badge poll on every open browser tab,
//! `/metrics`, the live feed, and the detection writer in
//! `src/daemon/processor.rs`. WAL was enabled throughout and a single connection
//! uses none of it.
//!
//! Measured on a synthetic three-year station (3 285 000 rows, the shipped
//! schema and indexes, `ANALYZE` run, `cache_size=-2000` as shipped): the
//! Reports History calendar held the lock for **1 271 ms** and the Life List for
//! **375 ms**, both of them pure reads, while a detection arriving in that
//! window waited on the one thread that drains the detection-event channel.
//!
//! # Why these tests are structural rather than timed
//!
//! A timing assertion ("the write finished in under N ms") is a flake waiting
//! for a loaded CI runner. Instead each test holds a reader open across a real
//! blocking point and asks whether a write can proceed *at all* while it is
//! held. With the pool that is instant; with one shared connection it cannot
//! happen, and the test reports a timeout rather than hanging.
//!
//! These build a **file-backed** state deliberately. `AppState::from_connection`
//! with `:memory:` — what most of this crate's tests use — has no path to open a
//! second connection to, so there is no pool and reads take the writer. That is
//! the documented fallback, and `reads_still_work_without_a_pool` pins it.

use std::sync::mpsc;
use std::time::Duration;

use birdnet_web::state::AppState;

/// Long enough that a loaded runner is not mistaken for a deadlock, short
/// enough that a real regression fails the suite rather than stalling it.
const PATIENCE: Duration = Duration::from_secs(10);

fn file_backed_station(dir: &std::path::Path) -> AppState {
    let db = dir.join("birds.db");
    let state = AppState::new(db).expect("open state");
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-06-15', '06:00:00', 'Erithacus rubecula', 'European Robin', 0.9)",
            [],
        )
        .expect("seed");
    });
    state
}

fn detection_count(state: &AppState) -> i64 {
    state.with_db(|conn| {
        conn.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .expect("count")
    })
}

/// A file-backed station gets readers; the fixture for everything below.
#[test]
fn a_file_backed_station_has_a_reader_pool() {
    let tmp = tempfile::tempdir().unwrap();
    let state = file_backed_station(tmp.path());
    assert!(
        state.reader_count() > 0,
        "a file-backed database must get a reader pool, or the rest of this \
         file proves nothing"
    );
}

/// THE regression, stated as the thing an operator would notice.
///
/// Observed failing with `with_read_db` routed to `with_db` — the 0.14.0
/// behaviour, one shared connection:
///
/// ```text
/// a detection could not be written while a page was reading — this is the
/// single-connection stall
/// ```
///
/// The reader here is held open across a channel receive, which is what a slow
/// aggregate query is from the writer's point of view: the lock is taken and
/// not coming back for a while.
#[test]
fn a_detection_can_be_written_while_a_page_is_reading() {
    let tmp = tempfile::tempdir().unwrap();
    let state = file_backed_station(tmp.path());

    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (reading_tx, reading_rx) = mpsc::channel::<()>();
    let (written_tx, written_rx) = mpsc::channel::<()>();

    let reader_state = state.clone();
    let reader = std::thread::spawn(move || {
        reader_state.with_read_db(|conn| {
            // A real read, so this is a genuinely open connection and not just
            // a sleeping thread.
            let _: i64 = conn
                .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
                .expect("read");
            reading_tx.send(()).expect("signal");
            // Hold it, the way a multi-second aggregate holds it.
            release_rx.recv().expect("release");
        });
    });

    reading_rx.recv_timeout(PATIENCE).expect("reader started");

    let writer_state = state.clone();
    let writer = std::thread::spawn(move || {
        writer_state.with_db(|conn| {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-06-15', '06:00:01', 'Turdus merula', 'Eurasian Blackbird', 0.8)",
                [],
            )
            .expect("insert");
        });
        written_tx.send(()).expect("signal");
    });

    let wrote = written_rx.recv_timeout(PATIENCE);

    // Let the reader go before asserting, so a failure reports the assertion
    // rather than hanging the harness on a joined thread.
    release_tx.send(()).expect("release");
    reader.join().expect("reader");
    writer.join().expect("writer");

    assert!(
        wrote.is_ok(),
        "a detection could not be written while a page was reading — this is \
         the single-connection stall"
    );
    assert_eq!(detection_count(&state), 2, "the write actually landed");
}

/// The counterpart, so the gate above is a discrimination rather than proof
/// that two threads exist: with the *writer* held, a second writer must still
/// wait. WAL allows one writer, and this pins that the split did not quietly
/// turn write serialisation off as well.
#[test]
fn a_second_writer_still_waits_for_the_first() {
    let tmp = tempfile::tempdir().unwrap();
    let state = file_backed_station(tmp.path());

    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (holding_tx, holding_rx) = mpsc::channel::<()>();
    let (second_tx, second_rx) = mpsc::channel::<()>();

    let first = {
        let state = state.clone();
        std::thread::spawn(move || {
            state.with_db(|_| {
                holding_tx.send(()).expect("signal");
                release_rx.recv().expect("release");
            });
        })
    };
    holding_rx.recv_timeout(PATIENCE).expect("first writer in");

    let second = std::thread::spawn(move || {
        state.with_db(|_| {});
        second_tx.send(()).expect("signal");
    });

    let got_in_early = second_rx.recv_timeout(Duration::from_millis(250));
    release_tx.send(()).expect("release");
    first.join().expect("first");
    second.join().expect("second");

    assert!(
        got_in_early.is_err(),
        "two writers ran at once — WAL permits one, and serialising them here \
         is what keeps the error legible instead of SQLITE_BUSY"
    );
}

/// Several pages at once, which is the ordinary case for a station with a kiosk
/// display, a phone and a laptop all polling the health badge. If reads were
/// still serialised the barrier would never release and this would time out.
#[test]
fn several_pages_read_at_the_same_time() {
    let tmp = tempfile::tempdir().unwrap();
    let state = file_backed_station(tmp.path());
    let readers = state.reader_count();
    assert!(readers >= 2, "need at least two readers to prove anything");

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(readers));
    let (done_tx, done_rx) = mpsc::channel::<()>();

    let handles: Vec<_> = (0..readers)
        .map(|_| {
            let state = state.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            let done_tx = done_tx.clone();
            std::thread::spawn(move || {
                state.with_read_db(|conn| {
                    let _: i64 = conn
                        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
                        .expect("read");
                    barrier.wait();
                });
                done_tx.send(()).expect("signal");
            })
        })
        .collect();
    drop(done_tx);

    for _ in 0..readers {
        done_rx
            .recv_timeout(PATIENCE)
            .expect("every reader must get in before any of them leaves");
    }
    for h in handles {
        h.join().expect("reader");
    }
}

/// A read routed to the pool must return the same answer as one routed to the
/// writer, including rows the writer committed a moment ago. WAL readers see
/// committed data; this pins that the pool is not serving a stale snapshot.
#[test]
fn a_pooled_read_sees_what_the_writer_just_committed() {
    let tmp = tempfile::tempdir().unwrap();
    let state = file_backed_station(tmp.path());

    for i in 0..5 {
        state.with_db(|conn| {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-06-15', ?1, 'Parus major', 'Great Tit', 0.7)",
                rusqlite::params![format!("08:0{i}:00")],
            )
            .expect("insert");
        });
        let via_reader: i64 = state.with_read_db(|conn| {
            conn.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
                .expect("count")
        });
        assert_eq!(
            via_reader,
            i64::from(i) + 2,
            "the pooled reader is behind the writer"
        );
    }
}

/// The in-memory fallback, named rather than left to be discovered: no pool, and
/// `with_read_db` still runs the closure by taking the writer.
#[test]
fn reads_still_work_without_a_pool() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));

    assert_eq!(
        state.reader_count(),
        0,
        "an in-memory database cannot be opened twice, so there is no pool"
    );
    let n: i64 = state.with_read_db(|conn| {
        conn.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .expect("count")
    });
    assert_eq!(n, 0, "the closure must still run, against the writer");
}
