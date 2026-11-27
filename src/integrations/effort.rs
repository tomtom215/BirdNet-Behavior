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
//! As the audio the detection pipeline analysed (RES-01). Each analysed file's
//! length is held, by the local date and hour in its name and its source, in
//! the metrics registry ([`MetricsRegistry::add_effort_pending`]), and every
//! [`POLL_EVERY`] this task writes what is held to `recording_effort` (by day)
//! and `recording_effort_hourly` (by hour). A segment the shed policy skipped,
//! one that vanished before it was read, and audio still waiting in a backlog
//! are not effort: nothing was listening to them. The date and hour are the
//! recording's, not the clock's at write time, because that is what the
//! detections divided by them are stamped with, and a backlog analysed late
//! would otherwise be credited to the wrong hour. A segment that crosses an
//! hour is credited to the hour it started in, an error of at most one segment.
//!
//! Until migration 63 the ledger sampled the capture supervisor's liveness
//! gauge instead and credited [`POLL_EVERY`] to every source that read "up",
//! which counted shed and dropped audio as listening. That sample is still
//! taken, and kept as `seconds_up` by hour beside `seconds_analysed`: the time
//! a source was up is a fact about the station, and the gap between the two is
//! audio captured and never analysed.
//!
//! [`MetricsRegistry::add_effort_pending`]: birdnet_web::metrics::MetricsRegistry::add_effort_pending

use std::time::Duration;

use birdnet_web::state::AppState;

/// How often source liveness is sampled, and the credit given per sample.
///
/// Matches the station-health poll. Five minutes bounds the error at 0.3 % of a
/// day per source while keeping the write rate to a handful of `UPSERT`s an
/// hour — negligible on an SD card whose wear budget is the reason the raw
/// audio lives on tmpfs.
const POLL_EVERY: Duration = Duration::from_secs(300);

/// Write held analysed audio to both ledgers, in one transaction.
///
/// Additive `UPSERT`s: a restart mid-day resumes the same rows rather than
/// starting second ones.
fn credit_analysed(
    conn: &rusqlite::Connection,
    credits: &std::collections::HashMap<birdnet_web::metrics::EffortKey, f64>,
) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    for ((date, hour, source), seconds) in credits {
        tx.execute(
            "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, ?2, ?3)
             ON CONFLICT(date, source) DO UPDATE SET seconds = seconds + excluded.seconds",
            rusqlite::params![date, source, seconds],
        )?;
        tx.execute(
            "INSERT INTO recording_effort_hourly (date, hour, source, seconds_analysed)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(date, hour, source)
             DO UPDATE SET seconds_analysed = seconds_analysed + excluded.seconds_analysed",
            rusqlite::params![date, hour, source, seconds],
        )?;
    }
    tx.commit()
}

/// Credit `seconds` of being up to `source`, in the current local hour, and
/// in the current UTC hour split by what the system said of its clock
/// (`clock_synced`, as the station-health poll last read it; `None` is
/// unknown, never assumed synchronised).
fn credit_up(
    conn: &rusqlite::Connection,
    source: &str,
    seconds: f64,
    clock_synced: Option<bool>,
) -> Result<(), rusqlite::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    birdnet_db::capture_ledger::credit_up_utc(conn, now, source, seconds, clock_synced)
        .map_err(db_to_sqlite)?;
    conn.execute(
        "INSERT INTO recording_effort_hourly (date, hour, source, seconds_up)
         VALUES (date('now','localtime'), CAST(strftime('%H','now','localtime') AS INTEGER), ?1, ?2)
         ON CONFLICT(date, hour, source) DO UPDATE SET seconds_up = seconds_up + excluded.seconds_up",
        rusqlite::params![source, seconds],
    )?;
    Ok(())
}

/// Hold one analysed file's audio for the ledger, under the date and hour its
/// name records and its source. A name that does not parse as a recording, or
/// a file whose length cannot be read, holds nothing.
pub fn note_analysed(state: &AppState, path: &std::path::Path, seconds: f64) {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let Some(recording) = birdnet_core::detection::types::RecordingFile::parse(name) else {
        return;
    };
    let Some(hour) = recording.time.get(0..2).and_then(|h| h.parse::<u8>().ok()) else {
        return;
    };
    let source = crate::daemon::disposition::derive_source_label(path);
    state
        .metrics()
        .add_effort_pending(&recording.date, hour, &source, seconds);
    if let Some(at) = recording_instant(&recording.date, &recording.time, now_unix()) {
        state.metrics().add_effort_pending_utc(
            birdnet_db::capture_ledger::hour_start(at),
            &source,
            seconds,
        );
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn db_to_sqlite(e: birdnet_db::sqlite::DbError) -> rusqlite::Error {
    match e {
        birdnet_db::sqlite::DbError::Sqlite(e) => e,
        other => rusqlite::Error::ToSqlConversionFailure(Box::new(other)),
    }
}

/// The instant a recording's local `date` and `time` name, for the UTC ledger.
///
/// The rule the daemon stamps `detected_at_utc` by
/// (`daemon::local_offset::detection_instant`, private to the daemon): the
/// zone rules' offset for the recording's own date, unless that puts it more
/// than five minutes in the future, which only the repeated autumn hour does
/// for a recording just made, and then the offset in force now. `None` when
/// the name does not give a civil time.
fn recording_instant(date: &str, time: &str, now: i64) -> Option<i64> {
    let live = birdnet_core::civil::unix_secs_from_local(
        date,
        time,
        birdnet_db::clock::local_utc_offset_secs(),
    )?;
    let Some(offset) = utc_offset_at(date, time) else {
        return Some(live);
    };
    let dated = birdnet_core::civil::unix_secs_from_local(date, time, offset)?;
    Some(if dated > now.saturating_add(300) {
        live
    } else {
        dated
    })
}

/// The zone rules' UTC offset at local `date` `time`, asked of `SQLite` as the
/// rest of the station asks it.
fn utc_offset_at(date: &str, time: &str) -> Option<i64> {
    let c = birdnet_core::civil::parse_civil(date, time)?;
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    );
    let conn = rusqlite::Connection::open_in_memory().ok()?;
    conn.query_row(
        "SELECT CAST(strftime('%s', ?1) AS INTEGER) - CAST(strftime('%s', ?1, 'utc') AS INTEGER)",
        [&stamp],
        |row| row.get::<_, Option<i64>>(0),
    )
    .ok()
    .flatten()
    .filter(|o| (-14 * 3600..=14 * 3600).contains(o))
}

/// Write the UTC ledger's held analysed audio; give back what could not be.
fn flush_analysed_utc(state: &AppState) -> bool {
    let credits = state.metrics().take_effort_pending_utc();
    if credits.is_empty() {
        return false;
    }
    let written = state.with_db(|conn| {
        let tx = conn.unchecked_transaction()?;
        for ((hour, source), seconds) in &credits {
            birdnet_db::capture_ledger::credit_analysed_utc(&tx, *hour, source, *seconds)
                .map_err(db_to_sqlite)?;
        }
        tx.commit()
    });
    match written {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "UTC effort could not be written; holding it for the next tick");
            for ((hour, source), seconds) in credits {
                state
                    .metrics()
                    .add_effort_pending_utc(hour, &source, seconds);
            }
            false
        }
    }
}

/// Write the held analysed audio. Credits that could not be written are held
/// again for the next tick. Returns whether anything was written.
fn flush_analysed(state: &AppState) -> bool {
    let credits = state.metrics().take_effort_pending();
    if credits.is_empty() {
        return false;
    }
    match state.with_db(|conn| credit_analysed(conn, &credits)) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "recording effort could not be written; holding it for the next tick");
            for ((date, hour, source), seconds) in credits {
                state
                    .metrics()
                    .add_effort_pending(&date, hour, &source, seconds);
            }
            false
        }
    }
}

/// Record one sample: credit every source the supervisor reports as up with
/// `seconds` of being up, in the hourly ledger. Not effort: see the module doc.
///
/// Returns how many sources were credited, for tests.
fn sample_once(state: &AppState, seconds: f64) -> usize {
    let sources = match state.with_db(|conn| {
        use birdnet_db::audio_sources::AudioSourceStore;
        AudioSourceStore::list(conn)
    }) {
        Ok(sources) => sources,
        Err(e) => {
            tracing::debug!(error = %e, "audio sources could not be read; no uptime sample this tick");
            return 0;
        }
    };
    let mut credited = 0;
    for source in sources.iter().filter(|s| s.disabled_at.is_none()) {
        // `Some(true)` only. `None` means the supervisor has not reported on
        // this source yet — at startup, or in web-only mode.
        if state.metrics().source_up(&source.id) == Some(true) {
            let id = source.id.clone();
            let clock = state.metrics().clock_synced();
            if let Err(e) = state.with_db(|conn| credit_up(conn, &id, seconds, clock)) {
                tracing::debug!(error = %e, source = %id, "uptime sample failed");
            } else {
                credited += 1;
            }
        }
    }
    credited
}

/// Copy the effort just credited into the analytics store.
///
/// The effort-corrected abundance divides there, and it was only ever given
/// the copy made at startup, so every week since the last restart was divided
/// by the listening recorded before it.
#[cfg(feature = "analytics")]
fn mirror_to_analytics(state: &AppState) {
    if let Some(Err(e)) = state.mirror_recent_recording_effort() {
        tracing::warn!(error = %e, "could not copy recording effort to the analytics store");
    }
}

#[cfg(not(feature = "analytics"))]
const fn mirror_to_analytics(_state: &AppState) {}

/// One tick of the recorder: what happens every [`POLL_EVERY`].
pub fn tick(state: &AppState, seconds: f64) {
    sample_once(state, seconds);
    flush_analysed_utc(state);
    if flush_analysed(state) {
        mirror_to_analytics(state);
    }
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
        let mut interval = tokio::time::interval(POLL_EVERY);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the immediate first tick: at t=0 no source has reported yet, so
        // it would credit nothing and only add a wakeup.
        interval.tick().await;
        loop {
            interval.tick().await;
            let probe = state.clone();
            let _ = tokio::task::spawn_blocking(move || tick(&probe, seconds)).await;
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

    fn credits(
        entries: &[(&str, u8, &str, f64)],
    ) -> std::collections::HashMap<birdnet_web::metrics::EffortKey, f64> {
        entries
            .iter()
            .map(|(d, h, s, secs)| (((*d).to_owned(), *h, (*s).to_owned()), *secs))
            .collect()
    }

    #[test]
    fn credit_accumulates_within_a_day() {
        let c = conn();
        credit_analysed(&c, &credits(&[("2026-05-19", 9, "local", 300.0)])).unwrap();
        credit_analysed(&c, &credits(&[("2026-05-19", 10, "local", 300.0)])).unwrap();
        // Two credits must add, not replace — otherwise a day's effort is
        // whatever the last one happened to be.
        assert_seconds(seconds_for(&c, "local"), 600.0);
    }

    #[test]
    fn sources_are_credited_independently() {
        let c = conn();
        credit_analysed(
            &c,
            &credits(&[
                ("2026-05-19", 9, "cam1", 300.0),
                ("2026-05-19", 9, "cam2", 900.0),
            ]),
        )
        .unwrap();
        assert_seconds(seconds_for(&c, "cam1"), 300.0);
        assert_seconds(seconds_for(&c, "cam2"), 900.0);
    }

    /// RES-12 / RES-13: the same uptime sample is credited to the UTC hour,
    /// under the clock state the station last read, and an unknown state is
    /// not counted as synchronised.
    #[test]
    fn uptime_is_credited_by_utc_hour_under_the_clock_state() {
        let c = conn();
        credit_up(&c, "mic", 300.0, Some(false)).unwrap();
        credit_up(&c, "mic", 300.0, None).unwrap();
        let (up, synced, unsynced): (f64, f64, f64) = c
            .query_row(
                "SELECT COALESCE(SUM(seconds_up), 0), COALESCE(SUM(seconds_up_clock_synced), 0), COALESCE(SUM(seconds_up_clock_unsynced), 0)
                   FROM recording_effort_utc WHERE source = 'mic'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_seconds(up, 600.0);
        assert_seconds(synced, 0.0);
        assert_seconds(unsynced, 300.0);
    }

    /// The analysed audio reaches the UTC ledger too, under the hour its
    /// recording began.
    #[test]
    fn analysed_audio_is_held_and_written_by_utc_hour() {
        let c = conn();
        let state = AppState::from_connection(c, std::path::PathBuf::from(":memory:"));
        note_analysed(
            &state,
            std::path::Path::new("/tmp/2026-05-19-birdnet-09:14:00.wav"),
            15.0,
        );
        assert!(flush_analysed_utc(&state), "something was held");
        let at = recording_instant("2026-05-19", "09:14:00", now_unix()).unwrap();
        let stored: (i64, f64) = state
            .with_db(|c| {
                c.query_row(
                    "SELECT hour_start_utc, seconds_analysed FROM recording_effort_utc",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .unwrap();
        assert_eq!(stored.0, birdnet_db::capture_ledger::hour_start(at));
        assert_seconds(stored.1, 15.0);
    }

    /// The uptime sample is stamped with the *local* date and hour, as the
    /// detections it sits beside are (see `birdnet_db::clock`).
    #[test]
    fn uptime_is_stamped_with_the_local_date_and_hour() {
        let c = conn();
        credit_up(&c, "local", 300.0, None).unwrap();
        let (stored, local, hour, local_hour): (String, String, i64, i64) = c
            .query_row(
                "SELECT date, date('now','localtime'), hour,
                        CAST(strftime('%H','now','localtime') AS INTEGER)
                   FROM recording_effort_hourly LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(stored, local);
        assert_eq!(hour, local_hour);
        // An uptime sample is not effort.
        assert_seconds(seconds_for(&c, "local"), 0.0);
    }
}
