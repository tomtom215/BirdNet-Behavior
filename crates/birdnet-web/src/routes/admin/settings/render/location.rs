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
    // The recording window itself. The page has always offered the sunrise /
    // sunset *offsets* while the mode they modify was CLI-only — and until
    // 0.12.0 the runtime ignored the configured mode entirely, so a station
    // set to `solar` recorded around the clock. `fixed:` windows keep their
    // free-text form because the spec carries the hours.
    let schedule = get_setting(s, "recording_schedule", "all-day");
    write!(out, r#"
  <section class="card" id="set-location" aria-labelledby="set-location-h">
    <h2 class="section-title" id="set-location-h">Location &amp; Recording Schedule</h2>
    <div class="grid-2">
      <div>
        <label for="latitude">Latitude</label>
        <input id="latitude" name="latitude" type="text" inputmode="decimal"
               pattern="-?[0-9]*[.,]?[0-9]*" value="{lat}"
               placeholder="e.g. 51.5074 or 51,5074">
      </div>
      <div>
        <label for="longitude">Longitude</label>
        <input id="longitude" name="longitude" type="text" inputmode="decimal"
               pattern="-?[0-9]*[.,]?[0-9]*" value="{lon}"
               placeholder="e.g. -0.1278 or -0,1278">
      </div>
    </div>
    <p class="hint flush">Decimal degrees. Either <code>.</code> or <code>,</code> works as the separator.</p>
    <div>
      <label for="station_name">Station Name</label>
      <input id="station_name" name="station_name" value="{station}" placeholder="e.g. My Garden, London">
      <p class="hint">Used in BirdWeather uploads and export metadata</p>
    </div>
    <div class="grid-2">
      <div>
        <label for="recording_schedule">Recording window</label>
        <input id="recording_schedule" name="recording_schedule" type="text" list="bnb-schedule-presets"
               value="{schedule}" placeholder="all-day">
        <datalist id="bnb-schedule-presets">
          <option value="all-day">Record continuously</option>
          <option value="solar">Sunrise to sunset (needs coordinates)</option>
          <option value="fixed:06:00-20:00">Fixed hours, UTC</option>
        </datalist>
        <p class="hint"><code>all-day</code>, <code>solar</code>, or <code>fixed:HH:MM-HH:MM</code>. Fixed hours are evaluated in <strong>UTC</strong>, not local time; solar needs no timezone. Until 0.12.0 this was settable only on the command line (BirdNET-Pi: RECORDING_SCHEDULE)</p>
      </div>
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
        <input id="pre_sunrise_offset" name="pre_sunrise_offset" type="number" value="{pre}" min="0" max="120" class="bnb-w-num">
        <br>
        <label for="post_sunset_offset" class="mt-sm">Extra minutes after sunset</label>
        <input id="post_sunset_offset" name="post_sunset_offset" type="number" value="{post}" min="0" max="120" class="bnb-w-num">
      </div>
    </div>
  </section>"#).unwrap_or_default();
}
