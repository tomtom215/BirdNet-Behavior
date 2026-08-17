//! How long the station actually listened.
//!
//! # Why a detection count is not an abundance
//!
//! A count of detections is a numerator over a denominator nobody was
//! recording. The denominator moves constantly and for reasons that have
//! nothing to do with birds:
//!
//! * a solar recording window is six hours longer in June than in December, so
//!   a species detected at the same rate all year appears to double;
//! * a week of downtime for a card swap removes a week of listening, and the
//!   dip reads as a departure;
//! * a microphone that fails in March halves the channels, and every species
//!   "declines" until it is replaced.
//!
//! Comparing raw counts across seasons or years — the whole reason to run a
//! station for years — therefore measures the station at least as much as the
//! birds. Dividing by listening effort is the elementary, standard correction;
//! what was missing was anywhere to put the effort.
//!
//! # How effort is measured here
//!
//! By sampling, not by integration. Every [`POLL_EVERY`] this task asks the
//! capture supervisor's own per-source gauge — the same
//! `birdnet_audio_source_up` the Station Health page reads — which sources are
//! currently up, and credits each of them with that interval.
//!
//! Sampling rather than instrumenting the capture threads directly is a
//! deliberate trade. It is accurate to ±[`POLL_EVERY`] per source per day
//! (five minutes in ~1440, or 0.3 %), it cannot itself disturb capture, it
//! survives a crash without losing more than one interval, and it needs no
//! coordination with the audio path. An exact integration would be more precise
//! than the question deserves: nobody is comparing abundances at a resolution
//! where five minutes a day matters, and the alternative — trusting a counter
//! that a hard power cut can leave mid-update — is less trustworthy, not more.
//!
//! Effort is credited to the **local** date, because that is the date the
//! detections it will be divided by are stamped with.

use std::time::Duration;

use birdnet_web::state::AppState;

/// How often source liveness is sampled, and the credit given per sample.
///
/// Matches the station-health poll. Five minutes bounds the error at 0.3 % of a
/// day per source while keeping the write rate to a handful of `UPSERT`s an
/// hour — negligible on an SD card whose wear budget is the reason the raw
/// audio lives on tmpfs.
const POLL_EVERY: Duration = Duration::from_secs(300);

/// Credit `seconds` of listening to `source` on the station's local date.
///
/// Additive `UPSERT`: a restart mid-day resumes the same row rather than
/// starting a second one, and a day that spans a restart still totals correctly.
fn credit(conn: &rusqlite::Connection, source: &str, seconds: f64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO recording_effort (date, source, seconds)
         VALUES (date('now','localtime'), ?1, ?2)
         ON CONFLICT(date, source) DO UPDATE SET seconds = seconds + excluded.seconds",
        rusqlite::params![source, seconds],
    )?;
    Ok(())
}

/// Record one sample: credit every source the supervisor reports as up.
///
/// Returns how many sources were credited, for the log line and for tests.
fn sample_once(state: &AppState, seconds: f64) -> usize {
    let sources = state.with_db(|conn| {
        use birdnet_db::audio_sources::AudioSourceStore;
        AudioSourceStore::list(conn).unwrap_or_default()
    });
    let mut credited = 0;
    for source in sources.iter().filter(|s| s.disabled_at.is_none()) {
        // `Some(true)` only. `None` means the supervisor has not reported on
        // this source yet — at startup, or in web-only mode — and crediting an
        // unknown as listening would inflate the denominator, which understates
        // abundance. Understating listening is the safe direction: it
        // overstates abundance, which is visible; overstating listening hides a
        // real signal, which is not.
        if state.metrics().source_up(&source.id) == Some(true) {
            let id = source.id.clone();
            if let Err(e) = state.with_db(|conn| credit(conn, &id, seconds)) {
                tracing::debug!(error = %e, source = %id, "recording-effort credit failed");
            } else {
                credited += 1;
            }
        }
    }
    credited
}

/// Spawn the recording-effort recorder.
///
/// Skipped in web-only mode by the caller: a station that is not capturing has
/// no listening to record, and writing zeroes would make a browsing session
/// look like a monitoring gap.
pub fn spawn_effort_recorder(state: AppState) {
    tokio::spawn(async move {
        tracing::info!(
            poll_secs = POLL_EVERY.as_secs(),
            "recording-effort recorder started"
        );
        let seconds = POLL_EVERY.as_secs_f64();
        let mut tick = tokio::time::interval(POLL_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the immediate first tick: at t=0 no source has reported yet, so
        // it would credit nothing and only add a wakeup.
        tick.tick().await;
        loop {
            tick.tick().await;
            let probe = state.clone();
            let _ = tokio::task::spawn_blocking(move || sample_once(&probe, seconds)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        conn
    }

    /// Assert two second-counts are equal.
    ///
    /// The values are exact small multiples of the poll interval, so an exact
    /// comparison would be correct — but clippy is right that a float equality
    /// in a test is a habit worth not having, and an epsilon costs nothing.
    fn assert_seconds(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected} seconds of effort, got {actual}"
        );
    }

    fn seconds_for(conn: &rusqlite::Connection, source: &str) -> f64 {
        conn.query_row(
            "SELECT seconds FROM recording_effort WHERE source = ?1",
            rusqlite::params![source],
            |r| r.get(0),
        )
        .unwrap_or(0.0)
    }

    #[test]
    fn credit_accumulates_within_a_day() {
        let c = conn();
        credit(&c, "local", 300.0).unwrap();
        credit(&c, "local", 300.0).unwrap();
        // Two samples must add, not replace — otherwise a day's effort is
        // whatever the last sample happened to be.
        assert_seconds(seconds_for(&c, "local"), 600.0);
    }

    #[test]
    fn sources_are_credited_independently() {
        let c = conn();
        credit(&c, "cam1", 300.0).unwrap();
        credit(&c, "cam2", 900.0).unwrap();
        assert_seconds(seconds_for(&c, "cam1"), 300.0);
        assert_seconds(seconds_for(&c, "cam2"), 900.0);
    }

    /// Effort is stamped with the *local* date.
    ///
    /// It is divided into detection counts, and those carry local civil dates
    /// (see `birdnet_db::clock`). Effort on a UTC date would misalign with its
    /// own numerator near midnight — worse than no correction, because the
    /// result still looks like a rate.
    #[test]
    fn effort_is_stamped_with_the_local_date() {
        let c = conn();
        credit(&c, "local", 300.0).unwrap();
        let (stored, local): (String, String) = c
            .query_row(
                "SELECT (SELECT date FROM recording_effort LIMIT 1), date('now','localtime')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored, local);
    }
}
