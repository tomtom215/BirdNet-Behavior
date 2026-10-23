//! Settings route handlers (GET / POST).

use crate::routes::form::HtmxForm;
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
    state
        .run_blocking(move |state| {
            Ok(Html(match load_settings_for(state, user.as_deref()) {
                Ok(settings_map) => render_settings_page(&settings_map, state.names()),
                Err(e) => crate::routes::admin::admin_shell(
                    "Settings",
                    "settings",
                    &unreadable_notice(&e),
                ),
            }))
        })
        .await
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
    state
        .run_blocking(move |state| {
            Ok(Html(match load_settings_for(state, user.as_deref()) {
                Ok(settings_map) => render_settings_form(&settings_map, state.names()),
                Err(e) => unreadable_notice(&e),
            }))
        })
        .await
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
    // `validate()` refuses these in the config file, but a value typed here
    // is overlaid after that check has run — the same gap as the thresholds.
    ("latitude", "Latitude", -90.0, 90.0),
    ("longitude", "Longitude", -180.0, 180.0),
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

/// Every settings field the station reads as a count and not a fraction (a
/// number of classes, a number of seconds): the
/// form key, the label the page shows, the range outside which the value
/// cannot do its job, and what a person typing the wrong thing needs told.
///
/// The bounds are `birdnet-core`'s own constants — the ones
/// [`birdnet_core::config::validate::validate`] enforces on the config file —
/// read, not copied, so the page cannot accept a number `--doctor` refuses or
/// refuse one it accepts. `the_form_and_the_validator_agree_about_a_whole_number`
/// holds the two *behaviours* equal at each edge, which is the claim that
/// matters if somebody later swaps a constant for a literal.
///
/// Kept apart from [`BOUNDED_FIELDS`] because the check is a different one: a
/// value in that list is parsed as a decimal, and `0.02` in this one would
/// truncate to `0`, which for `privacy_rank` is the rule switched off.
const WHOLE_NUMBER_FIELDS: &[(&str, &str, u64, u64, &str)] = &[
    (
        "privacy_rank",
        "Privacy Rank",
        birdnet_core::config::validate::PRIVACY_RANK_MIN,
        birdnet_core::config::validate::PRIVACY_RANK_MAX,
        "It counts classes, it is not a confidence (that is the Privacy Threshold field): \
         0 turns the rule off and 10 is BirdNET-Pi's own.",
    ),
    (
        "segment_duration",
        "Segment Duration",
        birdnet_core::config::validate::SEGMENT_DURATION_MIN,
        birdnet_core::config::validate::SEGMENT_DURATION_MAX,
        "It is the length of each recording chunk in seconds, not a count of chunks: \
         3 is one analysis window and 15 is the default.",
    ),
];

/// The config key each whole-number field maps to, for
/// `the_form_and_the_validator_agree_about_a_whole_number`.
#[cfg(test)]
const WHOLE_NUMBER_CONFIG_KEY_OF: &[(&str, &str)] = &[
    ("privacy_rank", "PRIVACY_RANK"),
    ("segment_duration", "SEGMENT_DURATION"),
];

/// What is wrong with one whole-number submission, or `None` when it is fine.
///
/// A blank value is fine: it clears the setting, and the station runs on the
/// default. Nothing is rounded, truncated or reinterpreted — a fraction, a
/// percent sign, a sign or a thousands separator is refused and quoted back, so
/// the number stored is always the number typed.
fn whole_number_problem(
    label: &str,
    raw: &str,
    min: u64,
    max: u64,
    advice: &str,
) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let rule = format!("{label} must be a whole number from {min} to {max}. {advice}");
    if raw.bytes().all(|b| b.is_ascii_digit()) {
        // Digits only. One too long for a u64 is out of range, not unreadable.
        return match raw.parse::<u64>() {
            Ok(v) if (min..=max).contains(&v) => None,
            _ => Some(format!("{rule} You entered {raw}.")),
        };
    }
    Some(format!("{rule} You entered “{raw}”."))
}

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
pub(crate) fn range_problems(form: &SettingsForm) -> Vec<String> {
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
        ("latitude", form.latitude.as_ref()),
        ("longitude", form.longitude.as_ref()),
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
    // The same rule for the counts: the submission, not the diff, and sized by
    // the table so a field added there and not here does not compile.
    let submitted: [(&str, Option<&String>); WHOLE_NUMBER_FIELDS.len()] = [
        ("privacy_rank", form.privacy_rank.as_ref()),
        ("segment_duration", form.segment_duration.as_ref()),
    ];
    for (key, value) in submitted {
        let Some(value) = value else { continue };
        let Some(&(_, label, min, max, advice)) =
            WHOLE_NUMBER_FIELDS.iter().find(|(k, ..)| *k == key)
        else {
            continue;
        };
        problems.extend(whole_number_problem(label, value, min, max, advice));
    }
    // The fifteen numbers the station parses and, outside their range, quietly
    // replaces or clamps. Read from the submission, like the rest, and from the
    // table `--doctor` checks the file against, so the two cannot disagree.
    for bound in birdnet_core::config::validate::FIELD_BOUNDS {
        if let Some(value) = bounded_value(form, bound.form_key)
            && !bound.accepts(value)
        {
            problems.push(bound.refusal(value.trim()));
        }
    }
    problems
}

/// The submitted text of one of the fields in
/// [`birdnet_core::config::validate::FIELD_BOUNDS`].
///
/// A match and not a lookup in a map, so that adding a bound without wiring the
/// field here is a failing test (`every_bounded_field_is_wired`) and not a bound
/// that is checked against nothing.
fn bounded_value<'a>(form: &'a SettingsForm, key: &str) -> Option<&'a String> {
    match key {
        "freq_shift_hz" => form.freq_shift_hz.as_ref(),
        "pre_sunrise_offset" => form.pre_sunrise_offset.as_ref(),
        "post_sunset_offset" => form.post_sunset_offset.as_ref(),
        "notify_cooldown" => form.notify_cooldown.as_ref(),
        "deadman_hours" => form.deadman_hours.as_ref(),
        "clip_retention_days" => form.clip_retention_days.as_ref(),
        "max_files_per_species" => form.max_files_per_species.as_ref(),
        "purge_threshold" => form.purge_threshold.as_ref(),
        "extraction_length" => form.extraction_length.as_ref(),
        "rare_species_days" => form.rare_species_days.as_ref(),
        "stream_retention_secs" => form.stream_retention_secs.as_ref(),
        "stream_max_mb" => form.stream_max_mb.as_ref(),
        "email_smtp_port" => form.email_smtp_port.as_ref(),
        "email_cooldown_secs" => form.email_cooldown_secs.as_ref(),
        "clip_target_lufs" => form.clip_target_lufs.as_ref(),
        _ => None,
    }
}

/// Problems with the fields that take one word out of a short list, phrased for
/// the person who typed them.
///
/// The page offers a `<select>` for the clip format and the species-info site, so
/// only a hand-made request or `PUT /api/v2/settings` can send anything else;
/// the recording window is a free-text box with suggestions, so a person can.
/// Each is held to the predicate `birdnet_core::config::validate` uses for the
/// same key in the config file, not to a second list, and a blank value clears
/// the setting as it does everywhere else.
///
/// An unknown value is not rejected downstream, it is *reinterpreted*: a window
/// the scheduler cannot parse becomes "record all day" with a line in the log,
/// and the page goes on showing the text that was typed.
fn choice_problems(form: &SettingsForm) -> Vec<String> {
    use birdnet_core::config::validate as v;
    let given = |value: &Option<String>| -> Option<String> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
            .map(str::to_owned)
    };
    let mut problems = Vec::new();
    if let Some(raw) = given(&form.recording_schedule) {
        if !v::is_recording_schedule(&raw) {
            problems.push(format!(
                "Recording window must be all-day, solar, or fixed:HH:MM-HH:MM \
                 (for example fixed:06:00-20:00). You entered “{raw}”."
            ));
        } else if let Some((start, end)) =
            raw.strip_prefix("fixed:").and_then(v::fixed_window_minutes)
            && birdnet_scheduler::RecordingWindow::fixed(start, end).is_err()
        {
            // `validate()` accepts this grammar; the scheduler does not. A
            // window that crosses midnight is the likely typo for a night
            // recorder, and it would be run as all-day.
            problems.push(format!(
                "Recording window “{raw}” does not end after it starts. A fixed window \
                 stays within one day, so the station would ignore it and record all day. \
                 Use solar, or a window such as fixed:06:00-20:00."
            ));
        }
    }
    if let Some(raw) = given(&form.audio_format)
        && !v::is_audio_format(&raw)
    {
        problems.push(format!(
            "Extracted Clip Format must be one of {}. You entered “{raw}”.",
            v::AUDIO_FORMATS.join(", ")
        ));
    }
    if let Some(raw) = given(&form.full_disk_action)
        && !v::is_full_disk_action(&raw)
    {
        problems.push(format!(
            "When the disk is full must be one of {}. You entered “{raw}”.",
            v::FULL_DISK_ACTIONS.join(", ")
        ));
    }
    if let Some(raw) = given(&form.notify_trigger) {
        let unknown = v::unknown_trigger_words(&raw);
        if !unknown.is_empty() {
            problems.push(format!(
                "Notification trigger takes each, new-species, new-species-daily or off \
                 (several may be listed, separated by commas). You entered “{raw}”."
            ));
        }
    }
    if let Some(raw) = given(&form.spectrogram_colormap)
        && !crate::routes::spectrogram::is_colormap(&raw)
    {
        problems.push(format!(
            "Spectrogram colour map must be one of viridis, magma, inferno, grey, \
             grey_inverted. You entered “{raw}”."
        ));
    }
    if let Some(raw) = given(&form.spectrogram_range_db)
        && !crate::routes::spectrogram::is_range_db(&raw)
    {
        problems.push(format!(
            "Spectrogram dynamic range must be one of 60, 80, 100, 120 (dB). You \
             entered “{raw}”."
        ));
    }
    if let Some(raw) = given(&form.time_format)
        && !crate::display::is_time_format(&raw)
    {
        problems.push(format!("Clock must be 24h or 12h. You entered “{raw}”."));
    }
    if let Some(raw) = given(&form.temperature_unit)
        && !crate::display::is_temperature_unit(&raw)
    {
        problems.push(format!(
            "Temperature unit must be celsius or fahrenheit. You entered “{raw}”."
        ));
    }
    if let Some(problem) = form
        .dashboard_image
        .as_deref()
        .and_then(crate::routes::dashboard_image::path_problem)
    {
        problems.push(problem);
    }
    if let Some(problem) = form
        .dashboard_image_title
        .as_deref()
        .and_then(crate::routes::dashboard_image::title_problem)
    {
        problems.push(problem);
    }
    if let Some(raw) = given(&form.info_site)
        && !v::is_info_site(&raw)
    {
        problems.push(format!(
            "Species Info Links must be one of {}. You entered “{raw}”.",
            v::INFO_SITES.join(", ")
        ));
    }
    problems
}

/// Everything wrong with a submission, for the settings page and for
/// `PUT /api/v2/settings` alike.
///
/// The two used to run different sets: the page ran the range, token and
/// language checks, the API only the ranges, so `{"database_lang": "xx"}` and a
/// BirdWeather token with a `/` in it were stored by a script and refused from the
/// page. One list means a check added here reaches both.
pub(crate) fn submission_problems(form: &SettingsForm) -> Vec<String> {
    let mut problems = range_problems(form);
    problems.extend(choice_problems(form));
    problems.extend(birdweather_token_problem(form));
    problems.extend(language_problem(form));
    problems.extend(stream_url_problems(form));
    problems.extend(super::extra::problems(&form.extra));
    problems.extend(super::masking::leftover_problems(form));
    problems
}

/// Fields saved with surrounding whitespace removed.
///
/// Opaque identifiers, which a copy from a web page tends to bring a newline
/// or a space with, and whole numbers, where a stray space would make the
/// daemon's `PRIVACY_RANK` read fail and fall back to the default. Not
/// passwords: a space can be part of one.
const TRIMMED_FIELDS: &[&str] = &["birdweather_token", "privacy_rank"];

/// A BirdWeather token holding anything but letters and digits, explained.
///
/// The token is placed in the upload address, so a stray character — an
/// invisible one from a copy, a `/` from the address bar — makes every upload
/// fail for good while the setting looks right. Only the characters are
/// checked: the length is BirdWeather's to define, and refusing a valid token
/// would turn sharing off for nothing.
fn birdweather_token_problem(form: &SettingsForm) -> Option<String> {
    let token = form.birdweather_token.as_deref()?.trim();
    if token.is_empty() || token.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    Some(
        "The BirdWeather token can only contain letters and digits — it goes into the \
         upload address. Copy it again from your station's page on BirdWeather."
            .to_owned(),
    )
}

/// A common-name language that is not one the station has names for, explained.
///
/// The page offers a list, so only a hand-made request can send anything else.
/// It is refused rather than stored because an unknown value is silently the
/// same as English at start-up, and a station whose owner picked German and
/// got English with no word of why is a worse failure than a refused form.
/// Empty is a real answer: it clears the setting.
fn language_problem(form: &SettingsForm) -> Option<String> {
    let lang = form.database_lang.as_deref()?.trim();
    if lang.is_empty() || birdnet_core::i18n::find_language(lang).is_some() {
        return None;
    }
    Some(format!(
        "{lang:?} is not a language species names are available in. Choose one from the list."
    ))
}

/// A network audio address or a heartbeat address that cannot be requested,
/// explained.
///
/// Both are typed by the owner (or imported from another station's
/// `birdnet.conf`) and then requested for as long as the station runs. A
/// stream address must start with a scheme the capture code knows; a heartbeat
/// address is requested with plain HTTP(S). The messages name the field and
/// never the value: both usually carry a key or a camera password.
fn stream_url_problems(form: &SettingsForm) -> Vec<String> {
    use birdnet_core::audio::capture::StreamProtocol;
    let mut problems = Vec::new();
    for (field, value) in [("rtsp_url", &form.rtsp_url), ("rtsp_urls", &form.rtsp_urls)] {
        let bad = value.as_deref().is_some_and(|list| {
            list.split(',')
                .map(str::trim)
                .filter(|piece| !piece.is_empty())
                .any(|piece| StreamProtocol::of(piece).is_none())
        });
        if bad {
            problems.push(format!(
                "An address in {field} does not start with a scheme this station can read \
                 ({}). Check each address, comma-separated, begins like rtsp://camera/stream.",
                StreamProtocol::SCHEMES
            ));
        }
    }
    if let Some(url) = form.heartbeat_url.as_deref().map(str::trim)
        && !url.is_empty()
    {
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            problems.push(
                "The heartbeat address must start with http:// or https:// — it is requested \
                 with an ordinary web request."
                    .to_owned(),
            );
        }
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
    HtmxForm(form): HtmxForm<SettingsForm>,
) -> Result<Html<String>, StatusCode> {
    let mut form = form;
    state.run_blocking(move |state| {
    // Compare submitted values against the current DB state so we only
    // persist the rows the operator actually changed. Without this the
    // page's render-time defaults (e.g. `night_inhibit=false` when no row
    // exists) would silently overlay over the file config / env every
    // time *any* unrelated setting is saved.
    // Refuse outright when the current values cannot be read: the diff below
    // would count every submitted field as changed and write all of them,
    // render-time defaults included, over the operator's real configuration.
    let existing = match load_all_settings(state) {
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
    // A credential the form was shown masked comes back as the mask; put what is
    // stored back in its place before anything checks or diffs the submission.
    super::masking::resolve(&mut form, &existing);
    let items = build_settings_items(&form, &existing, Unset::AsShownByTheForm);

    // Reject before writing, and reject the whole submission: a partial save
    // would leave the form showing one thing and the station running another.
    let problems = submission_problems(&form);
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
                    state,
                    Some(&request_user),
                    "settings.update",
                    None,
                    Some(&keys),
                );
            }
            // Which of what was just saved is in force already and which waits
            // for the station to start again. The page used to say "Settings are
            // applied when the station next starts" of all of them, which was
            // false of the species lists and the spectrogram options and,
            // worse, left the person with a link to a page rather than a button.
            let (waiting, in_force): (Vec<&str>, Vec<&str>) =
                items.iter().map(|(k, _, _)| *k).partition(|k| {
                    super::applies::applies_for(k).is_some_and(super::applies::Applies::needs_restart)
                });
            let readable = |keys: &[&str]| {
                keys.iter()
                    .map(|k| k.replace('_', " "))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let when = if waiting.is_empty() {
                if saved == 0 {
                    String::new()
                } else {
                    r#"<span class="save-note dim">These are in force already.</span>"#.to_owned()
                }
            } else {
                format!(
                    r#"<div class="applies-banner" role="status"><div class="applies-banner__text"><strong>{n} of these need a restart</strong> before the station uses them: {list}.{rest}</div><button type="button" class="bnb-btn primary" data-restart-now>Restart the station now</button><span class="bnb-meta" data-restart-status role="status"></span></div>"#,
                    n = waiting.len(),
                    list = crate::routes::pages::escape_html(&readable(&waiting)),
                    rest = if in_force.is_empty() {
                        String::new()
                    } else {
                        format!(" The others ({}) are in force already.", in_force.len())
                    },
                )
            };
            // A picture path is stored whatever it names (the file may appear
            // later), but a person who typed `/etc/passwd` is told now, not by
            // a Today page with no picture. Checked on the submission, not the
            // diff: a path already saved that is still not a picture is the
            // reason someone is on this page.
            let picture_note = form
                .dashboard_image
                .as_deref()
                .and_then(|path| {
                    crate::routes::dashboard_image::picture_problem(path)
                        .map(|why| format!("{}: {why}", path.trim()))
                })
                .map(|note| {
                    format!(
                        "The picture for the Today page will not show: {note}. The setting is saved, and the page looks again every ten seconds, so it will appear once the file is a picture."
                    )
                });
            let picture_html = picture_note.as_deref().map_or_else(String::new, |note| {
                format!(
                    r#"<p class="save-picture-note">{}</p>"#,
                    crate::routes::pages::escape_html(note)
                )
            });
            let zone_note = if items.iter().any(|(k, v, _)| *k == TIMEZONE_CLEARED_KEY && v.is_empty()) {
                r#"<span class="save-note dim"> The time zone recorded at setup was for the old location, so it was cleared; Diagnostics will no longer compare it with this station's clock.</span>"#
            } else {
                ""
            };
            let alert = if picture_note.is_some() {
                "alert-warn"
            } else {
                "alert-success"
            };
            let body = Html(format!(
                r#"<div class="alert {alert}" role="alert"
                    hx-swap-oob="true" id="settings-feedback">
                <svg class="alert-icon" width="16" height="16" fill="currentColor" viewBox="0 0 20 20" aria-hidden="true">
                    <path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clip-rule="evenodd"/>
                </svg>
                Settings saved ({saved} values updated).
                {when}{zone_note}{picture_html}
            </div>"#
            ));
            // O-18: toast the success outcome via OOB. The follow-up action is
            // offered only when something saved actually waits for a restart.
            let saved_message = if waiting.is_empty() {
                format!("Settings saved ({saved} values updated).")
            } else {
                format!(
                    "Settings saved ({saved} values updated). {} need a restart to apply.",
                    waiting.len()
                )
            };
            // Green says everything worked; with a picture that will not show it
            // did not, so the toast is the warning one and carries the reason.
            let toast = match &picture_note {
                Some(note) => Toast::warn(format!("{saved_message} {note}")),
                None => Toast::success(saved_message),
            };
            Ok(toast::with(
                body,
                if waiting.is_empty() {
                    toast
                } else {
                    toast.with_action("/admin/system/restart", "Restart →")
                },
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
})
.await
}

// ---------------------------------------------------------------------------
// GET /admin/settings/detect-location — auto-detect lat/lon from IP
// ---------------------------------------------------------------------------

/// Response body for the detect-location endpoint.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocationResult {
    /// Latitude in decimal degrees.
    pub lat: f64,
    /// Longitude in decimal degrees.
    pub lon: f64,
    /// Nearest city name reported by the provider; empty when unavailable.
    pub city: String,
    /// Country name reported by the provider; empty when unavailable.
    pub country: String,
    /// IANA timezone name (e.g. `"America/New_York"`) reported by the provider.
    /// Empty when the service omits it. Surfaced so first-run onboarding can
    /// store it for display and the doctor's clock check (the OS clock remains
    /// the source of truth for wall-clock times — see `doctor::clock`).
    pub timezone: String,
}

/// One IP-geolocation service: where to ask and how to read the answer.
struct GeoProvider {
    /// Host name, for the log; never sent anywhere.
    name: &'static str,
    /// An `https://` URL that answers with the caller's own address located.
    url: &'static str,
    /// Reads the response body.
    parse: fn(&[u8]) -> Result<LocationResult, String>,
}

/// The services asked, in order, until one answers. Both are free, need no key
/// and are reached over TLS; each is told the station's public IP address by
/// the request itself, which is the whole of what it learns. The second is
/// asked only when the first fails.
///
/// * `ipwho.is` — 1,000 requests a day per client address, commercial use
///   allowed, <https://ipwhois.io/documentation>.
/// * `get.geojs.io` — "No rate limits (yet)", <https://www.geojs.io/>.
///
/// `BirdNET-Pi` asked `ip-api.com`, whose free tier has no TLS: the
/// coordinates this fills in could be rewritten on the way, and the station's
/// address went across the network in clear.
const GEO_PROVIDERS: [GeoProvider; 2] = [
    GeoProvider {
        name: "ipwho.is",
        url: "https://ipwho.is/",
        parse: parse_ipwhois,
    },
    GeoProvider {
        name: "get.geojs.io",
        url: "https://get.geojs.io/v1/ip/geo.json",
        parse: parse_geojs,
    },
];

/// Longest response read from a provider; a real answer is under 1 KiB.
const GEO_MAX_BODY: usize = 64 * 1024;

/// A coordinate from a JSON number or a numeric string (`geojs` sends strings).
fn coordinate(v: Option<&serde_json::Value>, limit: f64) -> Option<f64> {
    let n = match v? {
        serde_json::Value::Number(n) => n.as_f64()?,
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (n.is_finite() && n.abs() <= limit).then_some(n)
}

/// A provider's free text made safe to store and show: control characters
/// dropped, at most `max` characters.
fn clean_text(v: Option<&serde_json::Value>, max: usize) -> String {
    v.and_then(serde_json::Value::as_str)
        .map(|s| s.chars().filter(|c| !c.is_control()).take(max).collect())
        .map(|s: String| s.trim().to_owned())
        .unwrap_or_default()
}

/// A timezone name only if it has the shape of an IANA one
/// (`Area/Location`, ASCII letters, digits and `_+-/`); the settings table
/// keeps it and the doctor reads it, so a provider's free text is not stored.
fn clean_timezone(v: Option<&serde_json::Value>) -> String {
    let t = clean_text(v, 64);
    let shaped = !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '-'));
    if shaped { t } else { String::new() }
}

/// The location in a decoded answer, or why it is not one. A coordinate that
/// is missing, not finite or out of range is a failed lookup, and so is
/// 0.0, 0.0: no provider's answer for a real station, and what an unknown
/// address is reported as.
fn located(
    root: &serde_json::Value,
    city: Option<&serde_json::Value>,
    country: Option<&serde_json::Value>,
    timezone: Option<&serde_json::Value>,
) -> Result<LocationResult, String> {
    let lat = coordinate(root.get("latitude"), 90.0);
    let lon = coordinate(root.get("longitude"), 180.0);
    match (lat, lon) {
        (Some(lat), Some(lon)) if lat != 0.0 || lon != 0.0 => Ok(LocationResult {
            lat,
            lon,
            city: clean_text(city, 100),
            country: clean_text(country, 100),
            timezone: clean_timezone(timezone),
        }),
        _ => Err("no usable coordinates in the response".into()),
    }
}

/// `https://ipwho.is/`: `{"success": true, "latitude": 37.77, "longitude":
/// -122.41, "city": ..., "country": ..., "timezone": {"id": "America/..."}}`,
/// and `{"success": false, "message": ...}` for a refusal.
fn parse_ipwhois(body: &[u8]) -> Result<LocationResult, String> {
    let root: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("invalid location response: {e}"))?;
    if root.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err("the provider reported a failure".into());
    }
    located(
        &root,
        root.get("city"),
        root.get("country"),
        root.get("timezone").and_then(|t| t.get("id")),
    )
}

/// `https://get.geojs.io/v1/ip/geo.json`: `{"latitude": "37.751", "longitude":
/// "-97.822", "country": ..., "timezone": "America/Chicago"}`. Coordinates are
/// strings; there is no city for most addresses and no success flag.
fn parse_geojs(body: &[u8]) -> Result<LocationResult, String> {
    let root: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("invalid location response: {e}"))?;
    located(
        &root,
        root.get("city"),
        root.get("country"),
        root.get("timezone"),
    )
}

/// The client for a provider: TLS only (a redirect to `http://` is refused
/// too), no more than five seconds.
fn geo_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .https_only(true)
        .build()
}

/// Ask one provider. The body is read at most [`GEO_MAX_BODY`] bytes deep and
/// only for a 2xx status.
async fn ask_provider(
    client: &reqwest::Client,
    url: &str,
    parse: fn(&[u8]) -> Result<LocationResult, String>,
) -> Result<LocationResult, String> {
    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("the provider answered {}", resp.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > GEO_MAX_BODY {
            return Err("the response is larger than any location answer".into());
        }
        body.extend_from_slice(&chunk);
    }
    parse(&body)
}

/// Detect the station's approximate location from its public IP address.
///
/// Returns `{"lat": ..., "lon": ..., "city": ..., "country": ..., "timezone": ...}`
/// on success, or `502` with an error message on failure. The request tells a
/// third-party service (see `GEO_PROVIDERS`) the station's public IP address
/// and nothing else; the wizard says so next to the button and the manual
/// repeats it.
///
/// BirdNET-Pi equivalent: `birdnet_analysis.sh` calls `curl ipinfo.io` on startup
/// to auto-populate `LATITUDE` / `LONGITUDE` when not configured.
///
/// # Errors
///
/// Returns `(StatusCode, String)` on HTTP client build failure, or when every
/// provider failed (no route, a non-2xx status, an answer that is not JSON or
/// has no usable coordinates).
pub async fn detect_location() -> Result<Json<LocationResult>, (StatusCode, String)> {
    let client = geo_client().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut last = String::from("no provider configured");
    for provider in &GEO_PROVIDERS {
        match ask_provider(&client, provider.url, provider.parse).await {
            Ok(found) => {
                tracing::info!(
                    lat = found.lat,
                    lon = found.lon,
                    city = %found.city,
                    provider = provider.name,
                    "auto-detected location"
                );
                return Ok(Json(found));
            }
            Err(e) => {
                tracing::warn!(provider = provider.name, error = %e, "location lookup failed");
                last = e;
            }
        }
    }
    Err((
        StatusCode::BAD_GATEWAY,
        format!("location lookup failed: {last}"),
    ))
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
/// An administrator sees every stored value except the credentials, which are
/// masked ([`super::masking`]): they typed them in once, and a page that prints
/// them back is a page that can be shoulder-surfed, screenshotted into a bug
/// report or cached by a proxy. A viewer is read-only on `/admin`, and
/// "read-only" had meant "can read the SMTP password, the BirdWeather token and
/// every notification URL in plaintext": the settings API has always masked
/// them, the forms did not.
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
    // Nobody sees a credential in clear: the administrator's forms show a saved
    // secret as a mask and an address with its password masked, and a save that
    // sends the mask back keeps what is stored (`masking`). A viewer's forms are
    // masked more broadly, by the rule the settings API applies.
    let mut shown = if user.is_some_and(crate::auth_middleware::RequestUser::is_admin) {
        super::masking::mask_for_admin(&raw)
    } else {
        mask_credentials(&raw)
    };
    // Not a setting: the keys saved since the station started, for the page's
    // "waiting for a restart" banner. Added here, after the masking, and not in
    // `load_all_settings`, so the API and the save's diff never see it.
    let pending = pending_restart_keys(state);
    if !pending.is_empty() {
        shown.insert(super::render::PENDING_KEY.to_owned(), pending.join(","));
    }
    // Also not a setting: whether the audio-source list exists, because once it
    // does the capture addresses below it are ignored (see `audio_sources_note`).
    if let Some(summary) = audio_sources_summary(state) {
        shown.insert(super::render::AUDIO_SOURCES_KEY.to_owned(), summary);
    }
    Ok(shown)
}

/// The station's active audio sources as the Audio Capture card lists them,
/// or `None` when it has none (or they could not be read).
///
/// Capture reads the `audio_sources` table alone as soon as one active row is in
/// it (`src/capture.rs`); the microphone and network-camera settings only seed
/// the table while it is empty. So the card offers those fields only while there
/// is no active row, and otherwise lists the rows and says where they are
/// edited. An unreadable table is `None`: the fields are then offered as they
/// always were, and the page says nothing false about sources it cannot see.
///
/// One line per source, `kind<TAB>name<TAB>device`, the device masked; see
/// `render::audio::sources_note`.
fn audio_sources_summary(state: &AppState) -> Option<String> {
    let rows = state
        .with_read_db(birdnet_db::audio_sources::AudioSourceStore::list)
        .map_err(|e| tracing::warn!(error = %e, "could not list the audio sources for the settings page"))
        .ok()?;
    let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let lines: Vec<String> = rows
        .iter()
        .filter(|s| s.disabled_at.is_none())
        .map(|s| {
            let device = if s.kind == birdnet_db::audio_sources::SourceKind::Rtsp {
                birdnet_core::config::redact::redact_value(&s.device_id)
            } else {
                s.device_id.clone()
            };
            format!(
                "{}\t{}\t{}",
                s.kind.as_str(),
                one_line(s.label.as_deref().unwrap_or("")),
                one_line(&device)
            )
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The settings saved since the station started (see [`crate::pending_restart`]).
///
/// A read that fails is not a reason to withhold the page: this only decides
/// whether to show a banner, and the page says nothing false without it.
fn pending_restart_keys(state: &AppState) -> Vec<String> {
    crate::pending_restart::changed_settings(state).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "could not tell which saved settings await a restart");
        Vec::new()
    })
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
    // A read: the table exists on any migrated database, so the `CREATE TABLE
    // IF NOT EXISTS` this used to run (errors discarded) had nothing to do, and
    // it kept this read on the one writer connection.
    state.with_read_db(|conn| {
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
                } else if TRIMMED_FIELDS.contains(&$key) {
                    raw.trim().to_owned()
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
        form.privacy_rank,
        "privacy_rank",
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
    push!(
        form.species_whitelist,
        "species_whitelist",
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
        form.dashboard_image,
        "dashboard_image",
        SettingsCategory::System
    );
    push!(
        form.dashboard_image_title,
        "dashboard_image_title",
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
        form.spectrogram_colormap,
        "spectrogram_colormap",
        SettingsCategory::System
    );
    push!(
        form.spectrogram_range_db,
        "spectrogram_range_db",
        SettingsCategory::System
    );
    push!(form.time_format, "time_format", SettingsCategory::System);
    push!(
        form.temperature_unit,
        "temperature_unit",
        SettingsCategory::System
    );
    push!(
        form.purge_threshold,
        "purge_threshold",
        SettingsCategory::System
    );
    push!(
        form.full_disk_action,
        "full_disk_action",
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

    // The table-driven settings: same rule, only what changed.
    items.extend(super::extra::changed(&form.extra, existing));

    // The time zone the setup wizard recorded was looked up for the coordinates
    // it saved with it. Moving the station here leaves it behind, and
    // Diagnostics compares it with the host clock and advises on it: 50.0/5.0
    // beside `America/Los_Angeles` told someone to set their clock to California.
    // This form has no time-zone field of its own (nothing here can say what the
    // new place's zone is), so a zone recorded for the old place is cleared
    // rather than kept. The wizard makes the same decision on its own form.
    let moved = items
        .iter()
        .any(|(k, _, _)| matches!(*k, "latitude" | "longitude"));
    if moved
        && existing
            .get("timezone")
            .is_some_and(|z| !z.trim().is_empty())
    {
        items.push((
            TIMEZONE_CLEARED_KEY,
            String::new(),
            SettingsCategory::Location,
        ));
    }
    items
}

/// The settings key of the time zone the wizard records; see
/// [`build_settings_items`] for why a move clears it.
const TIMEZONE_CLEARED_KEY: &str = "timezone";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // -- IP geolocation ---------------------------------------------------

    /// What `https://ipwho.is/` answered (captured 2026-10-21, trimmed to the
    /// members this code could read, the address replaced by a documentation one).
    const IPWHOIS_BODY: &str = r#"{"ip":"203.0.113.7","success":true,"type":"IPv4",
        "continent":"North America","country":"United States","country_code":"US",
        "region":"California","city":"San Francisco","latitude":37.774926,
        "longitude":-122.4194208,"timezone":{"id":"America/Los_Angeles","abbr":"PDT",
        "is_dst":true,"offset":-25200,"utc":"-07:00"}}"#;

    /// What `https://get.geojs.io/v1/ip/geo.json` answered (same capture).
    const GEOJS_BODY: &str = r#"{"accuracy":1000,"area_code":"0","asn":396982,
        "continent_code":"NA","country":"United States","country_code":"US",
        "ip":"203.0.113.7","latitude":"37.751","longitude":"-97.822",
        "organization":"AS396982 Google LLC","timezone":"America\/Chicago"}"#;

    #[test]
    fn ipwhois_is_read_into_a_location() {
        let got = parse_ipwhois(IPWHOIS_BODY.as_bytes()).expect("a real answer parses");
        assert_eq!(
            got,
            LocationResult {
                lat: 37.774_926,
                lon: -122.419_420_8,
                city: "San Francisco".into(),
                country: "United States".into(),
                timezone: "America/Los_Angeles".into(),
            }
        );
    }

    #[test]
    fn geojs_strings_are_read_into_a_location() {
        let got = parse_geojs(GEOJS_BODY.as_bytes()).expect("a real answer parses");
        assert_eq!(
            got,
            LocationResult {
                lat: 37.751,
                lon: -97.822,
                city: String::new(),
                country: "United States".into(),
                timezone: "America/Chicago".into(),
            }
        );
    }

    #[test]
    fn a_refusal_or_a_missing_coordinate_is_a_failed_lookup() {
        for body in [
            r#"{"success":false,"message":"Invalid IP address"}"#,
            r#"{"success":true,"longitude":10.0}"#,
            r#"{"success":true,"latitude":10.0}"#,
            // `success` is the provider's word; absent is not true.
            r#"{"latitude":10.0,"longitude":10.0}"#,
            r#"{"success":"true","latitude":10.0,"longitude":10.0}"#,
            "<html>Just a moment...</html>",
            "",
        ] {
            assert!(parse_ipwhois(body.as_bytes()).is_err(), "{body}");
        }
        for body in [
            r#"{"country":"X"}"#,
            r#"{"latitude":null,"longitude":null}"#,
            "[]",
        ] {
            assert!(parse_geojs(body.as_bytes()).is_err(), "{body}");
        }
    }

    /// Both sides of the range check: the edges are places, one step past is
    /// not, and neither is a value that is not a number at all.
    #[test]
    fn coordinates_are_finite_and_in_range() {
        let at = |lat: &str, lon: &str| {
            parse_geojs(format!(r#"{{"latitude":{lat},"longitude":{lon}}}"#).as_bytes())
        };
        assert!(at("90", "180").is_ok());
        assert!(at("-90", "-180").is_ok());
        assert!(at(r#""90.0""#, r#""-180.0""#).is_ok());
        for (lat, lon) in [
            ("90.0001", "0.5"),
            ("-90.0001", "0.5"),
            ("0.5", "180.0001"),
            ("0.5", "-180.0001"),
            ("91", "181"),
            (r#""NaN""#, "1"),
            ("1", r#""inf""#),
            (r#""-inf""#, "1"),
            (r#""north""#, "1"),
            ("true", "1"),
            ("1e999", "1"),
        ] {
            assert!(at(lat, lon).is_err(), "{lat}, {lon}");
        }
    }

    #[test]
    fn a_provider_that_does_not_know_the_address_is_not_a_location() {
        let at = |lat: &str, lon: &str| {
            parse_geojs(format!(r#"{{"latitude":{lat},"longitude":{lon}}}"#).as_bytes())
        };
        assert!(at("0", "0").is_err());
        assert!(at(r#""0.0""#, r#""0.0""#).is_err());
        // One coordinate on the equator or the meridian is a real place.
        assert!(at("0", "10").is_ok());
        assert!(at("10", "0").is_ok());
    }

    #[test]
    fn free_text_from_a_provider_is_cleaned_before_it_is_kept() {
        let body = serde_json::json!({
            "success": true, "latitude": 1.5, "longitude": 2.5,
            "city": format!("  Mü\u{7}nchen\n{}", "x".repeat(300)),
            "country": "Germany",
            "timezone": {"id": "Europe/Berlin; rm -rf /"},
        })
        .to_string();
        let got = parse_ipwhois(body.as_bytes()).expect("the coordinates are good");
        assert!(got.city.starts_with("München"), "{:?}", got.city);
        assert!(got.city.chars().count() <= 100);
        assert!(!got.city.chars().any(char::is_control));
        // A timezone with the wrong shape is dropped; the location is kept.
        assert_eq!(got.timezone, "");
        assert_eq!((got.lat, got.lon), (1.5, 2.5));
        for ok in [
            "America/Argentina/Buenos_Aires",
            "Etc/GMT+5",
            "UTC",
            "Asia/Ho_Chi_Minh",
        ] {
            let body = format!(r#"{{"latitude":1,"longitude":2,"timezone":"{ok}"}}"#);
            assert_eq!(parse_geojs(body.as_bytes()).unwrap().timezone, ok);
        }
    }

    /// The reason this code was changed: no provider is asked in clear text.
    #[test]
    fn every_provider_is_reached_over_tls() {
        assert!(!GEO_PROVIDERS.is_empty());
        for p in &GEO_PROVIDERS {
            assert!(p.url.starts_with("https://"), "{} is {}", p.name, p.url);
        }
    }

    /// A one-answer web server on a loopback port: `raw` is sent, whatever is
    /// asked. Returns its address.
    async fn serve_once(raw: Vec<u8>) -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0_u8; 2048];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(&raw).await;
                let _ = sock.shutdown().await;
            }
        });
        addr
    }

    fn http_response(status: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn plain_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    /// Counterpart pair: the station's client will not speak `http://` (so
    /// neither the first request nor a redirect can drop to clear text), while
    /// the very same server and answer are read by a client that may.
    #[tokio::test]
    async fn the_station_client_refuses_http_and_a_plain_one_reads_the_answer() {
        let addr = serve_once(http_response("200 OK", GEOJS_BODY.as_bytes())).await;
        let url = format!("http://{addr}/");
        let refused = ask_provider(&geo_client().unwrap(), &url, parse_geojs).await;
        assert!(refused.is_err(), "{refused:?}");

        let addr = serve_once(http_response("200 OK", GEOJS_BODY.as_bytes())).await;
        let got = ask_provider(&plain_client(), &format!("http://{addr}/"), parse_geojs)
            .await
            .expect("the fixture server answers");
        assert_eq!(got.timezone, "America/Chicago");
    }

    #[tokio::test]
    async fn a_failing_or_oversized_answer_is_a_failed_lookup() {
        let addr = serve_once(http_response(
            "429 Too Many Requests",
            GEOJS_BODY.as_bytes(),
        ))
        .await;
        let e = ask_provider(&plain_client(), &format!("http://{addr}/"), parse_geojs)
            .await
            .unwrap_err();
        assert!(e.contains("429"), "{e}");

        let big = vec![b' '; GEO_MAX_BODY + 1];
        let addr = serve_once(http_response("200 OK", &big)).await;
        let e = ask_provider(&plain_client(), &format!("http://{addr}/"), parse_geojs)
            .await
            .unwrap_err();
        assert!(e.contains("larger"), "{e}");
    }

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

    /// The two coordinates are bounded in `BOUNDED_FIELDS` and, as bare
    /// literals, in `validate()`'s `check_coords`, so unlike the five fractions
    /// there is no constant to read: this holds what each *does* equal, at the
    /// edges and at the typed mistakes (a comma decimal, a unit, an exponent).
    #[test]
    fn the_form_and_the_validator_agree_about_a_coordinate() {
        use birdnet_core::config::validate::{Severity, validate};
        for (field, config_key, limit) in [
            ("latitude", "LATITUDE", 90),
            ("longitude", "LONGITUDE", 180),
        ] {
            let mut probes: Vec<String> = vec![
                "0".into(),
                limit.to_string(),
                (-limit).to_string(),
                format!("{limit}.0000001"),
                format!("-{limit}.0000001"),
                format!("{limit},5"),
                (limit + 1).to_string(),
                "42,3601".into(),
                " 42.5 ".into(),
            ];
            probes.extend(
                [
                    "abc", "NaN", "inf", "-inf", "1e1", "1e3", "1,2,3", ".5", "5.", "+45", "45N",
                    "0x10", "",
                ]
                .map(str::to_owned),
            );
            for v in probes {
                let config = birdnet_core::config::Config::parse(&format!("{config_key}={v}"))
                    .expect("a key=value line parses");
                let validator_refuses = validate(&config)
                    .iter()
                    .any(|f| f.key == config_key && f.severity == Severity::Error);
                let mut form = empty_form();
                match field {
                    "latitude" => form.latitude = Some(v.clone()),
                    _ => form.longitude = Some(v.clone()),
                }
                let form_refuses = !range_problems(&form).is_empty();
                assert_eq!(
                    form_refuses,
                    validator_refuses,
                    "{field}={v:?}: the form {} it and `--doctor` {} it",
                    if form_refuses { "refuses" } else { "accepts" },
                    if validator_refuses {
                        "refuses"
                    } else {
                        "accepts"
                    },
                );
            }
        }
    }

    /// The three fields that take a word from a short list are held to the
    /// predicates `validate()` itself uses, so the form refuses exactly what
    /// `--doctor` refuses in the file. The one exception is the recording
    /// window, where the form is stricter on purpose (see the next test).
    #[test]
    fn the_form_and_the_validator_agree_about_a_choice() {
        use birdnet_core::config::validate::{Severity, validate};
        let cases: [(&str, &str, &[&str]); 3] = [
            (
                "recording_schedule",
                "RECORDING_SCHEDULE",
                &[
                    "all-day",
                    "solar",
                    "fixed:06:00-20:00",
                    "fixed:00:00-23:59",
                    " solar ",
                    "SOLAR",
                    "Solar",
                    "allday",
                    "sunrise-sunset",
                    "fixed:",
                    "fixed:06:00",
                    "fixed:6:0-7:0",
                    "fixed:24:00-25:00",
                    "fixed:06:60-07:00",
                    "fixed: 06:00-20:00",
                    "fixed:06:00 - 20:00",
                    "fixed:06:00-20:00 UTC",
                ],
            ),
            (
                "audio_format",
                "AUDIOFMT",
                &[
                    "wav", "mp3", "flac", "ogg", "WAV", " ogg ", "aiff", "S16_LE", "mp4", "wav,mp3",
                ],
            ),
            (
                "info_site",
                "INFO_SITE",
                &[
                    "ebird",
                    "allaboutbirds",
                    "none",
                    "eBird",
                    " none ",
                    "wikipedia",
                    "all about birds",
                ],
            ),
        ];
        for (field, config_key, probes) in cases {
            for &v in probes {
                let config = birdnet_core::config::Config::parse(&format!("{config_key}={v}"))
                    .expect("a key=value line parses");
                let validator_refuses = validate(&config)
                    .iter()
                    .any(|f| f.severity == Severity::Error);
                let mut form = empty_form();
                match field {
                    "recording_schedule" => form.recording_schedule = Some(v.to_owned()),
                    "audio_format" => form.audio_format = Some(v.to_owned()),
                    _ => form.info_site = Some(v.to_owned()),
                }
                let form_refuses = !choice_problems(&form).is_empty();
                assert_eq!(
                    form_refuses,
                    validator_refuses,
                    "{field}={v:?}: the form {} it and `--doctor` {} it",
                    if form_refuses { "refuses" } else { "accepts" },
                    if validator_refuses {
                        "refuses"
                    } else {
                        "accepts"
                    },
                );
            }
        }
    }

    /// `validate()` reads `fixed:20:00-06:00` as a recording window; the
    /// scheduler (`RecordingWindow::fixed`) refuses it, and the station then
    /// records all day with a line in the log. The form is stricter than the
    /// validator here, and says why, because the window that crosses midnight is
    /// the one a person recording owls types.
    #[test]
    fn a_window_the_scheduler_would_ignore_is_refused_and_one_it_runs_is_not() {
        for bad in [
            "fixed:20:00-06:00",
            "fixed:06:00-06:00",
            "fixed:23:59-00:00",
        ] {
            let mut form = empty_form();
            form.recording_schedule = Some(bad.to_owned());
            let problems = choice_problems(&form);
            assert_eq!(problems.len(), 1, "{bad}: {problems:?}");
            assert!(
                problems[0].contains("does not end after it starts")
                    && problems[0].contains("record all day")
                    && problems[0].contains(bad),
                "{bad}: {}",
                problems[0]
            );
        }
        for good in [
            "fixed:06:00-20:00",
            "fixed:00:00-23:59",
            "solar",
            "all-day",
            "",
        ] {
            let mut form = empty_form();
            form.recording_schedule = Some(good.to_owned());
            assert!(
                choice_problems(&form).is_empty(),
                "{good:?} is a valid answer"
            );
        }
    }

    /// The page and the API run one list. Before it, the API ran only the
    /// ranges.
    #[test]
    fn the_submission_list_holds_every_check_the_page_runs() {
        let mut form = empty_form();
        form.database_lang = Some("xx".to_owned());
        form.birdweather_token = Some("abc/123".to_owned());
        form.audio_format = Some("aiff".to_owned());
        form.confidence_threshold = Some("75".to_owned());
        let problems = submission_problems(&form);
        assert_eq!(problems.len(), 4, "{problems:?}");
    }

    /// The form and `--doctor` must take the same view of a whole number.
    ///
    /// Both read the same two constants, so a drift here would need somebody to
    /// replace a constant with a literal; this compares what each *does* at the
    /// edges and at the typed mistakes, which is the claim made in
    /// `WHOLE_NUMBER_FIELDS`. The form may be stricter than the validator on a
    /// shape no person types by accident (`+5`, which the validator reads as 5)
    /// but never looser, so every value here is one they must agree about.
    #[test]
    fn the_form_and_the_validator_agree_about_a_whole_number() {
        use birdnet_core::config::validate::{Severity, validate};
        for &(field, config_key) in WHOLE_NUMBER_CONFIG_KEY_OF {
            let &(_, _, min, max, _) = WHOLE_NUMBER_FIELDS
                .iter()
                .find(|(k, ..)| *k == field)
                .unwrap_or_else(|| panic!("{field} is not a whole-number field"));
            let mut probes: Vec<String> = [
                (min + 1).to_string(),
                min.to_string(),
                max.to_string(),
                (max - 1).to_string(),
                (max + 1).to_string(),
                (max * 5).to_string(),
            ]
            .into();
            probes.extend(
                [
                    "10",
                    "75",
                    "007",
                    "-1",
                    "0.5",
                    "0,5",
                    "0.02",
                    "10%",
                    "1,000",
                    "1e3",
                    "abc",
                    "99999999999999999999999",
                ]
                .map(str::to_owned),
            );
            for v in probes {
                let config = birdnet_core::config::Config::parse(&format!("{config_key}={v}"))
                    .expect("a key=value line parses");
                let validator_refuses = validate(&config)
                    .iter()
                    .any(|f| f.key == config_key && f.severity == Severity::Error);
                let mut form = empty_form();
                match field {
                    "privacy_rank" => form.privacy_rank = Some(v.clone()),
                    "segment_duration" => form.segment_duration = Some(v.clone()),
                    other => panic!("{other} is in the whole-number table but not in this test"),
                }
                let form_refuses = !range_problems(&form).is_empty();
                assert_eq!(
                    form_refuses,
                    validator_refuses,
                    "{field}={v:?}: the form {} it and `--doctor` {} it",
                    if form_refuses { "refuses" } else { "accepts" },
                    if validator_refuses {
                        "refuses"
                    } else {
                        "accepts"
                    },
                );
            }
        }
    }

    /// The typed mistakes are refused with a message that names the range, and
    /// a fraction is never truncated into the rule being switched off.
    #[test]
    fn a_privacy_rank_that_is_not_a_count_is_refused_and_says_what_a_count_is() {
        for bad in ["0.02", "0,02", "10%", "-5", "12001", "1,000", "ten"] {
            let mut form = empty_form();
            form.privacy_rank = Some(bad.to_owned());
            let problems = range_problems(&form);
            assert_eq!(problems.len(), 1, "{bad:?}: {problems:?}");
            let p = &problems[0];
            assert!(p.contains("Privacy Rank"), "{bad:?}: names the field: {p}");
            assert!(
                p.contains("from 0 to 12000"),
                "{bad:?}: names the range: {p}"
            );
            assert!(
                p.contains("0 turns the rule off"),
                "{bad:?}: says what 0 does: {p}"
            );
            assert!(p.contains(bad), "{bad:?}: quotes what was typed: {p}");
        }
        // Counterpart: counts, blanks and padding are not problems.
        for good in ["0", "10", "75", "12000", " 25 ", ""] {
            let mut form = empty_form();
            form.privacy_rank = Some(good.to_owned());
            assert!(
                range_problems(&form).is_empty(),
                "{good:?} is a valid answer"
            );
        }
    }

    /// Saved trimmed, and not rewritten when it is what is already stored.
    #[test]
    fn a_privacy_rank_is_stored_trimmed_and_an_unchanged_one_is_not_rewritten() {
        let mut form = empty_form();
        form.privacy_rank = Some(" 25 ".to_owned());
        let items = build_settings_items(&form, &HashMap::new(), Unset::AsShownByTheForm);
        let (key, value, category) = items
            .iter()
            .find(|(k, ..)| *k == "privacy_rank")
            .expect("a changed rank is written");
        assert_eq!((*key, value.as_str()), ("privacy_rank", "25"));
        assert_eq!(*category, SettingsCategory::Detection);

        let stored = HashMap::from([("privacy_rank".to_owned(), "25".to_owned())]);
        let again = build_settings_items(&form, &stored, Unset::AsShownByTheForm);
        assert!(
            !again.iter().any(|(k, ..)| *k == "privacy_rank"),
            "the stored value, padded, is not a change: {again:?}"
        );

        // The page shows the default for a key with no row; saving it back is
        // not a change either, or the first press of Save would pin it.
        form.privacy_rank = Some(super::super::render::form_default("privacy_rank"));
        let untouched = build_settings_items(&form, &HashMap::new(), Unset::AsShownByTheForm);
        assert!(
            !untouched.iter().any(|(k, ..)| *k == "privacy_rank"),
            "the default shown by the form is not a change: {untouched:?}"
        );
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
            spectrogram_colormap: None,
            spectrogram_range_db: None,
            time_format: None,
            temperature_unit: None,
            dashboard_image: None,
            dashboard_image_title: None,
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
            privacy_rank: None,
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
            species_whitelist: None,
            clip_retention_days: None,
            image_cache_dir: None,
            custom_image_dir: None,
            max_files_per_species: None,
            purge_threshold: None,
            full_disk_action: None,
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
            extra: HashMap::new(),
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

    /// A token copied from the BirdWeather page often brings a newline or a
    /// space with it, and went into the upload URL as typed.
    #[test]
    fn a_birdweather_token_is_saved_trimmed() {
        let form = SettingsForm {
            birdweather_token: Some("  abc123XYZ\n".to_string()),
            ..empty_form()
        };
        let items = build_settings_items(&form, &HashMap::new(), Unset::AsEmpty);
        let token = items
            .iter()
            .find(|(k, _, _)| *k == "birdweather_token")
            .expect("a changed token is persisted");
        assert_eq!(token.1, "abc123XYZ");
    }

    /// The token is placed in the upload address, so a character that does
    /// not belong in one is refused before anything is written — and a
    /// token of letters and digits, or none, is not.
    #[test]
    fn a_birdweather_token_with_a_stray_character_is_refused() {
        let with = |t: &str| SettingsForm {
            birdweather_token: Some(t.to_string()),
            ..empty_form()
        };
        let problem = birdweather_token_problem(&with("abc/123"));
        assert!(problem.is_some_and(|p| p.contains("BirdWeather")));
        assert_eq!(birdweather_token_problem(&with(" abc123XYZ \n")), None);
        assert_eq!(
            birdweather_token_problem(&with("")),
            None,
            "clearing the token turns sharing off"
        );
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
        use super::super::form::all_form_keys;
        use std::collections::BTreeSet;

        // Build the form the way a browser does, so the fields exercised are the
        // ones serde actually binds — the table-driven ones through the flattened
        // map, which is the part that could silently drop a key.
        let declared_keys = all_form_keys();
        let submitted: std::collections::HashMap<&str, &str> =
            declared_keys.iter().map(|k| (*k, "1")).collect();
        let form: SettingsForm =
            serde_json::from_value(serde_json::to_value(&submitted).expect("payload serialises"))
                .expect("a payload of every declared key deserialises into the form");

        let emitted: BTreeSet<&str> = build_settings_items(&form, &HashMap::new(), Unset::AsEmpty)
            .into_iter()
            .map(|(k, _, _)| k)
            .collect();
        let declared: BTreeSet<&str> = declared_keys.iter().copied().collect();

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

    /// "Bare spectrograms" was a checkbox, and an unticked checkbox sends no
    /// field at all, so once it was on there was no way to submit it off.
    /// Observed failing against the checkbox: the page rendered no `<select>`
    /// and the form could not say `false` over a stored `true`.
    #[test]
    fn bare_spectrograms_can_be_turned_off_again() {
        let html = super::super::render::render_settings_form(
            &HashMap::from([("raw_spectrogram".to_owned(), "true".to_owned())]),
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        assert!(html.contains(r#"<select id="raw_spectrogram" name="raw_spectrogram">"#));
        assert!(
            html.contains(r#"<option value="true" selected>"#),
            "the stored choice shows"
        );
        let form = SettingsForm {
            raw_spectrogram: Some("false".to_owned()),
            ..empty_form()
        };
        let stored = HashMap::from([("raw_spectrogram".to_owned(), "true".to_owned())]);
        let items = build_settings_items(&form, &stored, Unset::AsShownByTheForm);
        assert!(
            items
                .iter()
                .any(|(k, v, _)| *k == "raw_spectrogram" && v == "false"),
            "turning it off is a change: {items:?}"
        );
    }

    /// Every bounded field has a place in `bounded_value`, and a value one past
    /// each end is refused while the ends themselves are not. The counterpart
    /// matters: a table that refused everything would otherwise pass.
    #[test]
    fn every_bounded_field_is_wired() {
        use birdnet_core::config::validate::FIELD_BOUNDS;
        for b in FIELD_BOUNDS {
            let probe = |text: &str| {
                let mut form = empty_form();
                match b.form_key {
                    "freq_shift_hz" => form.freq_shift_hz = Some(text.to_owned()),
                    "pre_sunrise_offset" => form.pre_sunrise_offset = Some(text.to_owned()),
                    "post_sunset_offset" => form.post_sunset_offset = Some(text.to_owned()),
                    "notify_cooldown" => form.notify_cooldown = Some(text.to_owned()),
                    "deadman_hours" => form.deadman_hours = Some(text.to_owned()),
                    "clip_retention_days" => form.clip_retention_days = Some(text.to_owned()),
                    "max_files_per_species" => form.max_files_per_species = Some(text.to_owned()),
                    "purge_threshold" => form.purge_threshold = Some(text.to_owned()),
                    "extraction_length" => form.extraction_length = Some(text.to_owned()),
                    "rare_species_days" => form.rare_species_days = Some(text.to_owned()),
                    "stream_retention_secs" => form.stream_retention_secs = Some(text.to_owned()),
                    "stream_max_mb" => form.stream_max_mb = Some(text.to_owned()),
                    "email_smtp_port" => form.email_smtp_port = Some(text.to_owned()),
                    "email_cooldown_secs" => form.email_cooldown_secs = Some(text.to_owned()),
                    "clip_target_lufs" => form.clip_target_lufs = Some(text.to_owned()),
                    other => panic!("{other} is in FIELD_BOUNDS and this test cannot set it"),
                }
                range_problems(&form)
            };
            let fmt = |v: f64| format!("{v}");
            assert!(
                probe(&fmt(b.min)).is_empty(),
                "{}: the minimum is refused",
                b.form_key
            );
            assert!(
                probe(&fmt(b.max)).is_empty(),
                "{}: the maximum is refused",
                b.form_key
            );
            assert!(
                probe("").is_empty(),
                "{}: blank clears and is fine",
                b.form_key
            );
            let above = if b.whole { b.max + 1.0 } else { b.max + 0.5 };
            let below = if b.whole { b.min - 1.0 } else { b.min - 0.5 };
            assert_eq!(
                probe(&fmt(above)).len(),
                1,
                "{}: {above} is allowed",
                b.form_key
            );
            assert_eq!(
                probe(&fmt(below)).len(),
                1,
                "{}: {below} is allowed",
                b.form_key
            );
            assert_eq!(probe("lots").len(), 1, "{}: text is allowed", b.form_key);
            if b.whole {
                assert_eq!(
                    probe("1.5").len(),
                    1,
                    "{}: a fraction is allowed",
                    b.form_key
                );
            }
        }
    }

    #[test]
    fn a_stream_or_heartbeat_address_that_cannot_be_requested_is_refused_without_echoing_it() {
        let mut form = empty_form();
        form.rtsp_urls = Some("rtsp://cam:hunter2@10.0.0.5/s, ftp://10.0.0.6/s".to_owned());
        form.heartbeat_url = Some("file:///etc/passwd".to_owned());
        let problems = submission_problems(&form);
        let all = problems.join(" | ");
        assert!(all.contains("rtsp_urls"), "{all}");
        assert!(
            all.contains("heartbeat address must start with http"),
            "{all}"
        );
        assert!(!all.contains("hunter2") && !all.contains("passwd"), "{all}");
    }

    #[test]
    fn ordinary_stream_and_heartbeat_addresses_are_accepted() {
        let mut form = empty_form();
        form.rtsp_urls = Some(
            "rtsp://cam:pw@10.0.0.5:554/s, RTSPS://cam/s,https://icecast/stream.mp3, srt://h:1"
                .to_owned(),
        );
        form.heartbeat_url = Some("https://hc-ping.com/abc-123".to_owned());
        assert!(submission_problems(&form).is_empty());
        // Blank clears the setting and is not a problem.
        form.rtsp_urls = Some(String::new());
        form.heartbeat_url = Some("  ".to_owned());
        assert!(submission_problems(&form).is_empty());
    }
}
