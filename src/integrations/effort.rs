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
//! From what the detection pipeline did with each segment, not from whether a
//! source looked alive. The daemon's throughput observer reports every segment
//! it analysed (with its length, from the header), every one the shed policy
//! skipped under heat or backlog, and every one that vanished before it was
//! read; [`PendingEffort`] adds them up by the local date and hour the audio
//! was recorded and its source, and every [`POLL_EVERY`] the recorder folds the
//! sums into `analysis_effort` (hourly) and `recording_effort` (daily).
//!
//! Until 2026-11 this sampled the capture supervisor's liveness gauge and
//! credited each live source with the whole interval. That counted audio the
//! pipeline shed — half of it, on a hot afternoon — or never saw, and a source
//! whose gauge a detection had set looked like listening (RES-01, MSC-16). A
//! liveness gauge says capture is running; only the analysed audio says what
//! the detections were looked for in.
//!
//! Effort is credited to the **local** date and hour in the segment's name,
//! because that is the stamp the detections it will be divided by carry, and it
//! is the hour the audio was recorded, not the one it was analysed in: a
//! backlog cleared at noon is credited to the morning it holds.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use birdnet_web::state::AppState;

/// One local hour of one source's audio, as the pipeline dealt with it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Tally {
    seconds_analysed: f64,
    segments_analysed: u64,
    segments_shed: u64,
    segments_dropped: u64,
}

/// `(local date, local hour, source)`.
type HourKey = (String, u8, String);

/// What the pipeline analysed, shed or lost since the last fold, by the local
/// hour the audio was recorded in and its source.
///
/// Held in memory and folded into the ledger every [`POLL_EVERY`], which keeps
/// the write rate where the sampler had it (a handful of `UPSERT`s an hour)
/// instead of two per segment. A crash loses at most one interval.
#[derive(Debug, Default)]
pub struct PendingEffort {
    hours: Mutex<BTreeMap<HourKey, Tally>>,
}

/// The process's one [`PendingEffort`]: the daemon's throughput observer adds
/// to it, the recorder task folds it. They are started apart.
pub fn pending() -> &'static PendingEffort {
    static PENDING: std::sync::OnceLock<PendingEffort> = std::sync::OnceLock::new();
    PENDING.get_or_init(PendingEffort::default)
}

impl PendingEffort {
    /// `secs` of the recording at `path` were analysed. A recording that runs
    /// past the end of an hour is split between the hours it covers.
    ///
    /// The split is wall-clock arithmetic on the local stamp in the file name,
    /// so a segment that straddles a daylight-saving change is split as if the
    /// clock had not jumped (an error of at most the segment's length, twice a
    /// year).
    pub fn analysed(&self, path: &std::path::Path, secs: f64) {
        if !(secs.is_finite() && secs > 0.0) {
            return;
        }
        let Some((start, source)) = Self::stamp(path) else {
            return;
        };
        #[allow(clippy::cast_possible_truncation)]
        let end = start + (secs * 1000.0).round() as i64;
        let mut at = start;
        let mut first = true;
        let mut hours = self.lock();
        while at < end {
            let hour_end = (at.div_euclid(3_600_000) + 1) * 3_600_000;
            let upto = end.min(hour_end);
            #[allow(clippy::cast_precision_loss)]
            let part = (upto - at) as f64 / 1000.0;
            let tally = hours.entry(Self::key(at, &source)).or_default();
            tally.seconds_analysed += part;
            if first {
                tally.segments_analysed += 1;
                first = false;
            }
            at = upto;
        }
        drop(hours);
    }

    /// The recording at `path` was skipped by the shed policy.
    pub fn shed(&self, path: &std::path::Path) {
        self.count(path, |t| t.segments_shed += 1);
    }

    /// The recording at `path` vanished before the pipeline read it.
    pub fn dropped(&self, path: &std::path::Path) {
        self.count(path, |t| t.segments_dropped += 1);
    }

    fn count(&self, path: &std::path::Path, bump: impl FnOnce(&mut Tally)) {
        if let Some((start, source)) = Self::stamp(path) {
            bump(self.lock().entry(Self::key(start, &source)).or_default());
        }
    }

    /// Everything waiting, emptied.
    fn take(&self) -> BTreeMap<HourKey, Tally> {
        std::mem::take(&mut *self.lock())
    }

    /// Put back what could not be written, so the next fold tries again.
    fn restore(&self, back: BTreeMap<HourKey, Tally>) {
        let mut hours = self.lock();
        for (key, t) in back {
            let e = hours.entry(key).or_default();
            e.seconds_analysed += t.seconds_analysed;
            e.segments_analysed += t.segments_analysed;
            e.segments_shed += t.segments_shed;
            e.segments_dropped += t.segments_dropped;
        }
        drop(hours);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<HourKey, Tally>> {
        // A panic while holding this lock leaves a tally, not a broken
        // invariant: carry on with what is there.
        self.hours
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The recording's local start, in wall-clock milliseconds, and its
    /// source. `None` for a name that carries no stamp, which the daemon
    /// refuses to analyse anyway.
    fn stamp(path: &std::path::Path) -> Option<(i64, String)> {
        let name = path.file_name()?.to_str()?;
        let rf = birdnet_core::detection::types::RecordingFile::parse(name)?;
        let civil = birdnet_core::civil::parse_civil(&rf.date, &rf.time)?;
        let start = birdnet_core::civil::unix_secs_from_civil(&civil) * 1000;
        Some((start, crate::daemon::disposition::derive_source_label(path)))
    }

    fn key(wall_ms: i64, source: &str) -> HourKey {
        let c = birdnet_core::civil::civil_from_unix_secs(wall_ms.div_euclid(1000));
        (
            format!("{:04}-{:02}-{:02}", c.year, c.month, c.day),
            u8::try_from(c.hour).unwrap_or(0),
            source.to_owned(),
        )
    }
}

/// How often the analysed-audio tallies are folded into the ledger.
///
/// Matches the station-health poll. It keeps the write rate to a handful of
/// `UPSERT`s an hour — negligible on an SD card whose wear budget is the reason
/// the raw audio lives on tmpfs — and bounds what a crash can lose to five
/// minutes.
const POLL_EVERY: Duration = Duration::from_secs(300);

/// Write everything [`pending`] holds into `analysis_effort` and
/// `recording_effort`, in one transaction. Returns how many hour rows were
/// written; on failure the tallies go back to wait for the next fold.
fn fold(state: &AppState, pending: &PendingEffort) -> usize {
    let hours = pending.take();
    if hours.is_empty() {
        return 0;
    }
    let written = state.with_db(|conn| {
        let tx = conn.unchecked_transaction()?;
        for ((date, hour, source), t) in &hours {
            birdnet_db::sqlite::add_hourly_effort(
                &tx,
                &birdnet_db::sqlite::HourlyEffort {
                    date: date.clone(),
                    hour: *hour,
                    source: source.clone(),
                    seconds_analysed: t.seconds_analysed,
                    segments_analysed: t.segments_analysed,
                    segments_shed: t.segments_shed,
                    segments_dropped: t.segments_dropped,
                },
            )?;
        }
        tx.commit()?;
        Ok::<_, birdnet_db::sqlite::DbError>(hours.len())
    });
    match written {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "could not write the analysis effort; keeping it for the next try");
            pending.restore(hours);
            0
        }
    }
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

/// One recorder step: what the pipeline did since the last step goes into the
/// ledger, then the analytics copy is refreshed.
fn record_tick(state: &AppState) {
    if fold(state, pending()) > 0 {
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
        let mut tick = tokio::time::interval(POLL_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the immediate first tick: at t=0 no source has reported yet, so
        // it would credit nothing and only add a wakeup.
        tick.tick().await;
        loop {
            tick.tick().await;
            let probe = state.clone();
            let _ = tokio::task::spawn_blocking(move || record_tick(&probe)).await;
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

    /// The tests that drive the daemon's observer share the process's one
    /// [`pending`]; one at a time, or one folds the other's tallies.
    static GLOBAL_PENDING: Mutex<()> = Mutex::new(());

    /// Fifteen seconds of silence at 8 kHz, named the way capture names a
    /// local-microphone segment recorded at `time` on `date`.
    fn segment(dir: &std::path::Path, date: &str, time: &str) -> std::path::PathBuf {
        let path = dir.join(format!("{date}-birdnet-{time}.wav"));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..(8_000 * 15) {
            w.write_sample(0_i16).unwrap();
        }
        w.finalize().unwrap();
        path
    }

    fn state_with_local_source() -> AppState {
        use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
        let conn = conn();
        conn.insert(&NewAudioSource::defaults(
            "local",
            SourceKind::UsbAlsa,
            "plughw:1,0",
        ))
        .unwrap();
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    fn effort_of(state: &AppState, source: &str) -> f64 {
        state.with_db(|c| {
            c.query_row(
                "SELECT COALESCE(SUM(seconds), 0) FROM recording_effort WHERE source = ?1",
                rusqlite::params![source],
                |r| r.get(0),
            )
            .unwrap()
        })
    }

    /// RES-01 / MSC-16. Twenty 15-second segments recorded in one five-minute
    /// poll, the shed policy skipping every other one (as it does under heat or
    /// backlog), with the source's liveness gauge up throughout. The ledger is
    /// what was analysed, 150 s, and the hour's row says ten were shed. It used
    /// to credit the poll interval, 300 s, to any source whose gauge was up.
    ///
    /// Drives the daemon's own throughput observer, so the wiring from the
    /// pipeline to the ledger is what is tested, not a helper beside it.
    #[test]
    fn effort_is_what_the_pipeline_analysed_not_how_long_the_source_was_up() {
        use birdnet_core::detection::daemon::ShedReason;
        let _serial = GLOBAL_PENDING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_local_source();
        state.metrics().set_source_up("local", true);
        let observer = crate::daemon::throughput_observer(&state);
        for i in 0..20_u32 {
            let path = segment(
                dir.path(),
                "2026-06-01",
                &format!("14:{:02}:{:02}", (i * 15) / 60, (i * 15) % 60),
            );
            if i % 2 == 1 {
                observer.shed(&path, ShedReason::Thermal);
            } else {
                observer.analysed(&path);
            }
        }

        record_tick(&state);

        assert_seconds(effort_of(&state, "local"), 150.0);
        let hours = state
            .with_db(|c| birdnet_db::sqlite::hourly_effort(c, "2026-06-01", "2026-06-01"))
            .unwrap();
        let row = hours
            .iter()
            .find(|h| h.source == "local" && h.hour == 14)
            .unwrap_or_else(|| panic!("no 14:00 row for local: {hours:?}"));
        assert_seconds(row.seconds_analysed, 150.0);
        assert_eq!(
            (row.segments_analysed, row.segments_shed),
            (10, 10),
            "{row:?}"
        );
    }

    /// The counterpart, and the other half of MSC-16: a source whose gauge is
    /// up but whose audio nothing analysed is credited nothing, and audio
    /// analysed in full is credited in full, split at the hour it crosses.
    #[test]
    fn a_live_gauge_alone_is_not_listening_and_full_analysis_is_credited_in_full() {
        let _serial = GLOBAL_PENDING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_local_source();
        state.metrics().set_source_up("local", true);
        record_tick(&state);
        assert_seconds(effort_of(&state, "local"), 0.0);

        let observer = crate::daemon::throughput_observer(&state);
        observer.analysed(&segment(dir.path(), "2026-06-02", "06:59:50"));
        record_tick(&state);
        assert_seconds(effort_of(&state, "local"), 15.0);
        let hours = state
            .with_db(|c| birdnet_db::sqlite::hourly_effort(c, "2026-06-02", "2026-06-02"))
            .unwrap();
        let by_hour: Vec<(u8, f64, u64)> = hours
            .iter()
            .filter(|h| h.source == "local")
            .map(|h| (h.hour, h.seconds_analysed, h.segments_analysed))
            .collect();
        assert_eq!(by_hour.len(), 2, "{by_hour:?}");
        assert_seconds(by_hour[0].1, 10.0);
        assert_seconds(by_hour[1].1, 5.0);
        assert_eq!((by_hour[0].0, by_hour[1].0), (6, 7));
        assert_eq!(
            (by_hour[0].2, by_hour[1].2),
            (1, 0),
            "one segment, counted once"
        );
    }

    fn hour(date: &str, hour: u8, source: &str, seconds: f64) -> birdnet_db::sqlite::HourlyEffort {
        birdnet_db::sqlite::HourlyEffort {
            date: date.to_owned(),
            hour,
            source: source.to_owned(),
            seconds_analysed: seconds,
            segments_analysed: 1,
            segments_shed: 0,
            segments_dropped: 0,
        }
    }

    #[test]
    fn credit_accumulates_within_a_day() {
        let c = conn();
        birdnet_db::sqlite::add_hourly_effort(&c, &hour("2026-06-01", 9, "local", 300.0)).unwrap();
        birdnet_db::sqlite::add_hourly_effort(&c, &hour("2026-06-01", 9, "local", 300.0)).unwrap();
        // Two folds must add, not replace — otherwise a day's effort is
        // whatever the last fold happened to hold.
        assert_seconds(seconds_for(&c, "local"), 600.0);
        let hours = birdnet_db::sqlite::hourly_effort(&c, "2026-06-01", "2026-06-01").unwrap();
        assert_eq!(hours.len(), 1, "{hours:?}");
        assert_seconds(hours[0].seconds_analysed, 600.0);
        assert_eq!(hours[0].segments_analysed, 2);
    }

    #[test]
    fn sources_are_credited_independently() {
        let c = conn();
        birdnet_db::sqlite::add_hourly_effort(&c, &hour("2026-06-01", 9, "cam1", 300.0)).unwrap();
        birdnet_db::sqlite::add_hourly_effort(&c, &hour("2026-06-01", 9, "cam2", 900.0)).unwrap();
        assert_seconds(seconds_for(&c, "cam1"), 300.0);
        assert_seconds(seconds_for(&c, "cam2"), 900.0);
    }

    /// Effort is stamped with the *local* date the audio was recorded on.
    ///
    /// It is divided into detection counts, and those carry the local civil
    /// date in the segment's file name (see `birdnet_db::clock`). Effort on a
    /// UTC date, or on the date it happened to be analysed, would misalign
    /// with its own numerator near midnight and after a backlog — worse than
    /// no correction, because the result still looks like a rate. A segment
    /// that runs past midnight is credited to both dates.
    #[test]
    fn effort_is_stamped_with_the_local_date() {
        let pending = PendingEffort::default();
        pending.analysed(
            std::path::Path::new("/w/2026-03-08-birdnet-23:59:55.wav"),
            15.0,
        );
        let hours: Vec<_> = pending.take().into_iter().collect();
        assert_eq!(hours.len(), 2, "{hours:?}");
        assert_eq!(
            hours[0].0,
            ("2026-03-08".to_owned(), 23, "local".to_owned())
        );
        assert_eq!(hours[1].0, ("2026-03-09".to_owned(), 0, "local".to_owned()));
        assert_seconds(hours[0].1.seconds_analysed, 5.0);
        assert_seconds(hours[1].1.seconds_analysed, 10.0);
    }
}
