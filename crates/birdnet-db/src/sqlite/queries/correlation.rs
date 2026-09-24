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

/// Species frequently seen *after* a given species on the same day.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FollowOn {
    /// The trigger species.
    pub trigger: String,
    /// Species commonly seen on the same day.
    pub companion: String,
    /// Days on which both appeared.
    pub shared_days: i64,
    /// Average confidence of companion detections.
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

/// Query species that commonly appear on the same day as `trigger_species`.
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
        "WITH trigger_dates AS (
            SELECT DISTINCT Date FROM detections_analytic
            WHERE Com_Name = ?1
              AND Date >= DATE('now', 'localtime', '-' || ?2 || ' days')
         )
         SELECT
            ?1 AS trigger,
            d.Com_Name AS companion,
            COUNT(DISTINCT d.Date) AS shared_days,
            AVG(d.Confidence) AS avg_confidence
         FROM detections_analytic d
         JOIN trigger_dates td ON d.Date = td.Date
         WHERE d.Com_Name != ?1
         GROUP BY d.Com_Name
         ORDER BY shared_days DESC, avg_confidence DESC
         LIMIT ?3",
    )?;

    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);

    let rows = stmt
        .query_map(params![trigger_species, days, limit_i64], |row| {
            Ok(FollowOn {
                trigger: row.get(0)?,
                companion: row.get(1)?,
                shared_days: row.get(2)?,
                avg_confidence: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

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
}
