//! "Who heard it": what each classifier said about one detection.
//!
//! A station can run several classifiers over the same audio. The detection
//! list shows one row per species, carrying the winning classifier's score, so
//! until now nothing on the page said whether a second model agreed, disagreed,
//! scored the bird low, could not judge it, or failed. This card says which.
//!
//! # What it will not do
//!
//! It does not combine the scores. Each is a classifier's own output on its own
//! scale, and two classifiers are not calibrated alike; a mean, a maximum or a
//! "combined confidence" would be a number with nothing behind it, set in the
//! same column as a real model output. The card shows each model's answer
//! beside the bar that model was judged against, and counts them.
//!
//! # Four states, never a zero
//!
//! A model that has no class for the species has not scored it low — it has
//! said nothing, and printing 0% would report its silence as an opinion. The
//! four states ([`ModelScoreState`]) each get a different mark *and* different
//! words, so the distinction does not depend on colour.
//!
//! # When it is silent
//!
//! With nothing stored for the detection and no sign in the row itself that
//! more than one classifier agreed, the card renders nothing, like the
//! "Also heard by" card beside it: a one-classifier station never sees it.

use std::fmt::Write as _;

use birdnet_db::sqlite::{
    DbError, DetectionModelSummary, ModelScore, ModelScoreState, detection_model_summary,
    model_scores_for_detection,
};

use super::escape_html;

/// What the database holds about the classifiers behind one detection.
#[derive(Debug, Clone, Default)]
pub struct ModelEvidence {
    /// Each classifier's stored answer. Empty when none was stored.
    pub scores: Vec<ModelScore>,
    /// What the `detections` row itself recorded (migration 50).
    pub summary: Option<DetectionModelSummary>,
}

/// Read both halves for one detection.
///
/// # Errors
///
/// Returns `DbError` when either read fails; the caller decides what a failed
/// read looks like, and it must not look like an empty result.
pub fn read(
    conn: &rusqlite::Connection,
    date: &str,
    time: &str,
    sci_name: &str,
) -> Result<ModelEvidence, DbError> {
    Ok(ModelEvidence {
        scores: model_scores_for_detection(conn, date, time, sci_name)?,
        summary: detection_model_summary(conn, date, time, sci_name)?,
    })
}

/// The card for one detection. `None` means the read failed.
pub fn card(evidence: Option<&ModelEvidence>) -> String {
    let Some(ev) = evidence else {
        return super::error_states::inline("which classifiers heard this");
    };
    if !ev.scores.is_empty() {
        return per_model_card(&ev.scores);
    }
    summary_card(ev.summary.as_ref())
}

/// How many classifiers could judge the species, and how many of those heard it.
fn tally(scores: &[ModelScore]) -> (usize, usize) {
    let judged = scores.iter().filter(|s| s.state.has_score()).count();
    let heard = scores
        .iter()
        .filter(|s| s.state == ModelScoreState::Heard)
        .count();
    (heard, judged)
}

const fn models(n: usize) -> &'static str {
    if n == 1 { "model" } else { "models" }
}

/// The card when each classifier's answer was stored.
fn per_model_card(scores: &[ModelScore]) -> String {
    let (heard, judged) = tally(scores);
    let no_label = scores
        .iter()
        .filter(|s| s.state == ModelScoreState::NoLabel)
        .count();
    let failed = scores
        .iter()
        .filter(|s| s.state == ModelScoreState::Failed)
        .count();

    // The sentence a reader gets without opening anything. "Could judge" is the
    // honest denominator: a model with no class for the species is not a vote
    // against it, and one that failed has not voted.
    let headline = if judged == 0 {
        "No classifier was able to score this species for this detection.".to_owned()
    } else {
        format!(
            "Heard by {heard} of {judged} {} that could judge it.",
            models(judged)
        )
    };
    let mut aside = Vec::new();
    if no_label > 0 {
        aside.push(format!(
            "{no_label} {} no class for this species",
            if no_label == 1 {
                "model has"
            } else {
                "models have"
            }
        ));
    }
    if failed > 0 {
        aside.push(format!(
            "{failed} {} this window",
            if failed == 1 {
                "model failed"
            } else {
                "models failed"
            }
        ));
    }
    let aside = if aside.is_empty() {
        String::new()
    } else {
        format!(" {}.", capitalise(&aside.join("; ")))
    };
    // Every classifier that could judge it agreed: the one case worth a colour.
    // A split is information, not an alarm, so it stays neutral.
    let pill = if judged >= 2 && heard == judged {
        "bnb-pill moss"
    } else {
        "bnb-pill"
    };
    let pill_text = format!("{heard} of {judged}");

    let mut rows = String::new();
    for s in scores {
        rows.push_str(&row(s));
    }

    format!(
        r#"<div class="bnb-card pad ws-card">
  <div class="section-header"><div><div class="bnb-eyebrow">Models</div><h2 class="sh-h">Who heard it</h2></div><span class="{pill}">{pill_text}</span></div>
  <p class="ws-headline">{headline}{aside}</p>
  <table class="ws-table">
    <caption class="sr-only">What each classifier said about this detection</caption>
    <thead><tr><th scope="col">Model</th><th scope="col">What it said</th><th scope="col"><span class="sr-only">Score against its bar</span><span aria-hidden="true">Score · its bar</span></th></tr></thead>
    <tbody>{rows}</tbody>
  </table>
  <p class="bnb-meta ws-note">Each score is that model's own, on its own scale. They are not averaged, and a number from one model is not comparable to another's.</p>
  {windows}
</div>"#,
        windows = windows_note(scores),
    )
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// One model's row: its id, what it said in words, and the score on a rail.
fn row(s: &ModelScore) -> String {
    let (mark, class, words) = match (s.state, s.score) {
        (ModelScoreState::Heard, Some(score)) => (
            "\u{25CF}", // ●
            "heard",
            format!(
                "Heard it: <b>{}</b>{}",
                pct(score),
                bar_phrase(s.bar, "cleared its bar of")
            ),
        ),
        (ModelScoreState::BelowBar, Some(score)) => (
            "\u{25CB}", // ○
            "below",
            format!(
                "Scored it <b>{}</b>{}",
                pct(score),
                bar_phrase(s.bar, "under its bar of")
            ),
        ),
        (ModelScoreState::NoLabel, _) => (
            "\u{2013}", // –
            "nolabel",
            "Has no class for this species, so it cannot judge".to_owned(),
        ),
        // A score on a state that cannot have one is refused by the table, so
        // these two arms are a failed window and, defensively, a row that
        // disagrees with itself; both are shown as the model not answering.
        (ModelScoreState::Failed | ModelScoreState::Heard | ModelScoreState::BelowBar, _) => (
            "\u{2715}", // ✕
            "failed",
            "Did not answer: its window failed".to_owned(),
        ),
    };
    let rail = match (s.state.has_score(), s.score) {
        (true, Some(score)) => rail(score, s.bar, class),
        _ => String::from(r#"<span class="ws-rail none" aria-hidden="true"></span>"#),
    };
    format!(
        r#"<tr class="ws-row {class}"><th scope="row"><span class="mono">{id}</span></th><td><span class="ws-mark {class}" aria-hidden="true">{mark}</span> {words}<span class="ws-inline-rail">{rail}</span></td><td class="ws-railcell">{rail}</td></tr>"#,
        id = escape_html(&s.model_id),
    )
}

/// ", cleared its bar of 70%" — or nothing when the bar was not stored.
fn bar_phrase(own_bar: Option<f64>, lead: &str) -> String {
    own_bar.map_or_else(String::new, |b| format!(", {lead} {}", pct(b)))
}

/// A 0–1 score as the whole percent the rest of the app prints.
fn pct(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

/// A 0–100 position for a 0–1 value, clamped so a stray value cannot leave the rail.
fn pos(v: f64) -> f64 {
    (v * 100.0).clamp(0.0, 100.0)
}

/// The score on a 0–100% rail, with a tick where that model's own bar sits.
/// Decorative: the row's words carry the same numbers.
fn rail(score: f64, own_bar: Option<f64>, class: &str) -> String {
    let tick = own_bar.map_or_else(String::new, |b| {
        format!(
            r#"<span class="ws-tick" data-style="left:{:.1}%"></span>"#,
            pos(b)
        )
    });
    format!(
        r#"<span class="ws-rail" aria-hidden="true">{tick}<span class="ws-dot {class}" data-style="left:{:.1}%"></span></span>"#,
        pos(score)
    )
}

/// Each model judged its own window of the recording; say so, because
/// "the same chunk" is not "the same instants" when the windows differ.
fn windows_note(scores: &[ModelScore]) -> String {
    let mut items = String::new();
    for s in scores {
        if let (Some(a), Some(b)) = (s.window_start_secs, s.window_end_secs) {
            let _ = write!(
                items,
                "<dt><span class=\"mono\">{}</span></dt><dd>{a:.1}\u{2013}{b:.1} s of the recording</dd>",
                escape_html(&s.model_id)
            );
        }
    }
    if items.is_empty() {
        return String::new();
    }
    format!(
        r#"<details class="pt-disc ws-method"><summary>Method</summary><div><p>Classifiers listen in windows of different lengths. "Heard by" means each cleared its own bar for this species in the same chunk of audio, not that they listened to identical instants.</p><dl class="pt-def">{items}</dl></div></details>"#
    )
}

/// The card when only what the `detections` row itself recorded is known.
///
/// Renders only when more than one classifier cleared its bar — the one fact
/// the row alone can state. Anything less says nothing: `None` and `Some(1)`
/// both mean the row cannot tell a one-model station from a two-model one in
/// which the second said nothing.
fn summary_card(summary: Option<&DetectionModelSummary>) -> String {
    let Some(summary) = summary else {
        return String::new();
    };
    let Some(n) = summary.agreement.filter(|n| *n >= 2) else {
        return String::new();
    };
    let lead = summary.model_id.as_deref().map_or_else(
        || "One of them".to_owned(),
        |id| format!("<span class=\"mono\">{}</span>", escape_html(id)),
    );
    format!(
        r#"<div class="bnb-card pad ws-card">
  <div class="section-header"><div><div class="bnb-eyebrow">Models</div><h2 class="sh-h">Who heard it</h2></div><span class="bnb-pill moss">{n} {m}</span></div>
  <p class="ws-headline">{n} classifiers each cleared their own bar for this species in this chunk of audio.</p>
  <p class="bnb-meta ws-note">{lead} supplied the score shown on this page. The other scores were not kept for this detection, so they cannot be shown.</p>
</div>"#,
        m = models(usize::try_from(n).unwrap_or(2)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, state: ModelScoreState, score: Option<f64>, own_bar: Option<f64>) -> ModelScore {
        ModelScore {
            model_id: id.to_owned(),
            state,
            score,
            bar: own_bar,
            window_start_secs: None,
            window_end_secs: None,
        }
    }

    #[test]
    fn nothing_stored_and_no_sign_of_a_second_model_renders_nothing() {
        assert_eq!(card(Some(&ModelEvidence::default())), "");
        let one = ModelEvidence {
            scores: vec![],
            summary: Some(DetectionModelSummary {
                model_id: Some("birdnet".into()),
                agreement: Some(1),
            }),
        };
        assert_eq!(card(Some(&one)), "", "Some(1) is not agreement");
        let old = ModelEvidence {
            scores: vec![],
            summary: Some(DetectionModelSummary {
                model_id: None,
                agreement: None,
            }),
        };
        assert_eq!(
            card(Some(&old)),
            "",
            "a row from before the column says nothing"
        );
    }

    #[test]
    fn a_failed_read_says_so_and_does_not_look_like_nothing_to_report() {
        let html = card(None);
        assert!(html.contains("couldn't load"), "{html}");
        assert!(html.contains("role=\"alert\""), "{html}");
    }

    #[test]
    fn the_row_alone_can_state_that_several_classifiers_agreed_and_whose_score_is_shown() {
        let ev = ModelEvidence {
            scores: vec![],
            summary: Some(DetectionModelSummary {
                model_id: Some("v3".into()),
                agreement: Some(2),
            }),
        };
        let html = card(Some(&ev));
        assert!(html.contains("2 classifiers each cleared"), "{html}");
        assert!(
            html.contains(">v3<"),
            "names the model whose score is shown: {html}"
        );
        assert!(html.contains("were not kept"), "{html}");
        assert!(
            !html.contains("ws-table"),
            "no per-model table without scores: {html}"
        );
    }

    #[test]
    fn two_models_that_both_heard_it_read_as_two_of_two_and_the_pill_is_green() {
        let ev = ModelEvidence {
            scores: vec![
                s("v3", ModelScoreState::Heard, Some(0.87), Some(0.70)),
                s("perch", ModelScoreState::Heard, Some(0.64), Some(0.50)),
            ],
            summary: None,
        };
        let html = card(Some(&ev));
        assert!(
            html.contains("Heard by 2 of 2 models that could judge it."),
            "{html}"
        );
        assert!(html.contains("bnb-pill moss\">2 of 2<"), "{html}");
        assert!(
            html.contains("<b>87%</b>, cleared its bar of 70%"),
            "{html}"
        );
        assert!(
            html.contains("<b>64%</b>, cleared its bar of 50%"),
            "{html}"
        );
    }

    #[test]
    fn a_model_with_no_class_is_not_a_vote_against_and_prints_no_percentage() {
        let ev = ModelEvidence {
            scores: vec![
                s("v3", ModelScoreState::Heard, Some(0.91), Some(0.70)),
                s("regional", ModelScoreState::NoLabel, None, None),
            ],
            summary: None,
        };
        let html = card(Some(&ev));
        assert!(
            html.contains("Heard by 1 of 1 model that could judge it."),
            "the denominator counts only models that could judge: {html}"
        );
        assert!(
            html.contains("1 model has no class for this species"),
            "{html}"
        );
        assert!(
            html.contains("Has no class for this species, so it cannot judge"),
            "{html}"
        );
        // The no-label row must not smuggle a 0% in.
        let row = html
            .split("<tr class=\"ws-row nolabel\"")
            .nth(1)
            .expect("row");
        let row = row.split("</tr>").next().expect("row end");
        assert!(
            !row.contains('%'),
            "no percentage on a model that did not score: {row}"
        );
    }

    #[test]
    fn a_split_verdict_is_neutral_not_green_and_a_failed_window_is_said_in_words() {
        let ev = ModelEvidence {
            scores: vec![
                s("v3", ModelScoreState::Heard, Some(0.88), Some(0.70)),
                s("perch", ModelScoreState::BelowBar, Some(0.41), Some(0.50)),
                s("third", ModelScoreState::Failed, None, None),
            ],
            summary: None,
        };
        let html = card(Some(&ev));
        assert!(
            html.contains("Heard by 1 of 2 models that could judge it."),
            "{html}"
        );
        assert!(
            html.contains("bnb-pill\">1 of 2<"),
            "split is not green: {html}"
        );
        assert!(
            html.contains("Scored it <b>41%</b>, under its bar of 50%"),
            "{html}"
        );
        assert!(html.contains("1 model failed this window"), "{html}");
        assert!(html.contains("Did not answer: its window failed"), "{html}");
    }

    #[test]
    fn identity_does_not_rest_on_colour_each_state_has_its_own_mark() {
        let ev = ModelEvidence {
            scores: vec![
                s("a", ModelScoreState::Heard, Some(0.9), Some(0.7)),
                s("b", ModelScoreState::BelowBar, Some(0.4), Some(0.7)),
                s("c", ModelScoreState::NoLabel, None, None),
                s("d", ModelScoreState::Failed, None, None),
            ],
            summary: None,
        };
        let html = card(Some(&ev));
        for glyph in ["\u{25CF}", "\u{25CB}", "\u{2013}", "\u{2715}"] {
            assert!(html.contains(glyph), "missing mark {glyph:?}: {html}");
        }
    }

    #[test]
    fn a_model_id_typed_by_the_operator_is_escaped() {
        let ev = ModelEvidence {
            scores: vec![s("<b>x</b>", ModelScoreState::Heard, Some(0.9), Some(0.7))],
            summary: None,
        };
        let html = card(Some(&ev));
        assert!(!html.contains("<b>x</b>"), "{html}");
        assert!(html.contains("&lt;b&gt;x&lt;/b&gt;"), "{html}");
    }

    #[test]
    fn a_stray_score_cannot_leave_the_rail() {
        assert!((pos(1.7) - 100.0).abs() < f64::EPSILON);
        assert!(pos(-0.3).abs() < f64::EPSILON);
        assert!((pos(0.5) - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_windows_are_named_only_when_they_were_stored() {
        let mut a = s("v3", ModelScoreState::Heard, Some(0.9), Some(0.7));
        a.window_start_secs = Some(0.0);
        a.window_end_secs = Some(4.5);
        let mut b = s("perch", ModelScoreState::Heard, Some(0.8), Some(0.5));
        b.window_start_secs = Some(0.0);
        b.window_end_secs = Some(5.0);
        let with = card(Some(&ModelEvidence {
            scores: vec![a, b],
            summary: None,
        }));
        assert!(with.contains("0.0\u{2013}4.5 s"), "{with}");
        assert!(with.contains("0.0\u{2013}5.0 s"), "{with}");
        let without = card(Some(&ModelEvidence {
            scores: vec![s("v3", ModelScoreState::Heard, Some(0.9), Some(0.7))],
            summary: None,
        }));
        assert!(!without.contains("ws-method"), "{without}");
    }
}
