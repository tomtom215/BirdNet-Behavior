//! Settings route handlers (GET / POST).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::Form;
use std::collections::HashMap;

use birdnet_db::settings::{SettingsCategory, ensure_settings_table, list, set_many};

use super::form::SettingsForm;
use super::render::{render_settings_form, render_settings_page};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// GET /admin/settings — full page
// ---------------------------------------------------------------------------

pub async fn settings_page(
    State(state): State<AppState>,
) -> Result<Html<String>, StatusCode> {
    let settings_map = load_all_settings(&state);
    Ok(Html(render_settings_page(&settings_map)))
}

// ---------------------------------------------------------------------------
// GET /admin/settings/partial — HTMX partial (form body only)
// ---------------------------------------------------------------------------

pub async fn settings_partial(
    State(state): State<AppState>,
) -> Result<Html<String>, StatusCode> {
    let settings_map = load_all_settings(&state);
    Ok(Html(render_settings_form(&settings_map)))
}

// ---------------------------------------------------------------------------
// POST /admin/settings — save and return feedback partial
// ---------------------------------------------------------------------------

pub async fn save_settings(
    State(state): State<AppState>,
    Form(form): Form<SettingsForm>,
) -> Result<Html<String>, StatusCode> {
    let result = state.with_db(|conn| {
        ensure_settings_table(conn)?;
        let items = build_settings_items(&form);
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
// Private helpers
// ---------------------------------------------------------------------------

pub(super) fn load_all_settings(state: &AppState) -> HashMap<String, String> {
    state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        list(conn, None)
            .map(|rows| rows.into_iter().map(|s| (s.key, s.value)).collect())
            .unwrap_or_default()
    })
}

/// Convert the flat form into a list of `(key, value, category)` triples
/// for storage, filtering out any `None` fields.
fn build_settings_items(
    form: &SettingsForm,
) -> Vec<(&'static str, String, SettingsCategory)> {
    let mut items: Vec<(&'static str, String, SettingsCategory)> = Vec::new();

    macro_rules! push {
        ($field:expr, $key:literal, $cat:expr) => {
            if let Some(ref v) = $field {
                items.push(($key, v.clone(), $cat));
            }
        };
    }

    push!(form.alsa_device, "alsa_device", SettingsCategory::Audio);
    push!(form.rtsp_url, "rtsp_url", SettingsCategory::Audio);
    push!(form.segment_duration, "segment_duration", SettingsCategory::Audio);
    push!(form.latitude, "latitude", SettingsCategory::Location);
    push!(form.longitude, "longitude", SettingsCategory::Location);
    push!(form.station_name, "station_name", SettingsCategory::Location);
    push!(form.night_inhibit, "night_inhibit", SettingsCategory::Location);
    push!(form.pre_sunrise_offset, "pre_sunrise_offset", SettingsCategory::Location);
    push!(form.post_sunset_offset, "post_sunset_offset", SettingsCategory::Location);
    push!(form.confidence_threshold, "confidence_threshold", SettingsCategory::Detection);
    push!(form.sensitivity, "sensitivity", SettingsCategory::Detection);
    push!(form.overlap, "overlap", SettingsCategory::Detection);
    push!(form.apprise_url, "apprise_url", SettingsCategory::Notifications);
    push!(form.birdweather_token, "birdweather_token", SettingsCategory::Notifications);
    push!(form.notify_confidence, "notify_confidence", SettingsCategory::Notifications);
    push!(form.notify_cooldown, "notify_cooldown", SettingsCategory::Notifications);
    push!(form.species_exclude, "species_exclude", SettingsCategory::Species);
    push!(form.species_include, "species_include", SettingsCategory::Species);
    push!(form.recording_days, "recording_days", SettingsCategory::System);
    push!(form.image_cache_dir, "image_cache_dir", SettingsCategory::System);
    push!(form.email_smtp_host, "email_smtp_host", SettingsCategory::Notifications);
    push!(form.email_smtp_port, "email_smtp_port", SettingsCategory::Notifications);
    push!(form.email_smtp_user, "email_smtp_user", SettingsCategory::Notifications);
    push!(form.email_smtp_pass, "email_smtp_pass", SettingsCategory::Notifications);
    push!(form.email_from, "email_from", SettingsCategory::Notifications);
    push!(form.email_to, "email_to", SettingsCategory::Notifications);
    push!(form.email_from_name, "email_from_name", SettingsCategory::Notifications);
    push!(form.email_starttls, "email_starttls", SettingsCategory::Notifications);
    push!(form.email_min_confidence, "email_min_confidence", SettingsCategory::Notifications);
    push!(form.email_cooldown_secs, "email_cooldown_secs", SettingsCategory::Notifications);

    items
}
