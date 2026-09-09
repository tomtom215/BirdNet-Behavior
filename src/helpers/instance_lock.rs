//! One process per data directory (DD-25).
//!
//! A second instance on the same data directory — a `systemctl restart` that
//! overlaps a slow shutdown, an operator starting the binary by hand while
//! the unit is running — opened the same SQLite file, took `DuckDB`'s lock
//! conflict for corruption and quarantined the first process's live analytics
//! store, then died on `AddrInUse`. The damage was done before the bind.
//!
//! An advisory lock on `birdnet.lock` beside the database, taken before
//! anything else opens a file there and held for the life of the process, is
//! what makes the second instance stop first. It waits for the first to go
//! (the shutdown grace), then refuses to start with a message that names the
//! file and the reason.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The lock file's name, beside the database.
pub const LOCK_FILE_NAME: &str = "birdnet.lock";

/// How long to wait for a previous instance to release the directory:
/// the unit's `TimeoutStopSec=30`, so a restart overlapping a slow shutdown
/// resolves inside it. `BNB_INSTANCE_LOCK_GRACE_SECS` overrides it.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(30);

/// Polling interval while waiting.
const RETRY_EVERY: Duration = Duration::from_millis(250);

/// The held lock. Dropping it releases the directory.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    /// Where the lock lives.
    pub path: PathBuf,
}

/// The grace from the environment, else [`DEFAULT_GRACE`].
#[must_use]
pub fn grace_from_env() -> Duration {
    std::env::var("BNB_INSTANCE_LOCK_GRACE_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or(DEFAULT_GRACE, Duration::from_secs)
}

/// Take the data directory's lock, waiting up to `grace` for a previous
/// instance to release it.
///
/// # Errors
///
/// The lock file cannot be created, or another process still holds the lock
/// when the grace runs out. The message is for the journal: it names the
/// file and says another instance is running.
pub fn acquire(data_dir: &Path, grace: Duration) -> Result<InstanceLock, String> {
    let path = data_dir.join(LOCK_FILE_NAME);
    let file = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let deadline = Instant::now() + grace;
    let mut waited = false;
    loop {
        match file.try_lock() {
            Ok(()) => {
                if waited {
                    tracing::info!(path = %path.display(), "previous instance released the data directory");
                }
                return Ok(InstanceLock { _file: file, path });
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "another instance is running on this data directory ({} is locked and was \
                         not released within {}s); stop it first, or set \
                         BNB_INSTANCE_LOCK_GRACE_SECS to wait longer",
                        path.display(),
                        grace.as_secs()
                    ));
                }
                if !waited {
                    tracing::warn!(
                        path = %path.display(),
                        grace_secs = grace.as_secs(),
                        "data directory is locked by a previous instance; waiting for it to stop"
                    );
                    waited = true;
                }
                std::thread::sleep(
                    RETRY_EVERY.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("cannot lock {}: {e}", path.display()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_is_exclusive_within_a_process_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path(), Duration::from_millis(50)).unwrap();
        assert!(first.path.ends_with(LOCK_FILE_NAME));
        // A second handle in the same process: `flock` locks are per open
        // file description, so this is refused like a sibling's would be.
        let started = Instant::now();
        let err = acquire(dir.path(), Duration::from_millis(120)).unwrap_err();
        assert!(
            started.elapsed() >= Duration::from_millis(120),
            "waited the grace"
        );
        assert!(err.contains("another instance is running"), "{err}");
        drop(first);
        assert!(acquire(dir.path(), Duration::from_millis(50)).is_ok());
    }

    #[test]
    fn the_grace_reads_the_environment_variable_shape() {
        // Cannot set the variable here (`set_var` is unsafe on this edition);
        // the default is what a station gets.
        assert_eq!(grace_from_env(), DEFAULT_GRACE);
    }
}
