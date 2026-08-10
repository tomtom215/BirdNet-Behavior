//! Detection thresholds settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

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
    // `type="text" inputmode="decimal"` lets the operator type either
    // `0.75` (en) or `0,75` (de/es/fr/…) on any keyboard. A bare
    // `type="number"` accepts comma in EU browser locales but then
    // silently strips it to `075` on submit, corrupting the value 100×.
    // Server-side normalisation in `build_settings_items` converts both
    // shapes to the canonical period form before storage.
    write!(out,
        r#"
  <div class="card">
    <div class="section-title">Detection Settings</div>
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
  </div>"#
    ).unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;

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
