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
pub async fn settings_page(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::auth_middleware::RequestUser>>,
) -> Result<Html<String>, StatusCode> {
    Ok(Html(match load_settings_for(&state, user.as_deref()) {
        Ok(settings_map) => render_settings_page(&settings_map),
        Err(e) => crate::routes::admin::admin_shell("Settings", "settings", &unreadable_notice(&e)),
    }))
}

// ---------------------------------------------------------------------------
// GET /admin/settings/partial — HTMX partial (form body only)
// ---------------------------------------------------------------------------

/// Render the settings form partial for HTMX requests.
///
/// # Errors
///
/// Returns `StatusCode` on internal rendering failures.
pub async fn settings_partial(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::auth_middleware::RequestUser>>,
) -> Result<Html<String>, StatusCode> {
    Ok(Html(match load_settings_for(&state, user.as_deref()) {
        Ok(settings_map) => render_settings_form(&settings_map),
        Err(e) => unreadable_notice(&e),
    }))
}

// ---------------------------------------------------------------------------
// POST /admin/settings — save and return feedback partial
// ---------------------------------------------------------------------------

/// Every settings field the station reads as a bare number and compares
/// directly: the form key, the label the page shows, and the range outside
/// which the value cannot do its job.
///
/// The first five mirror [`birdnet_core::config::validate::NUMERIC_RANGES`],
/// and `the_form_bounds_match_the_validator` holds the two in step. The last
/// two are not in that list on purpose: `validate()` findings drive
/// `startup_config::choose`, which reverts the whole configuration file on an
/// error, and a mistyped *alert* threshold should not roll a station's every
/// setting back. They are bounded here, where they are typed, and nowhere
/// else — `EMAIL_MIN_CONFIDENCE` has no config-file counterpart at all.
const BOUNDED_FIELDS: &[(&str, &str, f64, f64)] = &[
    ("confidence_threshold", "Minimum Confidence", 0.0, 1.0),
    ("sensitivity", "Sensitivity", 0.5, 1.5),
    ("overlap", "Analysis Overlap", 0.0, 2.9),
    ("sf_thresh", "Species Frequency Threshold", 0.0, 1.0),
    ("privacy_threshold", "Privacy Threshold", 0.0, 1.0),
    ("notify_confidence", "Notification Min Confidence", 0.0, 1.0),
    ("email_min_confidence", "Alert Min Confidence", 0.0, 1.0),
];

/// The form key each bounded field maps to in the runtime configuration, for
/// the five that have one. Used only to keep the ranges above in step with
/// `birdnet-core`'s.
#[cfg(test)]
const CONFIG_KEY_OF: &[(&str, &str)] = &[
    ("confidence_threshold", "CONFIDENCE"),
    ("sensitivity", "SENSITIVITY"),
    ("overlap", "OVERLAP"),
    ("sf_thresh", "SF_THRESH"),
    ("privacy_threshold", "PRIVACY_THRESHOLD"),
];

/// Problems with the numbers in `form`, phrased for the person who typed them.
///
/// # Why this exists at all
///
/// `birdnet_core::config::validate` has checked these ranges since the
/// beginning, and `--doctor` runs it — but only ever against the config
/// **file**. A value typed here goes into the settings table, and the settings
/// table is overlaid onto the config at startup *after* validation has already
/// run (`src/app.rs`: validate, then `overlay_db_settings`). So the one field
/// most likely to be got wrong was the one field nothing checked.
///
/// The mistake is not hypothetical: the field is labelled "Minimum Confidence
/// (0–1)" and the model's score is a probability, but the app's own
/// notification templates offer `$confidencepct` beside `$confidence`, and
/// people think in percent. `75` parses, stores, and is then compared against a
/// score that can never exceed `1`. Every detection is discarded, for good,
/// and nothing anywhere says why — the station just goes quiet.
fn range_problems(form: &SettingsForm) -> Vec<String> {
    // Read from the submission rather than from the changed-values list.
    // `build_settings_items` drops any field whose value already matches the
    // database, so validating that list would wave through a bad value that is
    // *already stored* — the case where the station has stopped recording and
    // its owner is on this page looking for the reason.
    // Sized by `BOUNDED_FIELDS`, so adding a bounded field without wiring it
    // here does not compile.
    let submitted: [(&str, Option<&String>); BOUNDED_FIELDS.len()] = [
        ("confidence_threshold", form.confidence_threshold.as_ref()),
        ("sensitivity", form.sensitivity.as_ref()),
        ("overlap", form.overlap.as_ref()),
        ("sf_thresh", form.sf_thresh.as_ref()),
        ("privacy_threshold", form.privacy_threshold.as_ref()),
        ("notify_confidence", form.notify_confidence.as_ref()),
        ("email_min_confidence", form.email_min_confidence.as_ref()),
    ];
    let mut problems = Vec::new();
    for (key, value) in submitted {
        let Some(value) = value else { continue };
        let Some(&(_, label, min, max)) = BOUNDED_FIELDS.iter().find(|(k, ..)| *k == key) else {
            continue;
        };
        let normalised = birdnet_core::config::locale::normalize_decimal(value);
        let raw = normalised.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(value) = raw.parse::<f64>() else {
            problems.push(format!(
                "{label} must be a number between {min} and {max}. You entered “{raw}”."
            ));
            continue;
        };
        if (min..=max).contains(&value) {
            continue;
        }
        // The percentage slip, named. Anything from 1 to 100 on a 0–1 scale is
        // almost certainly a percentage, and saying so is more use than
        // restating the range a second time.
        let hint = if (min, max) == (0.0, 1.0) && (1.0..=100.0).contains(&value) {
            format!(" — if you meant {value:.0}%, enter {:.2}.", value / 100.0)
        } else {
            ".".to_owned()
        };
        problems.push(format!(
            "{label} must be between {min} and {max}. You entered {raw}{hint}"
        ));
    }
    problems
}

/// Save submitted settings and return an HTMX feedback partial.
///
/// # Errors
///
/// Returns `StatusCode` on database or internal failures.
pub async fn save_settings(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<SettingsForm>,
) -> Result<Html<String>, StatusCode> {
    // Compare submitted values against the current DB state so we only
    // persist the rows the operator actually changed. Without this the
    // page's render-time defaults (e.g. `night_inhibit=false` when no row
    // exists) would silently overlay over the file config / env every
    // time *any* unrelated setting is saved.
    // Refuse outright when the current values cannot be read: the diff below
    // would count every submitted field as changed and write all of them,
    // render-time defaults included, over the operator's real configuration.
    let existing = match load_all_settings(&state) {
        Ok(existing) => existing,
        Err(e) => {
            let body = Html(format!(
                r#"<div class="alert alert-error" id="settings-feedback" hx-swap-oob="true" role="alert">{}</div>"#,
                crate::routes::pages::escape_html(&format!(
                    "Nothing was saved: the station's current settings could not be read ({e})."
                ))
            ));
            return Ok(toast::with(
                body,
                Toast::error("Nothing was saved: the current settings could not be read."),
            ));
        }
    };
    let items = build_settings_items(&form, &existing, Unset::AsShownByTheForm);

    // Reject before writing, and reject the whole submission: a partial save
    // would leave the form showing one thing and the station running another.
    let problems = range_problems(&form);
    if !problems.is_empty() {
        let list = problems.iter().fold(String::new(), |mut acc, p| {
            use std::fmt::Write as _;
            let _ = write!(acc, "<li>{}</li>", crate::routes::pages::escape_html(p));
            acc
        });
        let body = Html(format!(
            r#"<div class="alert alert-error" id="settings-feedback" hx-swap-oob="true" role="alert">
                Nothing was saved. Fix these and try again:
                <ul class="save-problems">{list}</ul>
            </div>"#
        ));
        return Ok(toast::with(body, Toast::error(problems.join(" "))));
    }

    // The audit metadata, computed before the write and from the same `items`
    // the write uses, so the row cannot claim a key that was never submitted.
    // Names only — a diff carrying `CADDY_PWD=hunter2` would put the admin
    // password into a table `/admin/audit` renders.
    let after: Vec<(String, String)> = items
        .iter()
        .map(|(k, v, _)| ((*k).to_owned(), v.clone()))
        .collect();
    let before: Vec<(String, String)> = after
        .iter()
        .filter_map(|(k, _)| existing.get(k).map(|v| (k.clone(), v.clone())))
        .collect();
    let changed = crate::audit::changed_keys(&before, &after);

    let result = state.with_db(|conn| {
        ensure_settings_table(conn)?;
        let refs: Vec<(&str, &str, SettingsCategory)> =
            items.iter().map(|(k, v, c)| (*k, v.as_str(), *c)).collect();
        set_many(conn, &refs)?;
        Ok::<usize, birdnet_db::settings::SettingsError>(refs.len())
    });

    match result {
        Ok(saved) => {
            // Only when something actually changed. The settings page posts
            // every field on every save, so recording each submission would
            // turn the audit log into a click counter and bury the save that
            // moved the recording schedule.
            if let Some(keys) = changed {
                crate::audit::audit(
                    &state,
                    Some(&request_user),
                    "settings.update",
                    None,
                    Some(&keys),
                );
            }
            let body = Html(format!(
                r#"<div class="alert alert-success" role="alert"
                    hx-swap-oob="true" id="settings-feedback">
                <svg class="alert-icon" width="16" height="16" fill="currentColor" viewBox="0 0 20 20" aria-hidden="true">
                    <path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clip-rule="evenodd"/>
                </svg>
                Settings saved ({saved} values updated).
                <span class="save-note dim">Settings are applied when the station next starts — use <a href="/admin/system">Restart</a> to apply them now.</span>
            </div>"#
            ));
            // O-18: toast the success outcome via OOB, with a follow-up action
            // — settings only take effect on next restart, so surface the link.
            Ok(toast::with(
                body,
                // The action used to read "Open system", which says where to
                // go and not why. Three places told the reader about the
                // restart in three different ways — "Most settings require a
                // restart", "Changes apply on next restart", and, in the most
                // prominent of the three, nothing at all.
                Toast::success(format!(
                    "Settings saved ({saved} values updated). Restart to apply them."
                ))
                .with_action("/admin/system", "Restart →"),
            ))
        }
        Err(e) => {
            // Log the detail server-side; show the client a generic message.
            // The previous code interpolated the raw `SettingsError` Display
            // straight into the response HTML and toast, which both leaked
            // internal (DB/schema) detail and was an unescaped reflection of
            // error text — matching the `log_internal` policy used elsewhere
            // closes both.
            tracing::error!(error = %e, "failed to save settings");
            let body = Html(
                r#"<div class="alert alert-error" id="settings-feedback"
                        hx-swap-oob="true">
                    Failed to save settings — check the server logs for details.
                </div>"#
                    .to_string(),
            );
            Ok(toast::with(
                body,
                Toast::error(
                    "Failed to save settings — check the server logs for details.".to_string(),
                ),
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
    /// Latitude in decimal degrees.
    pub lat: f64,
    /// Longitude in decimal degrees.
    pub lon: f64,
    /// Nearest city name returned by ip-api.com; empty when unavailable.
    pub city: String,
    /// Country name returned by ip-api.com; empty when unavailable.
    pub country: String,
    /// IANA timezone name (e.g. `"America/New_York"`) reported by ip-api.com.
    /// Empty when the service omits it. Surfaced so first-run onboarding can
    /// store it for display and the doctor's clock check (the OS clock remains
    /// the source of truth for wall-clock times — see `doctor::clock`).
    pub timezone: String,
}

/// Detect the station's approximate location using the public ip-api.com service.
///
/// Returns `{"lat": ..., "lon": ..., "city": ..., "country": ..., "timezone": ...}`
/// on success, or `500` with an error message on failure.
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
        #[serde(default)]
        timezone: String,
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
        timezone: data.timezone,
    }))
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Every credential in `raw` masked, by the project's one redaction rule:
/// [`is_secret_key`](birdnet_core::config::redact::is_secret_key) by name, then
/// [`redact_value`](birdnet_core::config::redact::redact_value) by value shape
/// (an Apprise URL, a heartbeat URL, an RTSP password). The settings API and
/// the forms a viewer sees both use this, so the two cannot disagree about
/// which values are secret.
#[must_use]
pub(crate) fn mask_credentials(raw: &HashMap<String, String>) -> HashMap<String, String> {
    use birdnet_core::config::redact::{REDACTED, is_secret_key, redact_value};
    raw.iter()
        .map(|(k, v)| {
            let shown = if is_secret_key(k) {
                REDACTED.to_owned()
            } else {
                redact_value(v)
            };
            (k.clone(), shown)
        })
        .collect()
}

/// The settings as `user` may see them in a form.
///
/// An admin sees every stored value — they are the one who types them in. A
/// viewer is read-only on `/admin`, and "read-only" had meant "can read the
/// SMTP password, the BirdWeather token and every notification URL in
/// plaintext": the settings API has always masked them, the forms did not.
///
/// `None` — no identity on the request, which the admin gate never lets
/// happen — is treated as a viewer: the safe default for a form that would
/// otherwise print credentials.
///
/// # Errors
///
/// The settings could not be read; see [`load_all_settings`].
pub(crate) fn load_settings_for(
    state: &AppState,
    user: Option<&crate::auth_middleware::RequestUser>,
) -> Result<HashMap<String, String>, String> {
    let raw = load_all_settings(state)?;
    Ok(
        if user.is_some_and(crate::auth_middleware::RequestUser::is_admin) {
            raw
        } else {
            mask_credentials(&raw)
        },
    )
}

/// Every stored setting, by key.
///
/// # Errors
///
/// The read failed. It used to default to an empty map, and every caller
/// then went on as if the station had no settings: the form rendered its
/// defaults as the configuration, the save counted every field as changed
/// and wrote all of them, and the API reported `{}`.
pub(crate) fn load_all_settings(state: &AppState) -> Result<HashMap<String, String>, String> {
    state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        list(conn, None)
            .map(|rows| rows.into_iter().map(|s| (s.key, s.value)).collect())
            .map_err(|e| e.to_string())
    })
}

/// What a settings form says in place of itself when the settings could not
/// be read. The form is withheld, not rendered from defaults: saving a form of
/// defaults would write them over the real configuration.
pub(crate) fn unreadable_notice(detail: &str) -> String {
    format!(
        r#"<div class="alert alert-error" role="alert"><p><b>The station's settings could not be read</b>, so the form is not shown — saving it would write defaults over them. Nothing has changed. <a href="/admin/doctor">The doctor</a> can say what is wrong with the database.</p><p class="bnb-meta mono">{}</p></div>"#,
        crate::routes::pages::escape_html(detail)
    )
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
            | "clip_retention_days"
            | "max_files_per_species"
            | "purge_threshold"
            | "stream_retention_secs"
            | "stream_max_mb"
            | "email_smtp_port"
            | "email_cooldown_secs"
            | "notify_cooldown"
    )
}

/// What a submitted value is compared with when the settings table has no row
/// for its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unset {
    /// The settings page: a key with no row was shown at its form default, so
    /// submitting that default back is not a change.
    AsShownByTheForm,
    /// The API: the client saw no form, so any non-empty value it sends for a
    /// key with no row is a change.
    AsEmpty,
}

/// Whether `value` is what the operator already had for `key`: the stored
/// row when there is one (an empty row included — the form shows it empty),
/// otherwise nothing, or the form default when `unset` says the form showed it.
fn unchanged(
    existing: &std::collections::HashMap<String, String>,
    key: &str,
    value: &str,
    unset: Unset,
) -> bool {
    existing.get(key).map_or_else(
        || {
            value.is_empty()
                || (unset == Unset::AsShownByTheForm && value == super::render::form_default(key))
        },
        |stored| value == stored,
    )
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
pub(crate) fn build_settings_items(
    form: &SettingsForm,
    existing: &std::collections::HashMap<String, String>,
    unset: Unset,
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
                if !unchanged(existing, $key, &value, unset) {
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
    push!(form.audio_format, "audio_format", SettingsCategory::Audio);
    push!(form.freq_shift_hz, "freq_shift_hz", SettingsCategory::Audio);
    push!(
        form.clip_target_lufs,
        "clip_target_lufs",
        SettingsCategory::Audio
    );
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
        form.recording_schedule,
        "recording_schedule",
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
    push!(
        form.confirmation_level,
        "confirmation_level",
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
        form.notify_urls,
        "notify_urls",
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
        form.weekly_report_schedule,
        "weekly_report_schedule",
        SettingsCategory::Notifications
    );
    push!(
        form.heartbeat_url,
        "heartbeat_url",
        SettingsCategory::Notifications
    );
    push!(
        form.deadman_hours,
        "deadman_hours",
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
        form.clip_retention_days,
        "clip_retention_days",
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
        form.extraction_length,
        "extraction_length",
        SettingsCategory::System
    );
    push!(
        form.rare_species_days,
        "rare_species_days",
        SettingsCategory::System
    );
    push!(
        form.raw_spectrogram,
        "raw_spectrogram",
        SettingsCategory::System
    );
    push!(
        form.purge_threshold,
        "purge_threshold",
        SettingsCategory::System
    );
    push!(
        form.stream_retention_secs,
        "stream_retention_secs",
        SettingsCategory::System
    );
    push!(
        form.stream_max_mb,
        "stream_max_mb",
        SettingsCategory::System
    );
    push!(form.site_name, "site_name", SettingsCategory::System);
    push!(form.info_site, "info_site", SettingsCategory::System);
    push!(
        form.database_lang,
        "database_lang",
        SettingsCategory::System
    );
    // Auth: no rows. See the note on `SettingsForm` — the admin credential is
    // an Argon2id hash in the accounts table, not a settings value.
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

    /// The form's ranges and `birdnet-core`'s must not drift.
    ///
    /// `BOUNDED_FIELDS` carries its own numbers rather than looking them up,
    /// because two of its seven entries have no config-file counterpart and
    /// adding them to `validate()` would let a mistyped alert threshold revert
    /// a station's whole configuration. That leaves five pairs of numbers in
    /// two files, so this is what keeps them equal.
    #[test]
    fn the_form_bounds_match_the_validator() {
        for &(field, config_key) in CONFIG_KEY_OF {
            let form = BOUNDED_FIELDS
                .iter()
                .find(|(k, ..)| *k == field)
                .unwrap_or_else(|| panic!("{field} is not a bounded field"));
            let core = birdnet_core::config::validate::NUMERIC_RANGES
                .iter()
                .find(|(k, ..)| *k == config_key)
                .unwrap_or_else(|| panic!("{config_key} is not a validated range"));
            assert_eq!(
                (form.2, form.3),
                (core.1, core.2),
                "{field} / {config_key}: the form would accept a value \
                 `--doctor` rejects, or reject one it accepts"
            );
        }
    }

    /// The counterpart: the two alert thresholds must stay *out* of the
    /// validator's list. Putting them in is the change this whole arrangement
    /// exists to prevent.
    #[test]
    fn the_alert_thresholds_are_not_validated_against_the_config_file() {
        for key in ["APPRISE_MIN_CONFIDENCE", "EMAIL_MIN_CONFIDENCE"] {
            assert!(
                !birdnet_core::config::validate::NUMERIC_RANGES
                    .iter()
                    .any(|(k, ..)| *k == key),
                "{key} in NUMERIC_RANGES makes `is_usable` false for a config \
                 file carrying a bad alert threshold, and `startup_config::choose` \
                 then reverts every other setting with it"
            );
        }
    }

    fn empty_form() -> SettingsForm {
        SettingsForm {
            raw_spectrogram: None,
            extraction_length: None,
            rare_species_days: None,
            alsa_device: None,
            rtsp_url: None,
            rtsp_urls: None,
            segment_duration: None,
            audio_format: None,
            freq_shift_hz: None,
            clip_target_lufs: None,
            latitude: None,
            longitude: None,
            station_name: None,
            recording_schedule: None,
            heartbeat_url: None,
            deadman_hours: None,
            database_lang: None,
            confidence_threshold: None,
            sensitivity: None,
            overlap: None,
            sf_thresh: None,
            privacy_threshold: None,
            confirmation_level: None,
            apprise_url: None,
            apprise_config: None,
            notify_urls: None,
            birdweather_token: None,
            notify_confidence: None,
            notify_cooldown: None,
            notify_trigger: None,
            notify_species_only: None,
            notify_species_exclude: None,
            notify_title_template: None,
            notify_body_template: None,
            weekly_report_schedule: None,
            species_exclude: None,
            species_include: None,
            clip_retention_days: None,
            image_cache_dir: None,
            custom_image_dir: None,
            max_files_per_species: None,
            purge_threshold: None,
            stream_retention_secs: None,
            stream_max_mb: None,
            site_name: None,
            info_site: None,
            night_inhibit: None,
            pre_sunrise_offset: None,
            post_sunset_offset: None,
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
        let items = build_settings_items(&form, &HashMap::new(), Unset::AsEmpty);
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
        let items = build_settings_items(&form, &HashMap::new(), Unset::AsEmpty);
        let lon = items.iter().find(|(k, _, _)| *k == "longitude").unwrap();
        assert_eq!(lon.1, "-71.0589");
    }

    #[test]
    fn confidence_threshold_normalised() {
        let form = SettingsForm {
            confidence_threshold: Some("0,75".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &HashMap::new(), Unset::AsEmpty);
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
        let items = build_settings_items(&form, &existing, Unset::AsEmpty);
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
        let items = build_settings_items(&form, &existing, Unset::AsEmpty);
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
        let items = build_settings_items(&form, &HashMap::new(), Unset::AsEmpty);
        assert!(!items.iter().any(|(k, _, _)| *k == "latitude"));
        assert!(!items.iter().any(|(k, _, _)| *k == "confidence_threshold"));
        // Under `Unset::AsEmpty` (the API's baseline) `night_inhibit=false`
        // with no row still counts as a change. The settings page uses
        // `Unset::AsShownByTheForm`, where it does not — pinned end to end by
        // tests/saving_the_settings_form_unchanged_writes_nothing.rs.
    }

    #[test]
    fn changed_field_is_persisted() {
        let mut existing = HashMap::new();
        existing.insert("latitude".to_string(), "42.3601".to_string());
        let form = SettingsForm {
            latitude: Some("51.5074".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &existing, Unset::AsEmpty);
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
        let items = build_settings_items(&form, &existing, Unset::AsEmpty);
        let lat = items
            .iter()
            .find(|(k, _, _)| *k == "latitude")
            .expect("clearing should be persisted");
        assert_eq!(lat.1, "");
    }

    #[test]
    fn every_declared_key_is_actually_persisted() {
        // The other half of the contract `SETTINGS_FORM_KEYS` carries: the list
        // is what the binary classifies and enforces, so a key that is declared
        // but never reaches `set_many` would be classified as wired while still
        // doing nothing. Submitting a form with every field populated must emit
        // exactly the declared set — no more, no fewer.
        use super::super::form::SETTINGS_FORM_KEYS;
        use std::collections::BTreeSet;

        // Build the form the way a browser does, so the fields exercised are the
        // ones serde actually binds.
        let submitted: std::collections::HashMap<&str, &str> =
            SETTINGS_FORM_KEYS.iter().map(|k| (*k, "1")).collect();
        let form: SettingsForm =
            serde_json::from_value(serde_json::to_value(&submitted).expect("payload serialises"))
                .expect("a payload of every declared key deserialises into the form");

        let emitted: BTreeSet<&str> = build_settings_items(&form, &HashMap::new(), Unset::AsEmpty)
            .into_iter()
            .map(|(k, _, _)| k)
            .collect();
        let declared: BTreeSet<&str> = SETTINGS_FORM_KEYS.iter().copied().collect();

        assert_eq!(
            emitted, declared,
            "build_settings_items must persist exactly the declared form keys"
        );
    }

    #[test]
    fn non_numeric_field_passes_through_unchanged() {
        // station_name with a comma is a perfectly valid name (e.g.
        // "Backyard, Boston") — it must not be normalised.
        let form = SettingsForm {
            station_name: Some("Backyard, Boston".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &HashMap::new(), Unset::AsEmpty);
        let name = items.iter().find(|(k, _, _)| *k == "station_name").unwrap();
        assert_eq!(name.1, "Backyard, Boston");
    }
}
