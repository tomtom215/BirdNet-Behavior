//! Location & recording schedule settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let lat = get_setting(s, "latitude", "");
    let lon = get_setting(s, "longitude", "");
    let station = get_setting(s, "station_name", "");
    let inhibit = get_setting(s, "night_inhibit", "false");
    let pre = get_setting(s, "pre_sunrise_offset", "0");
    let post = get_setting(s, "post_sunset_offset", "0");
    let inh_yes = if inhibit == "true" { " selected" } else { "" };
    let inh_no = if inhibit == "true" { "" } else { " selected" };
    write!(out, r#"
  <div class="card">
    <div class="section-title">Location &amp; Recording Schedule</div>
    <div class="grid-2">
      <div>
        <label for="latitude">Latitude</label>
        <input id="latitude" name="latitude" value="{lat}" placeholder="e.g. 51.5074">
      </div>
      <div>
        <label for="longitude">Longitude</label>
        <input id="longitude" name="longitude" value="{lon}" placeholder="e.g. -0.1278">
      </div>
    </div>
    <div>
      <label for="station_name">Station Name</label>
      <input id="station_name" name="station_name" value="{station}" placeholder="e.g. My Garden, London">
      <p class="hint">Used in BirdWeather uploads and export metadata</p>
    </div>
    <div class="grid-2">
      <div>
        <label for="night_inhibit">Night Inhibit (suppress recording in darkness)</label>
        <select id="night_inhibit" name="night_inhibit">
          <option value="true"{inh_yes}>Yes — only record near sunrise/sunset</option>
          <option value="false"{inh_no}>No — record 24h</option>
        </select>
        <p class="hint">Requires latitude/longitude to compute sunrise/sunset</p>
      </div>
      <div>
        <label for="pre_sunrise_offset">Extra minutes before sunrise</label>
        <input id="pre_sunrise_offset" name="pre_sunrise_offset" type="number" value="{pre}" min="0" max="120" style="max-width:120px">
        <br>
        <label for="post_sunset_offset" style="margin-top:0.5rem;">Extra minutes after sunset</label>
        <input id="post_sunset_offset" name="post_sunset_offset" type="number" value="{post}" min="0" max="120" style="max-width:120px">
      </div>
    </div>
  </div>"#).unwrap_or_default();
}
