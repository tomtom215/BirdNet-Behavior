//! The Models board on Station Health: each classifier the station runs,
//! side by side.
//!
//! Before this, a station running two classifiers showed one undifferentiated
//! stream of detections, and the only trace of a classifier on the Health
//! screen was a banner sentence if one of them failed ten windows in a row.
//!
//! # Only what is real
//!
//! Every figure on a card is read from something the station records: the
//! window record the daemon keeps per classifier
//! ([`birdnet_core::inference::health::ModelHealth`], which includes the time
//! each answered window took), what each classifier is
//! ([`birdnet_core::inference::registry::ModelInfo`], stated by the daemon once
//! they have loaded), and the `model_id` / `model_agreement` columns of
//! `detections` (migration 50). A figure the station has not recorded is left
//! out, not printed as a dash: a card that has not yet had a window answered has
//! no mean time, and a station with no daemon (web-only) has no descriptors.
//!
//! The mean time per window is wall-clock time on this machine since the
//! classifier loaded, so it includes the first window's warm-up and whatever
//! else the board was doing. It is a measurement of this station, not a
//! benchmark of the model.
//!
//! # Agreement is only claimed when there is something to agree
//!
//! "Also cleared by another model" is printed only when more than one
//! classifier is running. On a one-classifier station every row reads
//! `model_agreement = 1`, and "0 agreed" would say a second opinion was sought
//! and withheld.

use birdnet_core::inference::health::{ErrorClass, ModelHealth};
use birdnet_core::inference::registry::ModelInfo;
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
#[derive(Debug, Clone, PartialEq)]
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
    /// What it is, when the daemon has stated it.
    pub info: Option<ModelInfo>,
    /// Mean microseconds per answered window since it loaded; `None` before it
    /// has answered one.
    pub mean_window_micros: Option<u64>,
}

/// Build the cards from the daemon's health record and today's per-model counts.
///
/// `leads` is `None` when the database read failed: the cards then say so
/// instead of printing zero detections for every model.
pub fn cards(
    health: &[(String, ModelHealth)],
    leads: Option<&[ModelLead]>,
    infos: &[ModelInfo],
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
                info: infos.iter().find(|i| &i.id == id).cloned(),
                mean_window_micros: h.mean_inference_micros(),
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
    let facts = c.info.as_ref().map_or_else(String::new, facts_line);
    let mean = c.mean_window_micros.map_or_else(String::new, |us| {
        format!("<span><b>{}</b> · per window</span>", fmt_duration(us))
    });
    format!(
        "<div class=\"bnb-card st-source{failing} ws-model\"><div class=\"st-source-head\">\
         <div><div class=\"st-source-name mono\">{id}</div>\
         <div class=\"st-source-type\">classifier</div></div>{chip}</div>\
         {facts}{day}\
         <div class=\"st-source-foot ws-foot\"><span><b>{last}</b> · last window</span>\
         {mean}<span><b>{windows}</b> · windows run</span></div>{fault}</div>",
        id = escape_html(&c.id),
        chip = chip(c.state),
        last = ago(c.last_window_age),
        windows = super::group_thousands(i64::try_from(c.windows).unwrap_or(i64::MAX)),
    )
}

/// What the classifier is, in one line: window, rate, classes and bar.
fn facts_line(info: &ModelInfo) -> String {
    format!(
        "<p class=\"ws-facts mono\">{window} window · {rate} · {labels} classes · bar {bar}</p>",
        window = fmt_seconds(info.window_secs),
        rate = fmt_khz(info.sample_rate),
        labels = super::group_thousands(i64::try_from(info.labels).unwrap_or(i64::MAX)),
        bar = fmt_percent(info.bar),
    )
}

/// `4.5 s`, `3 s`: one decimal unless it is a whole number of seconds.
fn fmt_seconds(secs: f32) -> String {
    let tenths = (secs * 10.0).round();
    if (tenths % 10.0).abs() < f32::EPSILON {
        format!("{:.0} s", tenths / 10.0)
    } else {
        format!("{:.1} s", tenths / 10.0)
    }
}

/// `32 kHz`, `44.1 kHz`.
fn fmt_khz(hz: u32) -> String {
    if hz.is_multiple_of(1000) {
        format!("{} kHz", hz / 1000)
    } else {
        format!("{:.1} kHz", f64::from(hz) / 1000.0)
    }
}

/// A 0 to 1 bar as the whole percent the rest of the app prints.
fn fmt_percent(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

/// A duration given in microseconds, in the unit a person reads it in: under 10
/// ms to a tenth, under a second in whole milliseconds, then seconds.
#[allow(clippy::cast_precision_loss)]
fn fmt_duration(micros: u64) -> String {
    let ms = micros as f64 / 1000.0;
    if ms < 10.0 {
        format!("{ms:.1} ms")
    } else if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
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
            ..ModelHealth::default()
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
        assert_eq!(section(&cards(&[], Some(&[]), &[], 1_000)), "");
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
        let got = cards(&health, Some(&[]), &[], 1_000);
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
        let got = cards(&health, Some(&[lead("a", 5, 2)]), &[], 10);
        assert_eq!(got[1].lead, Some(lead("b", 0, 0)));
    }

    #[test]
    fn a_failed_database_read_is_said_not_rounded_to_zero() {
        let health = vec![("a".to_owned(), ok(1, 1))];
        let html = section(&cards(&health, None, &[], 10));
        assert!(html.contains("could not be read"), "{html}");
        assert!(!html.contains("<b class=\"mono\">0</b>"), "{html}");
    }

    #[test]
    fn agreement_is_claimed_only_when_there_is_a_second_classifier() {
        let one = section(&cards(
            &[("a".to_owned(), ok(3, 5))],
            Some(&[lead("a", 40, 0)]),
            &[],
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
            &[],
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
        let html = section(&cards(&[("bats".to_owned(), h)], Some(&[]), &[], 1_000));
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
        let html = section(&cards(
            &[("<i>x</i>".to_owned(), ok(2, 9))],
            Some(&[]),
            &[],
            10,
        ));
        assert!(!html.contains("windows in a row failed"), "{html}");
        assert!(!html.contains("<i>x</i>"), "{html}");
        assert!(html.contains("&lt;i&gt;x&lt;/i&gt;"), "{html}");
    }

    #[test]
    fn a_classifier_that_never_ran_a_window_says_never_not_a_time() {
        let html = section(&cards(
            &[("a".to_owned(), ModelHealth::default())],
            Some(&[]),
            &[],
            10,
        ));
        assert!(html.contains("Waiting for audio"), "{html}");
        assert!(html.contains("<b>never</b> · last window"), "{html}");
    }

    fn info(id: &str, window_secs: f32, rate: u32, labels: usize, bar: f32) -> ModelInfo {
        ModelInfo {
            id: id.to_owned(),
            window_secs,
            sample_rate: rate,
            labels,
            bar,
        }
    }

    #[test]
    fn what_a_classifier_is_is_stated_only_when_the_daemon_has_stated_it() {
        let health = vec![("v3".to_owned(), ok(5, 9))];
        let without = section(&cards(&health, Some(&[]), &[], 10));
        assert!(!without.contains("ws-facts"), "{without}");
        let with = section(&cards(
            &health,
            Some(&[]),
            &[info("v3", 4.5, 32_000, 11_560, 0.70)],
            10,
        ));
        assert!(
            with.contains("4.5 s window · 32 kHz · 11,560 classes · bar 70%"),
            "{with}"
        );
    }

    #[test]
    fn a_descriptor_is_matched_to_its_own_classifier_by_id() {
        let health = vec![("a".to_owned(), ok(1, 1)), ("b".to_owned(), ok(1, 1))];
        let got = cards(
            &health,
            Some(&[]),
            &[
                info("b", 5.0, 32_000, 14_795, 0.5),
                info("a", 3.0, 48_000, 6_522, 0.7),
            ],
            10,
        );
        assert_eq!(got[0].info.as_ref().map(|i| i.labels), Some(6_522));
        assert_eq!(got[1].info.as_ref().map(|i| i.labels), Some(14_795));
        assert!(
            cards(&health, Some(&[]), &[info("zzz", 1.0, 1, 1, 0.1)], 10)[0]
                .info
                .is_none()
        );
    }

    #[test]
    fn the_mean_window_time_is_shown_only_once_a_window_has_answered() {
        let mut h = ok(3, 9);
        h.answered_micros = 1_250_000;
        h.timed_windows = 3;
        let shown = section(&cards(&[("v3".to_owned(), h)], Some(&[]), &[], 10));
        assert!(shown.contains("<b>417 ms</b> · per window"), "{shown}");
        let untimed = section(&cards(&[("v3".to_owned(), ok(3, 9))], Some(&[]), &[], 10));
        assert!(
            !untimed.contains("per window"),
            "no number, not a dash: {untimed}"
        );
    }

    #[test]
    fn durations_sample_rates_and_windows_are_written_the_way_a_person_reads_them() {
        assert_eq!(fmt_duration(800), "0.8 ms");
        assert_eq!(fmt_duration(9_000), "9.0 ms");
        assert_eq!(fmt_duration(10_000), "10 ms");
        assert_eq!(fmt_duration(999_000), "999 ms");
        assert_eq!(fmt_duration(1_000_000), "1.0 s");
        assert_eq!(fmt_duration(412_000), "412 ms");
        assert_eq!(fmt_duration(1_500_000), "1.5 s");
        assert_eq!(fmt_khz(32_000), "32 kHz");
        assert_eq!(fmt_khz(44_100), "44.1 kHz");
        assert_eq!(fmt_seconds(3.0), "3 s");
        assert_eq!(fmt_seconds(4.5), "4.5 s");
        assert_eq!(fmt_percent(0.7), "70%");
    }
}
