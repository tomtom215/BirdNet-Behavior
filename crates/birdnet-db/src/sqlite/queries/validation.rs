//! A validation sample: detections drawn at random, by a recorded seed, from a
//! recorded frame, in score strata, for review blind to the score (RES-07).
//!
//! # What this is, and what it is not
//!
//! The review queue is newest first and shows each detection's score, so the
//! verdicts it collects are neither a random sample nor independent of the
//! score. A *validation sample* is drawn here instead:
//!
//! * The **frame** is every detection row in a date range (optionally of named
//!   species), including rows a reviewer already rejected: it is the detector's
//!   output, not the station's accepted record. Its size is recorded.
//! * The frame is cut into **strata** by score (half-open bins `[lo, hi)`, the
//!   last closed at its top), optionally per species. A stratum's size `N_h` and
//!   the number drawn from it `n_h = min(per_stratum, N_h)` are recorded, so each
//!   item's **inclusion probability** `n_h / N_h` is known.
//! * Within a stratum, items are drawn without replacement by sorting the
//!   stratum on SHA-256(`seed`, the row's key) and taking the first `n_h`: the
//!   same seed and the same frame always draw the same items. Presentation order
//!   is shuffled the same way, so a reviewer cannot read the stratum from the
//!   position.
//! * The sample, its protocol (canonical JSON and digest) and its items are
//!   append-only. Verdicts are append-only too; the latest one per item counts.
//!
//! **Nothing here estimates accuracy.** A verdict sample supports a
//! design-weighted precision estimate *for the frame it was drawn from*
//! (Σ w·TP / Σ w·(TP+FP), `w = 1/π`), with an interval that accounts for the
//! design; the export gives the counts and weights, and the computation, its
//! assumptions and its interval are the analyst's. It says nothing about misses
//! (recall): every item is a detection, so a bird the detector never reported
//! cannot be drawn. A score is the model's output, not a probability.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};

use super::run_config::sha256_hex;
use crate::sqlite::connection::DbError;

/// The protocol's schema, recorded in it.
pub const VALIDATION_PROTOCOL_SCHEMA: &str = "validation-protocol-1";

/// Most items one sample may hold: a review session's worth, and a bound on the
/// transaction that writes it.
pub const MAX_SAMPLE_ITEMS: usize = 5_000;

/// Most rows a frame may hold.
pub const MAX_FRAME_ROWS: usize = 2_000_000;

/// The verdicts a validation reviewer gives.
pub const VERDICTS: [&str; 4] = ["present", "absent", "uncertain", "not_identifiable"];

/// How a sample is drawn. Recorded with the sample, verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleProtocol {
    /// First local date of the frame, `YYYY-MM-DD`, inclusive.
    pub from: String,
    /// Last local date of the frame, inclusive.
    pub to: String,
    /// Scientific names the frame is limited to; empty is every species.
    #[serde(default)]
    pub species: Vec<String>,
    /// Score bin edges, strictly increasing, from 0 to 1 inclusive.
    pub score_edges: Vec<f64>,
    /// Whether each species is its own set of strata.
    #[serde(default)]
    pub by_species: bool,
    /// Items drawn from each stratum, at most.
    pub per_stratum: u32,
    /// The seed. Choose it before looking at the frame, and record why.
    pub seed: u64,
    /// Whether the reviewer is shown no score and no model.
    #[serde(default = "default_blind")]
    pub blind: bool,
}

const fn default_blind() -> bool {
    true
}

/// Why a protocol was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError(pub String);

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProtocolError {}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

impl SampleProtocol {
    /// Check the protocol is one a sample can be drawn by.
    ///
    /// # Errors
    ///
    /// The first thing wrong with it.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let err = |m: &str| Err(ProtocolError(m.to_owned()));
        if !is_date(&self.from) || !is_date(&self.to) || self.from > self.to {
            return err("from and to are dates, YYYY-MM-DD, from not after to");
        }
        let e = &self.score_edges;
        if e.len() < 2
            || e.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || e.windows(2).any(|w| w[0] >= w[1])
        {
            return err("score_edges are at least two increasing numbers from 0 to 1");
        }
        if self.per_stratum == 0 {
            return err("per_stratum is at least 1");
        }
        Ok(())
    }

    /// Canonical JSON: the fields in declaration order with the schema first,
    /// species sorted.
    #[must_use]
    pub fn canonical_json(&self) -> String {
        let mut p = self.clone();
        p.species.sort();
        p.species.dedup();
        let body = serde_json::to_value(&p).unwrap_or_default();
        let mut map = BTreeMap::new();
        map.insert(
            "schema".to_owned(),
            serde_json::Value::String(VALIDATION_PROTOCOL_SCHEMA.into()),
        );
        if let serde_json::Value::Object(o) = body {
            for (k, v) in o {
                map.insert(k, v);
            }
        }
        serde_json::to_string(&map).unwrap_or_default()
    }

    /// The stratum a score (and species) falls in, `None` outside the edges.
    #[must_use]
    pub fn stratum_of(&self, sci_name: &str, score: f64) -> Option<String> {
        let e = &self.score_edges;
        let last = e.len() - 1;
        let i = (0..last).find(|&i| {
            score >= e[i] && (score < e[i + 1] || (i + 1 == last && score <= e[i + 1]))
        })?;
        let close = if i + 1 == last { ']' } else { ')' };
        let bin = format!("score[{},{}{close}", e[i], e[i + 1]);
        Some(if self.by_species {
            format!("{sci_name}|{bin}")
        } else {
            bin
        })
    }
}

/// A frame row: the key a detection is named by, its score and species.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameRow {
    /// Local date.
    pub date: String,
    /// Local time.
    pub time: String,
    /// Scientific name.
    pub sci_name: String,
    /// Clip file name, `''` when none.
    pub file_name: String,
    /// The detection's score.
    pub confidence: f64,
}

impl FrameRow {
    fn key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.date, self.time, self.sci_name, self.file_name
        )
    }
}

/// One drawn item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SampleItem {
    /// Presentation position, from 0.
    pub position: i64,
    /// Local date of the detection.
    pub date: String,
    /// Local time.
    pub time: String,
    /// Scientific name as detected.
    pub sci_name: String,
    /// Clip file name, `''` when none.
    pub file_name: String,
    /// The stratum it was drawn from.
    pub stratum: String,
    /// `N_h`, the stratum's size in the frame.
    pub stratum_size: i64,
    /// `n_h / N_h`.
    pub inclusion_probability: f64,
}

fn hash_rank(seed: u64, purpose: &str, key: &str) -> String {
    sha256_hex(format!("{seed}|{purpose}|{key}").as_bytes())
}

/// The frame of `protocol`, in key order.
///
/// # Errors
///
/// `DbError` on query failure.
pub fn frame(conn: &Connection, protocol: &SampleProtocol) -> Result<Vec<FrameRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Date, Time, Sci_Name, COALESCE(File_Name, ''), Confidence FROM detections
          WHERE Date >= ?1 AND Date <= ?2
          ORDER BY Date, Time, Sci_Name, COALESCE(File_Name, '')",
    )?;
    let wanted: std::collections::BTreeSet<&str> =
        protocol.species.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    let rows = stmt.query_map(params![protocol.from, protocol.to], |r| {
        Ok(FrameRow {
            date: r.get(0)?,
            time: r.get(1)?,
            sci_name: r.get(2)?,
            file_name: r.get(3)?,
            confidence: r.get(4)?,
        })
    })?;
    for row in rows {
        let row = row?;
        if wanted.is_empty() || wanted.contains(row.sci_name.as_str()) {
            out.push(row);
        }
        if out.len() > MAX_FRAME_ROWS {
            break;
        }
    }
    Ok(out)
}

/// Draw the sample from `frame` (pure): the same protocol and frame give the
/// same items in the same order.
#[must_use]
pub fn draw(protocol: &SampleProtocol, frame: &[FrameRow]) -> Vec<SampleItem> {
    let mut strata: BTreeMap<String, Vec<&FrameRow>> = BTreeMap::new();
    for row in frame {
        if let Some(s) = protocol.stratum_of(&row.sci_name, row.confidence) {
            strata.entry(s).or_default().push(row);
        }
    }
    let mut drawn = Vec::new();
    for (stratum, mut rows) in strata {
        let size = rows.len();
        let n = size.min(protocol.per_stratum as usize);
        rows.sort_by_cached_key(|r| (hash_rank(protocol.seed, "draw", &r.key()), r.key()));
        #[allow(clippy::cast_precision_loss)]
        let p = n as f64 / size as f64;
        for r in rows.into_iter().take(n) {
            drawn.push((stratum.clone(), size, p, r));
        }
    }
    drawn.sort_by_cached_key(|(_, _, _, r)| (hash_rank(protocol.seed, "order", &r.key()), r.key()));
    drawn
        .into_iter()
        .enumerate()
        .map(|(i, (stratum, size, p, r))| SampleItem {
            position: i64::try_from(i).unwrap_or(i64::MAX),
            date: r.date.clone(),
            time: r.time.clone(),
            sci_name: r.sci_name.clone(),
            file_name: r.file_name.clone(),
            stratum,
            stratum_size: i64::try_from(size).unwrap_or(i64::MAX),
            inclusion_probability: p,
        })
        .collect()
}

/// Why a sample could not be created.
#[derive(Debug)]
pub enum SampleError {
    /// The protocol was refused.
    Protocol(ProtocolError),
    /// The draw is larger than [`MAX_SAMPLE_ITEMS`] or the frame than
    /// [`MAX_FRAME_ROWS`].
    TooLarge(usize),
    /// The database.
    Db(DbError),
}

impl std::fmt::Display for SampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(e) => write!(f, "{e}"),
            Self::TooLarge(n) => write!(
                f,
                "the draw or its frame is too large ({n}); narrow the dates or lower per_stratum"
            ),
            Self::Db(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SampleError {}

impl From<DbError> for SampleError {
    fn from(e: DbError) -> Self {
        Self::Db(e)
    }
}

impl From<rusqlite::Error> for SampleError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.into())
    }
}

/// Draw and record a sample; its id.
///
/// # Errors
///
/// A refused protocol, a draw that is too large, or the database.
pub fn create_sample(conn: &Connection, protocol: &SampleProtocol) -> Result<i64, SampleError> {
    protocol.validate().map_err(SampleError::Protocol)?;
    let tx = conn.unchecked_transaction()?;
    let frame = frame(&tx, protocol)?;
    if frame.len() > MAX_FRAME_ROWS {
        return Err(SampleError::TooLarge(frame.len()));
    }
    let items = draw(protocol, &frame);
    if items.len() > MAX_SAMPLE_ITEMS {
        return Err(SampleError::TooLarge(items.len()));
    }
    let json = protocol.canonical_json();
    tx.execute(
        "INSERT INTO validation_samples (seed, blind, protocol_json, protocol_sha256, frame_size)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            // SQLite integers are signed; the seed's bits are kept.
            protocol.seed.cast_signed(),
            protocol.blind,
            json,
            sha256_hex(json.as_bytes()),
            i64::try_from(frame.len()).unwrap_or(i64::MAX),
        ],
    )?;
    let id = tx.last_insert_rowid();
    {
        let mut ins = tx.prepare(
            "INSERT INTO validation_sample_items
                 (sample_id, position, date, time, sci_name, file_name, stratum, stratum_size,
                  inclusion_probability)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for it in &items {
            ins.execute(params![
                id,
                it.position,
                it.date,
                it.time,
                it.sci_name,
                it.file_name,
                it.stratum,
                it.stratum_size,
                it.inclusion_probability
            ])?;
        }
    }
    tx.commit()?;
    Ok(id)
}

/// A recorded sample's header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sample {
    /// Its id.
    pub id: i64,
    /// When it was drawn, UTC.
    pub created_at: String,
    /// The seed.
    pub seed: u64,
    /// Whether review is blind.
    pub blind: bool,
    /// The protocol, canonical JSON.
    pub protocol_json: String,
    /// Its SHA-256.
    pub protocol_sha256: String,
    /// The frame's size when drawn.
    pub frame_size: i64,
}

/// One sample's header.
///
/// # Errors
///
/// `DbError` on query failure.
pub fn sample(conn: &Connection, id: i64) -> Result<Option<Sample>, DbError> {
    Ok(conn
        .query_row(
            "SELECT id, created_at, seed, blind, protocol_json, protocol_sha256, frame_size
               FROM validation_samples WHERE id = ?1",
            [id],
            |r| {
                Ok(Sample {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    seed: r.get::<_, i64>(2)?.cast_unsigned(),
                    blind: r.get(3)?,
                    protocol_json: r.get(4)?,
                    protocol_sha256: r.get(5)?,
                    frame_size: r.get(6)?,
                })
            },
        )
        .optional()?)
}

/// Every sample, newest first.
///
/// # Errors
///
/// `DbError` on query failure.
pub fn samples(conn: &Connection) -> Result<Vec<Sample>, DbError> {
    let mut stmt = conn.prepare("SELECT id FROM validation_samples ORDER BY id DESC")?;
    let ids = stmt
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.into_iter()
        .filter_map(|id| sample(conn, id).transpose())
        .collect()
}

/// A sample's items in presentation order.
///
/// # Errors
///
/// `DbError` on query failure.
pub fn sample_items(conn: &Connection, id: i64) -> Result<Vec<SampleItem>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT position, date, time, sci_name, file_name, stratum, stratum_size,
                inclusion_probability
           FROM validation_sample_items WHERE sample_id = ?1 ORDER BY position",
    )?;
    let rows = stmt
        .query_map([id], |r| {
            Ok(SampleItem {
                position: r.get(0)?,
                date: r.get(1)?,
                time: r.get(2)?,
                sci_name: r.get(3)?,
                file_name: r.get(4)?,
                stratum: r.get(5)?,
                stratum_size: r.get(6)?,
                inclusion_probability: r.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Record a verdict on item `position` of sample `id`. `Ok(false)` when the
/// item does not exist or the verdict is not one of [`VERDICTS`].
///
/// # Errors
///
/// `DbError` on insert failure.
pub fn record_verdict(
    conn: &Connection,
    id: i64,
    position: i64,
    verdict: &str,
    reviewer: Option<&str>,
) -> Result<bool, DbError> {
    if !VERDICTS.contains(&verdict) {
        return Ok(false);
    }
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM validation_sample_items
                         WHERE sample_id = ?1 AND position = ?2)",
        params![id, position],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO validation_verdicts (sample_id, position, verdict, reviewer_name)
         VALUES (?1, ?2, ?3, ?4)",
        params![id, position, verdict, reviewer],
    )?;
    Ok(true)
}

/// One item of a sample with what was found and what the detector said: the
/// export row. The score and model are read from the detection now; a
/// detection deleted or relabelled since the draw has none.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SampleExportRow {
    /// The item.
    pub item: SampleItem,
    /// The detection's score, `None` when the detection is gone.
    pub confidence: Option<f64>,
    /// The classifier id the row carries.
    pub model_id: Option<String>,
    /// The row's analysis run.
    pub run_id: Option<i64>,
    /// The latest verdict, `None` when not reviewed.
    pub verdict: Option<String>,
    /// Who gave it.
    pub reviewer_name: Option<String>,
    /// When.
    pub verdict_at: Option<String>,
    /// How many verdicts the item has had (a correction is a later one).
    pub verdict_count: i64,
}

/// A sample's export rows, in presentation order.
///
/// # Errors
///
/// `DbError` on query failure.
pub fn sample_export(conn: &Connection, id: i64) -> Result<Vec<SampleExportRow>, DbError> {
    let items = sample_items(conn, id)?;
    let mut det = conn.prepare(
        "SELECT Confidence, model_id, run_id FROM detections
          WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3 AND COALESCE(File_Name, '') = ?4
          ORDER BY rowid LIMIT 1",
    )?;
    let mut ver = conn.prepare(
        "SELECT verdict, reviewer_name, at, (SELECT COUNT(*) FROM validation_verdicts
                                               WHERE sample_id = ?1 AND position = ?2)
           FROM validation_verdicts WHERE sample_id = ?1 AND position = ?2
          ORDER BY id DESC LIMIT 1",
    )?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let detection: Option<(f64, Option<String>, Option<i64>)> = det
            .query_row(
                params![item.date, item.time, item.sci_name, item.file_name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let latest: Option<(String, Option<String>, String, i64)> = ver
            .query_row(params![id, item.position], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        let (confidence, model_id, run_id) = match detection {
            Some((c, m, r)) => (Some(c), m, r),
            None => (None, None, None),
        };
        let (verdict, reviewer_name, verdict_at, verdict_count) = match latest {
            Some((verdict, name, at, count)) => (Some(verdict), name, Some(at), count),
            None => (None, None, None, 0),
        };
        out.push(SampleExportRow {
            item,
            confidence,
            model_id,
            run_id,
            verdict,
            reviewer_name,
            verdict_at,
            verdict_count,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;

    fn protocol(seed: u64) -> SampleProtocol {
        SampleProtocol {
            from: "2026-05-01".into(),
            to: "2026-05-31".into(),
            species: vec![],
            score_edges: vec![0.0, 0.5, 0.8, 1.0],
            by_species: false,
            per_stratum: 3,
            seed,
            blind: true,
        }
    }

    fn seeded() -> (tempfile::NamedTempFile, Connection) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        for i in 0..40 {
            let score = f64::from(i) / 40.0;
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES ('2026-05-02', ?1, 'Pica pica', 'Magpie', ?2, ?3)",
                params![format!("06:{i:02}:00"), score, format!("c{i}.wav")],
            )
            .unwrap();
        }
        (tmp, conn)
    }

    #[test]
    fn the_same_seed_draws_the_same_sample_and_another_seed_another() {
        let (_t, conn) = seeded();
        let a = create_sample(&conn, &protocol(7)).unwrap();
        let b = create_sample(&conn, &protocol(7)).unwrap();
        let c = create_sample(&conn, &protocol(8)).unwrap();
        let ia = sample_items(&conn, a).unwrap();
        assert_eq!(ia, sample_items(&conn, b).unwrap());
        assert_ne!(
            ia,
            sample_items(&conn, c).unwrap(),
            "a seed decides the draw"
        );
        assert_eq!(ia.len(), 9, "three from each of three strata: {ia:?}");
        let s = sample(&conn, a).unwrap().unwrap();
        assert_eq!(s.frame_size, 40);
        assert_eq!(s.seed, 7);
        assert_eq!(s.protocol_sha256, sha256_hex(s.protocol_json.as_bytes()));
    }

    #[test]
    fn each_item_carries_its_strata_size_and_inclusion_probability() {
        let (_t, conn) = seeded();
        let id = create_sample(&conn, &protocol(1)).unwrap();
        let items = sample_items(&conn, id).unwrap();
        // 0..0.5: 20 rows; 0.5..0.8: 12; 0.8..=1: 8.
        let by: BTreeMap<&str, (i64, f64)> = items
            .iter()
            .map(|i| {
                (
                    i.stratum.as_str(),
                    (i.stratum_size, i.inclusion_probability),
                )
            })
            .collect();
        assert_eq!(by["score[0,0.5)"], (20, 3.0 / 20.0));
        assert_eq!(by["score[0.5,0.8)"], (12, 3.0 / 12.0));
        assert_eq!(by["score[0.8,1]"], (8, 3.0 / 8.0));
        for it in &items {
            let row = FrameRow {
                date: it.date.clone(),
                time: it.time.clone(),
                sci_name: it.sci_name.clone(),
                file_name: it.file_name.clone(),
                confidence: 0.0,
            };
            assert!(row.key().contains("Pica pica"));
        }
    }

    #[test]
    fn presentation_order_does_not_follow_the_strata() {
        let (_t, conn) = seeded();
        let id = create_sample(&conn, &protocol(3)).unwrap();
        let strata: Vec<String> = sample_items(&conn, id)
            .unwrap()
            .into_iter()
            .map(|i| i.stratum)
            .collect();
        let mut sorted = strata.clone();
        sorted.sort();
        assert_ne!(strata, sorted, "items are shuffled, not grouped by stratum");
    }

    #[test]
    fn a_verdict_is_appended_and_the_latest_counts() {
        let (_t, conn) = seeded();
        let id = create_sample(&conn, &protocol(1)).unwrap();
        assert!(record_verdict(&conn, id, 0, "absent", Some("ada")).unwrap());
        assert!(record_verdict(&conn, id, 0, "present", Some("ada")).unwrap());
        assert!(!record_verdict(&conn, id, 0, "probably", None).unwrap());
        assert!(!record_verdict(&conn, id, 999, "present", None).unwrap());
        let rows = sample_export(&conn, id).unwrap();
        assert_eq!(rows[0].verdict.as_deref(), Some("present"));
        assert_eq!(rows[0].verdict_count, 2);
        assert!(rows[0].confidence.is_some());
        assert_eq!(rows[1].verdict, None);
        assert!(conn.execute("DELETE FROM validation_verdicts", []).is_err());
        assert!(
            conn.execute("UPDATE validation_samples SET seed = 1", [])
                .is_err()
        );
        assert!(
            conn.execute("DELETE FROM validation_sample_items", [])
                .is_err()
        );
    }

    #[test]
    fn a_protocol_that_cannot_draw_is_refused() {
        let (_t, conn) = seeded();
        let mut p = protocol(1);
        p.score_edges = vec![0.5, 0.5];
        assert!(matches!(
            create_sample(&conn, &p),
            Err(SampleError::Protocol(_))
        ));
        let mut p = protocol(1);
        p.per_stratum = 0;
        assert!(p.validate().is_err());
        let mut p = protocol(1);
        p.from = "2026-06-01".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn the_top_edge_is_inside_the_last_stratum_and_a_score_off_the_edges_is_in_none() {
        let p = SampleProtocol {
            score_edges: vec![0.5, 0.75, 1.0],
            ..protocol(1)
        };
        assert_eq!(p.stratum_of("x", 1.0).as_deref(), Some("score[0.75,1]"));
        assert_eq!(p.stratum_of("x", 0.75).as_deref(), Some("score[0.75,1]"));
        assert_eq!(p.stratum_of("x", 0.5).as_deref(), Some("score[0.5,0.75)"));
        assert_eq!(p.stratum_of("x", 0.49), None);
    }
}
