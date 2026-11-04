//! What each classifier said about a detection (migration 57), and what the
//! `model_id` / `model_agreement` columns (migration 50) record per classifier.
//!
//! Scores here are each classifier's own, on its own scale. Nothing in this
//! module combines them: two models are not calibrated alike, and a blended
//! number would sit in the same column as a real model output while meaning
//! something nobody has checked.

use rusqlite::{Connection, params};

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

/// Store one classifier's answer for a detection (upsert).
///
/// # Errors
///
/// Returns `DbError` on query failure, including a row the table's `CHECK`
/// refuses: a score on a state that cannot have one, or no score on a state
/// that must.
pub fn insert_model_score(
    conn: &Connection,
    date: &str,
    time: &str,
    sci_name: &str,
    score: &ModelScore,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO detection_model_scores
             (Date, Time, Sci_Name, model_id, state, score, bar, window_start_secs, window_end_secs)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(Date, Time, Sci_Name, model_id) DO UPDATE SET
             state = excluded.state, score = excluded.score, bar = excluded.bar,
             window_start_secs = excluded.window_start_secs,
             window_end_secs = excluded.window_end_secs",
        params![
            date,
            time,
            sci_name,
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
    date: &str,
    time: &str,
    sci_name: &str,
    scores: &[ModelScore],
) -> Result<(), DbError> {
    let tx = conn.unchecked_transaction()?;
    for score in scores {
        insert_model_score(&tx, date, time, sci_name, score)?;
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
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<Vec<ModelScore>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT model_id, state, score, bar, window_start_secs, window_end_secs
           FROM detection_model_scores
          WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3
          ORDER BY CASE state WHEN 'heard' THEN 0 WHEN 'below_bar' THEN 1
                              WHEN 'no_label' THEN 2 ELSE 3 END, model_id",
    )?;
    let rows = stmt
        .query_map(params![date, time, sci_name], |row| {
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
        })?
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
        "SELECT s.Date, s.Time, s.Sci_Name, s.model_id, s.state, s.score, s.bar,
                s.window_start_secs, s.window_end_secs
           FROM detection_model_scores s
          WHERE (?1 IS NULL OR s.Date >= ?1) AND (?2 IS NULL OR s.Date <= ?2)
            AND EXISTS (SELECT 1 FROM detections_analytic d
                         WHERE d.Date = s.Date AND d.Time = s.Time AND d.Sci_Name = s.Sci_Name)
          ORDER BY s.Date, s.Time, s.Sci_Name, s.model_id
          LIMIT ?3",
    )?;
    let fetch = i64::from(limit) + 1;
    let mut rows = stmt
        .query_map(params![from, to, fetch], |row| {
            let state: String = row.get(4)?;
            let state = ModelScoreState::parse(&state).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    format!("unknown model score state {state:?}").into(),
                )
            })?;
            Ok(DetectionModelScore {
                date: row.get(0)?,
                time: row.get(1)?,
                sci_name: row.get(2)?,
                answer: ModelScore {
                    model_id: row.get(3)?,
                    state,
                    score: row.get(5)?,
                    bar: row.get(6)?,
                    window_start_secs: row.get(7)?,
                    window_end_secs: row.get(8)?,
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
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<Option<DetectionModelSummary>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT model_id, model_agreement FROM detections
          WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3
          ORDER BY rowid LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![date, time, sci_name], |row| {
        Ok(DetectionModelSummary {
            model_id: row.get(0)?,
            agreement: row.get(1)?,
        })
    })?;
    Ok(rows.next().transpose()?)
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

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

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

    #[test]
    fn scores_come_back_heard_first_then_below_bar_then_no_label_then_failed() {
        let conn = db();
        // Inserted in the opposite of the order they should come back in.
        for (m, st, sc) in [
            ("d-failed", ModelScoreState::Failed, None),
            ("c-nolabel", ModelScoreState::NoLabel, None),
            ("b-low", ModelScoreState::BelowBar, Some(0.41)),
            ("a-heard", ModelScoreState::Heard, Some(0.88)),
        ] {
            insert_model_score(
                &conn,
                "2026-11-01",
                "06:30:00",
                "Turdus merula",
                &score(m, st, sc),
            )
            .expect("insert score");
        }
        let got = model_scores_for_detection(&conn, "2026-11-01", "06:30:00", "Turdus merula")
            .expect("read");
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
            insert_model_score(
                &conn,
                date,
                "06:30:00",
                sci,
                &score("m", ModelScoreState::Heard, Some(0.9)),
            )
            .expect("score");
        }
        // An answer for a detection the table never had a row for.
        insert_model_score(
            &conn,
            "2026-11-02",
            "07:00:00",
            "orphan",
            &score("m", ModelScoreState::Heard, Some(0.9)),
        )
        .expect("orphan");
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
            "rejected and orphaned answers are left out"
        );
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
        let good = score("a", ModelScoreState::Heard, Some(0.9));
        // A second answer the table refuses: a score on a state that has none.
        let bad = score("b", ModelScoreState::NoLabel, Some(0.5));
        assert!(
            insert_model_scores(&conn, "2026-11-01", "06:30:00", "x", &[good.clone(), bad])
                .is_err()
        );
        assert!(
            model_scores_for_detection(&conn, "2026-11-01", "06:30:00", "x")
                .expect("read")
                .is_empty(),
            "the good one must not survive its sibling's failure"
        );
        insert_model_scores(
            &conn,
            "2026-11-01",
            "06:30:00",
            "x",
            &[good, score("b", ModelScoreState::BelowBar, Some(0.3))],
        )
        .expect("a valid set is stored");
        assert_eq!(
            model_scores_for_detection(&conn, "2026-11-01", "06:30:00", "x")
                .expect("read")
                .len(),
            2
        );
    }

    #[test]
    fn a_detection_nothing_stored_scores_for_has_none() {
        let conn = db();
        assert!(
            model_scores_for_detection(&conn, "2026-11-01", "06:30:00", "Turdus merula")
                .expect("read")
                .is_empty()
        );
    }

    #[test]
    fn another_detections_scores_are_not_returned() {
        let conn = db();
        insert_model_score(
            &conn,
            "2026-11-01",
            "06:30:00",
            "Parus major",
            &score("a", ModelScoreState::Heard, Some(0.9)),
        )
        .expect("insert");
        assert!(
            model_scores_for_detection(&conn, "2026-11-01", "06:30:00", "Turdus merula")
                .expect("read")
                .is_empty()
        );
    }

    #[test]
    fn the_table_refuses_a_score_on_a_state_that_cannot_have_one_and_the_reverse() {
        let conn = db();
        let bad_nolabel = score("m", ModelScoreState::NoLabel, Some(0.5));
        assert!(insert_model_score(&conn, "2026-11-01", "06:30:00", "x", &bad_nolabel).is_err());
        let bad_heard = score("m", ModelScoreState::Heard, None);
        assert!(insert_model_score(&conn, "2026-11-01", "06:30:00", "x", &bad_heard).is_err());
    }

    #[test]
    fn a_score_outside_zero_to_one_is_refused() {
        let conn = db();
        let too_high = score("m", ModelScoreState::Heard, Some(1.5));
        assert!(insert_model_score(&conn, "2026-11-01", "06:30:00", "x", &too_high).is_err());
    }

    #[test]
    fn storing_the_same_models_answer_again_replaces_it() {
        let conn = db();
        insert_model_score(
            &conn,
            "2026-11-01",
            "06:30:00",
            "x",
            &score("m", ModelScoreState::BelowBar, Some(0.3)),
        )
        .expect("first");
        insert_model_score(
            &conn,
            "2026-11-01",
            "06:30:00",
            "x",
            &score("m", ModelScoreState::Heard, Some(0.8)),
        )
        .expect("second");
        let got = model_scores_for_detection(&conn, "2026-11-01", "06:30:00", "x").expect("read");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].state, ModelScoreState::Heard);
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
        let old = detection_model_summary(&conn, "2026-11-01", "06:30:00", "Old row")
            .expect("read")
            .expect("row");
        assert_eq!((old.model_id, old.agreement), (None, None));
        let one = detection_model_summary(&conn, "2026-11-01", "06:30:00", "One model")
            .expect("read")
            .expect("row");
        assert_eq!(
            one.agreement,
            Some(1),
            "Some(1) and None are different facts"
        );
        assert!(
            detection_model_summary(&conn, "2026-11-01", "06:30:00", "No such")
                .expect("read")
                .is_none()
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
