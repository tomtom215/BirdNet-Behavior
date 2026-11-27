//! What the station knew about its own clock while it listened, and the
//! effort ledger by UTC hour (RES-12, RES-13, OOB-17).
//!
//! # The clock record
//!
//! A Raspberry Pi has no battery-backed clock. After a power cut with no
//! network it starts from the last time it saved, which is plausible, so
//! nothing refuses it, and every recording is stamped with a time that may be
//! hours or days late. Whether the system *says* its clock is synchronised is
//! knowable (`timedatectl`), and so is whether the clock moved against elapsed
//! time (the boot-time clock in `/proc/uptime` cannot be set). Neither says how
//! wrong an unsynchronised clock is; nothing on the station can.
//!
//! `clock_observations` keeps what was known, one row when it changes:
//!
//! * `state`: `synced`, `unsynced`, `unknown` (nothing could answer: a
//!   container without systemd), or `unset` (the clock reads a date before
//!   this software existed);
//! * `authority`: what answered;
//! * `boot_id` and `uptime_secs`: the boot the reading was taken in, and the
//!   time since that boot, so a step is the wall clock's movement less the
//!   elapsed time between two readings of one boot;
//! * `reason`: `first`, `changed`, `boot` or `step`.
//!
//! Between rows the state is the last row's. [`ClockHistory::state_at`]
//! answers "under which clock state was this instant stamped", which is what
//! an export needs per detection. An instant before the first row is
//! `unknown`, never `synced`.
//!
//! One limit is plain: the instants being looked up were stamped by the same
//! clock. Across a backward step two stretches of time carry the same
//! readings, and the lookup takes the later state for both. The step itself
//! is recorded, with its size, so a study can set those rows aside.
//!
//! # Effort by UTC hour
//!
//! `recording_effort_hourly` is keyed on the local date and hour, so the
//! repeated autumn hour holds two hours of listening in one row and the
//! skipped spring hour none. `recording_effort_utc` holds the same two
//! quantities keyed on the UTC hour's start, with the time a source was up
//! split by the clock state while it was. The time up under an unknown state
//! is `seconds_up` less the other two.

use rusqlite::{Connection, OptionalExtension, params};

use crate::sqlite::connection::DbError;

/// How far the wall clock may move against elapsed time between two readings
/// of one boot before it is recorded as a step, in seconds.
///
/// Readings are stored to the whole second, so two readings can disagree by
/// up to a second with no step at all; two seconds is the smallest threshold
/// that cannot fire on rounding alone. A synchronised clock is slewed in
/// milliseconds; an unsynchronised one is not adjusted at all; so a movement
/// past this is someone, or something, setting the clock.
pub const STEP_THRESHOLD_SECS: f64 = 2.0;

/// The clock's state as the system reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClockState {
    /// The system reports its clock synchronised to a time source.
    Synced,
    /// The system reports that it is not.
    Unsynced,
    /// Nothing could answer, or nothing was recorded for the instant.
    Unknown,
    /// The clock reads a date before this software existed.
    Unset,
}

impl ClockState {
    /// The value stored and exported.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::Unsynced => "unsynced",
            Self::Unknown => "unknown",
            Self::Unset => "unset",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "synced" => Self::Synced,
            "unsynced" => Self::Unsynced,
            "unset" => Self::Unset,
            _ => Self::Unknown,
        }
    }

    /// Whether a time stamped under this state can be trusted as the time of
    /// day. Only `synced` is; `unknown` is not "probably fine".
    #[must_use]
    pub const fn trusted(self) -> bool {
        matches!(self, Self::Synced)
    }
}

/// One reading of the clock, taken by the caller.
#[derive(Debug, Clone, PartialEq)]
pub struct ClockReading {
    /// The wall clock, Unix seconds.
    pub at_utc: i64,
    /// What the system says.
    pub state: ClockState,
    /// What answered: `timedatectl`, `timesync-file`, or `none`.
    pub authority: &'static str,
    /// `/proc/sys/kernel/random/boot_id`, when it could be read.
    pub boot_id: Option<String>,
    /// Seconds since boot (`/proc/uptime`), when it could be read.
    pub uptime_secs: Option<f64>,
}

/// One stored row.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ClockObservation {
    /// Wall clock when it was seen, Unix seconds.
    pub at_utc: i64,
    /// The state.
    pub state: ClockState,
    /// What answered.
    pub authority: String,
    /// Why a row was written.
    pub reason: String,
    /// The boot it was seen in.
    pub boot_id: Option<String>,
    /// For a `step`: how far the wall clock moved against elapsed time,
    /// seconds, positive forward.
    pub step_secs: Option<f64>,
}

/// Write `reading` when it says something the last row does not: the first
/// reading ever, a changed state, a new boot, or a step. Returns the reason a
/// row was written, or `None` when it was not.
///
/// # Errors
///
/// Any `SQLite` error.
#[allow(
    clippy::cast_precision_loss,
    reason = "Unix seconds are far below 2^52"
)]
pub fn observe(conn: &Connection, reading: &ClockReading) -> Result<Option<&'static str>, DbError> {
    let last: Option<(i64, String, Option<String>, Option<f64>)> = conn
        .query_row(
            "SELECT at_utc, state, boot_id, uptime_secs FROM clock_observations
              ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let mut step = None;
    let reason = match last {
        None => Some("first"),
        Some((at, state, boot, uptime)) => {
            let same_boot = matches!((&boot, &reading.boot_id), (Some(a), Some(b)) if a == b);
            if let (true, Some(then), Some(now)) = (same_boot, uptime, reading.uptime_secs) {
                let moved = (reading.at_utc - at) as f64 - (now - then);
                if moved.abs() >= STEP_THRESHOLD_SECS {
                    step = Some(moved);
                }
            }
            if !same_boot && boot != reading.boot_id {
                Some("boot")
            } else if step.is_some() {
                Some("step")
            } else if ClockState::parse(&state) != reading.state {
                Some("changed")
            } else {
                None
            }
        }
    };
    if let Some(reason) = reason {
        conn.execute(
            "INSERT INTO clock_observations (at_utc, state, authority, reason, boot_id, uptime_secs, step_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                reading.at_utc,
                reading.state.as_str(),
                reading.authority,
                reason,
                reading.boot_id,
                reading.uptime_secs,
                step,
            ],
        )?;
    }
    Ok(reason)
}

/// The most recent `limit` rows, newest first.
///
/// # Errors
///
/// Any `SQLite` error.
pub fn recent_observations(
    conn: &Connection,
    limit: u32,
) -> Result<Vec<ClockObservation>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT at_utc, state, authority, reason, boot_id, step_secs FROM clock_observations
          ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit], row_to_observation)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_to_observation(r: &rusqlite::Row<'_>) -> rusqlite::Result<ClockObservation> {
    Ok(ClockObservation {
        at_utc: r.get(0)?,
        state: ClockState::parse(&r.get::<_, String>(1)?),
        authority: r.get(2)?,
        reason: r.get(3)?,
        boot_id: r.get(4)?,
        step_secs: r.get(5)?,
    })
}

/// Every clock row in memory, for attributing many instants at once.
#[derive(Debug, Clone, Default)]
pub struct ClockHistory {
    /// `(at_utc, state)`, in the order written.
    rows: Vec<(i64, ClockState)>,
}

impl ClockHistory {
    /// Read every row.
    ///
    /// # Errors
    ///
    /// Any `SQLite` error.
    pub fn load(conn: &Connection) -> Result<Self, DbError> {
        let mut stmt = conn.prepare("SELECT at_utc, state FROM clock_observations ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, ClockState::parse(&r.get::<_, String>(1)?)))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { rows })
    }

    /// The state an instant was stamped under: the latest row, in the order
    /// written, at or before it. [`ClockState::Unknown`] before the first.
    #[must_use]
    pub fn state_at(&self, utc: i64) -> ClockState {
        self.rows
            .iter()
            .rev()
            .find(|(at, _)| *at <= utc)
            .map_or(ClockState::Unknown, |(_, s)| *s)
    }
}

/// The start of the UTC hour holding `utc`.
#[must_use]
pub const fn hour_start(utc: i64) -> i64 {
    utc.div_euclid(3600) * 3600
}

/// Credit `seconds` of `source` being up to the UTC hour holding `at_utc`,
/// split by the clock's state: `Some(true)` synchronised, `Some(false)` not,
/// `None` unknown (counted in `seconds_up` only).
///
/// # Errors
///
/// Any `SQLite` error.
pub fn credit_up_utc(
    conn: &Connection,
    at_utc: i64,
    source: &str,
    seconds: f64,
    clock_synced: Option<bool>,
) -> Result<(), DbError> {
    let synced = if clock_synced == Some(true) {
        seconds
    } else {
        0.0
    };
    let unsynced = if clock_synced == Some(false) {
        seconds
    } else {
        0.0
    };
    conn.execute(
        "INSERT INTO recording_effort_utc
             (hour_start_utc, source, seconds_up, seconds_up_clock_synced, seconds_up_clock_unsynced)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(hour_start_utc, source) DO UPDATE SET
             seconds_up = seconds_up + excluded.seconds_up,
             seconds_up_clock_synced = seconds_up_clock_synced + excluded.seconds_up_clock_synced,
             seconds_up_clock_unsynced = seconds_up_clock_unsynced + excluded.seconds_up_clock_unsynced",
        params![hour_start(at_utc), source, seconds, synced, unsynced],
    )?;
    Ok(())
}

/// Credit `seconds` of analysed audio of `source` to the UTC hour holding
/// `at_utc`, the instant the audio began.
///
/// # Errors
///
/// Any `SQLite` error.
pub fn credit_analysed_utc(
    conn: &Connection,
    at_utc: i64,
    source: &str,
    seconds: f64,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO recording_effort_utc (hour_start_utc, source, seconds_analysed)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(hour_start_utc, source) DO UPDATE SET
             seconds_analysed = seconds_analysed + excluded.seconds_analysed",
        params![hour_start(at_utc), source, seconds],
    )?;
    Ok(())
}

/// One row of the UTC ledger.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EffortUtcHour {
    /// The hour's start, Unix seconds UTC.
    pub hour_start_utc: i64,
    /// The source.
    pub source: String,
    /// Seconds of audio analysed, by the instant the audio began.
    pub seconds_analysed: f64,
    /// Seconds the source's capture was up.
    pub seconds_up: f64,
    /// Of those, while the system said its clock was synchronised.
    pub seconds_up_clock_synced: f64,
    /// Of those, while it said it was not.
    pub seconds_up_clock_unsynced: f64,
}

impl EffortUtcHour {
    /// Time up while nothing could say how the clock stood.
    #[must_use]
    pub fn seconds_up_clock_unknown(&self) -> f64 {
        (self.seconds_up - self.seconds_up_clock_synced - self.seconds_up_clock_unsynced).max(0.0)
    }
}

/// The UTC ledger over `[from_utc, to_utc)`, oldest first.
///
/// # Errors
///
/// Any `SQLite` error.
pub fn effort_utc_hours(
    conn: &Connection,
    from_utc: i64,
    to_utc: i64,
) -> Result<Vec<EffortUtcHour>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT hour_start_utc, source, seconds_analysed, seconds_up, seconds_up_clock_synced,
                seconds_up_clock_unsynced
           FROM recording_effort_utc
          WHERE hour_start_utc >= ?1 AND hour_start_utc < ?2
          ORDER BY hour_start_utc, source",
    )?;
    let rows = stmt
        .query_map([from_utc, to_utc], |r| {
            Ok(EffortUtcHour {
                hour_start_utc: r.get(0)?,
                source: r.get(1)?,
                seconds_analysed: r.get(2)?,
                seconds_up: r.get(3)?,
                seconds_up_clock_synced: r.get(4)?,
                seconds_up_clock_unsynced: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&c).unwrap();
        c
    }

    fn reading(at: i64, state: ClockState, boot: &str, uptime: f64) -> ClockReading {
        ClockReading {
            at_utc: at,
            state,
            authority: "timedatectl",
            boot_id: Some(boot.to_owned()),
            uptime_secs: Some(uptime),
        }
    }

    #[test]
    fn a_row_is_written_only_when_the_reading_says_something_new() {
        let c = db();
        let t = 1_790_000_000;
        assert_eq!(
            observe(&c, &reading(t, ClockState::Unsynced, "a", 100.0)).unwrap(),
            Some("first")
        );
        assert_eq!(
            observe(&c, &reading(t + 300, ClockState::Unsynced, "a", 400.0)).unwrap(),
            None
        );
        assert_eq!(
            observe(&c, &reading(t + 600, ClockState::Synced, "a", 700.0)).unwrap(),
            Some("changed")
        );
        assert_eq!(
            observe(&c, &reading(t + 900, ClockState::Synced, "b", 10.0)).unwrap(),
            Some("boot"),
            "a new boot is written even when the state is the same"
        );
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM clock_observations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3);
    }

    /// The acceptance gate RES-12 names: a step of +3600 s writes a `step`.
    #[test]
    fn a_clock_set_forward_an_hour_is_a_step_of_an_hour() {
        let c = db();
        let t = 1_790_000_000;
        observe(&c, &reading(t, ClockState::Unsynced, "a", 100.0)).unwrap();
        // 300 s elapsed by the boot clock, 3 900 s by the wall clock.
        assert_eq!(
            observe(&c, &reading(t + 3_900, ClockState::Unsynced, "a", 400.0)).unwrap(),
            Some("step")
        );
        let last = &recent_observations(&c, 1).unwrap()[0];
        assert_eq!(last.reason, "step");
        let step = last.step_secs.unwrap();
        assert!((step - 3_600.0).abs() < 1e-9, "{step}");
        // Counterpart: one second of rounding against the step row is not a
        // step (301 s by the wall clock, 300 s by the boot clock).
        assert_eq!(
            observe(&c, &reading(t + 4_201, ClockState::Unsynced, "a", 700.0)).unwrap(),
            None
        );
        // And a backward step is a step too.
        assert_eq!(
            observe(&c, &reading(t + 4_000, ClockState::Unsynced, "a", 1_000.0)).unwrap(),
            Some("step")
        );
        assert!((recent_observations(&c, 1).unwrap()[0].step_secs.unwrap() + 500.0).abs() < 1e-9);
    }

    #[test]
    fn an_instant_is_judged_by_the_state_in_force_and_before_any_row_is_unknown() {
        let c = db();
        observe(&c, &reading(1_000, ClockState::Unsynced, "a", 1.0)).unwrap();
        observe(&c, &reading(2_000, ClockState::Synced, "a", 1_001.0)).unwrap();
        let h = ClockHistory::load(&c).unwrap();
        assert_eq!(h.state_at(999), ClockState::Unknown);
        assert_eq!(h.state_at(1_000), ClockState::Unsynced);
        assert_eq!(h.state_at(1_999), ClockState::Unsynced);
        assert_eq!(h.state_at(2_000), ClockState::Synced);
        assert!(!ClockState::Unknown.trusted());
        assert!(ClockState::Synced.trusted());
    }

    #[test]
    fn up_time_is_split_by_the_clock_and_keyed_on_the_utc_hour() {
        let c = db();
        let h = 1_792_886_400; // 2026-10-25 01:00Z, an hour Berlin lives twice
        credit_up_utc(&c, h + 10, "mic", 300.0, Some(true)).unwrap();
        credit_up_utc(&c, h + 400, "mic", 300.0, Some(false)).unwrap();
        credit_up_utc(&c, h + 800, "mic", 300.0, None).unwrap();
        credit_up_utc(&c, h - 1, "mic", 300.0, Some(true)).unwrap();
        credit_analysed_utc(&c, h + 5, "mic", 15.0).unwrap();
        let rows = effort_utc_hours(&c, h - 3_600, h + 3_600).unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        let r = &rows[1];
        assert_eq!(r.hour_start_utc, h);
        assert!((r.seconds_up - 900.0).abs() < 1e-9);
        assert!((r.seconds_up_clock_synced - 300.0).abs() < 1e-9);
        assert!((r.seconds_up_clock_unsynced - 300.0).abs() < 1e-9);
        assert!((r.seconds_up_clock_unknown() - 300.0).abs() < 1e-9);
        assert!((r.seconds_analysed - 15.0).abs() < 1e-9);
        assert_eq!(
            hour_start(-1),
            -3_600,
            "an hour before the epoch rounds down"
        );
    }
}
