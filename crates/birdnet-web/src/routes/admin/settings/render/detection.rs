//! Detection thresholds settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let conf = get_setting(s, "confidence_threshold", "0.70");
    let sens = get_setting(s, "sensitivity", "1.0");
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
               value="{conf}" placeholder="0.70">
        <p class="hint">Detections below this threshold are discarded. Decimal separator: <code>.</code> or <code>,</code> (BirdNET-Pi: CONFIDENCE)</p>
      </div>
      <div>
        <label for="sensitivity">Sensitivity (0.5–1.5)</label>
        <input id="sensitivity" name="sensitivity" type="text"
               inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
               value="{sens}" placeholder="1.0">
        <p class="hint">Higher = more sensitive, more false positives (BirdNET-Pi: SENSITIVITY)</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="overlap">Analysis Overlap (0–2.9 seconds)</label>
        <input id="overlap" name="overlap" type="text"
               inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
               value="{over}" placeholder="0.0" style="max-width:120px">
        <p class="hint">Overlap between 3-second analysis windows. Higher = more CPU (BirdNET-Pi: OVERLAP)</p>
      </div>
      <div>
        <label for="sf_thresh">Species Frequency Threshold (0–1)</label>
        <input id="sf_thresh" name="sf_thresh" type="text"
               inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
               value="{sf}" placeholder="0.03" style="max-width:120px">
        <p class="hint">Filter unlikely species by occurrence frequency. Lower = more species. 0 = disabled (BirdNET-Pi: SF_THRESH)</p>
      </div>
    </div>
    <div>
      <label for="privacy_threshold">Privacy Threshold (0 = disabled)</label>
      <input id="privacy_threshold" name="privacy_threshold" type="text"
             inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
             value="{priv_t}" placeholder="0.0" style="max-width:120px">
      <p class="hint">Suppress detections when human voice is detected. Typical: 0.01–0.03. 0 = disabled (BirdNET-Pi: PRIVACY_THRESHOLD)</p>
    </div>
  </div>"#
    ).unwrap_or_default();
}
