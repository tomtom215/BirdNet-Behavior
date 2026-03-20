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
    let mut out = String::with_capacity(16_384);
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
    let rtsp_urls = get_setting(s, "rtsp_urls", "");
    let seg = get_setting(s, "segment_duration", "15");
    let channels = get_setting(s, "audio_channels", "1");
    let fmt = get_setting(s, "audio_format", "wav");
    let fmt_wav = if fmt == "wav" { " selected" } else { "" };
    let fmt_mp3 = if fmt == "mp3" { " selected" } else { "" };
    let fmt_flac = if fmt == "flac" { " selected" } else { "" };
    let fmt_ogg = if fmt == "ogg" { " selected" } else { "" };
    let freq_shift = get_setting(s, "freq_shift_hz", "0");
    out.push_str(&format!(r#"
  <div class="card">
    <div class="section-title">Audio Capture</div>
    <div class="grid-2">
      <div>
        <label for="alsa_device">ALSA Device</label>
        <input id="alsa_device" name="alsa_device" value="{alsa}" placeholder="e.g. plughw:1,0">
        <p class="hint">Leave blank to disable managed microphone capture. PulseAudio/PipeWire users: use "default" or leave blank and set ALSA_CARD env var.</p>
      </div>
      <div>
        <label for="rtsp_url">RTSP URL (single stream)</label>
        <input id="rtsp_url" name="rtsp_url" value="{rtsp}" placeholder="rtsp://camera.local:554/stream">
        <p class="hint">IP camera audio stream (requires ffmpeg)</p>
      </div>
    </div>
    <div>
      <label for="rtsp_urls">Multiple RTSP URLs (comma-separated)</label>
      <input id="rtsp_urls" name="rtsp_urls" value="{rtsp_urls}" placeholder="rtsp://cam1:554/stream,rtsp://cam2:554/stream">
      <p class="hint">Each URL becomes an independent capture pipeline (RTSP_1-, RTSP_2- prefixed filenames). Overrides single RTSP URL above when set.</p>
    </div>
    <div class="grid-2">
      <div>
        <label for="segment_duration">Segment Duration (seconds)</label>
        <input id="segment_duration" name="segment_duration" type="number" value="{seg}" min="5" max="60" style="max-width:120px">
        <p class="hint">Length of each recording chunk for analysis (BirdNET-Pi: RECORDING_LENGTH)</p>
      </div>
      <div>
        <label for="audio_channels">Audio Channels</label>
        <input id="audio_channels" name="audio_channels" type="number" value="{channels}" min="1" max="2" style="max-width:80px">
        <p class="hint">1 = mono (recommended), 2 = stereo (BirdNET-Pi: CHANNELS)</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="audio_format">Extracted Clip Format</label>
        <select id="audio_format" name="audio_format" style="max-width:180px">
          <option value="wav"{fmt_wav}>WAV (lossless, default)</option>
          <option value="mp3"{fmt_mp3}>MP3 (requires ffmpeg)</option>
          <option value="flac"{fmt_flac}>FLAC (lossless compressed, requires ffmpeg)</option>
          <option value="ogg"{fmt_ogg}>OGG (requires ffmpeg)</option>
        </select>
        <p class="hint">Format for saved detection audio clips (BirdNET-Pi: AUDIOFMT)</p>
      </div>
      <div>
        <label for="freq_shift_hz">Frequency Shift (Hz, 0 = disabled)</label>
        <input id="freq_shift_hz" name="freq_shift_hz" type="number" value="{freq_shift}"
               min="-12000" max="12000" step="500" style="max-width:120px">
        <p class="hint">Shift pitch of saved clips for accessibility (BirdNET-Pi: FREQ_SHIFT). Requires ffmpeg or sox. Typical: 1000–4000.</p>
      </div>
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

fn render_notifications_section(out: &mut String, s: &HashMap<String, String>) {
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
    let img = get_setting(s, "notify_image", "true");
    let img_yes = if img != "false" { " selected" } else { "" };
    let img_no = if img == "false" { " selected" } else { "" };
    let weekly = get_setting(s, "weekly_report_schedule", "monday");
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
    let mut weekly_opts = String::new();
    for d in days {
        let sel = if weekly == d { " selected" } else { "" };
        weekly_opts.push_str(&format!(
            "<option value=\"{d}\"{sel}>{}</option>",
            d.chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default()
                + &d[1..]
        ));
    }
    out.push_str(&format!(r#"
  <div class="card">
    <div class="section-title">Notifications (Apprise)</div>
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
    <div style="margin-top:0.5rem;">
      <a href="/admin/notifications/test" class="btn btn-primary" style="font-size:0.8rem;padding:0.3rem 0.8rem;text-decoration:none;">
        Test Notifications
      </a>
    </div>
    <div class="grid-2" style="margin-top:1rem;">
      <div>
        <label for="notify_trigger">Notification Trigger</label>
        <select id="notify_trigger" name="notify_trigger">
          <option value="each"{t_each}>Each detection</option>
          <option value="new-species"{t_new}>New species (this week)</option>
          <option value="new-species-daily"{t_daily}>New species (each day)</option>
        </select>
        <p class="hint">When to send notifications (BirdNET-Pi: APPRISE_NOTIFY_EACH_DETECTION etc.)</p>
      </div>
      <div>
        <label for="notify_image">Include Species Image</label>
        <select id="notify_image" name="notify_image" style="max-width:180px">
          <option value="true"{img_yes}>Yes — attach image</option>
          <option value="false"{img_no}>No — text only</option>
        </select>
        <p class="hint">Attach species photo to Telegram/Discord/etc. notifications</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="notify_confidence">Notification Min Confidence</label>
        <input id="notify_confidence" name="notify_confidence" type="number"
               value="{nconf}" min="0" max="1" step="0.05">
        <p class="hint">Only notify above this confidence (BirdNET-Pi: APPRISE_MIN_CONFIDENCE)</p>
      </div>
      <div>
        <label for="notify_cooldown">Notification Cooldown (seconds)</label>
        <input id="notify_cooldown" name="notify_cooldown" type="number"
               value="{ncool}" min="0" step="60">
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
    let customimg = get_setting(s, "custom_image_dir", "");
    let maxfiles = get_setting(s, "max_files_per_species", "0");
    let purge = get_setting(s, "purge_threshold", "95");
    let site = get_setting(s, "site_name", "");
    let isite = get_setting(s, "info_site", "ebird");
    let is_ebird = if isite == "ebird" { " selected" } else { "" };
    let is_aab = if isite == "allaboutbirds" {
        " selected"
    } else {
        ""
    };
    let is_none = if isite == "none" { " selected" } else { "" };
    let auth_user = get_setting(s, "auth_username", "");
    let auth_pass = get_setting(s, "auth_password", "");
    out.push_str(&format!(
        r#"
  <div class="card">
    <div class="section-title">System &amp; Display</div>
    <div class="grid-2">
      <div>
        <label for="site_name">Site Name</label>
        <input id="site_name" name="site_name" value="{site}" placeholder="My Bird Station">
        <p class="hint">Shown in page titles and headers (BirdNET-Pi: SITENAME)</p>
      </div>
      <div>
        <label for="info_site">Species Info Links</label>
        <select id="info_site" name="info_site">
          <option value="ebird"{is_ebird}>eBird</option>
          <option value="allaboutbirds"{is_aab}>AllAboutBirds</option>
          <option value="none"{is_none}>None</option>
        </select>
        <p class="hint">Species detail page links to external info (BirdNET-Pi: INFO_SITE)</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="recording_days">Keep Recordings (days)</label>
        <input id="recording_days" name="recording_days" type="number"
               value="{days}" min="1" max="365">
        <p class="hint">Audio files older than this are deleted automatically (BirdNET-Pi: PROCESSED_DAYS)</p>
      </div>
      <div>
        <label for="max_files_per_species">Max Files Per Species (0 = unlimited)</label>
        <input id="max_files_per_species" name="max_files_per_species" type="number"
               value="{maxfiles}" min="0" step="10">
        <p class="hint">Oldest files beyond this limit are auto-deleted (BirdNET-Pi: MAX_FILES_SPECIES)</p>
      </div>
    </div>
    <div>
      <label for="purge_threshold">Disk Purge Threshold (%)</label>
      <input id="purge_threshold" name="purge_threshold" type="number"
             value="{purge}" min="50" max="99" style="max-width:120px">
      <p class="hint">Start purging old recordings when disk usage exceeds this % (BirdNET-Pi: DISK_PURGE_THRESHOLD)</p>
    </div>
    <div class="grid-2">
      <div>
        <label for="image_cache_dir">Species Image Cache Directory</label>
        <input id="image_cache_dir" name="image_cache_dir" value="{imgcache}"
               placeholder="/var/lib/birdnet/images">
        <p class="hint">Leave blank to disable Wikipedia image caching</p>
      </div>
      <div>
        <label for="custom_image_dir">Custom Species Image Directory</label>
        <input id="custom_image_dir" name="custom_image_dir" value="{customimg}"
               placeholder="/home/pi/BirdNet-Behavior/custom_images">
        <p class="hint">Override Wikipedia images with custom photos (BirdNET-Pi: CUSTOM_IMAGE). Files: sci_name.jpg</p>
      </div>
    </div>
  </div>
  <div class="card">
    <div class="section-title">Web Authentication</div>
    <p class="hint" style="margin-bottom:1rem;">Leave blank to disable HTTP Basic Auth (allow open access).</p>
    <div class="grid-2">
      <div>
        <label for="auth_username">Username</label>
        <input id="auth_username" name="auth_username" value="{auth_user}" placeholder="birdnet"
               autocomplete="username">
        <p class="hint">Web UI login username (BirdNET-Pi: CADDY_USER)</p>
      </div>
      <div>
        <label for="auth_password">Password</label>
        <input id="auth_password" name="auth_password" type="password" value="{auth_pass}"
               placeholder="leave blank to keep current" autocomplete="new-password">
        <p class="hint">Web UI login password (BirdNET-Pi: CADDY_PWD)</p>
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
        // Audio
        assert!(html.contains("alsa_device"));
        assert!(html.contains("audio_format"));
        assert!(html.contains("audio_channels"));
        assert!(html.contains("rtsp_urls"));
        // Location
        assert!(html.contains("latitude"));
        assert!(html.contains("night_inhibit"));
        // Detection
        assert!(html.contains("confidence_threshold"));
        assert!(html.contains("sf_thresh"));
        assert!(html.contains("privacy_threshold"));
        // Notifications
        assert!(html.contains("apprise_url"));
        assert!(html.contains("apprise_config"));
        assert!(html.contains("notify_trigger"));
        assert!(html.contains("notify_species_only"));
        assert!(html.contains("notify_species_exclude"));
        assert!(html.contains("notify_title_template"));
        assert!(html.contains("notify_body_template"));
        assert!(html.contains("weekly_report_schedule"));
        assert!(html.contains("birdweather_token"));
        // System
        assert!(html.contains("max_files_per_species"));
        assert!(html.contains("purge_threshold"));
        assert!(html.contains("custom_image_dir"));
        assert!(html.contains("site_name"));
        assert!(html.contains("info_site"));
        assert!(html.contains("auth_username"));
        assert!(html.contains("auth_password"));
        // Email
        assert!(html.contains("email_smtp_host"));
        assert!(html.contains("email_to"));
    }
}
