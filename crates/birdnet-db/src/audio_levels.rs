//! What the microphones themselves sound like, hour by hour.
//!
//! # Why a station measures itself
//!
//! Every other number this project stores is about the birds. This one is about
//! the equipment, and it exists because the two are indistinguishable from the
//! outside.
//!
//! A microphone that fails *completely* is caught: the per-source
//! `birdnet_audio_source_up` gauge drops, the supervisor restarts it, and the
//! detection deadman fires if nothing is heard at all. A microphone that fails
//! *partially* — water in the capsule, a web across the port, a connector
//! loosened by a year of thermal cycling, a preamp drifting — is caught by
//! nothing. It keeps delivering audio, the process stays alive, the gauge stays
//! at 1, and the station goes on detecting the loud close birds while quietly
//! losing everything else. The only symptom is fewer detections, which is also
//! what the end of the breeding season looks like.
//!
//! The station's own **noise floor** separates the two. Ambient background does
//! not stop when the birds do. A quiet season moves the detections and leaves
//! the floor where it was; a deaf microphone takes the floor down with it.
//!
//! # What is stored
//!
//! One row per (local date, hour, source), holding sums and a count rather than
//! means, so a new observation folds in without revisiting the ones already
//! there. `noise_floor_min_dbfs` is kept beside the mean because a failing
//! capsule drags the minimum down first.
//!
//! Nothing here gates inference. `birdnet_core::audio::quality` can also decide
//! whether a chunk is worth analysing, and deliberately does not: that changes
//! which audio reaches the model and wants hardware validation behind it (see
//! the note in `src/cli.rs`). Observing costs nothing and answers the question
//! an unattended station cannot otherwise answer about itself.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

/// One acoustic observation, as sampled from a single recording segment.
#[derive(Debug, Clone, Copy)]
pub struct LevelSample {
    /// Estimated noise floor, dBFS (negative; quieter is more negative).
    pub noise_floor_dbfs: f32,
    /// Signal-to-noise estimate, dB.
    pub snr_db: f32,
    /// Spectral flatness in `[0, 1]`; 0 tonal, 1 white noise.
    pub spectral_flatness: f32,
    /// Whether rain or impulsive broadband noise was detected.
    pub rain: bool,
}

/// An hour's worth of observations for one source, as read back.
#[derive(Debug, Clone)]
pub struct HourlyLevel {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    /// Local hour of day, `0..=23`.
    pub hour: u8,
    /// Capture source label (`local`, an RTSP stream id, …).
    pub source: String,
    /// How many segments were sampled in this hour.
    pub samples: i64,
    /// Mean noise floor across those samples, dBFS.
    pub noise_floor_dbfs: f64,
    /// Quietest noise floor observed in the hour, dBFS.
    pub noise_floor_min_dbfs: f64,
    /// Mean SNR estimate, dB.
    pub snr_db: f64,
    /// Mean spectral flatness.
    pub spectral_flatness: f64,
    /// How many of the samples looked like rain.
    pub rain_samples: i64,
}

/// Fold one observation into its (date, hour, source) bucket.
///
/// # Errors
///
/// Returns `DbError` on `SQLite` failure.
pub fn record_sample(
    conn: &Connection,
    date: &str,
    hour: u8,
    source: &str,
    sample: LevelSample,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO audio_levels
             (date, hour, source, samples, noise_floor_sum_dbfs, noise_floor_min_dbfs,
              snr_sum_db, flatness_sum, rain_samples)
         VALUES (?1, ?2, ?3, 1, ?4, ?4, ?5, ?6, ?7)
         ON CONFLICT(date, hour, source) DO UPDATE SET
             samples              = audio_levels.samples + 1,
             noise_floor_sum_dbfs = audio_levels.noise_floor_sum_dbfs + excluded.noise_floor_sum_dbfs,
             -- MIN, not excluded.: a later, louder sample must not raise the
             -- quietest thing the station has heard this hour.
             noise_floor_min_dbfs = MIN(audio_levels.noise_floor_min_dbfs, excluded.noise_floor_min_dbfs),
             snr_sum_db           = audio_levels.snr_sum_db + excluded.snr_sum_db,
             flatness_sum         = audio_levels.flatness_sum + excluded.flatness_sum,
             rain_samples         = audio_levels.rain_samples + excluded.rain_samples",
        params![
            date,
            i64::from(hour),
            source,
            f64::from(sample.noise_floor_dbfs),
            f64::from(sample.snr_db),
            f64::from(sample.spectral_flatness),
            i64::from(sample.rain),
        ],
    )?;
    Ok(())
}

/// The most recent `hours` buckets, newest first.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn recent_hours(conn: &Connection, hours: u32) -> Result<Vec<HourlyLevel>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT date, hour, source, samples,
                noise_floor_sum_dbfs / samples,
                noise_floor_min_dbfs,
                snr_sum_db / samples,
                flatness_sum / samples,
                rain_samples
           FROM audio_levels
          ORDER BY date DESC, hour DESC, source
          LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![hours], |r| {
            Ok(HourlyLevel {
                date: r.get(0)?,
                hour: u8::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                source: r.get(2)?,
                samples: r.get(3)?,
                noise_floor_dbfs: r.get(4)?,
                noise_floor_min_dbfs: r.get(5)?,
                snr_db: r.get(6)?,
                spectral_flatness: r.get(7)?,
                rain_samples: r.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// How one source's recent noise floor compares with its own past.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceDrift {
    /// Capture source label.
    pub source: String,
    /// Mean noise floor over the recent window, dBFS.
    pub recent_dbfs: f64,
    /// Mean noise floor over the preceding baseline window, dBFS.
    ///
    /// `None` when the source has no observations that far back. "Never
    /// measured before" and "has not moved" are different answers, and a caller
    /// that cannot tell them apart will report a drift of zero for a station
    /// that has been running a week.
    pub baseline_dbfs: Option<f64>,
    /// Observations behind [`Self::recent_dbfs`].
    pub recent_samples: i64,
}

impl SourceDrift {
    /// How far the source has moved from its own baseline, in dB. Negative is
    /// quieter — the direction a failing microphone goes.
    #[must_use]
    pub fn moved_db(&self) -> Option<f64> {
        self.baseline_dbfs.map(|b| self.recent_dbfs - b)
    }
}

/// Mean noise floor per source over the last `days` local days, and over the
/// `baseline_days` before *those*.
///
/// A source with no observations in the recent span is omitted entirely.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn drift_by_source(
    conn: &Connection,
    days: u32,
    baseline_days: u32,
) -> Result<Vec<SourceDrift>, DbError> {
    // `date('now','localtime')` matches how every other date in this schema is
    // stored — the detections carry local wall clock, and a UTC window here
    // would slice the day at a different instant from everything beside it.
    let mut stmt = conn.prepare(
        "WITH recent AS (
             SELECT source,
                    SUM(noise_floor_sum_dbfs) / SUM(samples) AS mean_dbfs,
                    SUM(samples) AS n
               FROM audio_levels
              WHERE date > date('now','localtime', '-' || ?1 || ' days')
              GROUP BY source
         ), baseline AS (
             SELECT source,
                    SUM(noise_floor_sum_dbfs) / SUM(samples) AS mean_dbfs
               FROM audio_levels
              WHERE date <= date('now','localtime', '-' || ?1 || ' days')
                AND date >  date('now','localtime', '-' || ?2 || ' days')
              GROUP BY source
         )
         SELECT r.source, r.mean_dbfs, b.mean_dbfs, r.n
           FROM recent r
           LEFT JOIN baseline b ON b.source = r.source
          ORDER BY r.source",
    )?;
    let rows = stmt
        .query_map(params![days, days + baseline_days], |r| {
            Ok(SourceDrift {
                source: r.get(0)?,
                recent_dbfs: r.get(1)?,
                baseline_dbfs: r.get(2)?,
                recent_samples: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Delete buckets older than `keep_days` local days.
///
/// Returns how many rows were removed.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn prune(conn: &Connection, keep_days: u32) -> Result<usize, DbError> {
    let removed = conn.execute(
        "DELETE FROM audio_levels
          WHERE date < date('now','localtime', '-' || ?1 || ' days')",
        params![keep_days],
    )?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

    fn sample(noise: f32) -> LevelSample {
        LevelSample {
            noise_floor_dbfs: noise,
            snr_db: 6.0,
            spectral_flatness: 0.4,
            rain: false,
        }
    }

    #[test]
    fn observations_fold_into_one_bucket_per_hour_and_source() {
        let conn = db();
        for n in [-52.0, -48.0, -50.0] {
            record_sample(&conn, "2026-08-20", 6, "local", sample(n)).expect("record");
        }
        let rows = recent_hours(&conn, 10).expect("read");
        assert_eq!(rows.len(), 1, "three samples, one hour, one bucket");
        assert_eq!(rows[0].samples, 3);
        assert!(
            (rows[0].noise_floor_dbfs - (-50.0)).abs() < 1e-6,
            "mean of -52, -48, -50 is -50; got {}",
            rows[0].noise_floor_dbfs
        );
    }

    /// The minimum is the quietest thing heard, not the newest.
    ///
    /// `ON CONFLICT … DO UPDATE SET x = excluded.x` is the reflex here and it is
    /// wrong: it would make the column mean "the last sample", which is not a
    /// minimum at all and would hide exactly the downward step this table exists
    /// to show.
    #[test]
    fn a_louder_later_sample_does_not_raise_the_minimum() {
        let conn = db();
        record_sample(&conn, "2026-08-20", 6, "local", sample(-70.0)).expect("record");
        record_sample(&conn, "2026-08-20", 6, "local", sample(-30.0)).expect("record");
        let rows = recent_hours(&conn, 10).expect("read");
        assert!(
            (rows[0].noise_floor_min_dbfs - (-70.0)).abs() < 1e-6,
            "got {}",
            rows[0].noise_floor_min_dbfs
        );
    }

    #[test]
    fn sources_are_kept_apart() {
        let conn = db();
        record_sample(&conn, "2026-08-20", 6, "local", sample(-50.0)).expect("record");
        record_sample(&conn, "2026-08-20", 6, "cam1", sample(-20.0)).expect("record");
        let rows = recent_hours(&conn, 10).expect("read");
        assert_eq!(rows.len(), 2, "two microphones are two measurements");
    }

    /// The whole point: a microphone that goes deaf shows up as a drop against
    /// **its own** past, not against the average of every microphone.
    ///
    /// The two sources are given deliberately different baselines. An earlier
    /// version of this test gave them the same one, and a mutation that joined
    /// the baselines with `ON 1=1` — pooling every microphone into one number
    /// and producing a cartesian product — passed it. A blanket alarm had been
    /// standing in for a discriminator.
    #[test]
    fn drift_is_measured_against_each_sources_own_history() {
        let conn = db();
        // `local` sits at -50 dBFS throughout — a healthy, unchanged mic.
        // `cam1` sat at -65 and is now at -40: a 25 dB move, on that source
        // alone. Pooling the two baselines would put both near -57, which is
        // wrong for both and is what the assertions below reject.
        for d in 1..=40 {
            let date: String = conn
                .query_row(
                    "SELECT date('now','localtime', '-' || ?1 || ' days')",
                    params![d],
                    |r| r.get(0),
                )
                .expect("date");
            let cam1 = if d <= 5 { -40.0 } else { -65.0 };
            record_sample(&conn, &date, 6, "local", sample(-50.0)).expect("record");
            record_sample(&conn, &date, 6, "cam1", sample(cam1)).expect("record");
        }
        let drift = drift_by_source(&conn, 7, 30).expect("drift");
        assert_eq!(
            drift.len(),
            2,
            "one row per source — more means the join has multiplied them: {drift:?}"
        );
        let by = |name: &str| {
            drift
                .iter()
                .find(|d| d.source == name)
                .cloned()
                .expect("source present")
        };

        let local = by("local");
        let (recent_local, n_local) = (local.recent_dbfs, local.recent_samples);
        let base_local = local.baseline_dbfs.expect("local has history");
        assert!(
            (base_local - (-50.0)).abs() < 0.5,
            "local's baseline is its own -50, not a pooled average; got {base_local}"
        );
        assert!(
            (recent_local - base_local).abs() < 1.0,
            "a healthy source has not moved"
        );
        assert!(n_local > 0);

        let cam = by("cam1");
        let recent_cam = cam.recent_dbfs;
        let base_cam = cam.baseline_dbfs.expect("cam1 has history");
        assert!(
            (base_cam - (-65.0)).abs() < 0.5,
            "cam1's baseline is its own -65, not a pooled average; got {base_cam}"
        );
        let moved = recent_cam - base_cam;
        assert!(
            (20.0..30.0).contains(&moved),
            "cam1 should read ~25 dB louder than its own history; moved {moved} dB"
        );
    }

    #[test]
    fn a_source_with_no_history_reports_no_baseline() {
        let conn = db();
        let today: String = conn
            .query_row("SELECT date('now','localtime')", [], |r| r.get(0))
            .expect("date");
        record_sample(&conn, &today, 6, "local", sample(-50.0)).expect("record");
        let drift = drift_by_source(&conn, 7, 30).expect("drift");
        assert_eq!(drift.len(), 1);
        assert!(
            drift[0].baseline_dbfs.is_none(),
            "never measured before is not the same as has not moved"
        );
        assert!(drift[0].moved_db().is_none());
    }

    #[test]
    fn prune_keeps_the_window_and_drops_the_rest() {
        let conn = db();
        for d in [1_i64, 5, 40, 400] {
            let date: String = conn
                .query_row(
                    "SELECT date('now','localtime', '-' || ?1 || ' days')",
                    params![d],
                    |r| r.get(0),
                )
                .expect("date");
            record_sample(&conn, &date, 6, "local", sample(-50.0)).expect("record");
        }
        let removed = prune(&conn, 30).expect("prune");
        assert_eq!(removed, 2, "the 40- and 400-day-old buckets go");
        assert_eq!(recent_hours(&conn, 100).expect("read").len(), 2);
    }
}
