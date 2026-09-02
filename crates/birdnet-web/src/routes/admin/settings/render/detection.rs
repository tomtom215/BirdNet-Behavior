//! Detection thresholds settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use birdnet_core::detection::corroboration::{ConfirmationLevel, REFERENCE_SPAN};

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    // Display default mirrors the daemon's enforced default so the form never
    // advertises a threshold the station does not apply. `{:.2}` keeps the
    // familiar two-decimal form (0.70) from the shared 0.7 constant.
    let conf_default = format!("{:.2}", birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD);
    let conf = get_setting(s, "confidence_threshold", &conf_default);
    // Sensitivity default also mirrors the daemon's shared constant (BirdNET-Pi's
    // 1.25), so the form never advertises a value the station does not apply.
    let sens_default = format!("{:.2}", birdnet_core::config::DEFAULT_SENSITIVITY);
    let sens = get_setting(s, "sensitivity", &sens_default);
    let over = get_setting(s, "overlap", "0.0");
    let sf = get_setting(s, "sf_thresh", "0.03");
    let priv_t = get_setting(s, "privacy_threshold", "0.0");
    let confirm = get_setting(s, "confirmation_level", "off");
    // The one setting on this page whose effect depends on another setting on
    // the same page: with no overlap the gentler levels ask for agreement from
    // a neighbourhood of one, which every detection already has. Rather than
    // leave that to the manual, the option text carries the overlap each level
    // needs — computed from the filter, so it cannot drift from the filter.
    let chunk_secs =
        birdnet_core::detection::pipeline::PipelineConfig::default().chunk_duration_secs;
    let mut options = String::new();
    for level in [
        ConfirmationLevel::Off,
        ConfirmationLevel::Lenient,
        ConfirmationLevel::Moderate,
        ConfirmationLevel::Balanced,
        ConfirmationLevel::Strict,
    ] {
        let name = level.as_str();
        let selected = if name == confirm { " selected" } else { "" };
        let note = match level.minimum_overlap(chunk_secs) {
            None => "never rejects anything".to_owned(),
            Some(need) if need > 0.0 => {
                format!(
                    "{:.0}% — needs overlap {need} s or more",
                    level.required_fraction() * 100.0
                )
            }
            Some(_) => format!(
                "{:.0}% — works at any overlap",
                level.required_fraction() * 100.0
            ),
        };
        let _ = write!(
            options,
            r#"<option value="{name}"{selected}>{name} ({note})</option>"#
        );
    }
    // `type="text" inputmode="decimal"` lets the operator type either
    // `0.75` (en) or `0,75` (de/es/fr/…) on any keyboard. A bare
    // `type="number"` accepts comma in EU browser locales but then
    // silently strips it to `075` on submit, corrupting the value 100×.
    // Server-side normalisation in `build_settings_items` converts both
    // shapes to the canonical period form before storage.
    write!(out,
        r#"
  <section class="card" id="set-detection" aria-labelledby="set-detection-h">
    <h2 class="section-title" id="set-detection-h">Detection Settings</h2>
    <div class="grid-2">
      <div>
        <label for="confidence_threshold">Minimum Confidence (0–1)</label>
        <input id="confidence_threshold" name="confidence_threshold" type="text"
               inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
               value="{conf}" placeholder="{conf_default}">
        <p class="hint">Detections below this threshold are discarded. Decimal separator: <code>.</code> or <code>,</code> (BirdNET-Pi: CONFIDENCE)</p>
      </div>
      <div>
        <label for="sensitivity">Sensitivity (0.5–1.5)</label>
        <input id="sensitivity" name="sensitivity" type="text"
               inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
               value="{sens}" placeholder="{sens_default}">
        <p class="hint">Higher = more sensitive, more false positives. Applies to V2.4 models; the bundled V3.0 model uses calibrated probabilities and ignores it (BirdNET-Pi: SENSITIVITY)</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="overlap">Analysis Overlap (0–2.9 seconds)</label>
        <input id="overlap" name="overlap" type="text"
               inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
               value="{over}" placeholder="0.0" class="bnb-w-num">
        <p class="hint">Overlap between 3-second analysis windows. Higher = more CPU (BirdNET-Pi: OVERLAP)</p>
      </div>
      <div>
        <label for="sf_thresh">Species Frequency Threshold (0–1)</label>
        <input id="sf_thresh" name="sf_thresh" type="text"
               inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
               value="{sf}" placeholder="0.03" class="bnb-w-num">
        <p class="hint">Filter unlikely species by occurrence frequency. Lower = more species. 0 = disabled (BirdNET-Pi: SF_THRESH)</p>
      </div>
    </div>
    <div>
      <label for="privacy_threshold">Privacy Threshold (0 = disabled)</label>
      <input id="privacy_threshold" name="privacy_threshold" type="text"
             inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
             value="{priv_t}" placeholder="0.0" class="bnb-w-num">
      <p class="hint">Suppress detections when human voice is detected. Typical: 0.01–0.03. 0 = disabled (BirdNET-Pi: PRIVACY_THRESHOLD)</p>
    </div>
    <div>
      <label for="confirmation_level">Repeat Confirmation</label>
      <select id="confirmation_level" name="confirmation_level">{options}</select>
      <p class="hint">Record a species only when enough of the analysis windows within {REFERENCE_SPAN:.0} seconds heard the same thing. A real bird sings across several windows; a classifier artefact usually fires once. <strong>Needs Analysis Overlap above</strong> — with no overlap there is only one other window to agree with. Not a BirdNET-Pi setting.</p>
    </div>
  </section>"#
    ).unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_confirmation_select_shows_the_saved_level_and_not_the_default() {
        // A `<select>` renders every option whatever is saved, so the only
        // thing that carries the operator's choice back onto the page is which
        // option is marked `selected`. Getting that wrong is invisible from the
        // markup and silent in every test that only greps for the field name:
        // the page shows "off", the station runs `strict`, and the operator
        // has no way to tell which is true.
        let mut saved = HashMap::new();
        saved.insert("confirmation_level".to_string(), "strict".to_string());
        let mut out = String::new();
        render(&mut out, &saved);
        assert!(
            out.contains(r#"<option value="strict" selected>"#),
            "the saved level must come back marked selected:\n{out}"
        );
        assert!(
            !out.contains(r#"<option value="off" selected>"#),
            "exactly one option may be selected:\n{out}"
        );

        // Counterpart: with nothing saved the form must default to `off`, or
        // the page advertises a filter the station is not running.
        let mut fresh = String::new();
        render(&mut fresh, &HashMap::new());
        assert!(
            fresh.contains(r#"<option value="off" selected>"#),
            "with nothing saved the form must show `off`:\n{fresh}"
        );
    }

    #[test]
    fn every_confirmation_level_is_offered_with_the_overlap_it_needs() {
        // The page is where an operator meets this setting, and the interaction
        // that makes it useless — a gentle level at zero overlap — is between
        // two controls in the same section. The advice therefore has to be on
        // the option itself, and derived from the filter rather than typed out,
        // or it drifts the moment a fraction changes.
        let mut out = String::new();
        render(&mut out, &HashMap::new());
        let chunk_secs =
            birdnet_core::detection::pipeline::PipelineConfig::default().chunk_duration_secs;
        for level in [
            ConfirmationLevel::Lenient,
            ConfirmationLevel::Moderate,
            ConfirmationLevel::Balanced,
            ConfirmationLevel::Strict,
        ] {
            let need = level
                .minimum_overlap(chunk_secs)
                .expect("an enabled level has one");
            let expected = if need > 0.0 {
                format!("needs overlap {need} s or more")
            } else {
                "works at any overlap".to_owned()
            };
            assert!(
                out.contains(&format!(r#"<option value="{}""#, level.as_str())),
                "`{}` is missing from the select:\n{out}",
                level.as_str()
            );
            assert!(
                out.contains(&expected),
                "`{}` must be offered with its own advice ({expected}):\n{out}",
                level.as_str()
            );
        }
    }

    #[test]
    fn confidence_default_matches_daemon_constant() {
        // Regression guard for the UI/daemon default drift: with no saved
        // value the form must show exactly the threshold the daemon enforces
        // when `CONFIDENCE` is unset, so the operator never sees an advertised
        // threshold the station does not apply.
        let mut out = String::new();
        render(&mut out, &HashMap::new());
        let expected = format!(
            r#"value="{:.2}""#,
            birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD
        );
        assert!(
            out.contains(&expected),
            "confidence field should default to the shared constant ({expected})"
        );
        // And that shared default is the shipped 0.75, not the old 0.25.
        assert!(
            (birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD - 0.75).abs() < f32::EPSILON,
            "default confidence should be 0.75"
        );
    }

    #[test]
    fn sensitivity_default_matches_daemon_constant() {
        // Same drift guard for sensitivity: the form must show exactly the value
        // the daemon enforces when `SENSITIVITY` is unset.
        let mut out = String::new();
        render(&mut out, &HashMap::new());
        let expected = format!(
            r#"value="{:.2}""#,
            birdnet_core::config::DEFAULT_SENSITIVITY
        );
        assert!(
            out.contains(&expected),
            "sensitivity field should default to the shared constant ({expected})"
        );
        // And that shared default is BirdNET-Pi's 1.25.
        assert!(
            (birdnet_core::config::DEFAULT_SENSITIVITY - 1.25).abs() < f32::EPSILON,
            "default sensitivity should be 1.25"
        );
    }
}
