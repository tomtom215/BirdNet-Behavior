//! Species co-occurrence and correlation queries.
//!
//! Analyses which species tend to appear together: in the same five-minute
//! block for the pair views, on the same day for the companion lookup.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

/// Two species heard together, and how much of their time that is.
///
/// "Together" is the same five-minute block of the same day (`Time` 07:00–07:04
/// is one block). The strength of a pair is its [`overlap`](Self::overlap):
/// shared blocks as a share of the blocks in which either was heard, so two
/// common birds are not a strong pair merely for being common (`ANA7`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeciesPair {
    /// First species common name (alphabetically first).
    pub species_a: String,
    /// Second species common name.
    pub species_b: String,
    /// Five-minute blocks in which both were heard.
    pub shared_blocks: i64,
    /// Blocks in which `species_a` was heard.
    pub blocks_a: i64,
    /// Blocks in which `species_b` was heard.
    pub blocks_b: i64,
    /// `shared / (blocks_a + blocks_b − shared)`, from 0 to 1 (Jaccard).
    pub overlap: f64,
}

/// A species heard on the same days as another (the trigger), and how much of their days that is.
///
/// "The same day" is the same calendar day, in either order: not *after* the trigger, which an
/// earlier version of this comment said. The strength is [`overlap`](Self::overlap), the shared
/// days as a share of the days either was heard, so a resident heard every day is not every
/// species' first companion merely for being common (UIC-04).
#[derive(Debug, Clone, serde::Serialize)]
pub struct FollowOn {
    /// The trigger species.
    pub trigger: String,
    /// A species heard on at least one of the trigger's days.
    pub companion: String,
    /// Days on which both were heard.
    pub shared_days: i64,
    /// Days on which the trigger was heard, in the window.
    pub trigger_days: i64,
    /// Days on which the companion was heard, in the window.
    pub companion_days: i64,
    /// `shared / (trigger_days + companion_days − shared)`, from 0 to 1 (Jaccard).
    pub overlap: f64,
    /// Average confidence of the companion's detections on the shared days.
    pub avg_confidence: f64,
}

/// The `limit` strongest pairs over the last `days` days (today included),
/// strongest [`overlap`](SpeciesPair::overlap) first.
///
/// Only pairs sharing at least `min_shared` blocks are returned: with fewer,
/// two birds heard once, together, would rank as perfectly associated.
///
/// It used to count shared *days*, which is not what the page says ("heard
/// within five minutes of each other"), and ranks a dawn resident with a dusk
/// one as the strongest pair on the station.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn top_cooccurrence_pairs(
    conn: &Connection,
    days: u32,
    limit: usize,
    min_shared: u32,
) -> Result<Vec<SpeciesPair>, DbError> {
    use std::collections::HashMap;

    // One row per (block, species), in block order. The block key is an
    // integer — Julian day × 288 + five-minute slot — because the pair count
    // below groups on it. Counting pairs here rather than with a SQL self-join
    // measured 2.2–2.9 s for the join against 0.8 s for this scan on a
    // synthetic year of 547 500 detections (x86, SQLite 3.45).
    let mut stmt = conn.prepare(
        "SELECT DISTINCT
                CAST(julianday(Date) AS INTEGER) * 288
                    + CAST(SUBSTR(Time, 1, 2) AS INTEGER) * 12
                    + CAST(SUBSTR(Time, 4, 2) AS INTEGER) / 5 AS k,
                Com_Name
         FROM detections_analytic
         WHERE Date >= DATE('now', 'localtime', '-' || (?1 - 1) || ' days')
         ORDER BY k",
    )?;
    let mut rows = stmt.query(params![days])?;

    let mut names: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut blocks_of: Vec<i64> = Vec::new();
    let mut shared: HashMap<(usize, usize), i64> = HashMap::new();
    let mut block: Vec<usize> = Vec::new();
    let mut current: Option<i64> = None;
    let flush = |block: &mut Vec<usize>, shared: &mut HashMap<(usize, usize), i64>| {
        for (x, &a) in block.iter().enumerate() {
            for &b in &block[x + 1..] {
                *shared.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        block.clear();
    };
    while let Some(row) = rows.next()? {
        let k: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        if current != Some(k) {
            flush(&mut block, &mut shared);
            current = Some(k);
        }
        let id = *index.entry(name.clone()).or_insert_with(|| {
            names.push(name);
            blocks_of.push(0);
            names.len() - 1
        });
        blocks_of[id] += 1;
        block.push(id);
    }
    flush(&mut block, &mut shared);

    let min_shared = i64::from(min_shared);
    let mut pairs: Vec<SpeciesPair> = shared
        .into_iter()
        .filter(|&(_, n)| n >= min_shared)
        .map(|((a, b), n)| {
            let (a, b) = if names[a] <= names[b] { (a, b) } else { (b, a) };
            #[allow(clippy::cast_precision_loss)]
            let overlap = n as f64 / (blocks_of[a] + blocks_of[b] - n) as f64;
            SpeciesPair {
                species_a: names[a].clone(),
                species_b: names[b].clone(),
                shared_blocks: n,
                blocks_a: blocks_of[a],
                blocks_b: blocks_of[b],
                overlap,
            }
        })
        .collect();
    pairs.sort_by(|x, y| {
        y.overlap
            .total_cmp(&x.overlap)
            .then(y.shared_blocks.cmp(&x.shared_blocks))
            .then_with(|| x.species_a.cmp(&y.species_a))
            .then_with(|| x.species_b.cmp(&y.species_b))
    });
    pairs.truncate(limit);
    Ok(pairs)
}

/// The `limit` species heard on the same days as `trigger_species` over the last `days` days,
/// strongest [`overlap`](FollowOn::overlap) first (then most shared days, then surest, then by
/// name).
///
/// It used to rank by shared days alone, the measure the pair view left for the same reason:
/// the commonest residents share most of anyone's days, so they were every species' first
/// "companions" and the answer barely depended on the species asked about.
///
/// # Errors
///
/// Returns [`DbError`] on `SQLite` failure.
pub fn companion_species(
    conn: &Connection,
    trigger_species: &str,
    days: u32,
    limit: usize,
) -> Result<Vec<FollowOn>, DbError> {
    let mut stmt = conn.prepare(
        "WITH win AS (
            SELECT Date, Com_Name, Confidence FROM detections_analytic
            WHERE Date >= DATE('now', 'localtime', '-' || ?2 || ' days')
         ),
         trigger_dates AS (
            SELECT DISTINCT Date FROM win WHERE Com_Name = ?1
         ),
         days_heard AS (
            SELECT Com_Name, COUNT(DISTINCT Date) AS n FROM win
            WHERE Com_Name != ?1
            GROUP BY Com_Name
         ),
         shared AS (
            SELECT w.Com_Name,
                   COUNT(DISTINCT w.Date) AS shared_days,
                   AVG(w.Confidence) AS avg_confidence
            FROM win w
            JOIN trigger_dates td ON w.Date = td.Date
            WHERE w.Com_Name != ?1
            GROUP BY w.Com_Name
         )
         SELECT ?1, s.Com_Name, s.shared_days, (SELECT COUNT(*) FROM trigger_dates), h.n,
                s.avg_confidence
         FROM shared s
         JOIN days_heard h ON h.Com_Name = s.Com_Name",
    )?;

    let mut rows = stmt
        .query_map(params![trigger_species, days], |row| {
            let shared_days: i64 = row.get(2)?;
            let trigger_days: i64 = row.get(3)?;
            let companion_days: i64 = row.get(4)?;
            let either = trigger_days + companion_days - shared_days;
            #[allow(clippy::cast_precision_loss)]
            let overlap = if either > 0 {
                shared_days as f64 / either as f64
            } else {
                0.0
            };
            Ok(FollowOn {
                trigger: row.get(0)?,
                companion: row.get(1)?,
                shared_days,
                trigger_days,
                companion_days,
                overlap,
                avg_confidence: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.sort_by(|x, y| {
        y.overlap
            .total_cmp(&x.overlap)
            .then(y.shared_days.cmp(&x.shared_days))
            .then(y.avg_confidence.total_cmp(&x.avg_confidence))
            .then_with(|| x.companion.cmp(&y.companion))
    });
    rows.truncate(limit);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// `ANA7`: "together" means heard in the same five-minute block, and a
    /// pair is ranked by how much of its time it shares, not by how common
    /// its members are.
    ///
    /// Two residents heard every day — one at dawn, one at dusk, never within
    /// hours of each other — shared every *day*, so the page's "heard within
    /// five minutes of each other" ranked them as the strongest pair on the
    /// station. A rarer pair heard a minute apart three times is the real
    /// association.
    #[test]
    fn together_means_the_same_five_minutes_not_the_same_day() {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        let mut rows = Vec::new();
        for back in 1..=10 {
            let d = format!("DATE('now', 'localtime', '-{back} days')");
            rows.push(format!("({d},'05:30:00','Turdus merula','Blackbird',0.9)"));
            rows.push(format!("({d},'20:30:00','Strix aluco','Tawny Owl',0.9)"));
            if back <= 3 {
                rows.push(format!("({d},'07:01:00','Sitta europaea','Nuthatch',0.9)"));
                rows.push(format!(
                    "({d},'07:02:00','Certhia familiaris','Treecreeper',0.9)"
                ));
            }
        }
        conn.execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES {};",
            rows.join(",")
        ))
        .unwrap();

        let pairs = top_cooccurrence_pairs(&conn, 30, 10, 1).unwrap();
        let names: Vec<(&str, &str)> = pairs
            .iter()
            .map(|p| (p.species_a.as_str(), p.species_b.as_str()))
            .collect();
        assert_eq!(
            names.first(),
            Some(&("Nuthatch", "Treecreeper")),
            "the pair heard a minute apart is the strongest: {names:?}"
        );
        assert!(
            !names.contains(&("Blackbird", "Tawny Owl")),
            "dawn and dusk residents are never together: {names:?}"
        );
    }

    fn setup() -> Connection {
        // Apply the full migration chain rather than a hand-coded CREATE TABLE:
        // ADR-16 flags the latter as the source of three of the PR #35 bugs
        // because the hand-coded schema silently drifts the moment a new
        // migration adds a column. The migration list is the single source
        // of truth.
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        // Dates are computed at insert time via SQLite's DATE('now', 'localtime', '-N days')
        // so the fixture stays within the 30-day window used by the queries
        // under test, regardless of when the suite runs.
        conn.execute_batch(
            "INSERT INTO detections
              (Date, Time, Sci_Name, Com_Name, Confidence,
               Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name)
            VALUES
              (DATE('now', 'localtime', '-7 days'),'07:00:00','A sp','Robin',  0.9, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-7 days'),'07:05:00','B sp','Wren',   0.8, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-7 days'),'08:00:00','C sp','Finch',  0.7, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-6 days'),'07:00:00','A sp','Robin',  0.9, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-6 days'),'07:10:00','B sp','Wren',   0.8, NULL,NULL,NULL,NULL,NULL,NULL,''),
              (DATE('now', 'localtime', '-5 days'),'07:00:00','A sp','Robin',  0.9, NULL,NULL,NULL,NULL,NULL,NULL,'');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn top_pairs_basic() {
        let conn = setup();
        let pairs = top_cooccurrence_pairs(&conn, 30, 10, 1).unwrap();
        // Robin+Wren share 2 days, Robin+Finch share 1 day
        // Robin and Wren share two days but never a five-minute block (07:00
        // and 07:05, 07:00 and 07:10), so they are not a pair.
        assert!(pairs.is_empty(), "{pairs:?}");
    }

    #[test]
    fn companion_species_robin() {
        let conn = setup();
        let companions = companion_species(&conn, "Robin", 30, 10).unwrap();
        // Wren appears on 2 of 3 robin days, Finch on 1
        assert!(!companions.is_empty());
        assert_eq!(companions[0].companion, "Wren");
        assert_eq!(companions[0].shared_days, 2);
    }

    /// UIC-04. A companion is ranked by how much of their days the two share, not by how
    /// many: a resident heard every day shares every one of the trigger's days and is no
    /// companion for it. Nutcracker heard on ten days; Pine Grosbeak on eight, each one a
    /// Nutcracker day; House Sparrow on every day of the window.
    fn nutcracker_fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        let mut rows = Vec::new();
        for back in 1..=29 {
            let d = format!("DATE('now', 'localtime', '-{back} days')");
            rows.push(format!(
                "({d},'09:00:00','Passer domesticus','House Sparrow',0.9)"
            ));
            if back <= 10 {
                rows.push(format!(
                    "({d},'07:00:00','Nucifraga caryocatactes','Nutcracker',0.9)"
                ));
            }
            if back <= 8 {
                rows.push(format!(
                    "({d},'07:30:00','Pinicola enucleator','Pine Grosbeak',0.9)"
                ));
            }
        }
        conn.execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES {};",
            rows.join(",")
        ))
        .unwrap();
        conn
    }

    #[test]
    fn a_resident_heard_every_day_is_not_the_first_companion() {
        let conn = nutcracker_fixture();
        let names: Vec<String> = companion_species(&conn, "Nutcracker", 30, 10)
            .unwrap()
            .into_iter()
            .map(|c| c.companion)
            .collect();
        // Counterpart: the resident is still listed, below the true companion.
        assert_eq!(names, ["Pine Grosbeak", "House Sparrow"]);
    }

    /// The numbers a page prints beside a companion: "8 of the trigger's 10 days", and the share.
    #[test]
    fn a_companion_carries_the_days_behind_its_rank() {
        let conn = nutcracker_fixture();
        let c = companion_species(&conn, "Nutcracker", 30, 10).unwrap();
        let (grosbeak, sparrow) = (&c[0], &c[1]);
        assert_eq!(
            (
                grosbeak.shared_days,
                grosbeak.trigger_days,
                grosbeak.companion_days
            ),
            (8, 10, 8)
        );
        assert!(
            (grosbeak.overlap - 0.8).abs() < 1e-9,
            "{}",
            grosbeak.overlap
        );
        assert_eq!(
            (
                sparrow.shared_days,
                sparrow.trigger_days,
                sparrow.companion_days
            ),
            (10, 10, 29)
        );
        assert!(
            (sparrow.overlap - 10.0 / 29.0).abs() < 1e-9,
            "{}",
            sparrow.overlap
        );
        // The limit still cuts after the ranking, not before it.
        let one = companion_species(&conn, "Nutcracker", 30, 1).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].companion, "Pine Grosbeak");
    }
}
