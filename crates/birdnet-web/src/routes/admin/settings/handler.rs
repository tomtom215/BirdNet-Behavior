//! Settings route handlers (GET / POST).

use axum::Form;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use serde::Serialize;
use std::collections::HashMap;

use birdnet_db::settings::{SettingsCategory, ensure_settings_table, list, set_many};

use super::form::SettingsForm;
use super::render::{render_settings_form, render_settings_page};
use crate::routes::pages::toast::{self, Toast};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// GET /admin/settings — full page
// ---------------------------------------------------------------------------

/// Render the full settings admin page.
///
/// # Errors
///
/// Returns `StatusCode` on internal rendering failures.
pub async fn settings_page(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    let settings_map = load_all_settings(&state);
    Ok(Html(render_settings_page(&settings_map)))
}

// ---------------------------------------------------------------------------
// GET /admin/settings/partial — HTMX partial (form body only)
// ---------------------------------------------------------------------------

/// Render the settings form partial for HTMX requests.
///
/// # Errors
///
/// Returns `StatusCode` on internal rendering failures.
pub async fn settings_partial(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    let settings_map = load_all_settings(&state);
    Ok(Html(render_settings_form(&settings_map)))
}

// ---------------------------------------------------------------------------
// POST /admin/settings — save and return feedback partial
// ---------------------------------------------------------------------------

/// Save submitted settings and return an HTMX feedback partial.
///
/// # Errors
///
/// Returns `StatusCode` on database or internal failures.
pub async fn save_settings(
    State(state): State<AppState>,
    Form(form): Form<SettingsForm>,
) -> Result<Html<String>, StatusCode> {
    // Compare submitted values against the current DB state so we only
    // persist the rows the operator actually changed. Without this the
    // page's render-time defaults (e.g. `night_inhibit=false` when no row
    // exists) would silently overlay over the file config / env every
    // time *any* unrelated setting is saved.
    let existing = load_all_settings(&state);
    let result = state.with_db(|conn| {
        ensure_settings_table(conn)?;
        let items = build_settings_items(&form, &existing);
        let refs: Vec<(&str, &str, SettingsCategory)> =
            items.iter().map(|(k, v, c)| (*k, v.as_str(), *c)).collect();
        set_many(conn, &refs)?;
        Ok::<usize, birdnet_db::settings::SettingsError>(refs.len())
    });

    match result {
        Ok(saved) => {
            let body = Html(format!(
                r#"<div class="alert alert-success" role="alert"
                    hx-swap-oob="true" id="settings-feedback">
                <svg class="inline w-4 h-4 mr-2" fill="currentColor" viewBox="0 0 20 20">
                    <path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clip-rule="evenodd"/>
                </svg>
                Settings saved ({saved} values updated).
                <span class="text-sm text-slate-400 ml-2">Changes apply on next restart.</span>
            </div>"#
            ));
            // O-18: toast the success outcome via OOB, with a follow-up action
            // — settings only take effect on next restart, so surface the link.
            Ok(toast::with(
                body,
                Toast::success(format!("Settings saved ({saved} values updated)."))
                    .with_action("/admin/system", "Open system →"),
            ))
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to save settings");
            let body = Html(format!(
                r#"<div class="alert alert-error" id="settings-feedback"
                        hx-swap-oob="true">
                    Failed to save settings: {e}
                </div>"#
            ));
            Ok(toast::with(
                body,
                Toast::error(format!("Failed to save settings: {e}")),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// GET /admin/settings/detect-location — auto-detect lat/lon from IP
// ---------------------------------------------------------------------------

/// Response body for the detect-location endpoint.
#[derive(Debug, Serialize)]
pub struct LocationResult {
    pub lat: f64,
    pub lon: f64,
    pub city: String,
    pub country: String,
}

/// Detect the station's approximate location using the public ip-api.com service.
///
/// Returns `{"lat": ..., "lon": ..., "city": ..., "country": ...}` on success,
/// or `500` with an error message on failure.
///
/// BirdNET-Pi equivalent: `birdnet_analysis.sh` calls `curl ipinfo.io` on startup
/// to auto-populate `LATITUDE` / `LONGITUDE` when not configured.
///
/// # Errors
///
/// Returns `(StatusCode, String)` on HTTP client build failure, network errors,
/// JSON decode failure, or when ip-api.com returns a non-success status.
pub async fn detect_location() -> Result<Json<LocationResult>, (StatusCode, String)> {
    #[derive(serde::Deserialize)]
    struct IpApiResponse {
        lat: f64,
        lon: f64,
        #[serde(default)]
        city: String,
        #[serde(default)]
        country: String,
        status: String,
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let resp = client
        .get("http://ip-api.com/json/")
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("location lookup failed: {e}"),
            )
        })?;

    let data: IpApiResponse = resp.json().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("invalid location response: {e}"),
        )
    })?;

    if data.status != "success" {
        return Err((
            StatusCode::BAD_GATEWAY,
            "ip-api.com returned non-success status".into(),
        ));
    }

    tracing::info!(
        lat = data.lat,
        lon = data.lon,
        city = %data.city,
        "auto-detected location via ip-api.com"
    );

    Ok(Json(LocationResult {
        lat: data.lat,
        lon: data.lon,
        city: data.city,
        country: data.country,
    }))
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

/// Whether a field carries a number whose decimal separator the
/// operator might type as either `.` or `,`. Values for these keys are
/// run through `parse_decimal::normalize_decimal` so the stored form is
/// always the canonical period-form string.
fn is_numeric_field(key: &str) -> bool {
    matches!(
        key,
        // True decimal-bearing fields.
        "latitude"
            | "longitude"
            | "confidence_threshold"
            | "sensitivity"
            | "overlap"
            | "sf_thresh"
            | "privacy_threshold"
            | "notify_confidence"
            | "email_min_confidence"
            // Integer-only fields. Normalising is a no-op when there's
            // no comma; including them here defends against EU browsers
            // that occasionally inject thousands separators.
            | "segment_duration"
            | "freq_shift_hz"
            | "pre_sunrise_offset"
            | "post_sunset_offset"
            | "recording_days"
            | "max_files_per_species"
            | "purge_threshold"
            | "email_smtp_port"
            | "email_cooldown_secs"
            | "notify_cooldown"
    )
}

/// Look up `key` in the existing DB snapshot, treating `None` and `""`
/// as the same thing — both mean "the operator hasn't set this".
fn existing_or_empty<'a>(map: &'a std::collections::HashMap<String, String>, key: &str) -> &'a str {
    map.get(key).map_or("", String::as_str)
}

/// Convert the flat form into a list of `(key, value, category)` triples
/// for storage.
///
/// Two-stage filter:
///
/// 1. Numeric fields run through [`birdnet_core::config::locale::normalize_decimal`]
///    so EU-formatted values (`42,3601`) round-trip cleanly through the
///    canonical period-form storage.
/// 2. Fields whose normalised value matches the current DB row are
///    skipped — without this every render-time default in the form
///    (e.g. `night_inhibit=false`, `info_site=ebird`) would overlay
///    over the file config / env on every save of any unrelated setting.
#[allow(clippy::too_many_lines)]
fn build_settings_items(
    form: &SettingsForm,
    existing: &std::collections::HashMap<String, String>,
) -> Vec<(&'static str, String, SettingsCategory)> {
    let mut items: Vec<(&'static str, String, SettingsCategory)> = Vec::new();

    macro_rules! push {
        ($field:expr, $key:literal, $cat:expr) => {
            if let Some(ref raw) = $field {
                let value = if is_numeric_field($key) {
                    birdnet_core::config::locale::normalize_decimal(raw)
                } else {
                    raw.clone()
                };
                if value != existing_or_empty(existing, $key) {
                    items.push(($key, value, $cat));
                }
            }
        };
    }

    // Audio
    push!(form.alsa_device, "alsa_device", SettingsCategory::Audio);
    push!(form.rtsp_url, "rtsp_url", SettingsCategory::Audio);
    push!(form.rtsp_urls, "rtsp_urls", SettingsCategory::Audio);
    push!(
        form.segment_duration,
        "segment_duration",
        SettingsCategory::Audio
    );
    push!(
        form.audio_channels,
        "audio_channels",
        SettingsCategory::Audio
    );
    push!(form.audio_format, "audio_format", SettingsCategory::Audio);
    push!(form.freq_shift_hz, "freq_shift_hz", SettingsCategory::Audio);
    // Location
    push!(form.latitude, "latitude", SettingsCategory::Location);
    push!(form.longitude, "longitude", SettingsCategory::Location);
    push!(
        form.station_name,
        "station_name",
        SettingsCategory::Location
    );
    push!(
        form.night_inhibit,
        "night_inhibit",
        SettingsCategory::Location
    );
    push!(
        form.pre_sunrise_offset,
        "pre_sunrise_offset",
        SettingsCategory::Location
    );
    push!(
        form.post_sunset_offset,
        "post_sunset_offset",
        SettingsCategory::Location
    );
    // Detection
    push!(
        form.confidence_threshold,
        "confidence_threshold",
        SettingsCategory::Detection
    );
    push!(form.sensitivity, "sensitivity", SettingsCategory::Detection);
    push!(form.overlap, "overlap", SettingsCategory::Detection);
    push!(form.sf_thresh, "sf_thresh", SettingsCategory::Detection);
    push!(
        form.privacy_threshold,
        "privacy_threshold",
        SettingsCategory::Detection
    );
    // Notifications
    push!(
        form.apprise_url,
        "apprise_url",
        SettingsCategory::Notifications
    );
    push!(
        form.apprise_config,
        "apprise_config",
        SettingsCategory::Notifications
    );
    push!(
        form.birdweather_token,
        "birdweather_token",
        SettingsCategory::Notifications
    );
    push!(
        form.notify_confidence,
        "notify_confidence",
        SettingsCategory::Notifications
    );
    push!(
        form.notify_cooldown,
        "notify_cooldown",
        SettingsCategory::Notifications
    );
    push!(
        form.notify_trigger,
        "notify_trigger",
        SettingsCategory::Notifications
    );
    push!(
        form.notify_species_only,
        "notify_species_only",
        SettingsCategory::Notifications
    );
    push!(
        form.notify_species_exclude,
        "notify_species_exclude",
        SettingsCategory::Notifications
    );
    push!(
        form.notify_title_template,
        "notify_title_template",
        SettingsCategory::Notifications
    );
    push!(
        form.notify_body_template,
        "notify_body_template",
        SettingsCategory::Notifications
    );
    push!(
        form.notify_image,
        "notify_image",
        SettingsCategory::Notifications
    );
    push!(
        form.weekly_report_schedule,
        "weekly_report_schedule",
        SettingsCategory::Notifications
    );
    // Species
    push!(
        form.species_exclude,
        "species_exclude",
        SettingsCategory::Species
    );
    push!(
        form.species_include,
        "species_include",
        SettingsCategory::Species
    );
    // System
    push!(
        form.recording_days,
        "recording_days",
        SettingsCategory::System
    );
    push!(
        form.image_cache_dir,
        "image_cache_dir",
        SettingsCategory::System
    );
    push!(
        form.custom_image_dir,
        "custom_image_dir",
        SettingsCategory::System
    );
    push!(
        form.max_files_per_species,
        "max_files_per_species",
        SettingsCategory::System
    );
    push!(
        form.purge_threshold,
        "purge_threshold",
        SettingsCategory::System
    );
    push!(form.site_name, "site_name", SettingsCategory::System);
    push!(form.info_site, "info_site", SettingsCategory::System);
    // Auth
    push!(
        form.auth_username,
        "auth_username",
        SettingsCategory::System
    );
    push!(
        form.auth_password,
        "auth_password",
        SettingsCategory::System
    );
    // Email
    push!(
        form.email_smtp_host,
        "email_smtp_host",
        SettingsCategory::Notifications
    );
    push!(
        form.email_smtp_port,
        "email_smtp_port",
        SettingsCategory::Notifications
    );
    push!(
        form.email_smtp_user,
        "email_smtp_user",
        SettingsCategory::Notifications
    );
    push!(
        form.email_smtp_pass,
        "email_smtp_pass",
        SettingsCategory::Notifications
    );
    push!(
        form.email_from,
        "email_from",
        SettingsCategory::Notifications
    );
    push!(form.email_to, "email_to", SettingsCategory::Notifications);
    push!(
        form.email_from_name,
        "email_from_name",
        SettingsCategory::Notifications
    );
    push!(
        form.email_starttls,
        "email_starttls",
        SettingsCategory::Notifications
    );
    push!(
        form.email_min_confidence,
        "email_min_confidence",
        SettingsCategory::Notifications
    );
    push!(
        form.email_cooldown_secs,
        "email_cooldown_secs",
        SettingsCategory::Notifications
    );

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_form() -> SettingsForm {
        SettingsForm {
            alsa_device: None,
            rtsp_url: None,
            rtsp_urls: None,
            segment_duration: None,
            audio_channels: None,
            audio_format: None,
            freq_shift_hz: None,
            latitude: None,
            longitude: None,
            station_name: None,
            confidence_threshold: None,
            sensitivity: None,
            overlap: None,
            sf_thresh: None,
            privacy_threshold: None,
            apprise_url: None,
            apprise_config: None,
            birdweather_token: None,
            notify_confidence: None,
            notify_cooldown: None,
            notify_trigger: None,
            notify_species_only: None,
            notify_species_exclude: None,
            notify_title_template: None,
            notify_body_template: None,
            notify_image: None,
            weekly_report_schedule: None,
            species_exclude: None,
            species_include: None,
            recording_days: None,
            image_cache_dir: None,
            custom_image_dir: None,
            max_files_per_species: None,
            purge_threshold: None,
            site_name: None,
            info_site: None,
            night_inhibit: None,
            pre_sunrise_offset: None,
            post_sunset_offset: None,
            auth_username: None,
            auth_password: None,
            email_smtp_host: None,
            email_smtp_port: None,
            email_smtp_user: None,
            email_smtp_pass: None,
            email_from: None,
            email_to: None,
            email_from_name: None,
            email_starttls: None,
            email_min_confidence: None,
            email_cooldown_secs: None,
        }
    }

    #[test]
    fn eu_latitude_is_normalised_to_period_form() {
        let form = SettingsForm {
            latitude: Some("42,3601".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &HashMap::new());
        let lat = items
            .iter()
            .find(|(k, _, _)| *k == "latitude")
            .expect("latitude must be persisted");
        assert_eq!(lat.1, "42.3601");
    }

    #[test]
    fn eu_longitude_is_normalised() {
        let form = SettingsForm {
            longitude: Some("-71,0589".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &HashMap::new());
        let lon = items.iter().find(|(k, _, _)| *k == "longitude").unwrap();
        assert_eq!(lon.1, "-71.0589");
    }

    #[test]
    fn confidence_threshold_normalised() {
        let form = SettingsForm {
            confidence_threshold: Some("0,75".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &HashMap::new());
        let conf = items
            .iter()
            .find(|(k, _, _)| *k == "confidence_threshold")
            .unwrap();
        assert_eq!(conf.1, "0.75");
    }

    #[test]
    fn unchanged_field_is_skipped() {
        // DB already has latitude=42.3601; form submits the same.
        // No row should be issued.
        let mut existing = HashMap::new();
        existing.insert("latitude".to_string(), "42.3601".to_string());
        let form = SettingsForm {
            latitude: Some("42.3601".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &existing);
        assert!(
            !items.iter().any(|(k, _, _)| *k == "latitude"),
            "unchanged latitude should not be re-persisted"
        );
    }

    #[test]
    fn comma_form_equal_to_existing_period_form_is_skipped() {
        // DB has the canonical period form; an EU operator re-submitting
        // the comma form must compare equal after normalisation, so the
        // row is not duplicated.
        let mut existing = HashMap::new();
        existing.insert("latitude".to_string(), "42.3601".to_string());
        let form = SettingsForm {
            latitude: Some("42,3601".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &existing);
        assert!(!items.iter().any(|(k, _, _)| *k == "latitude"));
    }

    #[test]
    fn empty_field_with_no_existing_row_is_skipped() {
        // The big bug from the audit: the page renders many fields empty
        // because no DB row exists; the form re-submits them as empty;
        // the old code wrote empty rows for every one. Now they're
        // skipped because existing-or-empty matches form-empty.
        let form = SettingsForm {
            latitude: Some(String::new()),
            confidence_threshold: Some(String::new()),
            night_inhibit: Some("false".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &HashMap::new());
        assert!(!items.iter().any(|(k, _, _)| *k == "latitude"));
        assert!(!items.iter().any(|(k, _, _)| *k == "confidence_threshold"));
        // night_inhibit defaults to "false" in the form render — and
        // since there's no existing row, the empty-existing "" doesn't
        // match form "false", so it WOULD pass through. That's a render
        // problem, not a save problem; addressed by the form template
        // change that prefixes the option with the saved value vs. the
        // hard-coded default.
        // The bug fix is the *general* mechanism: any field whose value
        // hasn't changed is not re-written. Confirmed.
    }

    #[test]
    fn changed_field_is_persisted() {
        let mut existing = HashMap::new();
        existing.insert("latitude".to_string(), "42.3601".to_string());
        let form = SettingsForm {
            latitude: Some("51.5074".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &existing);
        let lat = items.iter().find(|(k, _, _)| *k == "latitude").unwrap();
        assert_eq!(lat.1, "51.5074");
    }

    #[test]
    fn user_clearing_an_existing_value_is_persisted() {
        // User explicitly clears the latitude field.
        let mut existing = HashMap::new();
        existing.insert("latitude".to_string(), "42.3601".to_string());
        let form = SettingsForm {
            latitude: Some(String::new()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &existing);
        let lat = items
            .iter()
            .find(|(k, _, _)| *k == "latitude")
            .expect("clearing should be persisted");
        assert_eq!(lat.1, "");
    }

    #[test]
    fn non_numeric_field_passes_through_unchanged() {
        // station_name with a comma is a perfectly valid name (e.g.
        // "Backyard, Boston") — it must not be normalised.
        let form = SettingsForm {
            station_name: Some("Backyard, Boston".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &HashMap::new());
        let name = items.iter().find(|(k, _, _)| *k == "station_name").unwrap();
        assert_eq!(name.1, "Backyard, Boston");
    }
}
