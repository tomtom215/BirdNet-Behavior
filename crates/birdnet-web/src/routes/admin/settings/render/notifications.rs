//! Apprise + `BirdWeather` notification settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::get_setting;

#[allow(clippy::too_many_lines)]
pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let apprise = get_setting(s, "apprise_url", "");
    let apprise_cfg = get_setting(s, "apprise_config", "");
    let bw = get_setting(s, "birdweather_token", "");
    let nconf = get_setting(s, "notify_confidence", "0.80");
    let ncool = get_setting(s, "notify_cooldown", "300");
    let trigger = get_setting(s, "notify_trigger", "each");
    let t_each = if trigger == "each" { " selected" } else { "" };
    let t_new = if trigger == "new-species" {
        " selected"
    } else {
        ""
    };
    let t_daily = if trigger == "new-species-daily" {
        " selected"
    } else {
        ""
    };
    let only = get_setting(s, "notify_species_only", "");
    let nexcl = get_setting(s, "notify_species_exclude", "");
    let title_tmpl = get_setting(s, "notify_title_template", "");
    let body_tmpl = get_setting(s, "notify_body_template", "");
    let weekly = get_setting(s, "weekly_report_schedule", "monday");
    // Both were command-line only until 0.12.0, which put the two ways to find
    // out that a station has gone quiet out of reach of anyone not on a
    // terminal — the operators least likely to notice by other means.
    let hb = get_setting(s, "heartbeat_url", "");
    let deadman = get_setting(s, "deadman_hours", "24");
    let weekly_opts = render_weekly_options(weekly);
    write!(out, r#"
  <section class="card" id="set-notifications" aria-labelledby="set-notifications-h">
    <h2 class="section-title" id="set-notifications-h">Notifications (Apprise)</h2>
    <div class="grid-2">
      <div>
        <label for="apprise_url">Apprise Server URL</label>
        <input id="apprise_url" name="apprise_url" value="{apprise}" placeholder="http://localhost:8000">
        <p class="hint">Leave blank to disable HTTP push notifications (BirdNET-Pi: APPRISE_URL)</p>
      </div>
      <div>
        <label for="apprise_config">Apprise Config File (CLI mode)</label>
        <input id="apprise_config" name="apprise_config" value="{apprise_cfg}" placeholder="/etc/birdnet/apprise.yml">
        <p class="hint">Use apprise CLI with -c flag for 80+ notification services (BirdNET-Pi: APPRISE_CONFIG_FILE)</p>
      </div>
    </div>
    <div class="mt-sm">
      <a href="/admin/notifications/test" class="btn btn-primary btn-sm">
        Test Notifications
      </a>
    </div>
    <div class="grid-2 mt-md">
      <div>
        <label for="notify_trigger">Notification Trigger</label>
        <select id="notify_trigger" name="notify_trigger">
          <option value="each"{t_each}>Each detection</option>
          <option value="new-species"{t_new}>New species (this week)</option>
          <option value="new-species-daily"{t_daily}>New species (each day)</option>
        </select>
        <p class="hint">When to send notifications (BirdNET-Pi: APPRISE_NOTIFY_EACH_DETECTION etc.)</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="notify_confidence">Notification Min Confidence</label>
        <input id="notify_confidence" name="notify_confidence" type="text"
               inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
               value="{nconf}" placeholder="0.80">
        <p class="hint">Only notify above this confidence. Either <code>.</code> or <code>,</code> works (BirdNET-Pi: APPRISE_MIN_CONFIDENCE)</p>
      </div>
      <div>
        <label for="notify_cooldown">Notification Cooldown (seconds)</label>
        <input id="notify_cooldown" name="notify_cooldown" type="text"
               inputmode="numeric" pattern="[0-9]*"
               value="{ncool}" placeholder="0">
        <p class="hint">Minimum time between notifications per species (BirdNET-Pi: MIN_SECONDS_BETWEEN_NOTIFICATIONS_PER_SPECIES)</p>
      </div>
    </div>
    <div>
      <label for="notify_species_only">Notify only for these species (comma-separated common names)</label>
      <textarea id="notify_species_only" name="notify_species_only" rows="2"
                placeholder="e.g. European Robin, Great Spotted Woodpecker">{only}</textarea>
      <p class="hint">Leave empty to notify for all species (BirdNET-Pi: APPRISE_ONLY_NOTIFY_SPECIES_NAMES)</p>
    </div>
    <div>
      <label for="notify_species_exclude">Never notify for these species (comma-separated common names)</label>
      <textarea id="notify_species_exclude" name="notify_species_exclude" rows="2"
                placeholder="e.g. House Sparrow, Feral Pigeon">{nexcl}</textarea>
      <p class="hint">Species excluded from all notifications (dual-filter with notify-only list above)</p>
    </div>
    <div>
      <label for="notify_title_template">Notification Title Template</label>
      <input id="notify_title_template" name="notify_title_template" value="{title_tmpl}"
             placeholder="Bird Detection: $comname">
      <p class="hint">Variables: $comname $sciname $confidence $confidencepct $date $time $week $latitude $longitude. Leave blank for default. (BirdNET-Pi: APPRISE_TITLE_TEMPLATE)</p>
    </div>
    <div>
      <label for="notify_body_template">Notification Body Template</label>
      <textarea id="notify_body_template" name="notify_body_template" rows="2"
                placeholder="$comname ($sciname) detected ($confidencepct% confidence) at $time on $date">{body_tmpl}</textarea>
      <p class="hint">Leave blank for default template. (BirdNET-Pi: APPRISE_BODY_TEMPLATE)</p>
    </div>
    <div class="grid-2">
      <div>
        <label for="birdweather_token">BirdWeather Station Token</label>
        <input id="birdweather_token" name="birdweather_token" value="{bw}" placeholder="Token from BirdWeather app">
        <p class="hint">Uploads detections to BirdWeather community map (BirdNET-Pi: BIRDWEATHER_ID)</p>
      </div>
      <div>
        <label for="weekly_report_schedule">Weekly Report Day</label>
        <select id="weekly_report_schedule" name="weekly_report_schedule">
          {weekly_opts}
        </select>
        <p class="hint">Day to send weekly summary via Apprise. "Disabled" to turn off.</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="heartbeat_url">Heartbeat URL</label>
        <input id="heartbeat_url" name="heartbeat_url" type="url" value="{hb}"
               placeholder="https://hc-ping.com/…">
        <p class="hint">Pinged periodically so an outside monitor (Healthchecks.io, Uptime Kuma, …) can alert you when the station stops reporting. Leave blank to disable (BirdNET-Pi: HEARTBEAT_URL)</p>
      </div>
      <div>
        <label for="deadman_hours">Dead-man alert after (hours of no detections)</label>
        <input id="deadman_hours" name="deadman_hours" type="number" value="{deadman}"
               min="0" max="720" class="bnb-w-num">
        <p class="hint">Notifies you when nothing has been detected for this long — the symptom of a mic that died quietly. <b>0 disables it.</b> Long winter nights are normal, so keep this comfortably above your quietest stretch (BirdNET-Pi: DEADMAN_HOURS)</p>
      </div>
    </div>
  </section>"#).unwrap_or_default();
}

fn render_weekly_options(selected: &str) -> String {
    let days = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "disabled",
    ];
    let mut opts = String::new();
    for d in days {
        let sel = if selected == d { " selected" } else { "" };
        let label = d
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            + &d[1..];
        write!(opts, "<option value=\"{d}\"{sel}>{label}</option>").unwrap_or_default();
    }
    opts
}
