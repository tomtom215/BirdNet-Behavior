//! Admin settings routes — GET / POST /admin/settings.
//!
//! Renders a tabbed settings form (Audio, Location, Detection, Notifications,
//! Species, System) backed by the SQLite `settings` table.  Form submission
//! uses HTMX for in-place partial updates.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Form, Router, routing::get};
use serde::Deserialize;
use std::collections::HashMap;

use birdnet_db::settings::{SettingsCategory, ensure_settings_table, list, set_many};

use crate::state::AppState;

/// Mount settings routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/settings", get(settings_page).post(save_settings))
        .route("/admin/settings/partial", get(settings_partial))
}

// ---------------------------------------------------------------------------
// GET /admin/settings — full page
// ---------------------------------------------------------------------------

pub async fn settings_page(
    State(state): State<AppState>,
) -> Result<Html<String>, StatusCode> {
    let settings_map = load_all_settings(&state);
    let html = render_settings_page(&settings_map);
    Ok(Html(html))
}

// ---------------------------------------------------------------------------
// GET /admin/settings/partial — HTMX partial (form body only)
// ---------------------------------------------------------------------------

async fn settings_partial(
    State(state): State<AppState>,
) -> Result<Html<String>, StatusCode> {
    let settings_map = load_all_settings(&state);
    Ok(Html(render_settings_form(&settings_map)))
}

// ---------------------------------------------------------------------------
// POST /admin/settings — save and return feedback partial
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    // Audio
    alsa_device: Option<String>,
    rtsp_url: Option<String>,
    segment_duration: Option<String>,
    // Location
    latitude: Option<String>,
    longitude: Option<String>,
    station_name: Option<String>,
    // Detection
    confidence_threshold: Option<String>,
    sensitivity: Option<String>,
    overlap: Option<String>,
    // Notifications
    apprise_url: Option<String>,
    birdweather_token: Option<String>,
    notify_confidence: Option<String>,
    notify_cooldown: Option<String>,
    // Species
    species_exclude: Option<String>,
    species_include: Option<String>,
    // System
    recording_days: Option<String>,
    image_cache_dir: Option<String>,
}

async fn save_settings(
    State(state): State<AppState>,
    Form(form): Form<SettingsForm>,
) -> Result<Html<String>, StatusCode> {
    let result = state.with_db(|conn| {
        ensure_settings_table(conn)?;
        let mut items: Vec<(&str, String, SettingsCategory)> = Vec::new();

        macro_rules! push_if_set {
            ($field:expr, $key:literal, $cat:expr) => {
                if let Some(ref v) = $field {
                    items.push(($key, v.clone(), $cat));
                }
            };
        }

        push_if_set!(form.alsa_device, "alsa_device", SettingsCategory::Audio);
        push_if_set!(form.rtsp_url, "rtsp_url", SettingsCategory::Audio);
        push_if_set!(form.segment_duration, "segment_duration", SettingsCategory::Audio);
        push_if_set!(form.latitude, "latitude", SettingsCategory::Location);
        push_if_set!(form.longitude, "longitude", SettingsCategory::Location);
        push_if_set!(form.station_name, "station_name", SettingsCategory::Location);
        push_if_set!(form.confidence_threshold, "confidence_threshold", SettingsCategory::Detection);
        push_if_set!(form.sensitivity, "sensitivity", SettingsCategory::Detection);
        push_if_set!(form.overlap, "overlap", SettingsCategory::Detection);
        push_if_set!(form.apprise_url, "apprise_url", SettingsCategory::Notifications);
        push_if_set!(form.birdweather_token, "birdweather_token", SettingsCategory::Notifications);
        push_if_set!(form.notify_confidence, "notify_confidence", SettingsCategory::Notifications);
        push_if_set!(form.notify_cooldown, "notify_cooldown", SettingsCategory::Notifications);
        push_if_set!(form.species_exclude, "species_exclude", SettingsCategory::Species);
        push_if_set!(form.species_include, "species_include", SettingsCategory::Species);
        push_if_set!(form.recording_days, "recording_days", SettingsCategory::System);
        push_if_set!(form.image_cache_dir, "image_cache_dir", SettingsCategory::System);

        // Convert to the slice format set_many expects
        let refs: Vec<(&str, &str, SettingsCategory)> = items
            .iter()
            .map(|(k, v, c)| (*k, v.as_str(), c.clone()))
            .collect();

        set_many(conn, &refs)?;
        Ok::<usize, birdnet_db::settings::SettingsError>(refs.len())
    });

    match result {
        Ok(saved) => Ok(Html(format!(
            r#"<div class="alert alert-success" role="alert"
                    hx-swap-oob="true" id="settings-feedback">
                <svg class="inline w-4 h-4 mr-2" fill="currentColor" viewBox="0 0 20 20">
                    <path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clip-rule="evenodd"/>
                </svg>
                Settings saved ({saved} values updated).
                <span class="text-sm text-slate-400 ml-2">Changes apply on next restart.</span>
            </div>"#
        ))),
        Err(e) => {
            tracing::error!(error = %e, "failed to save settings");
            Ok(Html(format!(
                r#"<div class="alert alert-error" id="settings-feedback"
                        hx-swap-oob="true">
                    Failed to save settings: {e}
                </div>"#
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_all_settings(state: &AppState) -> HashMap<String, String> {
    state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        list(conn, None)
            .map(|rows| rows.into_iter().map(|s| (s.key, s.value)).collect())
            .unwrap_or_default()
    })
}

fn get_setting<'a>(map: &'a HashMap<String, String>, key: &str, default: &'a str) -> &'a str {
    map.get(key).map_or(default, String::as_str)
}

fn render_settings_page(settings: &HashMap<String, String>) -> String {
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
      .tabs {{ display: flex; gap: 0.5rem; margin-bottom: 1.5rem; flex-wrap: wrap; }}
      .tab {{ padding: 0.4rem 1rem; border-radius: 0.375rem; border: 1px solid #334155;
               color: #94a3b8; cursor: pointer; font-size: 0.875rem; background: transparent; }}
      .tab.active, .tab:hover {{ background: #0ea5e9; color: #fff; border-color: #0ea5e9; }}
    </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem; padding:1rem 0; border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/admin" class="active">Admin</a>
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

fn render_settings_form(settings: &HashMap<String, String>) -> String {
    let alsa_device = get_setting(settings, "alsa_device", "");
    let rtsp_url = get_setting(settings, "rtsp_url", "");
    let segment_duration = get_setting(settings, "segment_duration", "15");
    let latitude = get_setting(settings, "latitude", "");
    let longitude = get_setting(settings, "longitude", "");
    let station_name = get_setting(settings, "station_name", "");
    let confidence = get_setting(settings, "confidence_threshold", "0.70");
    let sensitivity = get_setting(settings, "sensitivity", "1.0");
    let overlap = get_setting(settings, "overlap", "0.0");
    let apprise_url = get_setting(settings, "apprise_url", "");
    let bw_token = get_setting(settings, "birdweather_token", "");
    let notify_conf = get_setting(settings, "notify_confidence", "0.80");
    let notify_cooldown = get_setting(settings, "notify_cooldown", "300");
    let species_exclude = get_setting(settings, "species_exclude", "");
    let species_include = get_setting(settings, "species_include", "");
    let recording_days = get_setting(settings, "recording_days", "30");
    let image_cache_dir = get_setting(settings, "image_cache_dir", "");

    format!(r##"<form hx-post="/admin/settings" hx-target="#settings-feedback"
               hx-swap="innerHTML" hx-indicator="#save-spinner">

  <!-- Audio -->
  <div class="card">
    <div class="section-title">Audio Capture</div>
    <div class="grid-2">
      <div>
        <label for="alsa_device">ALSA Device</label>
        <input id="alsa_device" name="alsa_device" value="{alsa_device}" placeholder="e.g. plughw:1,0">
        <p class="hint">Leave blank to disable managed microphone capture</p>
      </div>
      <div>
        <label for="rtsp_url">RTSP URL</label>
        <input id="rtsp_url" name="rtsp_url" value="{rtsp_url}" placeholder="rtsp://camera.local:554/stream">
        <p class="hint">IP camera audio stream (requires ffmpeg)</p>
      </div>
    </div>
    <div>
      <label for="segment_duration">Segment Duration (seconds)</label>
      <input id="segment_duration" name="segment_duration" type="number" value="{segment_duration}" min="5" max="60" style="max-width:120px">
    </div>
  </div>

  <!-- Location -->
  <div class="card">
    <div class="section-title">Location</div>
    <div class="grid-2">
      <div>
        <label for="latitude">Latitude</label>
        <input id="latitude" name="latitude" value="{latitude}" placeholder="e.g. 51.5074">
      </div>
      <div>
        <label for="longitude">Longitude</label>
        <input id="longitude" name="longitude" value="{longitude}" placeholder="e.g. -0.1278">
      </div>
    </div>
    <div>
      <label for="station_name">Station Name</label>
      <input id="station_name" name="station_name" value="{station_name}" placeholder="e.g. My Garden, London">
      <p class="hint">Used in BirdWeather uploads and export metadata</p>
    </div>
  </div>

  <!-- Detection -->
  <div class="card">
    <div class="section-title">Detection Settings</div>
    <div class="grid-2">
      <div>
        <label for="confidence_threshold">Minimum Confidence (0–1)</label>
        <input id="confidence_threshold" name="confidence_threshold" type="number"
               value="{confidence}" min="0" max="1" step="0.05">
        <p class="hint">Detections below this threshold are discarded</p>
      </div>
      <div>
        <label for="sensitivity">Sensitivity</label>
        <input id="sensitivity" name="sensitivity" type="number"
               value="{sensitivity}" min="0.5" max="1.5" step="0.05">
        <p class="hint">Higher = more sensitive (more false positives)</p>
      </div>
    </div>
    <div>
      <label for="overlap">Overlap (0–2.9 seconds)</label>
      <input id="overlap" name="overlap" type="number"
             value="{overlap}" min="0" max="2.9" step="0.1" style="max-width:120px">
    </div>
  </div>

  <!-- Notifications -->
  <div class="card">
    <div class="section-title">Notifications</div>
    <div>
      <label for="apprise_url">Apprise URL</label>
      <input id="apprise_url" name="apprise_url" value="{apprise_url}" placeholder="http://localhost:8000">
      <p class="hint">Leave blank to disable push notifications</p>
    </div>
    <div>
      <label for="birdweather_token">BirdWeather Station Token</label>
      <input id="birdweather_token" name="birdweather_token" value="{bw_token}" placeholder="Token from BirdWeather app">
    </div>
    <div class="grid-2">
      <div>
        <label for="notify_confidence">Notification Min Confidence</label>
        <input id="notify_confidence" name="notify_confidence" type="number"
               value="{notify_conf}" min="0" max="1" step="0.05">
      </div>
      <div>
        <label for="notify_cooldown">Notification Cooldown (seconds)</label>
        <input id="notify_cooldown" name="notify_cooldown" type="number"
               value="{notify_cooldown}" min="0" step="60">
        <p class="hint">Minimum time between notifications for the same species</p>
      </div>
    </div>
  </div>

  <!-- Species Filters -->
  <div class="card">
    <div class="section-title">Species Filters</div>
    <div>
      <label for="species_exclude">Excluded Species (comma-separated common names)</label>
      <textarea id="species_exclude" name="species_exclude" rows="3"
                placeholder="e.g. House Sparrow, Feral Pigeon">{species_exclude}</textarea>
      <p class="hint">These species will never be saved or notified</p>
    </div>
    <div>
      <label for="species_include">Allow-list (empty = all species)</label>
      <textarea id="species_include" name="species_include" rows="3"
                placeholder="e.g. European Robin, Eurasian Blackbird">{species_include}</textarea>
      <p class="hint">When set, only these species are saved or notified</p>
    </div>
  </div>

  <!-- System -->
  <div class="card">
    <div class="section-title">System</div>
    <div class="grid-2">
      <div>
        <label for="recording_days">Keep Recordings (days)</label>
        <input id="recording_days" name="recording_days" type="number"
               value="{recording_days}" min="1" max="365">
        <p class="hint">Audio files older than this are deleted automatically</p>
      </div>
      <div>
        <label for="image_cache_dir">Species Image Cache Directory</label>
        <input id="image_cache_dir" name="image_cache_dir" value="{image_cache_dir}"
               placeholder="/var/lib/birdnet/images">
        <p class="hint">Leave blank to disable Wikipedia image caching</p>
      </div>
    </div>
  </div>

  <div style="display:flex; align-items:center; gap:1rem;">
    <button type="submit" class="btn btn-primary">Save Settings</button>
    <span id="save-spinner" class="htmx-indicator" style="color:#94a3b8; font-size:0.875rem;">
      Saving…
    </span>
    <span style="color:#64748b; font-size:0.8rem;">
      Most settings require a restart to take effect.
    </span>
  </div>
</form>"##)
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
    }
}
