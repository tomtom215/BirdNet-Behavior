//! Which saved settings the running station has not picked up yet.
//!
//! Most settings are read when the station starts (see
//! [`crate::routes::admin::settings::applies`]), so a value saved a minute ago
//! is stored and not in force. The settings table already says when each row was
//! last written; this remembers when the station finished starting, and the
//! difference is the list of changes still waiting for a restart. Nothing is
//! kept beside the settings: after the restart the answer is empty because
//! every row is older than the new start.
//!
//! A row written and then written back to what it was is still listed — the
//! table remembers that it was written, not what it held — which errs toward
//! offering a restart that was not needed, and never toward hiding one that was.

use std::sync::Mutex;

/// When the station finished starting, in the settings table's own clock.
#[derive(Debug, Default)]
pub struct PendingRestart {
    booted_at: Mutex<Option<String>>,
}

impl PendingRestart {
    /// Record that the station has finished starting. `at` is SQLite's
    /// `datetime('now')` at that moment (UTC, `YYYY-MM-DD HH:MM:SS`), so it
    /// compares as text with a row's `updated_at`.
    pub fn mark_booted(&self, at: String) {
        if let Ok(mut guard) = self.booted_at.lock() {
            *guard = Some(at);
        }
    }

    /// When the station finished starting, if it has said.
    #[must_use]
    pub fn booted_at(&self) -> Option<String> {
        self.booted_at.lock().ok().and_then(|g| g.clone())
    }

    /// The keys, in the order given, whose row was written after the station
    /// started. Empty before [`Self::mark_booted`] has been called (a test, a
    /// tool), when there is no running station to be behind.
    #[must_use]
    pub fn changed_since_boot<'a, I>(&self, rows: I) -> Vec<String>
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let Some(booted) = self.booted_at.lock().ok().and_then(|g| g.clone()) else {
            return Vec::new();
        };
        rows.into_iter()
            .filter(|(_, updated_at)| *updated_at > booted.as_str())
            .map(|(key, _)| key.to_owned())
            .collect()
    }
}

/// The settings written since the station started, by key, in table order.
///
/// # Errors
///
/// The settings table could not be read. A caller that only decorates a page
/// with the answer may go on without it; one that decides something must not
/// read the failure as "nothing is waiting".
pub fn changed_settings(state: &crate::state::AppState) -> Result<Vec<String>, String> {
    let rows = state
        .with_read_db(|conn| birdnet_db::settings::list(conn, None))
        .map_err(|e| e.to_string())?;
    Ok(state
        .pending_restart()
        .changed_since_boot(rows.iter().map(|r| (r.key.as_str(), r.updated_at.as_str()))))
}

/// Whether any audio source was added, changed or removed since the station
/// started. Capture reads its sources once, at start.
///
/// # Errors
///
/// The `audio_sources` table could not be read.
pub fn sources_changed(state: &crate::state::AppState) -> Result<bool, String> {
    let Some(booted) = state.pending_restart().booted_at() else {
        return Ok(false);
    };
    state
        .with_read_db(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM audio_sources WHERE created_at > ?1 OR updated_at > ?1 OR disabled_at > ?1",
                [&booted],
                |r| r.get::<_, i64>(0),
            )
        })
        .map(|n| n > 0)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [(&str, &str); 3] = [
        ("latitude", "2026-10-21 08:00:05"),
        ("longitude", "2026-10-21 07:59:59"),
        ("station_name", "2026-10-21 08:00:00"),
    ];

    /// Strictly after: a row written in the very second the station finished
    /// starting is the seeding that start did, not a change made since.
    #[test]
    fn only_rows_written_after_the_start_are_pending() {
        let p = PendingRestart::default();
        p.mark_booted("2026-10-21 08:00:00".to_owned());
        assert_eq!(p.changed_since_boot(ROWS), ["latitude"]);
    }

    /// Before the start has been recorded there is no running station to be
    /// behind, so nothing is pending.
    #[test]
    fn nothing_is_pending_before_a_start_is_recorded() {
        assert!(
            PendingRestart::default()
                .changed_since_boot(ROWS)
                .is_empty()
        );
    }
}
