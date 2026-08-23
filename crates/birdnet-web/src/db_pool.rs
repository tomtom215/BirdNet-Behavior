//! One writer, several readers — the concurrency WAL was already offering.
//!
//! # What this replaces
//!
//! `AppState` held a single `Mutex<Connection>` and every path in the
//! application took it: each page render, the 30-second health-badge poll on
//! every open browser tab, `/metrics`, the live feed, **and the detection
//! writer**. WAL was enabled the whole time, and a single connection uses none
//! of what WAL is for.
//!
//! The cost was measured on a synthetic three-year station — 3 285 000 rows,
//! the shipped schema and indexes, `ANALYZE` run, `PRAGMA cache_size=-2000` as
//! shipped, on a desktop SSD with a warm page cache:
//!
//! | Surface | Query | Lock held for |
//! |---|---|---|
//! | Reports → History calendar | `detections_per_day` | 1 271 ms |
//! | Life List | `species_first_seen` | 375 ms |
//! | any count through the analytic view | `COUNT(*)` | 132 ms |
//!
//! A detection landing while someone opened the History calendar waited behind
//! it, on the one thread that drains the detection-event channel. On a Pi with
//! an SD card, longer.
//!
//! # The shape, and why it is this shape
//!
//! Reads go to a small pool of **read-only** connections; writes keep the
//! single writer mutex, which is also what SQLite wants — WAL permits exactly
//! one writer, so serialising writes in-process is not a limitation but the
//! rule being honoured where the error message is legible.
//!
//! Read-only is load-bearing rather than decorative. Splitting 252 call sites
//! by hand is how a write ends up on the read path, and a read-only connection
//! turns that mistake into `attempt to write a readonly database` on the first
//! test that touches it, instead of something that works by luck until two
//! requests interleave.
//!
//! # Databases that cannot be opened twice
//!
//! An in-memory database — which is what most of this crate's tests use, via
//! `AppState::from_connection` — has no path to open a second connection to.
//! There the pool is simply absent and reads take the writer, exactly as
//! before. That is a real behavioural difference between test and production,
//! so it is named here rather than left to be discovered: the *concurrency*
//! gates in `tests/db_pool_concurrency.rs` build a file-backed state on purpose.

use std::path::Path;
use std::sync::{Condvar, Mutex, PoisonError};

use rusqlite::Connection;

/// How many read-only connections to keep.
///
/// Small on purpose. Each connection carries its own page cache — 2 MB at the
/// shipped `cache_size=-2000` — so the pool costs about 8 MB against the unit's
/// `MemoryMax=1G`, and a Raspberry Pi 4/5 has four cores to run them on anyway.
/// The number to beat is one; four is enough that a kiosk, a phone and a laptop
/// polling the health badge do not queue behind each other, and beyond that the
/// disk is the limit rather than the pool.
const READER_COUNT: usize = 4;

/// A fixed set of read-only connections, handed out one at a time.
#[derive(Debug)]
pub struct ReaderPool {
    /// Connections not currently in use.
    idle: Mutex<Vec<Connection>>,
    /// Signalled whenever a connection is returned.
    returned: Condvar,
}

/// Returns a borrowed connection to the pool however the caller leaves.
///
/// A `Drop` guard rather than a plain push at the end: the release profile sets
/// `panic = "abort"`, but tests unwind, and a connection lost to one panicking
/// test would shrink the pool for every test after it in the same binary —
/// eventually to nothing, at which point the suite hangs in [`Condvar::wait`]
/// instead of failing. A hang is a much worse test failure than an assertion.
struct Lease<'p> {
    pool: &'p ReaderPool,
    conn: Option<Connection>,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.pool
                .idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(conn);
            self.pool.returned.notify_one();
        }
    }
}

impl ReaderPool {
    /// Open [`READER_COUNT`] read-only connections to `path`.
    ///
    /// Returns `None` when the pool cannot be built — the database is
    /// in-memory, the file does not exist yet, or the platform refused a
    /// read-only handle. The caller then serves reads from the writer, which is
    /// slower but correct; a station that cannot open a second connection
    /// should keep answering pages, not fail to start.
    ///
    /// Opened *after* migration by construction: `AppState` builds this once the
    /// writer has migrated, because a read-only connection cannot create the
    /// `-wal` and `-shm` files a WAL database needs.
    #[must_use]
    pub fn open(path: &Path) -> Option<Self> {
        let mut conns = Vec::with_capacity(READER_COUNT);
        for i in 0..READER_COUNT {
            match birdnet_db::sqlite::open_readonly(path) {
                Ok(conn) => conns.push(conn),
                Err(e) => {
                    // One that fails after others succeeded still leaves a
                    // usable, smaller pool. Zero means no pool at all.
                    tracing::debug!(
                        error = %e,
                        opened = i,
                        path = %path.display(),
                        "could not open a read-only connection"
                    );
                    break;
                }
            }
        }
        if conns.is_empty() {
            return None;
        }
        tracing::info!(
            readers = conns.len(),
            path = %path.display(),
            "read-only connection pool open"
        );
        Some(Self {
            idle: Mutex::new(conns),
            returned: Condvar::new(),
        })
    }

    /// How many connections this pool holds. Test and diagnostic use.
    #[must_use]
    pub fn len(&self) -> usize {
        self.idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether every connection is currently lent out.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Run `f` against a read-only connection, waiting for a free one.
    ///
    /// Waits rather than falling back to the writer when every reader is busy.
    /// Falling back would re-introduce the stall precisely under the load this
    /// exists to survive — a dawn-chorus burst with three browser tabs open is
    /// when the detection writer least wants a page render in front of it.
    ///
    /// # Panics
    ///
    /// Only if `f` panics, which propagates after the connection has been
    /// returned to the pool by the [`Lease`] guard. The pool's own locking
    /// recovers from poisoning rather than unwrapping.
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        let mut idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner);
        let conn = loop {
            if let Some(conn) = idle.pop() {
                break conn;
            }
            idle = self
                .returned
                .wait(idle)
                .unwrap_or_else(PoisonError::into_inner);
        };
        drop(idle);
        let lease = Lease {
            pool: self,
            conn: Some(conn),
        };
        f(lease.conn.as_ref().expect("leased connection is present"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(dir: &Path) -> std::path::PathBuf {
        let db = dir.join("birds.db");
        let conn = birdnet_db::sqlite::open_or_create(&db).expect("open");
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-06-15', '06:00:00', 'Erithacus rubecula', 'European Robin', 0.9)",
            [],
        )
        .expect("seed");
        db
    }

    #[test]
    fn a_file_backed_database_gets_a_pool() {
        let tmp = tempfile::tempdir().unwrap();
        let db = station(tmp.path());
        let pool = ReaderPool::open(&db).expect("pool");
        assert_eq!(pool.len(), READER_COUNT);
        let n: i64 = pool.with(|c| {
            c.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
                .unwrap()
        });
        assert_eq!(n, 1);
    }

    #[test]
    fn a_missing_database_gets_no_pool() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(ReaderPool::open(&tmp.path().join("absent.db")).is_none());
    }

    /// The property that makes the split safe to introduce one call site at a
    /// time: a write on the read path fails immediately and says so, rather than
    /// succeeding on a connection that was never meant to take it.
    #[test]
    fn a_write_through_a_reader_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let db = station(tmp.path());
        let pool = ReaderPool::open(&db).expect("pool");
        let err = pool.with(|c| {
            c.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-06-16', '07:00:00', 'X', 'Y', 0.5)",
                [],
            )
            .expect_err("a reader must refuse a write")
        });
        assert!(
            err.to_string().contains("readonly"),
            "the refusal must name the reason: {err}"
        );
    }

    /// Connections are returned, so the pool does not shrink with use.
    #[test]
    fn a_borrowed_connection_comes_back() {
        let tmp = tempfile::tempdir().unwrap();
        let db = station(tmp.path());
        let pool = ReaderPool::open(&db).expect("pool");
        for _ in 0..(READER_COUNT * 3) {
            pool.with(|c| {
                c.query_row::<i64, _, _>("SELECT 1", [], |r| r.get(0))
                    .unwrap()
            });
            assert_eq!(pool.len(), READER_COUNT, "the pool must not leak readers");
        }
    }

    /// And comes back on a panic, or one bad test would hang every later one on
    /// the `Condvar` instead of failing.
    #[test]
    fn a_panicking_reader_still_returns_its_connection() {
        let tmp = tempfile::tempdir().unwrap();
        let db = station(tmp.path());
        let pool = ReaderPool::open(&db).expect("pool");
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool.with(|_| panic!("boom"));
        }));
        assert!(caught.is_err(), "the panic must propagate");
        assert_eq!(
            pool.len(),
            READER_COUNT,
            "a panic must not cost the pool a connection"
        );
    }

    /// More readers than one, actually concurrent. Four threads each hold a
    /// connection at the same time; if the pool served them one at a time the
    /// barrier would never release and this would hang rather than pass.
    #[test]
    fn readers_run_at_the_same_time() {
        let tmp = tempfile::tempdir().unwrap();
        let db = station(tmp.path());
        let pool = std::sync::Arc::new(ReaderPool::open(&db).expect("pool"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(READER_COUNT));

        std::thread::scope(|s| {
            for _ in 0..READER_COUNT {
                let pool = std::sync::Arc::clone(&pool);
                let barrier = std::sync::Arc::clone(&barrier);
                s.spawn(move || {
                    pool.with(|c| {
                        c.query_row::<i64, _, _>("SELECT 1", [], |r| r.get(0))
                            .unwrap();
                        barrier.wait();
                    });
                });
            }
        });

        assert_eq!(pool.len(), READER_COUNT);
    }
}
