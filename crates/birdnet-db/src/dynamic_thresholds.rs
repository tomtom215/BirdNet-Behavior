//! Persistence for the learned per-species thresholds.
//!
//! The rules live in `birdnet_core::detection::dynamic_threshold`; this is only
//! the table. `birdnet-db` does not depend on `birdnet-core`, so the row type
//! here is a plain record and the binary maps between the two.
//!
//! # Why anything is stored at all
//!
//! The state is "which species has this site confirmed present, and when does
//! that lapse". Held only in memory, a station that reboots nightly for a
//! backup would start every morning having forgotten, and the feature would
//! never accumulate past its first level.

use rusqlite::{Connection, params};

use crate::sqlite::DbError;

/// One species' learned state, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnedThreshold {
    /// Scientific name — the identity, matching the detection pipeline's key.
    pub sci_name: String,
    /// Level, `0..=3`; the meaning belongs to `birdnet-core`.
    pub level: u8,
    /// Confirmations counted in the current episode.
    pub confirmations: u32,
    /// Epoch milliseconds at which the level lapses.
    pub expires_at_ms: i64,
    /// Epoch milliseconds of the first confirmation of this episode.
    pub first_learned_ms: i64,
    /// Epoch milliseconds of the most recent confirmation.
    pub last_confirmed_ms: i64,
}

/// Replace the stored set with `rows`.
///
/// A whole-set replace rather than per-species upserts: the in-memory tracker
/// is the authority and a species it has dropped must not survive in the table.
/// Incremental writes would leave lapsed rows behind for a sweep that might not
/// run, and the table would grow to every species ever confirmed.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure. Transactional, so a failure leaves
/// the previous set intact rather than a partial one.
pub fn replace_all(conn: &Connection, rows: &[LearnedThreshold]) -> Result<(), DbError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM dynamic_thresholds", [])?;
    for row in rows {
        tx.execute(
            "INSERT INTO dynamic_thresholds
                 (sci_name, level, confirmations, expires_at_ms,
                  first_learned_ms, last_confirmed_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                row.sci_name,
                i64::from(row.level),
                i64::from(row.confirmations),
                row.expires_at_ms,
                row.first_learned_ms,
                row.last_confirmed_ms,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Every stored row, including any that have lapsed.
///
/// Lapsed rows are returned rather than filtered here so that the expiry rule
/// has one home. `birdnet-core` decides what "lapsed" means; a `WHERE
/// expires_at_ms > ?` here would be a second copy of that rule, in a different
/// language, that could disagree with it.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn load_all(conn: &Connection) -> Result<Vec<LearnedThreshold>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT sci_name, level, confirmations, expires_at_ms,
                first_learned_ms, last_confirmed_ms
           FROM dynamic_thresholds
          ORDER BY sci_name",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let level: i64 = r.get(1)?;
            let confirmations: i64 = r.get(2)?;
            Ok(LearnedThreshold {
                sci_name: r.get(0)?,
                level: u8::try_from(level).unwrap_or(0),
                confirmations: u32::try_from(confirmations).unwrap_or(0),
                expires_at_ms: r.get(3)?,
                first_learned_ms: r.get(4)?,
                last_confirmed_ms: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The stored levels of species that still have a detection the analytics show.
///
/// A learned level is a public fact about a species at this site ("it was heard
/// with high confidence, often enough to lower its bar"). The public read
/// routes answer from it, and a detection an owner later **rejected** is not one
/// the rest of the public pages count (`detections_analytic` leaves it out): the
/// level would keep saying a species was confirmed that the owner has since
/// said was not there. This returns only the rows whose species has at least
/// one detection in `detections_analytic`, which is the same visibility rule the
/// species pages use. The tracker's own table is not touched; this is a read
/// filter for the public surfaces.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn load_visible(conn: &Connection) -> Result<Vec<LearnedThreshold>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT d.sci_name, d.level, d.confirmations, d.expires_at_ms,
                d.first_learned_ms, d.last_confirmed_ms
           FROM dynamic_thresholds d
          WHERE EXISTS (SELECT 1 FROM detections_analytic a WHERE a.Sci_Name = d.sci_name)
          ORDER BY d.sci_name",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let level: i64 = r.get(1)?;
            let confirmations: i64 = r.get(2)?;
            Ok(LearnedThreshold {
                sci_name: r.get(0)?,
                level: u8::try_from(level).unwrap_or(0),
                confirmations: u32::try_from(confirmations).unwrap_or(0),
                expires_at_ms: r.get(3)?,
                first_learned_ms: r.get(4)?,
                last_confirmed_ms: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Delete one species' stored level, returning how many rows went (0 or 1).
///
/// The name is matched without regard to ASCII case, as the read route is: a
/// reset of `turdus merula` used to report success, record itself in the audit
/// trail and delete nothing, because the stored name is `Turdus merula`.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn delete_one(conn: &Connection, sci_name: &str) -> Result<usize, DbError> {
    Ok(conn.execute(
        "DELETE FROM dynamic_thresholds WHERE sci_name = ?1 COLLATE NOCASE",
        params![sci_name],
    )?)
}

/// Delete every stored level, returning how many rows went.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn delete_all(conn: &Connection) -> Result<usize, DbError> {
    Ok(conn.execute("DELETE FROM dynamic_thresholds", [])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

    fn row(name: &str, level: u8, expires: i64) -> LearnedThreshold {
        LearnedThreshold {
            sci_name: name.to_string(),
            level,
            confirmations: u32::from(level),
            expires_at_ms: expires,
            first_learned_ms: 0,
            last_confirmed_ms: expires - 1000,
        }
    }

    /// The table, the read route and the tracker must agree on what a name is:
    /// `turdus merula` is `Turdus merula`. The counterpart is that a different
    /// species is untouched.
    /// A species whose every detection was rejected is no longer a species the
    /// public pages may call confirmed; one with a surviving detection is.
    #[test]
    fn a_level_is_visible_only_while_a_detection_for_it_is_not_rejected() {
        let conn = db();
        replace_all(
            &conn,
            &[
                row("Turdus merula", 2, 5000),
                row("Strix aluco", 1, 9000),
                row("Pica pica", 1, 9000),
            ],
        )
        .expect("write");
        let add = |sci: &str, time: &str, verdict: Option<&str>| {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, review_verdict)
                 VALUES ('2026-05-01', ?2, ?1, ?1, 0.9, ?3)",
                params![sci, time, verdict],
            )
            .expect("insert");
        };
        add("Turdus merula", "06:00:00", None);
        add("Strix aluco", "06:00:00", Some("rejected"));
        // A second, kept detection of the blackbird, and a rejected one: the
        // species stays while any detection stands.
        add("Turdus merula", "07:00:00", Some("rejected"));
        // Pica pica has no detection at all.
        let seen: Vec<String> = load_visible(&conn)
            .unwrap()
            .into_iter()
            .map(|r| r.sci_name)
            .collect();
        assert_eq!(seen, ["Turdus merula"], "{seen:?}");
        // The counterpart: the unfiltered read still has all three, so the
        // tracker's own persistence is untouched.
        assert_eq!(load_all(&conn).unwrap().len(), 3);
    }

    #[test]
    fn deleting_one_ignores_the_case_of_the_name() {
        let conn = db();
        replace_all(
            &conn,
            &[row("Turdus merula", 2, 5000), row("Strix aluco", 1, 9000)],
        )
        .expect("write");
        assert_eq!(delete_one(&conn, "turdus merula").unwrap(), 1);
        assert_eq!(
            delete_one(&conn, "TURDUS MERULA").unwrap(),
            0,
            "already gone"
        );
        let left = load_all(&conn).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].sci_name, "Strix aluco", "another species went too");
    }

    #[test]
    fn deleting_one_or_all_reports_what_went() {
        let conn = db();
        replace_all(
            &conn,
            &[row("Turdus merula", 2, 5000), row("Strix aluco", 1, 9000)],
        )
        .expect("write");
        assert_eq!(delete_one(&conn, "Strix aluco").unwrap(), 1);
        assert_eq!(delete_one(&conn, "Strix aluco").unwrap(), 0);
        assert_eq!(load_all(&conn).unwrap().len(), 1);
        assert_eq!(delete_all(&conn).unwrap(), 1);
        assert_eq!(delete_all(&conn).unwrap(), 0);
    }

    #[test]
    fn a_set_round_trips_in_name_order() {
        let conn = db();
        replace_all(
            &conn,
            &[row("Turdus merula", 2, 5000), row("Strix aluco", 1, 9000)],
        )
        .expect("write");

        let back = load_all(&conn).expect("read");
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].sci_name, "Strix aluco");
        assert_eq!(back[0].level, 1);
        assert_eq!(back[0].expires_at_ms, 9000);
        assert_eq!(back[1].sci_name, "Turdus merula");
        assert_eq!(back[1].level, 2);
    }

    /// A replace really replaces: a species dropped by the tracker must not
    /// survive in the table.
    ///
    /// This is the whole reason the write is a whole-set replace rather than
    /// an upsert per species. Without it the table accumulates every species
    /// ever confirmed and a restart reloads them all.
    #[test]
    fn replacing_drops_what_is_no_longer_tracked() {
        let conn = db();
        replace_all(
            &conn,
            &[row("Strix aluco", 1, 9000), row("Turdus merula", 1, 9000)],
        )
        .expect("first");
        replace_all(&conn, &[row("Strix aluco", 2, 9000)]).expect("second");

        let back = load_all(&conn).expect("read");
        assert_eq!(back.len(), 1, "the blackbird should be gone");
        assert_eq!(back[0].sci_name, "Strix aluco");
        assert_eq!(back[0].level, 2, "and the owl should carry its new level");
    }

    /// Writing an empty set clears the table rather than being a no-op.
    #[test]
    fn an_empty_set_clears_the_table() {
        let conn = db();
        replace_all(&conn, &[row("Strix aluco", 1, 9000)]).expect("write");
        replace_all(&conn, &[]).expect("clear");
        assert!(load_all(&conn).expect("read").is_empty());
    }

    /// Lapsed rows come back, because expiry is not this layer's rule.
    ///
    /// If this filtered, the definition of "lapsed" would exist here as well
    /// as in `birdnet-core`, in a different language, and the two could
    /// disagree without either being obviously wrong.
    #[test]
    fn a_lapsed_row_is_returned_rather_than_filtered_here() {
        let conn = db();
        replace_all(&conn, &[row("Strix aluco", 1, 1)]).expect("write");
        let back = load_all(&conn).expect("read");
        assert_eq!(
            back.len(),
            1,
            "the storage layer must not apply the expiry rule"
        );
        assert_eq!(back[0].expires_at_ms, 1);
    }
}
