//! What each classifier said about a detection (migrations 57 and 58), and what the
//! `model_id` / `model_agreement` columns (migration 50) record per classifier.
//!
//! Scores here are each classifier's own, on its own scale. Nothing in this
//! module combines them: two models are not calibrated alike, and a blended
//! number would sit in the same column as a real model output while meaning
//! something nobody has checked.

use rusqlite::{Connection, params};

use super::DetectionKey;
use crate::sqlite::connection::DbError;

/// What one classifier could say about one detection.
///
/// Four states rather than a score and a flag: "it was asked and scored the
/// species low", "it has no class for the species", and "its window failed" are
/// different facts, and a reader that rounds them all to zero reports a model's
/// silence as its opinion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelScoreState {
    /// Scored at or above its own bar.
    Heard,
    /// Scored, and did not clear its own bar.
    BelowBar,
    /// Has no class for this species, so it cannot judge.
    NoLabel,
    /// Its window for this chunk failed.
    Failed,
}

impl ModelScoreState {
    /// The value stored in `detection_model_scores.state`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Heard => "heard",
            Self::BelowBar => "below_bar",
            Self::NoLabel => "no_label",
            Self::Failed => "failed",
        }
    }

    /// Read a stored value; `None` for anything the table's `CHECK` forbids.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "heard" => Some(Self::Heard),
            "below_bar" => Some(Self::BelowBar),
            "no_label" => Some(Self::NoLabel),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// Whether this state carries a score. The table holds the same rule as a
    /// `CHECK`, so a stored row never disagrees.
    #[must_use]
    pub const fn has_score(self) -> bool {
        matches!(self, Self::Heard | Self::BelowBar)
    }
}

/// One classifier's answer for one detection.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ModelScore {
    /// The classifier's id, as configured (`detections.model_id` uses the same).
    pub model_id: String,
    /// What it could say.
    pub state: ModelScoreState,
    /// Its own score, 0 to 1. `None` exactly when [`ModelScoreState::has_score`]
    /// is false.
    pub score: Option<f64>,
    /// The bar that score was judged against (its own threshold, else the
    /// station's, as loaded). `None` when it did not score.
    pub bar: Option<f64>,
    /// Where in the recording its window started, seconds. Classifiers' windows
    /// differ in length, so "the same chunk" is not "the same instants".
    pub window_start_secs: Option<f64>,
    /// Where its window ended, seconds.
    pub window_end_secs: Option<f64>,
}

/// Store one classifier's answer for a detection.
///
/// A plain insert, deliberately not an upsert: an answer already stored under
/// this key belongs to a detection that is already recorded, and replacing it
/// is how one detection came to carry another's scores. A second write for the
/// same key is an error the caller can log.
///
/// # Errors
///
/// Returns `DbError` on query failure, including a key that already has this
/// classifier's answer, and a row the table's `CHECK` refuses: a score on a
/// state that cannot have one, or no score on a state that must.
pub fn insert_model_score(
    conn: &Connection,
    key: &DetectionKey<'_>,
    score: &ModelScore,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO detection_model_scores
             (Date, Time, Sci_Name, File_Name, model_id, state, score, bar,
              window_start_secs, window_end_secs)
         VALUES (?1, ?2, ?3, COALESCE(?4, ''), ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            key.date,
            key.time,
            key.sci_name,
            key.file_name,
            score.model_id,
            score.state.as_str(),
            score.score,
            score.bar,
            score.window_start_secs,
            score.window_end_secs,
        ],
    )?;
    Ok(())
}

/// Store every classifier's answer for a detection, all or none.
///
/// A detection's answers are one fact (who heard it, and what each said), so a
/// failure part-way must not leave a card reading "heard by 1 of 1" over a set
/// that was meant to hold two.
///
/// # Errors
///
/// Returns `DbError` on query failure, including a row the table's `CHECK`
/// refuses; nothing is stored then.
pub fn insert_model_scores(
    conn: &Connection,
    key: &DetectionKey<'_>,
    scores: &[ModelScore],
) -> Result<(), DbError> {
    let tx = conn.unchecked_transaction()?;
    for score in scores {
        insert_model_score(&tx, key, score)?;
    }
    tx.commit()?;
    Ok(())
}

/// Every stored classifier answer for one detection.
///
/// Heard first, then scored below the bar, then no class for the species, then
/// failed; within a state, by classifier id.
///
/// Empty when nothing has stored any — a station whose daemon does not write
/// them, or a row older than the table. Empty is not "no classifier heard it".
///
/// # Errors
///
/// Returns `DbError` on query failure, or when a stored state is not one this
/// code knows.
pub fn model_scores_for_detection(
    conn: &Connection,
    key: &DetectionKey<'_>,
) -> Result<Vec<ModelScore>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT model_id, state, score, bar, window_start_secs, window_end_secs
           FROM detection_model_scores
          WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3
            AND File_Name = COALESCE(?4, '')
          ORDER BY CASE state WHEN 'heard' THEN 0 WHEN 'below_bar' THEN 1
                              WHEN 'no_label' THEN 2 ELSE 3 END, model_id",
    )?;
    let rows = stmt
        .query_map(
            params![key.date, key.time, key.sci_name, key.file_name],
            |row| {
                let state: String = row.get(1)?;
                let state = ModelScoreState::parse(&state).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        format!("unknown model score state {state:?}").into(),
                    )
                })?;
                Ok(ModelScore {
                    model_id: row.get(0)?,
                    state,
                    score: row.get(2)?,
                    bar: row.get(3)?,
                    window_start_secs: row.get(4)?,
                    window_end_secs: row.get(5)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One classifier's answer, with the detection it is about.
///
/// The long form a spreadsheet or a script wants: one row per (detection,
/// classifier), which is also how the table stores it. A wide form would need a
/// column per classifier, and the set of classifiers changes over a station's
/// life.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DetectionModelScore {
    /// Detection date (local wall clock, as `detections.Date`).
    pub date: String,
    /// Detection time (local wall clock, as `detections.Time`).
    pub time: String,
    /// The species the answer is about.
    pub sci_name: String,
    /// The detection's clip, as `detections.File_Name` (empty when it has
    /// none). With the date, time and species it names one detection.
    pub file_name: String,
    /// The classifier's answer.
    #[serde(flatten)]
    pub answer: ModelScore,
}

/// Every stored classifier answer for detections in a date range, oldest first.
///
/// Answers are returned only for detections the analytics stand behind (the
/// `detections_analytic` view), so a detection a reviewer rejected is left out
/// here exactly as it is left out of the detections export. The second value is
/// `true` when more than `limit` rows matched and the list was cut.
///
/// # Errors
///
/// Returns `DbError` on query failure, or when a stored state is not one this
/// code knows.
pub fn model_scores_between(
    conn: &Connection,
    from: Option<&str>,
    to: Option<&str>,
    limit: u32,
) -> Result<(Vec<DetectionModelScore>, bool), DbError> {
    let mut stmt = conn.prepare(
        "SELECT s.Date, s.Time, s.Sci_Name, s.File_Name, s.model_id, s.state, s.score, s.bar,
                s.window_start_secs, s.window_end_secs
           FROM detection_model_scores s
          WHERE (?1 IS NULL OR s.Date >= ?1) AND (?2 IS NULL OR s.Date <= ?2)
            AND EXISTS (SELECT 1 FROM detections_analytic d
                         WHERE d.Date = s.Date AND d.Time = s.Time AND d.Sci_Name = s.Sci_Name
                           AND COALESCE(d.File_Name, '') = s.File_Name)
          ORDER BY s.Date, s.Time, s.Sci_Name, s.File_Name, s.model_id
          LIMIT ?3",
    )?;
    let fetch = i64::from(limit) + 1;
    let mut rows = stmt
        .query_map(params![from, to, fetch], |row| {
            let state: String = row.get(5)?;
            let state = ModelScoreState::parse(&state).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    format!("unknown model score state {state:?}").into(),
                )
            })?;
            Ok(DetectionModelScore {
                date: row.get(0)?,
                time: row.get(1)?,
                sci_name: row.get(2)?,
                file_name: row.get(3)?,
                answer: ModelScore {
                    model_id: row.get(4)?,
                    state,
                    score: row.get(6)?,
                    bar: row.get(7)?,
                    window_start_secs: row.get(8)?,
                    window_end_secs: row.get(9)?,
                },
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let truncated = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    Ok((rows, truncated))
}

/// What `detections` itself recorded about the classifiers behind one row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DetectionModelSummary {
    /// The classifier whose answer the row carries. `None` for a row written
    /// before migration 50, when nothing recorded it.
    pub model_id: Option<String>,
    /// How many classifiers cleared their own bar for this species in this
    /// chunk. `None` is *nobody was counting*; `Some(1)` is *one classifier was
    /// asked and one answered*. They are not the same, and neither is zero.
    pub agreement: Option<i64>,
}

/// The `model_id` / `model_agreement` pair for one detection, or `None` when
/// there is no such row.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_model_summary(
    conn: &Connection,
    key: &DetectionKey<'_>,
) -> Result<Option<DetectionModelSummary>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT model_id, model_agreement FROM detections
          WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3
            AND COALESCE(File_Name, '') = COALESCE(?4, '')
          ORDER BY rowid LIMIT 1",
    )?;
    let mut rows = stmt.query_map(
        params![key.date, key.time, key.sci_name, key.file_name],
        |row| {
            Ok(DetectionModelSummary {
                model_id: row.get(0)?,
                agreement: row.get(1)?,
            })
        },
    )?;
    Ok(rows.next().transpose()?)
}

/// The clips of every detection that shares a date, time and species, in the
/// form [`DetectionKey::file_name`] takes (`""` for a row with no clip).
///
/// One name means the triple already identifies a detection; several mean two
/// sources heard one bird in one second, and a caller that was given only the
/// triple has to be told which it meant.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_files_at(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT COALESCE(File_Name, '') FROM detections
          WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3
          ORDER BY 1",
    )?;
    let rows = stmt
        .query_map(params![date, time, sci_name], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One classifier's day: the detections that carry its answer, and how many of
/// those another classifier also cleared.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ModelLead {
    /// The classifier's id.
    pub model_id: String,
    /// Detections today whose stored answer is this classifier's.
    pub detections: i64,
    /// Of those, how many at least two classifiers cleared their bar for.
    pub agreed: i64,
}

/// Per-classifier detection counts for one date, most detections first.
///
/// Rows with no recorded `model_id` (written before migration 50) are not
/// counted for any classifier: nobody was counting, and attributing them to
/// whichever model runs now would be a guess. Rejected detections are left out,
/// as everywhere else the analytics read.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn model_lead_counts(conn: &Connection, date: &str) -> Result<Vec<ModelLead>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT model_id, COUNT(*), COALESCE(SUM(COALESCE(model_agreement, 0) >= 2), 0)
           FROM detections_analytic
          WHERE Date = ?1 AND model_id IS NOT NULL
          GROUP BY model_id
          ORDER BY COUNT(*) DESC, model_id",
    )?;
    let rows = stmt
        .query_map(params![date], |row| {
            Ok(ModelLead {
                model_id: row.get(0)?,
                detections: row.get(1)?,
                agreed: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::{delete_detection_at, relabel_detection_at};

    const DATE: &str = "2026-11-01";
    const TIME: &str = "06:30:00";

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

    /// The identity of a detection at the test's date and time.
    const fn key<'a>(sci: &'a str, file: Option<&'a str>) -> DetectionKey<'a> {
        DetectionKey {
            date: DATE,
            time: TIME,
            sci_name: sci,
            file_name: file,
        }
    }

    /// A detection whose clip is `<sci>.wav`.
    fn insert_detection(conn: &Connection, sci: &str, model: Option<&str>, agree: Option<i64>) {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week,
                  Sens, Overlap, File_Name, model_id, model_agreement)
             VALUES ('2026-11-01', '06:30:00', ?1, ?1, 0.9, 0.0, 0.0, 0.7, 44,
                     1.25, 0.0, ?1 || '.wav', ?2, ?3)",
            params![sci, model, agree],
        )
        .expect("insert detection");
    }

    /// A detection of `sci` heard from the source whose clip is `file` (`None`
    /// for a row with no clip at all).
    fn add_detection(conn: &Connection, sci: &str, file: Option<&str>) {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week,
                  Sens, Overlap, File_Name)
             VALUES ('2026-11-01', '06:30:00', ?1, ?1, 0.9, 0.0, 0.0, 0.7, 44,
                     1.25, 0.0, ?2)",
            params![sci, file],
        )
        .expect("insert detection");
    }

    fn score(model: &str, state: ModelScoreState, s: Option<f64>) -> ModelScore {
        ModelScore {
            model_id: model.to_owned(),
            state,
            score: s,
            bar: s.map(|_| 0.7),
            window_start_secs: Some(0.0),
            window_end_secs: Some(3.0),
        }
    }

    fn answers_for(conn: &Connection, k: &DetectionKey<'_>) -> Vec<ModelScore> {
        model_scores_for_detection(conn, k).expect("read")
    }

    fn rows_in_table(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM detection_model_scores", [], |r| {
            r.get(0)
        })
        .expect("count")
    }

    #[test]
    fn scores_come_back_heard_first_then_below_bar_then_no_label_then_failed() {
        let conn = db();
        let k = key("Turdus merula", None);
        // Inserted in the opposite of the order they should come back in.
        for (m, st, sc) in [
            ("d-failed", ModelScoreState::Failed, None),
            ("c-nolabel", ModelScoreState::NoLabel, None),
            ("b-low", ModelScoreState::BelowBar, Some(0.41)),
            ("a-heard", ModelScoreState::Heard, Some(0.88)),
        ] {
            insert_model_score(&conn, &k, &score(m, st, sc)).expect("insert score");
        }
        let got = answers_for(&conn, &k);
        let order: Vec<_> = got.iter().map(|s| (s.model_id.as_str(), s.state)).collect();
        assert_eq!(
            order,
            [
                ("a-heard", ModelScoreState::Heard),
                ("b-low", ModelScoreState::BelowBar),
                ("c-nolabel", ModelScoreState::NoLabel),
                ("d-failed", ModelScoreState::Failed),
            ]
        );
        assert_eq!(got[0].score, Some(0.88));
        assert_eq!(
            got[2].score, None,
            "a model with no class has no score, not 0"
        );
    }

    #[test]
    fn answers_between_dates_are_returned_oldest_first_and_only_for_standing_detections() {
        let conn = db();
        for (date, sci) in [
            ("2026-11-01", "A"),
            ("2026-11-02", "B"),
            ("2026-11-03", "C"),
        ] {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week,
                      Sens, Overlap, File_Name)
                 VALUES (?1, '06:30:00', ?2, ?2, 0.9, 0.0, 0.0, 0.7, 44, 1.25, 0.0, ?2 || '.wav')",
                params![date, sci],
            )
            .expect("insert detection");
            let file = format!("{sci}.wav");
            let k = DetectionKey {
                date,
                time: TIME,
                sci_name: sci,
                file_name: Some(&file),
            };
            insert_model_score(&conn, &k, &score("m", ModelScoreState::Heard, Some(0.9)))
                .expect("score");
        }
        // An answer for a detection the table never had a row for.
        let orphan = DetectionKey {
            date: "2026-11-02",
            time: "07:00:00",
            sci_name: "orphan",
            file_name: None,
        };
        insert_model_score(
            &conn,
            &orphan,
            &score("m", ModelScoreState::Heard, Some(0.9)),
        )
        .expect("orphan");
        // An answer for the right species, date and time but a clip no detection
        // has: it belongs to some other detection, not to A.
        let other_clip = DetectionKey {
            date: "2026-11-01",
            time: TIME,
            sci_name: "A",
            file_name: Some("some-other-source.wav"),
        };
        insert_model_score(
            &conn,
            &other_clip,
            &score("m", ModelScoreState::Heard, Some(0.5)),
        )
        .expect("other clip");
        conn.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Sci_Name = 'C'",
            [],
        )
        .expect("reject");

        let (all, cut) = model_scores_between(&conn, None, None, 100).expect("read");
        let names: Vec<_> = all.iter().map(|r| r.sci_name.as_str()).collect();
        assert_eq!(
            names,
            ["A", "B"],
            "rejected, orphaned and other-clip answers are left out"
        );
        assert_eq!(all[0].file_name, "A.wav", "each answer names its clip");
        assert!(!cut);
        let (one, _) =
            model_scores_between(&conn, Some("2026-11-02"), Some("2026-11-02"), 100).expect("read");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].sci_name, "B");
        let (capped, cut) = model_scores_between(&conn, None, None, 1).expect("read");
        assert_eq!(capped.len(), 1);
        assert!(cut, "more matched than the limit, and it says so");
    }

    #[test]
    fn a_detection_row_carries_its_classifier_and_agreement_and_a_legacy_row_carries_neither() {
        let conn = db();
        insert_detection(&conn, "Two models", Some("v3"), Some(2));
        insert_detection(&conn, "Legacy", None, None);
        let two = crate::sqlite::detection_at(&conn, "2026-11-01", "06:30:00", Some("Two models"))
            .expect("read")
            .expect("row");
        assert_eq!(two.model_id.as_deref(), Some("v3"));
        assert_eq!(two.model_agreement, Some(2));
        let old = crate::sqlite::detection_at(&conn, "2026-11-01", "06:30:00", Some("Legacy"))
            .expect("read")
            .expect("row");
        assert_eq!((old.model_id.as_deref(), old.model_agreement), (None, None));
        // And the JSON omits what is unknown rather than printing a null.
        let json = serde_json::to_value(&old).expect("json");
        assert!(
            json.get("model_id").is_none() && json.get("model_agreement").is_none(),
            "{json}"
        );
        let json = serde_json::to_value(&two).expect("json");
        assert_eq!(json["model_agreement"], 2);
    }

    #[test]
    fn a_set_of_answers_is_stored_whole_or_not_at_all() {
        let conn = db();
        let k = key("x", None);
        let good = score("a", ModelScoreState::Heard, Some(0.9));
        // A second answer the table refuses: a score on a state that has none.
        let bad = score("b", ModelScoreState::NoLabel, Some(0.5));
        assert!(insert_model_scores(&conn, &k, &[good.clone(), bad]).is_err());
        assert!(
            answers_for(&conn, &k).is_empty(),
            "the good one must not survive its sibling's failure"
        );
        insert_model_scores(
            &conn,
            &k,
            &[good, score("b", ModelScoreState::BelowBar, Some(0.3))],
        )
        .expect("a valid set is stored");
        assert_eq!(answers_for(&conn, &k).len(), 2);
    }

    #[test]
    fn a_detection_nothing_stored_scores_for_has_none() {
        let conn = db();
        assert!(answers_for(&conn, &key("Turdus merula", None)).is_empty());
    }

    #[test]
    fn another_detections_scores_are_not_returned() {
        let conn = db();
        insert_model_score(
            &conn,
            &key("Parus major", None),
            &score("a", ModelScoreState::Heard, Some(0.9)),
        )
        .expect("insert");
        assert!(answers_for(&conn, &key("Turdus merula", None)).is_empty());
    }

    #[test]
    fn the_table_refuses_a_score_on_a_state_that_cannot_have_one_and_the_reverse() {
        let conn = db();
        let k = key("x", None);
        let bad_nolabel = score("m", ModelScoreState::NoLabel, Some(0.5));
        assert!(insert_model_score(&conn, &k, &bad_nolabel).is_err());
        let bad_heard = score("m", ModelScoreState::Heard, None);
        assert!(insert_model_score(&conn, &k, &bad_heard).is_err());
    }

    #[test]
    fn a_score_outside_zero_to_one_is_refused() {
        let conn = db();
        let too_high = score("m", ModelScoreState::Heard, Some(1.5));
        assert!(insert_model_score(&conn, &key("x", None), &too_high).is_err());
    }

    /// The writer used to be an upsert, so a second detection with the same key
    /// replaced the first one's answers without a word. It is a plain insert now:
    /// the second write is an error, and the first answer is still what is stored.
    #[test]
    fn a_second_answer_for_the_same_detection_and_classifier_is_refused_and_the_first_stands() {
        let conn = db();
        let k = key("x", Some("a.wav"));
        insert_model_score(&conn, &k, &score("m", ModelScoreState::BelowBar, Some(0.3)))
            .expect("first");
        assert!(
            insert_model_score(&conn, &k, &score("m", ModelScoreState::Heard, Some(0.8))).is_err(),
            "replacing an answer silently is how one detection got another's scores"
        );
        let got = answers_for(&conn, &k);
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].state, got[0].score),
            (ModelScoreState::BelowBar, Some(0.3))
        );
    }

    /// Two sources hearing one bird in one second are two detections that share
    /// date, time and species. Each keeps the answers its own classifiers gave.
    #[test]
    fn two_detections_that_share_a_second_and_a_species_keep_their_own_answers() {
        let conn = db();
        add_detection(&conn, "Pica pica", Some("pond.wav"));
        add_detection(&conn, "Pica pica", Some("lake.wav"));
        let pond = key("Pica pica", Some("pond.wav"));
        let lake = key("Pica pica", Some("lake.wav"));
        insert_model_score(
            &conn,
            &pond,
            &score("m", ModelScoreState::Heard, Some(0.91)),
        )
        .expect("pond");
        insert_model_score(
            &conn,
            &lake,
            &score("m", ModelScoreState::Heard, Some(0.76)),
        )
        .expect("lake must not collide with pond");
        assert_eq!(answers_for(&conn, &pond)[0].score, Some(0.91));
        assert_eq!(answers_for(&conn, &lake)[0].score, Some(0.76));
        let (all, _) = model_scores_between(&conn, None, None, 100).expect("read");
        let by_file: Vec<_> = all
            .iter()
            .map(|r| (r.file_name.as_str(), r.answer.score))
            .collect();
        assert_eq!(
            by_file,
            [("lake.wav", Some(0.76)), ("pond.wav", Some(0.91))],
            "the export tells the two apart"
        );
    }

    /// `None` and `Some("")` both name a detection with no clip, as everywhere
    /// else a detection is addressed; neither names a detection that has one.
    #[test]
    fn a_detection_with_no_clip_is_found_by_no_clip_and_not_by_a_clip() {
        let conn = db();
        add_detection(&conn, "Pica pica", None);
        insert_model_score(
            &conn,
            &key("Pica pica", None),
            &score("m", ModelScoreState::Heard, Some(0.9)),
        )
        .expect("insert");
        assert_eq!(answers_for(&conn, &key("Pica pica", None)).len(), 1);
        assert_eq!(answers_for(&conn, &key("Pica pica", Some(""))).len(), 1);
        assert!(answers_for(&conn, &key("Pica pica", Some("pond.wav"))).is_empty());
        let (all, _) = model_scores_between(&conn, None, None, 100).expect("read");
        assert_eq!(all.len(), 1, "a no-clip detection's answers are exported");
        assert_eq!(all[0].file_name, "");
    }

    #[test]
    fn the_clips_that_share_a_date_time_and_species_are_listed() {
        let conn = db();
        add_detection(&conn, "Pica pica", Some("pond.wav"));
        add_detection(&conn, "Pica pica", Some("lake.wav"));
        add_detection(&conn, "Pica pica", None);
        add_detection(&conn, "Parus major", Some("tit.wav"));
        assert_eq!(
            detection_files_at(&conn, DATE, TIME, "Pica pica").expect("read"),
            ["", "lake.wav", "pond.wav"],
            "a row with no clip is the empty name, and nothing else is listed"
        );
        assert!(
            detection_files_at(&conn, DATE, TIME, "No such")
                .expect("read")
                .is_empty()
        );
    }

    /// Nothing used to remove a detection's answers, so deleting a detection left
    /// rows that belonged to nothing.
    #[test]
    fn deleting_a_detection_deletes_its_answers_and_only_its_own() {
        let conn = db();
        add_detection(&conn, "Pica pica", Some("pond.wav"));
        add_detection(&conn, "Pica pica", Some("lake.wav"));
        let pond = key("Pica pica", Some("pond.wav"));
        let lake = key("Pica pica", Some("lake.wav"));
        for k in [&pond, &lake] {
            insert_model_scores(
                &conn,
                k,
                &[
                    score("a", ModelScoreState::Heard, Some(0.9)),
                    score("b", ModelScoreState::BelowBar, Some(0.3)),
                ],
            )
            .expect("answers");
        }
        assert_eq!(rows_in_table(&conn), 4);
        assert!(delete_detection_at(&conn, &pond).expect("delete"));
        assert!(
            answers_for(&conn, &pond).is_empty(),
            "the deleted detection's answers go with it"
        );
        assert_eq!(
            answers_for(&conn, &lake).len(),
            2,
            "the other detection that shares its date, time and species is untouched"
        );
        assert_eq!(rows_in_table(&conn), 2);
    }

    /// The trigger is on the table, not on one delete function, so a bulk delete
    /// (a species removed, an import batch undone) cleans up too.
    #[test]
    fn a_delete_that_does_not_go_through_the_delete_function_cleans_up_as_well() {
        let conn = db();
        add_detection(&conn, "Pica pica", Some("pond.wav"));
        add_detection(&conn, "Parus major", Some("tit.wav"));
        for (sci, file) in [("Pica pica", "pond.wav"), ("Parus major", "tit.wav")] {
            insert_model_score(
                &conn,
                &key(sci, Some(file)),
                &score("m", ModelScoreState::Heard, Some(0.9)),
            )
            .expect("answer");
        }
        conn.execute("DELETE FROM detections WHERE Sci_Name = 'Pica pica'", [])
            .expect("delete a species");
        assert_eq!(
            rows_in_table(&conn),
            1,
            "only the survivor's answer is left"
        );
        conn.execute("DELETE FROM detections", [])
            .expect("delete all");
        assert_eq!(rows_in_table(&conn), 0);
    }

    /// Relabelling says the classifiers were wrong about the species. Their
    /// answers were about the old one, and are not shown under the new name.
    #[test]
    fn relabelling_a_detection_drops_the_answers_about_the_species_it_no_longer_is() {
        let conn = db();
        insert_detection(&conn, "Old name", Some("v3"), Some(2));
        insert_detection(&conn, "Bystander", Some("v3"), Some(2));
        let old = key("Old name", Some("Old name.wav"));
        let bystander = key("Bystander", Some("Bystander.wav"));
        for k in [&old, &bystander] {
            insert_model_score(&conn, k, &score("m", ModelScoreState::Heard, Some(0.9)))
                .expect("answer");
        }
        // An edit that does not change the species leaves the answers alone.
        conn.execute(
            "UPDATE detections SET is_locked = 1 WHERE Sci_Name = 'Old name'",
            [],
        )
        .expect("lock");
        assert_eq!(answers_for(&conn, &old).len(), 1, "a lock is not a relabel");

        assert!(relabel_detection_at(&conn, &old, "New name", "New name").expect("relabel"));
        assert!(
            answers_for(&conn, &old).is_empty(),
            "nothing is left keyed to the old name"
        );
        assert!(
            answers_for(&conn, &key("New name", Some("Old name.wav"))).is_empty(),
            "and nothing is shown under the new one"
        );
        assert_eq!(answers_for(&conn, &bystander).len(), 1);
        let row = crate::sqlite::detection_at(&conn, DATE, TIME, Some("New name"))
            .expect("read")
            .expect("row");
        assert_eq!(
            (row.model_id.as_deref(), row.model_agreement),
            (Some("v3"), Some(2)),
            "the detection keeps its own record of who found it"
        );
    }

    /// The relabel function sets the species and its common name together, but
    /// the cleanup is a property of the table, not of that function: an update of
    /// the species column alone drops the answers too.
    #[test]
    fn an_update_of_only_the_species_column_drops_the_answers_about_the_old_one() {
        let conn = db();
        add_detection(&conn, "Old name", Some("a.wav"));
        let old = key("Old name", Some("a.wav"));
        insert_model_score(&conn, &old, &score("m", ModelScoreState::Heard, Some(0.9)))
            .expect("answer");
        conn.execute(
            "UPDATE detections SET Sci_Name = 'New name' WHERE Sci_Name = 'Old name'",
            [],
        )
        .expect("rename the species only");
        assert!(answers_for(&conn, &old).is_empty());
    }

    /// The counterpart: writing the species column back unchanged is not a
    /// relabel, so the answers about that species stay.
    #[test]
    fn writing_the_species_back_unchanged_keeps_the_answers() {
        let conn = db();
        add_detection(&conn, "Same name", Some("a.wav"));
        let k = key("Same name", Some("a.wav"));
        insert_model_score(&conn, &k, &score("m", ModelScoreState::Heard, Some(0.9)))
            .expect("answer");
        conn.execute("UPDATE detections SET Sci_Name = Sci_Name", [])
            .expect("rewrite with the same value");
        assert_eq!(answers_for(&conn, &k).len(), 1);
    }

    #[test]
    fn state_strings_round_trip_and_unknown_ones_are_refused() {
        for s in [
            ModelScoreState::Heard,
            ModelScoreState::BelowBar,
            ModelScoreState::NoLabel,
            ModelScoreState::Failed,
        ] {
            assert_eq!(ModelScoreState::parse(s.as_str()), Some(s));
        }
        assert_eq!(ModelScoreState::parse("agreed"), None);
        assert!(ModelScoreState::Heard.has_score() && ModelScoreState::BelowBar.has_score());
        assert!(!ModelScoreState::NoLabel.has_score() && !ModelScoreState::Failed.has_score());
    }

    #[test]
    fn a_row_written_before_the_columns_is_nobody_was_counting_not_one_model() {
        let conn = db();
        insert_detection(&conn, "Old row", None, None);
        insert_detection(&conn, "One model", Some("birdnet"), Some(1));
        let old = detection_model_summary(&conn, &key("Old row", Some("Old row.wav")))
            .expect("read")
            .expect("row");
        assert_eq!((old.model_id, old.agreement), (None, None));
        let one = detection_model_summary(&conn, &key("One model", Some("One model.wav")))
            .expect("read")
            .expect("row");
        assert_eq!(
            one.agreement,
            Some(1),
            "Some(1) and None are different facts"
        );
        assert!(
            detection_model_summary(&conn, &key("No such", None))
                .expect("read")
                .is_none()
        );
    }

    #[test]
    fn the_summary_is_the_one_detections_when_several_share_a_second_and_a_species() {
        let conn = db();
        for (file, model, agree) in [("pond.wav", "v3", 2), ("lake.wav", "perch", 1)] {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week,
                      Sens, Overlap, File_Name, model_id, model_agreement)
                 VALUES ('2026-11-01', '06:30:00', 'Pica pica', 'Pica pica', 0.9, 0.0, 0.0,
                         0.7, 44, 1.25, 0.0, ?1, ?2, ?3)",
                params![file, model, agree],
            )
            .expect("insert");
        }
        let lake = detection_model_summary(&conn, &key("Pica pica", Some("lake.wav")))
            .expect("read")
            .expect("row");
        assert_eq!(
            (lake.model_id.as_deref(), lake.agreement),
            (Some("perch"), Some(1)),
            "the lake detection's summary, not whichever row came first"
        );
    }

    #[test]
    fn lead_counts_group_by_model_count_agreement_and_skip_rows_with_no_model() {
        let conn = db();
        insert_detection(&conn, "A", Some("v3"), Some(2));
        insert_detection(&conn, "B", Some("v3"), Some(1));
        insert_detection(&conn, "C", Some("v3"), Some(3));
        insert_detection(&conn, "D", Some("perch"), Some(1));
        insert_detection(&conn, "E", None, None);
        insert_detection(&conn, "F", Some("perch"), None);
        let got = model_lead_counts(&conn, "2026-11-01").expect("read");
        assert_eq!(
            got,
            [
                ModelLead {
                    model_id: "v3".into(),
                    detections: 3,
                    agreed: 2
                },
                ModelLead {
                    model_id: "perch".into(),
                    detections: 2,
                    agreed: 0
                },
            ],
            "the NULL-model row is attributed to no one"
        );
        assert!(
            model_lead_counts(&conn, "2026-11-02")
                .expect("read")
                .is_empty()
        );
    }

    #[test]
    fn a_rejected_detection_is_not_counted_for_its_model() {
        let conn = db();
        insert_detection(&conn, "A", Some("v3"), Some(2));
        insert_detection(&conn, "B", Some("v3"), Some(2));
        conn.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Sci_Name = 'B'",
            [],
        )
        .expect("reject");
        let got = model_lead_counts(&conn, "2026-11-01").expect("read");
        assert_eq!(got[0].detections, 1);
    }
}
