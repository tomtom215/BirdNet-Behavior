//! The Models board on Station Health: each classifier the station runs,
//! side by side.
//!
//! Before this, a station running two classifiers showed one undifferentiated
//! stream of detections, and the only trace of a classifier on the Health
//! screen was a banner sentence if one of them failed ten windows in a row.
//!
//! # Only what is real
//!
//! Every figure on a card is read from something the station records:
//! the window record the daemon keeps per classifier
//! ([`birdnet_core::inference::health::ModelHealth`]), and the `model_id` /
//! `model_agreement` columns of `detections` (migration 50). Window length,
//! label count, the model's own bar and per-window latency are not published by
//! the daemon, so they are not shown; a card does not print a dash where a
//! number belongs to make it look complete.
//!
//! # Agreement is only claimed when there is something to agree
//!
//! "Also cleared by another model" is printed only when more than one
//! classifier is running. On a one-classifier station every row reads
//! `model_agreement = 1`, and "0 agreed" would say a second opinion was sought
//! and withheld.

use birdnet_core::inference::health::{ErrorClass, ModelHealth};
use birdnet_db::sqlite::ModelLead;

use super::escape_html;

/// Where one classifier stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelState {
    /// Has answered windows, and is not on a run of failures.
    Running,
    /// On a run of failed windows long enough to say so.
    Failing,
    /// Loaded, and has not been given a window yet.
    Waiting,
}

/// One classifier's card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCard {
    /// The operator's name for it.
    pub id: String,
    /// Where it stands.
    pub state: ModelState,
    /// Windows run since it was loaded, good or not.
    pub windows: u64,
    /// Seconds since it last ran a window.
    pub last_window_age: Option<u64>,
    /// Seconds since it last answered one cleanly.
    pub last_success_age: Option<u64>,
    /// Why its latest window failed, while the run of failures lasts.
    pub error: Option<ErrorClass>,
    /// Failed windows in a row.
    pub consecutive_failures: u32,
    /// Its share of today's detections; `None` when that read failed.
    pub lead: Option<ModelLead>,
}

/// Build the cards from the daemon's health record and today's per-model counts.
///
/// `leads` is `None` when the database read failed: the cards then say so
/// instead of printing zero detections for every model.
pub fn cards(
    health: &[(String, ModelHealth)],
    leads: Option<&[ModelLead]>,
    now: u64,
) -> Vec<ModelCard> {
    health
        .iter()
        .map(|(id, h)| {
            let state = if h.is_failing() {
                ModelState::Failing
            } else if h.windows == 0 {
                ModelState::Waiting
            } else {
                ModelState::Running
            };
            // A classifier that has led nothing today has no row in the
            // grouped query, which is a count of zero, not a missing one.
            let lead = leads.map(|all| {
                all.iter()
                    .find(|l| &l.model_id == id)
                    .cloned()
                    .unwrap_or_else(|| ModelLead {
                        model_id: id.clone(),
                        detections: 0,
                        agreed: 0,
                    })
            });
            ModelCard {
                id: id.clone(),
                state,
                windows: h.windows,
                last_window_age: h.last_window_at.map(|t| now.saturating_sub(t)),
                last_success_age: h.last_success_at.map(|t| now.saturating_sub(t)),
                error: h.error_class,
                consecutive_failures: h.consecutive_failures,
                lead,
            }
        })
        .collect()
}

/// The section body: a lede, then one card per classifier. Empty before the
/// daemon has reported a single window, because there is nothing to show yet.
pub fn section(cards: &[ModelCard]) -> String {
    if cards.is_empty() {
        return String::new();
    }
    let several = cards.len() >= 2;
    let lede = if several {
        "Your station is running several classifiers over the same audio. Each is judged \
         against its own bar and each keeps its own score; the scores are never averaged."
    } else {
        "The classifier your station is running."
    };
    let mut out = format!(
        "<h2 class=\"st-h3\">Models</h2><p class=\"bnb-meta ws-lede\">{lede}</p><div class=\"st-sources ws-models\">"
    );
    for c in cards {
        out.push_str(&card(c, several));
    }
    out.push_str("</div>");
    out
}

const fn chip(state: ModelState) -> &'static str {
    match state {
        ModelState::Running => {
            "<span class=\"bnb-pill moss\"><span class=\"bnb-dot live\"></span> Running</span>"
        }
        ModelState::Failing => {
            "<span class=\"bnb-pill rare\"><span class=\"bnb-dot rare\"></span> Failing</span>"
        }
        ModelState::Waiting => {
            "<span class=\"bnb-pill\"><span class=\"bnb-dot\"></span> Waiting for audio</span>"
        }
    }
}

/// How long ago, in the words the rest of the Health screen uses.
fn ago(secs: Option<u64>) -> String {
    secs.map_or_else(
        || "never".to_owned(),
        super::station_health::format_freshness,
    )
}

fn card(c: &ModelCard, several: bool) -> String {
    let failing = if c.state == ModelState::Failing {
        " stalled"
    } else {
        ""
    };
    let day = day_line(c, several);
    let fault = fault_line(c);
    format!(
        "<div class=\"bnb-card st-source{failing} ws-model\"><div class=\"st-source-head\">\
         <div><div class=\"st-source-name mono\">{id}</div>\
         <div class=\"st-source-type\">classifier</div></div>{chip}</div>\
         {day}\
         <div class=\"st-source-foot\"><span><b>{last}</b> · last window</span>\
         <span><b>{windows}</b> · windows run</span></div>{fault}</div>",
        id = escape_html(&c.id),
        chip = chip(c.state),
        last = ago(c.last_window_age),
        windows = super::group_thousands(i64::try_from(c.windows).unwrap_or(i64::MAX)),
    )
}

/// Today's share, and agreement only when there is a second classifier to agree.
fn day_line(c: &ModelCard, several: bool) -> String {
    let Some(lead) = &c.lead else {
        return "<p class=\"bnb-meta ws-day\">Today's detections could not be read.</p>".to_owned();
    };
    let n = super::group_thousands(lead.detections);
    if !several {
        return format!(
            "<p class=\"ws-day\"><b class=\"mono\">{n}</b> detections today carry this classifier's answer.</p>"
        );
    }
    if lead.detections == 0 {
        return "<p class=\"ws-day\"><b class=\"mono\">0</b> detections today carry this \
                classifier's answer.</p>"
            .to_owned();
    }
    format!(
        "<p class=\"ws-day\"><b class=\"mono\">{n}</b> detections today carry this classifier's \
         answer; <b class=\"mono\">{agreed}</b> of them another classifier also cleared.</p>",
        agreed = super::group_thousands(lead.agreed),
    )
}

/// The reason line, only while a run of failures lasts.
fn fault_line(c: &ModelCard) -> String {
    if c.state != ModelState::Failing {
        return String::new();
    }
    let why = match c.error {
        Some(ErrorClass::NonFiniteOutput) => "it is answering with invalid scores",
        Some(ErrorClass::Inference) | None => "its windows are returning errors",
    };
    format!(
        "<div class=\"st-source-retry\">\u{2715} {n} windows in a row failed: {why}. Last good \
         answer: {last}.</div>",
        n = c.consecutive_failures,
        last = ago(c.last_success_age),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_core::inference::health::FAILING_AFTER;

    fn ok(windows: u64, last: u64) -> ModelHealth {
        ModelHealth {
            consecutive_failures: 0,
            windows,
            last_window_at: Some(last),
            last_success_at: Some(last),
            error_class: None,
        }
    }

    fn lead(id: &str, detections: i64, agreed: i64) -> ModelLead {
        ModelLead {
            model_id: id.to_owned(),
            detections,
            agreed,
        }
    }

    #[test]
    fn a_station_the_daemon_has_not_reported_on_has_no_section() {
        assert_eq!(section(&cards(&[], Some(&[]), 1_000)), "");
    }

    #[test]
    fn the_state_comes_from_the_window_record() {
        let mut failing = ok(20, 900);
        failing.consecutive_failures = FAILING_AFTER;
        failing.error_class = Some(ErrorClass::Inference);
        let health = vec![
            ("a".to_owned(), ok(10, 990)),
            ("b".to_owned(), failing),
            ("c".to_owned(), ModelHealth::default()),
        ];
        let got = cards(&health, Some(&[]), 1_000);
        let states: Vec<_> = got.iter().map(|c| c.state).collect();
        assert_eq!(
            states,
            [
                ModelState::Running,
                ModelState::Failing,
                ModelState::Waiting
            ]
        );
        assert_eq!(got[0].last_window_age, Some(10));
        assert_eq!(got[2].last_window_age, None);
    }

    #[test]
    fn a_classifier_that_led_nothing_today_is_zero_not_missing() {
        let health = vec![("a".to_owned(), ok(1, 1)), ("b".to_owned(), ok(1, 1))];
        let got = cards(&health, Some(&[lead("a", 5, 2)]), 10);
        assert_eq!(got[1].lead, Some(lead("b", 0, 0)));
    }

    #[test]
    fn a_failed_database_read_is_said_not_rounded_to_zero() {
        let health = vec![("a".to_owned(), ok(1, 1))];
        let html = section(&cards(&health, None, 10));
        assert!(html.contains("could not be read"), "{html}");
        assert!(!html.contains("<b class=\"mono\">0</b>"), "{html}");
    }

    #[test]
    fn agreement_is_claimed_only_when_there_is_a_second_classifier() {
        let one = section(&cards(
            &[("a".to_owned(), ok(3, 5))],
            Some(&[lead("a", 40, 0)]),
            10,
        ));
        assert!(
            one.contains("The classifier your station is running."),
            "{one}"
        );
        assert!(!one.contains("another classifier also cleared"), "{one}");
        assert!(one.contains(">40<"), "{one}");

        let two = section(&cards(
            &[("a".to_owned(), ok(3, 5)), ("b".to_owned(), ok(3, 5))],
            Some(&[lead("a", 40, 12), lead("b", 9, 0)]),
            10,
        ));
        assert!(two.contains("never averaged"), "{two}");
        assert!(two.contains(">12<"), "{two}");
        assert!(two.contains("another classifier also cleared"), "{two}");
    }

    #[test]
    fn a_failing_classifier_says_why_and_when_it_last_worked() {
        let mut h = ok(50, 100);
        h.consecutive_failures = FAILING_AFTER;
        h.error_class = Some(ErrorClass::NonFiniteOutput);
        h.last_window_at = Some(990);
        let html = section(&cards(&[("bats".to_owned(), h)], Some(&[]), 1_000));
        assert!(html.contains("Failing"), "{html}");
        assert!(html.contains("invalid scores"), "{html}");
        assert!(
            html.contains(&format!("{FAILING_AFTER} windows in a row failed")),
            "{html}"
        );
        assert!(html.contains("st-source stalled"), "{html}");
    }

    #[test]
    fn a_healthy_card_has_no_fault_line_and_an_operator_typed_id_is_escaped() {
        let html = section(&cards(&[("<i>x</i>".to_owned(), ok(2, 9))], Some(&[]), 10));
        assert!(!html.contains("windows in a row failed"), "{html}");
        assert!(!html.contains("<i>x</i>"), "{html}");
        assert!(html.contains("&lt;i&gt;x&lt;/i&gt;"), "{html}");
    }

    #[test]
    fn a_classifier_that_never_ran_a_window_says_never_not_a_time() {
        let html = section(&cards(
            &[("a".to_owned(), ModelHealth::default())],
            Some(&[]),
            10,
        ));
        assert!(html.contains("Waiting for audio"), "{html}");
        assert!(html.contains("<b>never</b> · last window"), "{html}");
    }
}
