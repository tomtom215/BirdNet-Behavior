//! HTML rendering for the admin settings page.
//!
//! Split into section sub-modules for maintainability:
//!
//! | Module          | Responsibility                                 |
//! |-----------------|------------------------------------------------|
//! | `audio`         | Audio capture section (ALSA, RTSP, format)     |
//! | `location`      | Location & recording schedule section           |
//! | `detection`     | Detection thresholds section                   |
//! | `notifications` | Apprise + BirdWeather notifications section     |
//! | `species`       | Species filter lists section                   |
//! | `system`        | System, display, auth section                  |
//! | `email`         | SMTP email alerts section                      |

mod audio;
mod detection;
mod email;
mod location;
mod notifications;
mod species;
mod system;

use std::collections::HashMap;

pub(in crate::routes::admin::settings) fn get_setting<'a>(
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
    out.push_str(
        r##"<form hx-post="/admin/settings" hx-target="#settings-feedback"
               hx-swap="innerHTML" hx-indicator="#save-spinner">"##,
    );
    audio::render(&mut out, settings);
    location::render(&mut out, settings);
    detection::render(&mut out, settings);
    notifications::render(&mut out, settings);
    species::render(&mut out, settings);
    system::render(&mut out, settings);
    email::render(&mut out, settings);
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
    out
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
