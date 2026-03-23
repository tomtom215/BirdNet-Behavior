//! Detection thresholds settings section.

use std::collections::HashMap;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let conf = get_setting(s, "confidence_threshold", "0.70");
    let sens = get_setting(s, "sensitivity", "1.0");
    let over = get_setting(s, "overlap", "0.0");
    let sf = get_setting(s, "sf_thresh", "0.03");
    let priv_t = get_setting(s, "privacy_threshold", "0.0");
    out.push_str(&format!(
        r#"
  <div class="card">
    <div class="section-title">Detection Settings</div>
    <div class="grid-2">
      <div>
        <label for="confidence_threshold">Minimum Confidence (0–1)</label>
        <input id="confidence_threshold" name="confidence_threshold" type="number"
               value="{conf}" min="0" max="1" step="0.05">
        <p class="hint">Detections below this threshold are discarded (BirdNET-Pi: CONFIDENCE)</p>
      </div>
      <div>
        <label for="sensitivity">Sensitivity (0.5–1.5)</label>
        <input id="sensitivity" name="sensitivity" type="number"
               value="{sens}" min="0.5" max="1.5" step="0.05">
        <p class="hint">Higher = more sensitive, more false positives (BirdNET-Pi: SENSITIVITY)</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="overlap">Analysis Overlap (0–2.9 seconds)</label>
        <input id="overlap" name="overlap" type="number"
               value="{over}" min="0" max="2.9" step="0.1" style="max-width:120px">
        <p class="hint">Overlap between 3-second analysis windows. Higher = more CPU (BirdNET-Pi: OVERLAP)</p>
      </div>
      <div>
        <label for="sf_thresh">Species Frequency Threshold (0–1)</label>
        <input id="sf_thresh" name="sf_thresh" type="number"
               value="{sf}" min="0" max="1" step="0.01" style="max-width:120px">
        <p class="hint">Filter unlikely species by occurrence frequency. Lower = more species. 0 = disabled (BirdNET-Pi: SF_THRESH)</p>
      </div>
    </div>
    <div>
      <label for="privacy_threshold">Privacy Threshold (0 = disabled)</label>
      <input id="privacy_threshold" name="privacy_threshold" type="number"
             value="{priv_t}" min="0" max="1" step="0.01" style="max-width:120px">
      <p class="hint">Suppress detections when human voice is detected. Typical: 0.01–0.03. 0 = disabled (BirdNET-Pi: PRIVACY_THRESHOLD)</p>
    </div>
  </div>"#
    ));
}
