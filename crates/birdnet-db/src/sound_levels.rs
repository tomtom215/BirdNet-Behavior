//! Third-octave band levels, folded into hourly buckets.
//!
//! The banded counterpart to [`crate::audio_levels`]. That table keeps one
//! broadband noise floor per source per hour, which notices a microphone going
//! deaf; this one keeps a level per band, which says *how* it went deaf — and
//! separates that from wind, from a resonating mount, and from an oscillating
//! preamp, which all move a broadband figure the same way.
//!
//! # Energy, not decibels, in the running sum
//!
//! `mean_power_sum` accumulates linear power and the read path converts back.
//! Summing decibel values and dividing by the count would answer a different
//! question — for a band that is quiet for most of an hour and loud for one
//! observation, about 43 dB away from this one — and it would be the wrong
//! answer in exactly the case the table exists to catch. See
//! `birdnet_core::audio::soundlevel::BandLevel::mean_db`.
//!
//! # Why the writer does not depend on `birdnet-core`
//!
//! `birdnet-db` does not depend on `birdnet-core`, so [`BandObservation`] is a
//! plain record rather than a re-export of `BandLevel`. The binary maps one to
//! the other at the call site, which keeps the storage crate free of the audio
//! pipeline and means a future second producer of band levels is not forced
//! through the meter's own types.

use rusqlite::{Connection, params};

use crate::sqlite::DbError;

/// One band's contribution from one observation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandObservation {
    /// Nominal ISO 266 centre frequency, in hertz — the band's identity here.
    pub band_hz: f32,
    /// Energy mean of the observation, in decibels.
    pub mean_db: f32,
    /// Quietest second within the observation.
    pub min_db: f32,
    /// Loudest second within the observation.
    pub max_db: f32,
}

/// The broadband figures accompanying one observation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BroadbandObservation {
    /// A-weighted level, in dB(A).
    pub a_weighted_db: f32,
    /// Unweighted (Z-weighted) level.
    pub z_weighted_db: f32,
    /// Calibration offset in force when this was measured, in decibels.
    ///
    /// Stored per row rather than as a setting, because a reading taken before
    /// an operator calibrated and one taken after are in different units, and
    /// a series that silently mixes them is worse than one that has a gap.
    pub calibration_db: f32,
}

/// An hour of band levels for one source and one band, as read back.
#[derive(Debug, Clone, PartialEq)]
pub struct HourlyBand {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    /// Local hour of day, `0..=23`.
    pub hour: u8,
    /// Capture source label.
    pub source: String,
    /// Nominal centre frequency, in hertz.
    pub band_hz: f32,
    /// Observations folded into this bucket.
    pub samples: i64,
    /// Energy mean across those observations, in decibels.
    pub mean_db: f32,
    /// Quietest second seen in the hour.
    pub min_db: f32,
    /// Loudest second seen in the hour.
    pub max_db: f32,
}

/// An hour of broadband levels for one source.
#[derive(Debug, Clone, PartialEq)]
pub struct HourlyBroadband {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    /// Local hour of day, `0..=23`.
    pub hour: u8,
    /// Capture source label.
    pub source: String,
    /// Observations folded into this bucket.
    pub samples: i64,
    /// Energy mean of the A-weighted level across those observations.
    pub a_weighted_db: f32,
    /// Energy mean of the unweighted level.
    pub z_weighted_db: f32,
    /// Calibration offset in force, in decibels; `0.0` means uncalibrated dBFS.
    pub calibration_db: f32,
}

/// Decibels to linear power.
fn to_power(db: f32) -> f64 {
    10.0_f64.powf(f64::from(db) / 10.0)
}

/// Linear power back to decibels, floored so a zero-power bucket is a number.
fn to_db(power: f64) -> f32 {
    if !power.is_finite() || power <= 0.0 {
        return FLOOR_DB;
    }
    #[allow(clippy::cast_possible_truncation)]
    let db = (10.0 * power.log10()) as f32;
    if db.is_finite() {
        db.max(FLOOR_DB)
    } else {
        FLOOR_DB
    }
}

/// The floor these figures are clamped to, matching the meter's own.
///
/// Duplicated rather than imported: `birdnet-db` does not depend on
/// `birdnet-core`, and `the_floor_matches_the_meters` in the binary's tests
/// asserts the two stay equal so this cannot drift into a second answer.
pub const FLOOR_DB: f32 = -120.0;

/// Fold one observation's bands into their `(date, hour, source, band)` buckets.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure. The write is transactional: either
/// every band of the observation lands or none does, so a bucket can never
/// hold an hour where some bands saw more observations than others.
pub fn record_observation(
    conn: &Connection,
    date: &str,
    hour: u8,
    source: &str,
    bands: &[BandObservation],
    broadband: BroadbandObservation,
) -> Result<(), DbError> {
    // `unchecked_transaction`, not `transaction`: the caller reaches this
    // through `AppState::with_db`, which hands out `&Connection` while holding
    // the single writer mutex. Taking `&mut Connection` here would force that
    // seam open for one call site. The safety condition the unchecked variant
    // asks for — that no other transaction is live on this connection — is
    // exactly what that mutex guarantees.
    let tx = conn.unchecked_transaction()?;
    for band in bands {
        tx.execute(
            "INSERT INTO sound_levels
                 (date, hour, source, band_hz, samples, mean_power_sum, min_db, max_db)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7)
             ON CONFLICT(date, hour, source, band_hz) DO UPDATE SET
                 samples        = sound_levels.samples + 1,
                 mean_power_sum = sound_levels.mean_power_sum + excluded.mean_power_sum,
                 -- MIN/MAX, not excluded.: a later observation must widen the
                 -- hour's range, never replace it.
                 min_db         = MIN(sound_levels.min_db, excluded.min_db),
                 max_db         = MAX(sound_levels.max_db, excluded.max_db)",
            params![
                date,
                i64::from(hour),
                source,
                f64::from(band.band_hz),
                to_power(band.mean_db),
                f64::from(band.min_db),
                f64::from(band.max_db),
            ],
        )?;
    }

    tx.execute(
        "INSERT INTO sound_level_broadband
             (date, hour, source, samples, a_power_sum, z_power_sum, calibration_db)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)
         ON CONFLICT(date, hour, source) DO UPDATE SET
             samples     = sound_level_broadband.samples + 1,
             a_power_sum = sound_level_broadband.a_power_sum + excluded.a_power_sum,
             z_power_sum = sound_level_broadband.z_power_sum + excluded.z_power_sum,
             -- The newest observation's calibration wins. An operator who
             -- calibrates mid-hour gets the hour labelled with the offset that
             -- most of its remaining observations will carry, and the next
             -- hour is unambiguous.
             calibration_db = excluded.calibration_db",
        params![
            date,
            i64::from(hour),
            source,
            to_power(broadband.a_weighted_db),
            to_power(broadband.z_weighted_db),
            f64::from(broadband.calibration_db),
        ],
    )?;

    tx.commit()?;
    Ok(())
}

/// Every band of the most recent hour that has any data, for one source.
///
/// Returns an empty vector when the source has never been sampled.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn latest_hour(conn: &Connection, source: &str) -> Result<Vec<HourlyBand>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT date, hour, source, band_hz, samples, mean_power_sum, min_db, max_db
           FROM sound_levels
          WHERE source = ?1
            AND (date, hour) = (
                SELECT date, hour FROM sound_levels
                 WHERE source = ?1
                 ORDER BY date DESC, hour DESC
                 LIMIT 1
            )
          ORDER BY band_hz",
    )?;
    let rows = stmt
        .query_map(params![source], row_to_band)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Every band across the last `hours` buckets, newest first.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn recent_bands(conn: &Connection, hours: u32) -> Result<Vec<HourlyBand>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT date, hour, source, band_hz, samples, mean_power_sum, min_db, max_db
           FROM sound_levels
          WHERE (date, hour) IN (
              SELECT DISTINCT date, hour FROM sound_levels
               ORDER BY date DESC, hour DESC
               LIMIT ?1
          )
          ORDER BY date DESC, hour DESC, source, band_hz",
    )?;
    let rows = stmt
        .query_map(params![hours], row_to_band)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The broadband figures for the last `hours` buckets, newest first.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn recent_broadband(conn: &Connection, hours: u32) -> Result<Vec<HourlyBroadband>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT date, hour, source, samples, a_power_sum, z_power_sum, calibration_db
           FROM sound_level_broadband
          ORDER BY date DESC, hour DESC, source
          LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![hours], |r| {
            let samples: i64 = r.get(3)?;
            let n = f64::from(i32::try_from(samples.max(1)).unwrap_or(i32::MAX));
            let a_sum: f64 = r.get(4)?;
            let z_sum: f64 = r.get(5)?;
            let hour: i64 = r.get(1)?;
            Ok(HourlyBroadband {
                date: r.get(0)?,
                hour: u8::try_from(hour).unwrap_or(0),
                source: r.get(2)?,
                samples,
                a_weighted_db: to_db(a_sum / n),
                z_weighted_db: to_db(z_sum / n),
                #[allow(clippy::cast_possible_truncation)]
                calibration_db: r.get::<_, f64>(6)? as f32,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Delete buckets older than `keep_days`, returning how many band rows went.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn prune(conn: &Connection, keep_days: u32) -> Result<usize, DbError> {
    let cutoff = format!("-{keep_days} days");
    let removed = conn.execute(
        "DELETE FROM sound_levels WHERE date < date('now', 'localtime', ?1)",
        params![cutoff],
    )?;
    conn.execute(
        "DELETE FROM sound_level_broadband WHERE date < date('now', 'localtime', ?1)",
        params![cutoff],
    )?;
    Ok(removed)
}

/// Shared row mapping for the two band queries.
fn row_to_band(r: &rusqlite::Row<'_>) -> rusqlite::Result<HourlyBand> {
    let samples: i64 = r.get(4)?;
    let n = f64::from(i32::try_from(samples.max(1)).unwrap_or(i32::MAX));
    let power_sum: f64 = r.get(5)?;
    let hour: i64 = r.get(1)?;
    #[allow(clippy::cast_possible_truncation)]
    Ok(HourlyBand {
        date: r.get(0)?,
        hour: u8::try_from(hour).unwrap_or(0),
        source: r.get(2)?,
        band_hz: r.get::<_, f64>(3)? as f32,
        samples,
        mean_db: to_db(power_sum / n),
        min_db: r.get::<_, f64>(6)? as f32,
        max_db: r.get::<_, f64>(7)? as f32,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

    fn obs(band_hz: f32, mean: f32, min: f32, max: f32) -> BandObservation {
        BandObservation {
            band_hz,
            mean_db: mean,
            min_db: min,
            max_db: max,
        }
    }

    fn broadband(a: f32, z: f32) -> BroadbandObservation {
        BroadbandObservation {
            a_weighted_db: a,
            z_weighted_db: z,
            calibration_db: 0.0,
        }
    }

    /// A single observation reads back as itself.
    #[test]
    fn one_observation_round_trips() {
        let conn = db();
        record_observation(
            &conn,
            "2026-05-01",
            7,
            "local",
            &[obs(1000.0, -42.0, -50.0, -30.0)],
            broadband(-45.0, -38.0),
        )
        .expect("record");

        let rows = latest_hour(&conn, "local").expect("read");
        assert_eq!(rows.len(), 1);
        assert!((rows[0].band_hz - 1000.0).abs() < 0.01);
        assert_eq!(rows[0].samples, 1);
        assert!((rows[0].mean_db - -42.0).abs() < 0.01);
        assert!((rows[0].min_db - -50.0).abs() < 0.01);
        assert!((rows[0].max_db - -30.0).abs() < 0.01);
    }

    /// Two observations fold into one bucket with an **energy** mean.
    ///
    /// −40 dB and −20 dB average to −23.0 dB in energy and −30 dB in
    /// decibels. Asserting only the first would be satisfied by any
    /// monotone combination, so the second is asserted as excluded — this is
    /// the whole reason `mean_power_sum` is a power sum.
    #[test]
    fn observations_fold_with_an_energy_mean() {
        let conn = db();
        for mean in [-40.0_f32, -20.0] {
            record_observation(
                &conn,
                "2026-05-01",
                7,
                "local",
                &[obs(1000.0, mean, mean, mean)],
                broadband(mean, mean),
            )
            .expect("record");
        }

        let rows = latest_hour(&conn, "local").expect("read");
        assert_eq!(rows.len(), 1, "the two observations must share one bucket");
        assert_eq!(rows[0].samples, 2);
        assert!(
            (rows[0].mean_db - -23.01).abs() < 0.05,
            "the hour's mean is {:.2} dB; the energy mean of -40 and -20 is -23.01 and the \
             arithmetic mean of the decibel values is -30.00",
            rows[0].mean_db
        );

        let bb = recent_broadband(&conn, 10).expect("read");
        assert_eq!(bb.len(), 1);
        assert!(
            (bb[0].a_weighted_db - -23.01).abs() < 0.05,
            "the broadband figures must use the same energy mean, got {:.2}",
            bb[0].a_weighted_db
        );
    }

    /// The hour's min and max widen; they are never replaced.
    ///
    /// The `MIN`/`MAX` in the upsert is the whole of this, and writing
    /// `excluded.min_db` instead — which is what an upsert usually wants —
    /// would make the hour report only its most recent observation's range
    /// while still counting every sample.
    #[test]
    fn the_hourly_range_widens_and_is_never_replaced() {
        let conn = db();
        record_observation(
            &conn,
            "2026-05-01",
            7,
            "local",
            &[obs(1000.0, -40.0, -60.0, -35.0)],
            broadband(-40.0, -40.0),
        )
        .expect("first");
        // A later, narrower observation: its min is higher and its max lower.
        record_observation(
            &conn,
            "2026-05-01",
            7,
            "local",
            &[obs(1000.0, -41.0, -45.0, -38.0)],
            broadband(-41.0, -41.0),
        )
        .expect("second");

        let rows = latest_hour(&conn, "local").expect("read");
        assert!(
            (rows[0].min_db - -60.0).abs() < 0.01,
            "min is {:.1}; the quietest second of the hour was -60.0 and a later, louder \
             observation must not raise it",
            rows[0].min_db
        );
        assert!(
            (rows[0].max_db - -35.0).abs() < 0.01,
            "max is {:.1}; the loudest second of the hour was -35.0",
            rows[0].max_db
        );
    }

    /// Different hours, sources and bands are different buckets.
    #[test]
    fn buckets_are_keyed_on_date_hour_source_and_band() {
        let conn = db();
        let one = |c: &Connection, date: &str, hour: u8, source: &str, band: f32| {
            record_observation(
                c,
                date,
                hour,
                source,
                &[obs(band, -40.0, -40.0, -40.0)],
                broadband(-40.0, -40.0),
            )
            .expect("record");
        };
        one(&conn, "2026-05-01", 7, "local", 1000.0);
        one(&conn, "2026-05-01", 7, "local", 2000.0);
        one(&conn, "2026-05-01", 8, "local", 1000.0);
        one(&conn, "2026-05-01", 7, "garden", 1000.0);

        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM sound_levels", [], |r| r.get(0))
            .expect("count");
        assert_eq!(
            total, 4,
            "each of the four differs in exactly one key column"
        );

        // `latest_hour` scopes to one source and to its newest hour only.
        let latest = latest_hour(&conn, "local").expect("read");
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].hour, 8);
        assert!((latest[0].band_hz - 1000.0).abs() < 0.01);
    }

    /// `latest_hour` returns bands in ascending frequency order.
    ///
    /// A chart draws them in the order it receives them; unordered bands make
    /// a spectrum that looks like noise.
    #[test]
    fn latest_hour_orders_bands_by_frequency() {
        let conn = db();
        record_observation(
            &conn,
            "2026-05-01",
            7,
            "local",
            &[
                obs(8000.0, -50.0, -50.0, -50.0),
                obs(125.0, -30.0, -30.0, -30.0),
                obs(1000.0, -40.0, -40.0, -40.0),
            ],
            broadband(-40.0, -40.0),
        )
        .expect("record");

        let rows = latest_hour(&conn, "local").expect("read");
        let order: Vec<f32> = rows.iter().map(|r| r.band_hz).collect();
        let expected = [125.0_f32, 1000.0, 8000.0];
        assert_eq!(order.len(), expected.len(), "got {order:?}");
        for (got, want) in order.iter().zip(expected.iter()) {
            assert!(
                (got - want).abs() < 0.01,
                "got {order:?}, expected {expected:?}"
            );
        }
    }

    /// A source that has never been sampled reads back empty, not an error.
    #[test]
    fn an_unsampled_source_reads_back_empty() {
        let conn = db();
        assert!(latest_hour(&conn, "never-seen").expect("read").is_empty());
        assert!(recent_bands(&conn, 10).expect("read").is_empty());
        assert!(recent_broadband(&conn, 10).expect("read").is_empty());
    }

    /// Pruning removes old buckets from both tables and keeps recent ones.
    ///
    /// Both halves: a prune that deleted everything would satisfy "old rows
    /// are gone" perfectly.
    #[test]
    fn prune_removes_old_buckets_and_keeps_recent_ones() {
        let conn = db();
        record_observation(
            &conn,
            "2020-01-01",
            7,
            "local",
            &[obs(1000.0, -40.0, -40.0, -40.0)],
            broadband(-40.0, -40.0),
        )
        .expect("old");
        let today: String = conn
            .query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))
            .expect("today");
        record_observation(
            &conn,
            &today,
            7,
            "local",
            &[obs(1000.0, -40.0, -40.0, -40.0)],
            broadband(-40.0, -40.0),
        )
        .expect("new");

        let removed = prune(&conn, 30).expect("prune");
        assert_eq!(removed, 1, "exactly the 2020 band row should go");

        let bands: i64 = conn
            .query_row("SELECT COUNT(*) FROM sound_levels", [], |r| r.get(0))
            .expect("count");
        let broad: i64 = conn
            .query_row("SELECT COUNT(*) FROM sound_level_broadband", [], |r| {
                r.get(0)
            })
            .expect("count");
        assert_eq!(bands, 1, "today's band row must survive");
        assert_eq!(broad, 1, "and its broadband row with it");
    }

    /// A failure partway through an observation leaves no partial hour.
    ///
    /// The write is one transaction precisely so a bucket cannot end up with
    /// some bands counting more observations than others, which would make
    /// every energy mean in the hour wrong by a different amount.
    #[test]
    fn a_rejected_band_rolls_back_the_whole_observation() {
        let conn = db();
        // NaN cannot be stored as a REAL primary key component in a way that
        // round-trips, but a NULL-violating write is simpler to provoke: drive
        // the same insert with a band count of zero first to establish the
        // baseline, then corrupt the table so the second band fails.
        record_observation(
            &conn,
            "2026-05-01",
            7,
            "local",
            &[obs(1000.0, -40.0, -40.0, -40.0)],
            broadband(-40.0, -40.0),
        )
        .expect("baseline");

        conn.execute(
            "CREATE TRIGGER reject_2k BEFORE INSERT ON sound_levels
             WHEN NEW.band_hz = 2000.0
             BEGIN SELECT RAISE(ABORT, 'nope'); END",
            [],
        )
        .expect("trigger");

        let result = record_observation(
            &conn,
            "2026-05-01",
            7,
            "local",
            &[
                obs(1000.0, -10.0, -10.0, -10.0),
                obs(2000.0, -10.0, -10.0, -10.0),
            ],
            broadband(-10.0, -10.0),
        );
        assert!(result.is_err(), "the rejected band must fail the write");

        let rows = latest_hour(&conn, "local").expect("read");
        assert_eq!(rows.len(), 1, "no partial band should have landed");
        assert_eq!(
            rows[0].samples, 1,
            "the 1 kHz band must still show one observation, not two"
        );
        let broad: i64 = conn
            .query_row(
                "SELECT samples FROM sound_level_broadband WHERE source = 'local'",
                [],
                |r| r.get(0),
            )
            .expect("broadband");
        assert_eq!(broad, 1, "the broadband row must not have been advanced");
    }
}
