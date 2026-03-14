//! HTML rendering for the admin settings page.

use std::collections::HashMap;

pub(super) fn get_setting<'a>(
    map: &'a HashMap<String, String>,
    key: &str,
    default: &'a str,
) -> &'a str {
    map.get(key).map_or(default, String::as_str)
}

pub(super) fn render_settings_page(settings: &HashMap<String, String>) -> String {
    let form_html = render_settings_form(settings);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Admin Settings — BirdNet-Behavior</title>
    <script src="/static/htmx.min.js"></script>
    <link rel="stylesheet" href="/static/style.css">
    <style>
      body {{ background: #0f172a; color: #e2e8f0; font-family: system-ui,sans-serif; }}
      .container {{ max-width: 900px; margin: 0 auto; padding: 2rem 1rem; }}
      nav a {{ color: #94a3b8; text-decoration: none; margin-right: 1.5rem; }}
      nav a.active, nav a:hover {{ color: #38bdf8; }}
      .card {{ background: #1e293b; border: 1px solid #334155; border-radius: 0.75rem;
               padding: 1.5rem; margin-bottom: 1.5rem; }}
      .section-title {{ font-size: 1.1rem; font-weight: 600; color: #38bdf8;
                        margin-bottom: 1rem; border-bottom: 1px solid #334155; padding-bottom: 0.5rem; }}
      label {{ display: block; font-size: 0.85rem; color: #94a3b8; margin-bottom: 0.25rem; }}
      input, textarea, select {{ width: 100%; background: #0f172a; border: 1px solid #334155;
                                  border-radius: 0.375rem; padding: 0.5rem 0.75rem; color: #e2e8f0;
                                  font-size: 0.875rem; box-sizing: border-box; margin-bottom: 1rem; }}
      input:focus, textarea:focus {{ outline: none; border-color: #38bdf8; }}
      .grid-2 {{ display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }}
      .btn {{ padding: 0.5rem 1.5rem; border-radius: 0.375rem; border: none; cursor: pointer;
               font-weight: 600; font-size: 0.875rem; }}
      .btn-primary {{ background: #0ea5e9; color: #fff; }}
      .btn-primary:hover {{ background: #38bdf8; }}
      .alert-success {{ background: #064e3b; border: 1px solid #065f46; color: #6ee7b7;
                         border-radius: 0.375rem; padding: 0.75rem 1rem; margin-bottom: 1rem; }}
      .alert-error {{ background: #450a0a; border: 1px solid #7f1d1d; color: #fca5a5;
                       border-radius: 0.375rem; padding: 0.75rem 1rem; margin-bottom: 1rem; }}
      .hint {{ font-size: 0.75rem; color: #64748b; margin-top: -0.75rem; margin-bottom: 1rem; }}
    </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem; padding:1rem 0; border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/admin" class="active">Admin</a>
    <a href="/admin/species">Species Lists</a>
    <a href="/admin/migrate">Migration</a>
    <a href="/admin/system">System</a>
  </nav>

  <h1 style="font-size:1.5rem;font-weight:700;margin-bottom:1.5rem;color:#f1f5f9;">
    Admin Settings
  </h1>

  <div id="settings-feedback"></div>

  {form_html}
</div>
</body>
</html>"#
    )
}

pub(super) fn render_settings_form(settings: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(8192);
    render_form_open(&mut out);
    render_audio_section(&mut out, settings);
    render_location_section(&mut out, settings);
    render_detection_section(&mut out, settings);
    render_notifications_section(&mut out, settings);
    render_species_section(&mut out, settings);
    render_system_section(&mut out, settings);
    render_email_section(&mut out, settings);
    render_form_close(&mut out);
    out
}

fn render_form_open(out: &mut String) {
    out.push_str(
        r##"<form hx-post="/admin/settings" hx-target="#settings-feedback"
               hx-swap="innerHTML" hx-indicator="#save-spinner">"##,
    );
}

fn render_audio_section(out: &mut String, s: &HashMap<String, String>) {
    let alsa = get_setting(s, "alsa_device", "");
    let rtsp = get_setting(s, "rtsp_url", "");
    let seg = get_setting(s, "segment_duration", "15");
    out.push_str(&format!(r#"
  <div class="card">
    <div class="section-title">Audio Capture</div>
    <div class="grid-2">
      <div>
        <label for="alsa_device">ALSA Device</label>
        <input id="alsa_device" name="alsa_device" value="{alsa}" placeholder="e.g. plughw:1,0">
        <p class="hint">Leave blank to disable managed microphone capture</p>
      </div>
      <div>
        <label for="rtsp_url">RTSP URL</label>
        <input id="rtsp_url" name="rtsp_url" value="{rtsp}" placeholder="rtsp://camera.local:554/stream">
        <p class="hint">IP camera audio stream (requires ffmpeg)</p>
      </div>
    </div>
    <div>
      <label for="segment_duration">Segment Duration (seconds)</label>
      <input id="segment_duration" name="segment_duration" type="number" value="{seg}" min="5" max="60" style="max-width:120px">
    </div>
  </div>"#));
}

fn render_location_section(out: &mut String, s: &HashMap<String, String>) {
    let lat = get_setting(s, "latitude", "");
    let lon = get_setting(s, "longitude", "");
    let station = get_setting(s, "station_name", "");
    let inhibit = get_setting(s, "night_inhibit", "false");
    let pre = get_setting(s, "pre_sunrise_offset", "0");
    let post = get_setting(s, "post_sunset_offset", "0");
    let inh_yes = if inhibit == "true" { " selected" } else { "" };
    let inh_no = if inhibit != "true" { " selected" } else { "" };
    out.push_str(&format!(r#"
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
  </div>"#));
}

fn render_detection_section(out: &mut String, s: &HashMap<String, String>) {
    let conf = get_setting(s, "confidence_threshold", "0.70");
    let sens = get_setting(s, "sensitivity", "1.0");
    let over = get_setting(s, "overlap", "0.0");
    out.push_str(&format!(
        r#"
  <div class="card">
    <div class="section-title">Detection Settings</div>
    <div class="grid-2">
      <div>
        <label for="confidence_threshold">Minimum Confidence (0–1)</label>
        <input id="confidence_threshold" name="confidence_threshold" type="number"
               value="{conf}" min="0" max="1" step="0.05">
        <p class="hint">Detections below this threshold are discarded</p>
      </div>
      <div>
        <label for="sensitivity">Sensitivity</label>
        <input id="sensitivity" name="sensitivity" type="number"
               value="{sens}" min="0.5" max="1.5" step="0.05">
        <p class="hint">Higher = more sensitive (more false positives)</p>
      </div>
    </div>
    <div>
      <label for="overlap">Overlap (0–2.9 seconds)</label>
      <input id="overlap" name="overlap" type="number"
             value="{over}" min="0" max="2.9" step="0.1" style="max-width:120px">
    </div>
  </div>"#
    ));
}

fn render_notifications_section(out: &mut String, s: &HashMap<String, String>) {
    let apprise = get_setting(s, "apprise_url", "");
    let bw = get_setting(s, "birdweather_token", "");
    let nconf = get_setting(s, "notify_confidence", "0.80");
    let ncool = get_setting(s, "notify_cooldown", "300");
    out.push_str(&format!(r#"
  <div class="card">
    <div class="section-title">Notifications</div>
    <div>
      <label for="apprise_url">Apprise URL</label>
      <input id="apprise_url" name="apprise_url" value="{apprise}" placeholder="http://localhost:8000">
      <p class="hint">Leave blank to disable push notifications</p>
    </div>
    <div style="margin-top:0.5rem;">
      <a href="/admin/notifications/test" class="btn btn-primary" style="font-size:0.8rem;padding:0.3rem 0.8rem;text-decoration:none;">
        Test Notifications
      </a>
    </div>
    <div style="margin-top:1rem;">
      <label for="birdweather_token">BirdWeather Station Token</label>
      <input id="birdweather_token" name="birdweather_token" value="{bw}" placeholder="Token from BirdWeather app">
    </div>
    <div class="grid-2">
      <div>
        <label for="notify_confidence">Notification Min Confidence</label>
        <input id="notify_confidence" name="notify_confidence" type="number"
               value="{nconf}" min="0" max="1" step="0.05">
      </div>
      <div>
        <label for="notify_cooldown">Notification Cooldown (seconds)</label>
        <input id="notify_cooldown" name="notify_cooldown" type="number"
               value="{ncool}" min="0" step="60">
        <p class="hint">Minimum time between notifications for the same species</p>
      </div>
    </div>
  </div>"#));
}

fn render_species_section(out: &mut String, s: &HashMap<String, String>) {
    let excl = get_setting(s, "species_exclude", "");
    let incl = get_setting(s, "species_include", "");
    out.push_str(&format!(
        r#"
  <div class="card">
    <div class="section-title">Species Filters</div>
    <p class="hint" style="margin-bottom:1rem;">
      Or manage species lists interactively on the
      <a href="/admin/species" style="color:#38bdf8;">Species Lists</a> page.
    </p>
    <div>
      <label for="species_exclude">Excluded Species (comma-separated common names)</label>
      <textarea id="species_exclude" name="species_exclude" rows="3"
                placeholder="e.g. House Sparrow, Feral Pigeon">{excl}</textarea>
      <p class="hint">These species will never be saved or notified</p>
    </div>
    <div>
      <label for="species_include">Allow-list (empty = all species)</label>
      <textarea id="species_include" name="species_include" rows="3"
                placeholder="e.g. European Robin, Eurasian Blackbird">{incl}</textarea>
      <p class="hint">When set, only these species are saved or notified</p>
    </div>
  </div>"#
    ));
}

fn render_system_section(out: &mut String, s: &HashMap<String, String>) {
    let days = get_setting(s, "recording_days", "30");
    let imgcache = get_setting(s, "image_cache_dir", "");
    out.push_str(&format!(
        r#"
  <div class="card">
    <div class="section-title">System</div>
    <div class="grid-2">
      <div>
        <label for="recording_days">Keep Recordings (days)</label>
        <input id="recording_days" name="recording_days" type="number"
               value="{days}" min="1" max="365">
        <p class="hint">Audio files older than this are deleted automatically</p>
      </div>
      <div>
        <label for="image_cache_dir">Species Image Cache Directory</label>
        <input id="image_cache_dir" name="image_cache_dir" value="{imgcache}"
               placeholder="/var/lib/birdnet/images">
        <p class="hint">Leave blank to disable Wikipedia image caching</p>
      </div>
    </div>
  </div>"#
    ));
}

fn render_email_section(out: &mut String, s: &HashMap<String, String>) {
    let host = get_setting(s, "email_smtp_host", "");
    let port = get_setting(s, "email_smtp_port", "587");
    let user = get_setting(s, "email_smtp_user", "");
    let pass = get_setting(s, "email_smtp_pass", "");
    let from = get_setting(s, "email_from", "");
    let to = get_setting(s, "email_to", "");
    let name = get_setting(s, "email_from_name", "BirdNet-Behavior");
    let tls = get_setting(s, "email_starttls", "true");
    let tls_yes = if tls != "false" { " selected" } else { "" };
    let tls_no = if tls == "false" { " selected" } else { "" };
    let econf = get_setting(s, "email_min_confidence", "0.80");
    let ecool = get_setting(s, "email_cooldown_secs", "300");
    out.push_str(&format!(r#"
  <div class="card">
    <div class="section-title">Email Alerts (SMTP)</div>
    <p class="hint" style="margin-bottom:1rem;">Leave SMTP host blank to disable email alerts.</p>
    <div class="grid-2">
      <div>
        <label for="email_smtp_host">SMTP Host</label>
        <input id="email_smtp_host" name="email_smtp_host" value="{host}" placeholder="smtp.gmail.com">
      </div>
      <div>
        <label for="email_smtp_port">SMTP Port</label>
        <input id="email_smtp_port" name="email_smtp_port" type="number" value="{port}" min="1" max="65535" style="max-width:120px">
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_smtp_user">SMTP Username</label>
        <input id="email_smtp_user" name="email_smtp_user" value="{user}" placeholder="you@gmail.com">
      </div>
      <div>
        <label for="email_smtp_pass">SMTP Password / App Password</label>
        <input id="email_smtp_pass" name="email_smtp_pass" type="password" value="{pass}" placeholder="app-specific password">
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_from">From Address</label>
        <input id="email_from" name="email_from" value="{from}" placeholder="alerts@example.com">
      </div>
      <div>
        <label for="email_to">To Address</label>
        <input id="email_to" name="email_to" value="{to}" placeholder="you@example.com">
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_from_name">From Name</label>
        <input id="email_from_name" name="email_from_name" value="{name}" placeholder="BirdNet-Behavior">
      </div>
      <div>
        <label for="email_starttls">Use STARTTLS</label>
        <select id="email_starttls" name="email_starttls" style="max-width:180px">
          <option value="true"{tls_yes}>Yes (port 587)</option>
          <option value="false"{tls_no}>No — implicit TLS (port 465)</option>
        </select>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_min_confidence">Alert Min Confidence</label>
        <input id="email_min_confidence" name="email_min_confidence" type="number" value="{econf}" min="0" max="1" step="0.05">
        <p class="hint">Only email for detections above this threshold</p>
      </div>
      <div>
        <label for="email_cooldown_secs">Alert Cooldown (seconds)</label>
        <input id="email_cooldown_secs" name="email_cooldown_secs" type="number" value="{ecool}" min="0" step="60">
        <p class="hint">Min time between emails per species</p>
      </div>
    </div>
  </div>"#));
}

fn render_form_close(out: &mut String) {
    out.push_str(
        r#"
  <div style="display:flex; align-items:center; gap:1rem;">
    <button type="submit" class="btn btn-primary">Save Settings</button>
    <span id="save-spinner" class="htmx-indicator" style="color:#94a3b8; font-size:0.875rem;">
      Saving…
    </span>
    <span style="color:#64748b; font-size:0.8rem;">
      Most settings require a restart to take effect.
    </span>
  </div>
</form>"#,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_setting_default() {
        let map = HashMap::new();
        assert_eq!(get_setting(&map, "missing", "fallback"), "fallback");
    }

    #[test]
    fn get_setting_present() {
        let map = HashMap::from([("key".to_string(), "val".to_string())]);
        assert_eq!(get_setting(&map, "key", "default"), "val");
    }

    #[test]
    fn render_settings_form_contains_fields() {
        let settings = HashMap::new();
        let html = render_settings_form(&settings);
        assert!(html.contains("alsa_device"));
        assert!(html.contains("latitude"));
        assert!(html.contains("confidence_threshold"));
        assert!(html.contains("apprise_url"));
        assert!(html.contains("birdweather_token"));
        assert!(html.contains("email_smtp_host"));
        assert!(html.contains("email_to"));
        assert!(html.contains("night_inhibit"));
    }
}
