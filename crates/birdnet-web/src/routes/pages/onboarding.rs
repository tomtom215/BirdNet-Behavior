//! First-run onboarding wizard.
//!
//! A full-bleed, no-chrome seven-step setup flow (Welcome → Model → Location →
//! Microphone → Accuracy → Notifications → Done) served at `/onboarding`,
//! behind the admin gate (see [`gated_router`]). It exists so that a person can
//! take a station from first boot to listening with a browser and nothing else:
//!
//! * **Model.** The bird-identifying model is a 54 MB download the binary does
//!   not ship. This step shows whether it is installed and, if not, one button
//!   installs it (progress while it downloads), through the same checked
//!   installer as `--install-model`. See [`crate::classifier_setup`].
//! * **Location.** Coordinates (with a lookup), the station's name and the
//!   language bird names are shown in.
//! * **Microphone.** The sound cards the kernel lists that can record are
//!   offered by name, with a test that measures their level; a network camera's
//!   address or a device name can be typed instead. What is chosen becomes an
//!   `audio_sources` row, the same one the Capture tab makes.
//! * **Accuracy** and **Alerts** are plain-language presets.
//! * **Done.** `POST /onboarding/save` writes everything, and the next page
//!   (`/onboarding/done`) shows what is true now — model, microphone — with the
//!   one button that restarts the station so it starts listening.
//!
//! **Every value the page shows is real.** The Microphone step and the final
//! summary card used to be a mock-up — a hard-coded "UMC202HD · card 1 ·
//! 48 kHz" microphone marked *recommended*, a "Boston, MA" location, and an
//! `http://birdnet.local/` dashboard address — shown identically to every
//! station regardless of its hardware, whereabouts or how it was reached. The
//! microphone rows are rendered from `audio_sources` and from the kernel's list
//! of sound cards, and the rows that depend on operator input are placeholders
//! the page script fills from the form, so a station with no capture source is
//! told so instead of being congratulated on a device it does not have.
//!
//! **A failed read is not an empty station.** The page reads the audio sources
//! and the stored settings before it draws anything; if either read fails it
//! says so and shows no wizard, because a wizard drawn from defaults would
//! offer to save them over the real configuration (the hidden confidence and
//! alert fields are never empty, so they are written on every completion).
//!
//! The Accuracy step exists because the minimum-confidence threshold decides
//! whether anything is recorded at all, and nothing in the setup path used to
//! mention it. Its cards are pre-selected on
//! [`DEFAULT_CONFIDENCE_THRESHOLD`](birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD),
//! so clicking straight through yields exactly what the daemon would have
//! enforced anyway.
//!
//! A fresh station (no detections yet, not onboarded) is redirected here from
//! the dashboard — see `pages::dashboard`.

use std::fmt::Write as _;

use crate::client_ip::ClientIp;
use axum::extract::State;
use axum::extract::{Extension, Request};
use axum::http::{StatusCode, header};
use axum::response::{Html, Redirect};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::{Form, Router};

use birdnet_core::audio::capture::alsa::CaptureDevice;
use birdnet_db::audio_sources::{AudioSource, AudioSourceStore, NewAudioSource, SourceKind};
use birdnet_db::settings::{self, SettingsCategory};

use crate::routes::admin::audio::{detail_for, kind_label};
use crate::routes::pages::escape_html;
use crate::state::AppState;
use birdnet_db::accounts::{self, AuditLog as _, UserStore as _};

/// Every settings key `POST /onboarding/save` can persist.
///
/// The admin settings form has had a guard for this since twenty of its fields
/// turned out to be editable, persisted, and connected to nothing: the binary
/// classifies each key in `SETTINGS_FORM_KEYS` and a test fails on any that is
/// unclassified. The wizard wrote *outside* that list, so it was never
/// covered — and did the same thing, shipping a notification choice
/// (`notification_mode`) that nothing anywhere read.
///
/// This list closes the gap: the same test now classifies these keys too, so a
/// new wizard field cannot ship inert either.
///
/// Kept in sync with the wizard form (`OnboardingForm`) by a test in this module.
pub const ONBOARDING_SETTING_KEYS: &[&str] = &[
    "latitude",
    "longitude",
    "timezone",
    "station_name",
    "database_lang",
    "notify_trigger",
    "confidence_threshold",
    "onboarding_complete",
];

/// The wizard, page and save together, mounted behind the admin auth
/// middleware.
///
/// The save writes the station's coordinates, time zone and notification
/// policy, which is configuration and belongs behind the same gate as the
/// settings form. The page used to stay public "so a fresh station still shows
/// it", which put the two halves on opposite sides of the gate: on the
/// installer's default station — a generated `CADDY_PWD`, and on a headless
/// install no location — the first page load was the wizard, and Finish
/// answered a bare `401 Sign in required.` whose `Location` led, after signing
/// in, to `405 GET /onboarding/save` with every answer gone (`ON-6`).
///
/// A fresh station still shows the wizard to anyone: with no admin password —
/// a fresh Docker run, or an operator who cleared it — the middleware bypasses.
/// With a password set, the operator signs in first and then answers the seven
/// steps as that account, which is also what lets the Location step's
/// auto-detect (an admin endpoint) work.
///
/// `GET /onboarding/save` is where the old 401's `Location` sent people after
/// login; it goes back to the wizard instead of answering 405.
pub fn gated_router() -> Router<AppState> {
    Router::new()
        .route("/onboarding", get(onboarding_page))
        .route(
            "/onboarding/save",
            get(|| async { Redirect::to("/onboarding") }).post(onboarding_save),
        )
        .route("/onboarding/done", get(onboarding_done))
        .route("/onboarding/mic-test", axum::routing::post(mic_test))
}

/// The settings the wizard must show as they already are, rather than as it
/// wishes they were.
///
/// Every one of these was hardcoded in the markup. For the text fields that
/// made the page merely uninformative — a station configured at install time
/// rendered blank boxes, so setup looked like it had lost the operator's
/// answers. For the two hidden fields it was worse: because they are never
/// empty, they slip past `onboarding_save`'s skip-if-blank guard and are
/// written on every completion. An operator who had set `CONFIDENCE=0.6` and
/// then clicked through the wizard had it silently reset to the default.
///
/// It is therefore **not** `Default`: a read that failed must not become a
/// page drawn from defaults, which would offer to save them over the real
/// values. [`Prefill::load`] returns the error and the page says so.
struct Prefill {
    latitude: String,
    longitude: String,
    station_name: String,
    database_lang: String,
    confidence: String,
    notify_trigger: String,
}

impl Prefill {
    /// Read the current values, falling back to the wizard's recommended
    /// defaults only where the station genuinely has no setting yet — a row
    /// that is absent, not a table that could not be read.
    fn load(conn: &rusqlite::Connection) -> Result<Self, settings::SettingsError> {
        let get = |key: &str, default: &str| settings::get_or(conn, key, default);
        Ok(Self {
            latitude: get("latitude", "")?,
            longitude: get("longitude", "")?,
            station_name: get("station_name", "")?,
            database_lang: get("database_lang", "")?,
            confidence: get(
                "confidence_threshold",
                // `Display`, not `{:.2}`: the cards carry `0.9`, `0.6`, `0.7`, and the
                // card that matches the stored value is chosen by comparing them.
                &birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD.to_string(),
            )?,
            notify_trigger: get("notify_trigger", DEFAULT_NOTIFY_TRIGGER)?,
        })
    }
}

/// The wizard's recommended notification trigger, used only when the station
/// has none set.
const DEFAULT_NOTIFY_TRIGGER: &str = "new-species";

async fn onboarding_page(State(state): State<AppState>, req: Request) -> Response {
    state
        .run_blocking(move |state| {
            // A signed-in *viewer* used to be walked through every step and then
            // refused at the finish line. `cookie_auth_middleware` allows safe methods
            // to any authenticated user and gates unsafe ones behind `require_admin`,
            // so `GET /onboarding` rendered the whole wizard and `POST
            // /onboarding/save` answered a bare "Forbidden — admin role required for
            // this action." with every answer gone. That is the same shape as the ON-6
            // bug this module's own doc records as fixed, for a different actor: the
            // integration test covers the unauthenticated and open-station cases and
            // never a viewer.
            if let Some(user) = req
                .extensions()
                .get::<crate::auth_middleware::RequestUser>()
                && !user.is_admin()
            {
                return not_admin_page();
            }
            // `list` already excludes soft-deleted rows (`WHERE disabled_at IS NULL`).
            //
            // A failed read is not an empty station. This used to be
            // `.unwrap_or_else(|_| (Vec::new(), Prefill::default()))`, so a
            // database that could not be read drew the wizard of a station with
            // no microphone and default settings, and its Finish would have
            // saved those defaults over the real ones.
            let loaded = state.with_read_db(|conn| {
                let sources = AudioSourceStore::list(conn).map_err(|e| e.to_string())?;
                let prefill = Prefill::load(conn).map_err(|e| e.to_string())?;
                Ok::<_, String>((sources, prefill))
            });
            let (sources, prefill) = match loaded {
                Ok(read) => read,
                Err(err) => {
                    tracing::error!(error = %err, "onboarding: could not read the audio sources or settings");
                    return read_failed_page();
                }
            };
            let needs_password = !crate::auth_middleware::admin_password_configured(state);
            let query = req.uri().query().unwrap_or("");
            let error = query
                .split('&')
                .find_map(|p| p.strip_prefix("error="))
                .unwrap_or("");
            let devices = birdnet_core::audio::capture::alsa::list_capture_devices();
            let parts = PageParts {
                mic_body: render_mic_body(
                    &sources,
                    &devices,
                    capture_verdict(state).as_deref(),
                    mic_notice(error),
                ),
                mic_summary: escape_html(&mic_summary(&sources)),
                model_panel: crate::routes::admin::classifier::render_panel(
                    &state
                        .classifier_setup()
                        .snapshot(state.detection_daemon_running()),
                ),
                lang_options: crate::routes::admin::settings::render::language_options(
                    &prefill.database_lang,
                    state.names(),
                ),
                // A refused password sends the person back to the first step, a
                // refused microphone to the fourth: the step that failed.
                start_step: match error {
                    "password" => 1,
                    e if e.starts_with("mic") => 4,
                    _ => 1,
                },
            };
            let body = render_page(
                &parts,
                &prefill,
                PasswordStep {
                    needed: needs_password,
                    error: error == "password",
                },
            );
            // `no-store` because the Welcome step can carry a password form: without
            // it the back-forward cache restores the typed fields on Back. `/login`
            // already does this.
            (
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                Html(body),
            )
                .into_response()
        })
        .await
}

/// What the wizard says instead of itself when the station could not read what
/// it needs to draw it. Not a `200`: a monitor, and the browser's history, should
/// not treat it as the wizard.
fn read_failed_page() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Html(format!(
            r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Setup · BirdNet-Behavior</title>
<link rel="stylesheet" href="/static/css/app.css?v={v}"></head>
<body><main id="main-content" class="ob-root"><div class="empty-state">
<h1 class="display es-h">Setup cannot start: the station could not read its own records.</h1>
<p class="bnb-meta es-sub">The wizard shows what is already set and offers to change it. It could not read the audio sources or the settings, and a wizard drawn from blanks would offer to save those blanks over them. Nothing has been changed.</p>
<p class="bnb-meta es-sub"><a href="/admin/doctor">Diagnostics</a> says what is wrong with the database; reload this page once it is fixed.</p>
</div></main></body></html>"#,
            v = env!("CARGO_PKG_VERSION"),
        )),
    )
        .into_response()
}

/// What a signed-in non-admin sees instead of a wizard they cannot submit.
fn not_admin_page() -> Response {
    (
        StatusCode::FORBIDDEN,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Html(format!(
            r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Setup · BirdNet-Behavior</title>
<link rel="stylesheet" href="/static/css/app.css?v={v}"></head>
<body><main id="main-content" class="ob-root"><div class="empty-state">
<h1 class="display es-h">Setup needs an administrator.</h1>
<p class="bnb-meta es-sub">This station has not finished its first-run setup, and your account cannot change its settings. Ask whoever administers it to sign in and complete setup — it takes about a minute.</p>
<p class="bnb-meta es-sub"><a href="/station">Station health</a> is readable without an admin account.</p>
</div></main></body></html>"#,
            v = env!("CARGO_PKG_VERSION"),
        )),
    )
        .into_response()
}

/// Whether the Welcome step asks for the admin password, and whether the
/// last attempt was refused.
#[derive(Clone, Copy, Default)]
struct PasswordStep {
    needed: bool,
    error: bool,
}

/// The shortest password the accounts page accepts; the wizard holds the
/// same line so the two cannot disagree about what a password is.
const MIN_PASSWORD_LEN: usize = 10;

/// The Welcome step's password block (DD-14): rendered only while the
/// station has no admin password, because that is the station every
/// `/admin/*` page is open on until somebody sets one. The first browser to
/// finish setup owns the station; the same POST that saves the settings
/// creates the password and hands that browser the session.
fn render_password_step(step: PasswordStep) -> String {
    if !step.needed {
        return String::new();
    }
    let error = if step.error {
        r#"<p class="ob-password-error" role="alert">The two passwords did not match, or the password was shorter than 10 characters. Nothing was saved.</p>"#
    } else {
        ""
    };
    format!(
        r#"<div class="bnb-card pad ob-mt-16" id="ob-password">
            <div class="ob-eyebrow">First, the admin password</div>
            <p class="ob-p">Right now anyone who can reach this station can change its settings. Choose the password for <code>admin</code>; this browser is signed in with it when you finish.</p>
            {error}
            <div class="ob-field"><label for="ob-password-1">Password (at least {MIN_PASSWORD_LEN} characters)</label><input id="ob-password-1" name="password" type="password" minlength="{MIN_PASSWORD_LEN}" autocomplete="new-password"></div>
            <div class="ob-field"><label for="ob-password-2">Password again</label><input id="ob-password-2" name="password_confirm" type="password" minlength="{MIN_PASSWORD_LEN}" autocomplete="new-password"></div>
            <p class="bnb-meta" id="ob-password-hint">Change it any time under Settings → Access.</p>
          </div>"#
    )
}

/// What the server fills into the wizard template, besides the stored settings.
struct PageParts {
    /// The microphone step's body, already markup.
    mic_body: String,
    /// The summary's microphone line, **already escaped**.
    mic_summary: String,
    /// The classifier panel, already markup.
    model_panel: String,
    /// The `<option>`s of the bird-name language picker, already markup.
    lang_options: String,
    /// The step the page opens on (1 unless a save was refused on a later one).
    start_step: u8,
}

/// Substitute the server-filled placeholders into the wizard template.
///
/// A single pass, which is what the comment here has claimed since it was
/// written — the code underneath it was a seven-link `.replace()` chain with
/// `{{mic_body}}` substituted *first*. A chained replace re-scans what it just
/// inserted, so an audio source labelled `{{password_step}}` had the whole
/// password `<form>` spliced into the microphone card, unescaped, by the later
/// links in the chain. `escape_html` cannot prevent that: it neutralises HTML,
/// not the template's own brace syntax, and braces are not among the five
/// characters it escapes. The payloads reachable this way are the app's own
/// strings rather than arbitrary script, so this was a correctness defect and
/// not an XSS hole — but it was exactly the defect the comment promised was
/// impossible.
///
/// Substituting in one pass means an inserted value is never re-read as
/// template. An unrecognised `{{name}}` is copied through verbatim so a typo in
/// the template shows up as a literal placeholder rather than silently
/// swallowing the text around it.
fn render_page(parts: &PageParts, prefill: &Prefill, password: PasswordStep) -> String {
    // Substituted in one pass each. `mic_body`, `model_panel` and `lang_options`
    // are already-rendered markup and `mic_summary` is pre-escaped by the
    // caller; the `prefill` values come from the database and land inside HTML
    // attributes, so they are escaped here rather than trusted.
    let password_step = render_password_step(password);
    let latitude = escape_html(&prefill.latitude);
    let longitude = escape_html(&prefill.longitude);
    let station_name = escape_html(&prefill.station_name);
    let confidence = escape_html(&prefill.confidence);
    let notify_trigger = escape_html(&prefill.notify_trigger);
    let start_step = parts.start_step.to_string();
    let setup_script = crate::routes::admin::classifier::CONTROLS_SCRIPT.as_str();
    let lookup = |name: &str| -> Option<String> {
        Some(match name {
            "mic_body" => parts.mic_body.clone(),
            "mic_summary" => parts.mic_summary.clone(),
            "model_panel" => parts.model_panel.clone(),
            "lang_options" => parts.lang_options.clone(),
            "start_step" => start_step.clone(),
            "setup_script" => setup_script.to_owned(),
            "latitude" => latitude.clone(),
            "longitude" => longitude.clone(),
            "station_name" => station_name.clone(),
            "confidence" => confidence.clone(),
            "notify_trigger" => notify_trigger.clone(),
            "password_step" => password_step.clone(),
            // Versioned stylesheet URL — see `pages::with_asset_version`.
            "version" => env!("CARGO_PKG_VERSION").to_owned(),
            _ => return None,
        })
    };

    fill(ONBOARDING_HTML.as_str(), &lookup)
}

/// One pass over `template`, replacing each `{{name}}` that `lookup` knows.
///
/// An unknown `{{name}}` is copied through verbatim, which keeps a typo visible
/// in the output instead of dropping the surrounding text; a value that was
/// inserted is never scanned again.
fn fill(template: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(template.len() + 4096);
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            // An unterminated `{{` — emit it and stop scanning.
            out.push_str("{{");
            rest = after;
            break;
        };
        let name = &after[..close];
        if let Some(value) = lookup(name) {
            out.push_str(&value);
        } else {
            out.push_str("{{");
            out.push_str(name);
            out.push_str("}}");
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

/// The page after Finish: what is true now.
fn render_done_page(model_panel: &str, microphone: &str, restart_needed: bool) -> String {
    // "Listening" is claimed only when the engine is up with a classifier AND
    // nothing saved is waiting for a restart; either one false and the page says
    // "almost".
    let listening = model_panel.contains(r#"data-cls-state="ready""#) && !restart_needed;
    let (eyebrow, heading) = if listening {
        ("All set", "You're <em>listening</em>.")
    } else {
        ("Saved", "Almost <em>listening</em>.")
    };
    let restart_block = if restart_needed {
        r#"<p class="ob-p">What you chose is saved. The station reads most of it when it starts, so it needs one restart to put it to work. This takes a few seconds, and this page updates by itself when the station is back.</p><div class="ob-mt-16"><button type="button" class="bnb-btn primary" data-restart-now>Restart the station now</button> <span class="bnb-meta" data-restart-status role="status"></span></div>"#
    } else {
        r#"<p class="ob-p">What you chose is saved and the running station already has it.</p>"#
    };
    let lookup = |name: &str| -> Option<String> {
        Some(match name {
            "eyebrow" => eyebrow.to_owned(),
            "heading" => heading.to_owned(),
            "restart_block" => restart_block.to_owned(),
            "model_panel" => model_panel.to_owned(),
            "microphone" => microphone.to_owned(),
            "setup_script" => crate::routes::admin::classifier::CONTROLS_SCRIPT
                .as_str()
                .to_owned(),
            "version" => env!("CARGO_PKG_VERSION").to_owned(),
            _ => return None,
        })
    };
    fill(ONBOARDING_DONE_HTML.as_str(), &lookup)
}

/// What the capture supervisor says about the station's sources right now.
///
/// The Microphone step listed what is *configured* and stopped there, which
/// answers the wrong question. Being a row in `audio_sources` says nothing
/// about audio flowing: a source whose ALSA card index moved on reboot, or
/// whose `arecord` has died, is configured and silent — and this is the step
/// where an operator is deciding whether their station will hear anything.
/// `today.rs` records the identical bug being fixed for the first-run
/// checklist one screen over.
///
/// Reported as one honest sentence rather than a chip per card, because the
/// supervisor keys its sources by *gauge* label: RTSP streams keep their
/// stream id, but every local microphone collapses to `"local"`, so a station
/// with two local mics has no per-card mapping to render. `None` when no
/// supervisor is publishing (`--web-only`, or the daemon not started), in
/// which case the step says nothing rather than guessing.
fn capture_verdict(state: &AppState) -> Option<String> {
    let sources = state
        .capture_status()
        .map(|h| birdnet_core::audio::capture::read_capture_status(&h).sources)?;
    if sources.is_empty() {
        return None;
    }
    let total = sources.len();
    let faulted = sources.iter().filter(|s| s.state.is_fault()).count();
    Some(if faulted == 0 {
        format!(
            r#"<p class="bnb-meta ob-mt-16" role="status"><span class="bnb-pill moss"><span class="bnb-dot live"></span> reporting</span> All {total} {noun} sending audio right now.</p>"#,
            noun = if total == 1 {
                "source is"
            } else {
                "sources are"
            },
        )
    } else {
        format!(
            r#"<p class="bnb-meta ob-mt-16 ob-password-error" role="alert">{faulted} of {total} sources {verb} sending audio right now. Setup will still finish, but nothing will be detected from {those} until it recovers — <a href="/station">Station health</a> says which and why.</p>"#,
            verb = if faulted == 1 { "is not" } else { "are not" },
            those = if faulted == 1 { "it" } else { "them" },
        )
    })
}

/// The microphone step: what is configured, what the kernel lists that could be
/// added, and the two ways to name one that it does not.
///
/// This step used to be a mock-up: a hard-coded "UMC202HD · USB audio · card 1
/// · 48 kHz" card marked *recommended* and pre-selected, plus a "Built-in
/// microphone · card 0" and two more cards offering RTSP and folder-watching.
/// None of it was real — no station was consulted, nothing was clickable to any
/// effect, and a first-run operator was shown hardware they do not own,
/// described as already detected. Every card here is either a row in
/// `audio_sources` or a device the kernel listed a moment ago.
///
/// `notice` is a refusal from the save, shown first.
fn render_mic_body(
    sources: &[AudioSource],
    devices: &[CaptureDevice],
    verdict: Option<&str>,
    notice: Option<&str>,
) -> String {
    let mut out = String::new();
    if let Some(n) = notice {
        let _ = write!(
            out,
            r#"<p class="ob-password-error" role="alert">{}</p>"#,
            escape_html(n)
        );
    }

    // What is already configured: shown as it is, never offered as a choice.
    if !sources.is_empty() {
        out.push_str(r#"<div class="ob-cards" id="mic-cards">"#);
        for s in sources {
            let icon = if matches!(s.kind, SourceKind::Rtsp) {
                "📡"
            } else {
                "🎤"
            };
            // Unlabelled sources take the device id as their heading, so repeating
            // it on the detail line would print `plughw:CARD=PRO,DEV=0` twice.
            let label = friendly_label(s);
            let device = shown_device(s);
            let detail = match label {
                Some(_) => format!("{device} · {}", detail_for(s)),
                None => detail_for(s),
            };
            let _ = write!(
                out,
                concat!(
                    r#"<div class="ob-card ob-card--static"><span class="ic" aria-hidden="true">{icon}</span><div class="ob-grow">"#,
                    r#"<div class="t">{name} <span class="bnb-pill ob-ml-6">{kind}</span></div>"#,
                    r#"<div class="s mono">{detail}</div></div></div>"#,
                ),
                icon = icon,
                name = escape_html(label.unwrap_or(&device)),
                kind = escape_html(kind_label(s.kind)),
                detail = escape_html(&detail),
            );
        }
        out.push_str("</div>");
        if let Some(v) = verdict {
            out.push_str(v);
        }
    }

    // The choice. With sources configured, "keep" leads and is the default;
    // without, the first device the kernel listed is, because choosing nothing
    // by default would finish a station that hears nothing.
    let free: Vec<&CaptureDevice> = devices
        .iter()
        .filter(|d| !is_configured(d, sources))
        .collect();
    let first_is_default = sources.is_empty() && !free.is_empty();
    out.push_str(
        r#"<fieldset class="mic-list" id="mic-choice"><legend class="sr-only">Microphone</legend>"#,
    );
    if sources.is_empty() {
        out.push_str(
            r#"<div class="ob-card ob-card--static"><span class="ic" aria-hidden="true">🔇</span><div class="ob-grow"><div class="t">No audio source configured</div><div class="s">"#,
        );
        out.push_str(if free.is_empty() {
            "No microphone was found on this station. Plug a USB microphone in and press Look again, or use a network camera below. Nothing will be detected until one is added."
        } else {
            "Nothing will be detected until a microphone or network stream is chosen. These were found on this station:"
        });
        out.push_str("</div></div></div>");
    } else {
        out.push_str(&radio_card(
            "keep",
            "keep",
            "✓",
            "Keep it as it is",
            "The microphone above stays; nothing is added.",
            true,
            None,
        ));
    }
    for (i, d) in free.iter().enumerate() {
        let spec = d.spec();
        out.push_str(&radio_card(
            &format!("alsa:{spec}"),
            &format!("alsa{i}"),
            "🎤",
            &d.name,
            &format!(
                "{} · found on this station · {spec}",
                if d.usb {
                    "USB microphone"
                } else {
                    "Sound card"
                }
            ),
            first_is_default && i == 0,
            Some(&spec),
        ));
    }
    out.push_str(&radio_card(
        "rtsp",
        "rtsp",
        "📡",
        "A network camera or stream",
        "An RTSP, HTTP, HLS, RTMP, UDP or SRT address. Needs the ffmpeg program on the station.",
        false,
        None,
    ));
    out.push_str(
        r#"<div class="mic-manual" id="mic-rtsp-field" hidden><label for="mic-rtsp-url">Stream address</label><input id="mic-rtsp-url" name="mic_rtsp_url" type="text" inputmode="url" autocomplete="off" spellcheck="false" placeholder="rtsp://camera.local:554/stream"><p class="bnb-meta">It starts with <code>rtsp://</code>, <code>http://</code>, <code>https://</code>, <code>rtmp://</code>, <code>udp://</code> or <code>srt://</code>. A camera's password goes in the address, as <code>rtsp://user:password@camera.local/stream</code>.</p></div>"#,
    );
    out.push_str(&radio_card(
        "manual",
        "manual",
        "⌨",
        "A microphone by its device name",
        "For a sound card the list above does not show, written the way ALSA names it, like plughw:CARD=Device,DEV=0.",
        false,
        None,
    ));
    out.push_str(
        r#"<div class="mic-manual" id="mic-manual-field" hidden><label for="mic-manual-dev">Device name</label><input id="mic-manual-dev" name="mic_manual_device" type="text" autocomplete="off" spellcheck="false" placeholder="plughw:CARD=Device,DEV=0"></div>"#,
    );
    if sources.is_empty() {
        out.push_str(&radio_card(
            "none",
            "none",
            "⏭",
            "Skip for now",
            "Finish setup without a microphone and add one later in Settings → Capture.",
            free.is_empty(),
            None,
        ));
    }
    out.push_str("</fieldset>");
    out.push_str(
        r#"<p class="bnb-meta mic-or"><a href="/onboarding">Look again</a> after plugging a microphone in. More options (gain, filters, several microphones) are in <a href="/station/capture">Settings → Capture</a> once setup is done.</p>"#,
    );
    out
}

/// One choice in the microphone step: a real radio button wrapped in a card.
///
/// `test` adds a "Test" button that records two seconds from that device and
/// reports how loud it was.
fn radio_card(
    value: &str,
    id: &str,
    icon: &str,
    title: &str,
    sub: &str,
    checked: bool,
    test: Option<&str>,
) -> String {
    let test_button = test.map_or_else(String::new, |spec| {
        format!(
            r#"<button type="button" class="bnb-btn ghost" data-mic-test="{spec}">Test</button><span class="mic-level" data-mic-level role="status"></span>"#,
            spec = escape_html(spec),
        )
    });
    format!(
        r#"<label class="ob-card{sel}" data-mic-card><input class="ob-pick" type="radio" name="mic_choice" value="{value}" aria-labelledby="mic-t-{id}" aria-describedby="mic-s-{id}"{chk}><span class="ic" aria-hidden="true">{icon}</span><div class="ob-grow"><div class="t" id="mic-t-{id}">{title}</div><div class="s mono-ish" id="mic-s-{id}">{sub}</div></div>{test_button}</label>"#,
        sel = if checked { " sel" } else { "" },
        chk = if checked { " checked" } else { "" },
        value = escape_html(value),
        title = escape_html(title),
        sub = escape_html(sub),
    )
}

/// Whether a sound card the kernel listed is one the station already uses,
/// however the source names it (`plughw:1,0` by position or
/// `plughw:CARD=Device,DEV=0` by name).
fn is_configured(device: &CaptureDevice, sources: &[AudioSource]) -> bool {
    use birdnet_core::audio::capture::alsa::{CardRef, parse_card_ref};
    sources
        .iter()
        .filter(|s| matches!(s.kind, SourceKind::UsbAlsa))
        .any(|s| match parse_card_ref(&s.device_id) {
            Some(CardRef::Index(i)) => i.parse::<u32>().is_ok_and(|i| i == device.card),
            Some(CardRef::Id(id)) => id == device.id,
            None => false,
        })
}

/// The sentence for a refused microphone, from the code the save redirected with.
fn mic_notice(error: &str) -> Option<&'static str> {
    match error {
        "mic-url" => Some(
            "That stream address was not accepted. It has to start with rtsp://, http://, https://, rtmp://, udp:// or srt://. Nothing was saved.",
        ),
        "mic-device" => Some(
            "That device name was not accepted. A sound card is written like plughw:CARD=Device,DEV=0 or plughw:2,0. Nothing was saved.",
        ),
        _ => None,
    }
}

/// The operator's own name for a source, when they gave it one.
///
/// A label of `""` or `"   "` is the admin form's "no label" state, not a name,
/// so it is treated as absent rather than rendered as a blank heading.
/// The source's address as the wizard may print it.
///
/// The wizard adds an RTSP stream with no label, so its address is what names
/// the card and the summary — and an address is `rtsp://user:password@camera/…`.
/// `redact_value`, the helper the settings page and the settings API use, keeps
/// the host and path (what makes a camera recognisable) and drops the password.
fn shown_device(s: &AudioSource) -> String {
    birdnet_core::config::redact::redact_value(&s.device_id)
}

fn friendly_label(s: &AudioSource) -> Option<&str> {
    s.label.as_deref().map(str::trim).filter(|l| !l.is_empty())
}

/// One-line description of the capture setup for the final step's summary.
///
/// Replaced a hard-coded "UMC202HD · USB · 48 kHz" that was shown to every
/// station regardless of what it actually captures with.
fn mic_summary(sources: &[AudioSource]) -> String {
    match sources {
        [] => "None configured — no birds will be detected".to_string(),
        [one] => format!(
            "{} · {}",
            friendly_label(one).map_or_else(|| shown_device(one), str::to_owned),
            detail_for(one)
        ),
        many => format!("{} sources configured", many.len()),
    }
}

/// Fields the first-run wizard submits. All optional: clicking straight through
/// still marks onboarding complete (so the first-boot redirect stops firing)
/// without writing empty settings.
#[derive(Default, serde::Deserialize)]
struct OnboardingForm {
    #[serde(default)]
    latitude: String,
    #[serde(default)]
    longitude: String,
    #[serde(default)]
    timezone: String,
    #[serde(default)]
    notification_mode: String,
    #[serde(default)]
    confidence_threshold: String,
    /// What the station should call itself (`station_name`).
    #[serde(default)]
    station_name: String,
    /// The language bird names are shown in (`database_lang`).
    #[serde(default)]
    database_lang: String,
    /// The microphone step's choice: `keep`, `none`, `rtsp`, `manual` or
    /// `alsa:<device>`. Absent (a script-less submit of an older form) is `keep`.
    #[serde(default)]
    mic_choice: String,
    /// The stream address, when `mic_choice` is `rtsp`.
    #[serde(default)]
    mic_rtsp_url: String,
    /// The device name, when `mic_choice` is `manual`.
    #[serde(default)]
    mic_manual_device: String,
    /// The admin password, on a station that has none (DD-14).
    #[serde(default)]
    password: String,
    #[serde(default)]
    password_confirm: String,
}

/// Whether `raw` is a confidence threshold the daemon will accept.
///
/// The wizard's cards can only emit `0.4`/`0.6`/`0.7`/`0.9`, so this exists
/// for the hand-crafted POST: `birdnet_core::config::validate` treats a
/// `CONFIDENCE` outside 0–1 as an *error*. That check only ever reads the
/// config **file**; a value saved here is laid over it afterwards and never
/// validated again, so an unvalidated write would leave a station that runs and
/// records nothing (`75` is above every score the model can give).
///
/// Never looser than the validator, and stricter in one way: a comma decimal
/// (`0,7`) is refused here, where the validator reads it.
/// `the_wizard_never_accepts_what_the_validator_refuses` holds both.
fn valid_confidence(raw: &str) -> bool {
    raw.parse::<f64>().is_ok_and(|v| (0.0..=1.0).contains(&v))
}

/// Whether `raw` is a notification trigger the runtime understands.
///
/// `TriggerMode::parse` maps anything it does not recognise to
/// `EachDetection` — an alert on every single detection. So an unvalidated
/// write here would turn a typo, or a stale value from an older wizard, into
/// the chattiest possible setting: the exact opposite of what an operator
/// choosing a quieter mode asked for. Kept in step with
/// `birdnet_integrations::notification::TriggerMode::parse`.
fn valid_trigger(raw: &str) -> bool {
    matches!(raw, "each" | "new-species" | "new-species-daily")
}

/// A station name the wizard stores: trimmed, at most 80 characters, no control
/// characters (it goes into page titles, exports and BirdWeather uploads).
fn valid_station_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    (!name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control))
        .then(|| name.to_owned())
}

/// A language code the station has names for. `en` is stored as `en`, so a
/// station that picked English says so and is not read as "never chose".
fn valid_language(raw: &str) -> Option<&'static str> {
    birdnet_core::i18n::find_language(raw.trim()).map(|l| l.code)
}

/// What the microphone step asked for, checked.
#[derive(Debug, PartialEq, Eq)]
enum MicRequest {
    /// Add nothing.
    Nothing,
    /// Add a sound card by device name.
    Alsa {
        device: String,
        label: Option<String>,
    },
    /// Add a network stream.
    Stream { url: String },
    /// The choice could not be used; the code is the `?error=` to redirect with.
    Refused(&'static str),
}

/// Whether `device` is something a person could mean by a sound card: the shapes
/// `alsa::parse_card_ref` and the Capture tab accept, in a bounded alphabet.
fn plausible_device(device: &str) -> bool {
    !device.is_empty()
        && device.len() <= 64
        && device.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b':' | b'=' | b',' | b'_' | b'-' | b'.')
        })
        && birdnet_core::audio::capture::alsa::parse_card_ref(device).is_some()
}

/// Turn the submitted choice into a request, or say what was wrong with it.
///
/// A device typed or picked is held to the shapes a card is named by; a stream
/// has to name a protocol capture can read (the Capture tab's own check). An
/// unknown choice is `Nothing`, not a guess: the safe reading of a form from an
/// older page is "leave the microphones alone".
fn mic_request(form: &OnboardingForm, known: &[CaptureDevice]) -> MicRequest {
    let choice = form.mic_choice.trim();
    if let Some(device) = choice.strip_prefix("alsa:") {
        let device = device.trim();
        // Picked from the list: it must be one the kernel lists, so a hand-made
        // POST cannot name an arbitrary device string through this path.
        return known.iter().find(|d| d.spec() == device).map_or(
            MicRequest::Refused("mic-device"),
            |d| MicRequest::Alsa {
                device: d.spec(),
                label: Some(d.name.clone()),
            },
        );
    }
    match choice {
        "rtsp" => {
            let url = form.mic_rtsp_url.trim();
            if birdnet_core::audio::capture::StreamProtocol::of(url).is_some() && url.len() <= 2048
            {
                MicRequest::Stream {
                    url: url.to_owned(),
                }
            } else {
                MicRequest::Refused("mic-url")
            }
        }
        "manual" => {
            let device = form.mic_manual_device.trim();
            if plausible_device(device) {
                MicRequest::Alsa {
                    device: device.to_owned(),
                    label: None,
                }
            } else {
                MicRequest::Refused("mic-device")
            }
        }
        _ => MicRequest::Nothing,
    }
}

/// The largest latitude, in degrees, the wizard stores. The same number
/// `birdnet_core::config::validate` holds `LATITUDE` to (`check_coords`);
/// `the_wizard_never_accepts_what_the_validator_refuses` compares the two.
const LATITUDE_LIMIT: f64 = 90.0;

/// The largest longitude, in degrees, the wizard stores. See [`LATITUDE_LIMIT`].
const LONGITUDE_LIMIT: f64 = 180.0;

/// A coordinate the station can use: parses after the decimal-separator
/// normalisation the settings form applies, and lies inside the range the
/// config validator accepts. Returns the canonical string to store.
///
/// The wizard used to guard these with `is_empty()` alone, so
/// `latitude=999&longitude=abc` was persisted, the settings overlay dropped
/// the pair silently at the next start, and the doctor said "no
/// latitude/longitude set" while the wizard prefilled `999`.
fn valid_coordinate(raw: &str, limit: f64) -> Option<String> {
    let value = birdnet_core::config::locale::normalize_decimal(raw.trim());
    let parsed: f64 = value.parse().ok()?;
    (parsed.is_finite() && parsed.abs() <= limit).then_some(value)
}

/// The zoneinfo tree `timedatectl` and the tz database read; when it is
/// present, a zone the wizard stores must exist in it.
/// Where the zoneinfo tree lives.
///
/// [`plausible_timezone`] checks a name against it, and the doctor reads the
/// zoneinfo version from it.
pub const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";

/// Whether `raw` names a time zone this host knows.
///
/// Shape first — an IANA name is one to three `/`-separated segments of
/// letters, digits, `_`, `+` and `-`, the first starting with a letter — and
/// then, on a host that has a zoneinfo tree (every Debian image the installer
/// targets, and the container image), the file must exist there. A host with
/// no tree falls back to the shape check rather than refusing every zone,
/// because the auto-detect step writes this value and a station with no
/// tzdata still needs it stored. `Mars/Olympus` used to be accepted, and the
/// doctor then told the operator to run `timedatectl set-timezone
/// Mars/Olympus`.
pub fn plausible_timezone(raw: &str) -> bool {
    let name = raw.trim();
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let segments: Vec<&str> = name.split('/').collect();
    let shape_ok = (1..=3).contains(&segments.len())
        && segments.iter().all(|seg| {
            !seg.is_empty()
                && seg != &"."
                && seg != &".."
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
        })
        && segments[0]
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic());
    if !shape_ok {
        return false;
    }
    let tree = std::path::Path::new(ZONEINFO_DIR);
    if tree.is_dir() {
        return tree.join(name).is_file();
    }
    true
}

/// Persist the wizard's choices and mark onboarding complete, then return to the
/// dashboard. Only non-empty values are written; the DB settings overlay applies
/// latitude/longitude and the confidence threshold on the next start.
async fn onboarding_save(
    State(state): State<AppState>,
    client: Option<Extension<ClientIp>>,
    scheme: Option<Extension<crate::request_scheme::RequestScheme>>,
    headers: axum::http::HeaderMap,
    Form(form): Form<OnboardingForm>,
) -> Response {
    state.run_blocking(move |state| {
    // The microphone choice is checked before anything is written, like the
    // password below: a refused answer saves nothing and sends the person back
    // to the step that failed, rather than leaving a station half set up.
    let known_devices = birdnet_core::audio::capture::alsa::list_capture_devices();
    let mic = mic_request(&form, &known_devices);
    if let MicRequest::Refused(code) = mic {
        return Redirect::to(&format!("/onboarding?error={code}")).into_response();
    }
    // The password first (DD-14), and only on a station that has none: with
    // one set, the operator signed in to reach this page and the fields are
    // not rendered. A refused password saves nothing else either — the
    // operator is sent back to the step that failed rather than left with a
    // configured, still-open station and no idea why.
    let mut set_cookie = None;
    if !crate::auth_middleware::admin_password_configured(state) && !form.password.is_empty() {
        if form.password.len() < MIN_PASSWORD_LEN || form.password != form.password_confirm {
            return Redirect::to("/onboarding?error=password").into_response();
        }
        match set_first_admin_password(state, &form.password) {
            Ok(admin_id) => {
                set_cookie = super::super::auth_pages::mint_session_cookie(
                    state,
                    admin_id,
                    client.as_deref(),
                    scheme.as_deref(),
                    &headers,
                );
            }
            // Lost the race: a password was set between the page's check and this
            // write. Nothing of this submission is saved, and the sign-in page is
            // where the person who set it goes next.
            Err(FirstPasswordError::AlreadySet) => {
                tracing::warn!("onboarding: the admin password was set by another submission");
                return Redirect::to("/login").into_response();
            }
            Err(e) => {
                tracing::error!(error = %e, "onboarding: could not set the admin password");
                return Redirect::to("/onboarding?error=password").into_response();
            }
        }
    }
    state.with_db(|conn| {
        // Idempotent safety net; the table is also created by migration 14.
        settings::ensure_settings_table(conn).ok();

        let mut items: Vec<(&str, &str, SettingsCategory)> = Vec::new();
        let lat = form.latitude.trim();
        let lon = form.longitude.trim();
        let tz = form.timezone.trim();
        let mode = form.notification_mode.trim();
        // A location is a pair: both coordinates in range, or nothing. Half a
        // pair used to be written, so two wizard runs — one with the longitude
        // left blank — could leave the station at a point nobody typed. The
        // page script already treats a half pair as "Not set"; the store now
        // agrees with it. A rejected value is logged rather than dropped in
        // silence, so the log says why the doctor still reports no location.
        let coordinates = match (
            valid_coordinate(lat, LATITUDE_LIMIT),
            valid_coordinate(lon, LONGITUDE_LIMIT),
        ) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            _ if lat.is_empty() && lon.is_empty() => None,
            _ => {
                tracing::warn!(
                    latitude = lat,
                    longitude = lon,
                    "onboarding: location not stored; both coordinates must be numbers within ±90 / ±180"
                );
                None
            }
        };
        if let Some((lat, lon)) = coordinates.as_ref() {
            items.push(("latitude", lat, SettingsCategory::Location));
            items.push(("longitude", lon, SettingsCategory::Location));
        }
        // The name and the language of bird names: each only when given, so a
        // blank box clears nothing and an unknown language is not stored (it
        // would be read as English and look like a choice).
        let name = valid_station_name(&form.station_name);
        if let Some(name) = name.as_deref() {
            items.push(("station_name", name, SettingsCategory::Location));
        }
        let lang = valid_language(&form.database_lang);
        if let Some(code) = lang {
            items.push(("database_lang", code, SettingsCategory::System));
        } else if !form.database_lang.trim().is_empty() {
            tracing::warn!(
                language = form.database_lang.trim(),
                "onboarding: language not stored; the station has no names for it"
            );
        }
        if plausible_timezone(tz) {
            items.push(("timezone", tz, SettingsCategory::Location));
        } else if !tz.is_empty() {
            tracing::warn!(
                timezone = tz,
                "onboarding: time zone not stored; it is not a zone this host knows"
            );
        }
        // `notify_trigger` — the key the notification filter actually reads
        // (bridged onto `APPRISE_TRIGGER`). The wizard used to write
        // `notification_mode`, which nothing anywhere consumed: the operator
        // picked "Quiet" or "Everything", it was persisted, and it governed
        // nothing. Only the three values `TriggerMode::parse` understands are
        // accepted, because an unknown value silently parses as "every
        // detection" — the chattiest mode, and the opposite of what someone
        // choosing a quieter one intends.
        if valid_trigger(mode) {
            items.push(("notify_trigger", mode, SettingsCategory::Notifications));
        }
        // Only persisted when it parses inside 0–1. The wizard's own cards can
        // only produce 0.4/0.6/0.7/0.9, but the field is a plain form value:
        // a hand-crafted POST must not be able to seed a threshold the
        // detection pipeline can never meet (see `valid_confidence`) — that
        // would turn the setup wizard into a way to silence the station.
        let conf = form.confidence_threshold.trim();
        if valid_confidence(conf) {
            items.push(("confidence_threshold", conf, SettingsCategory::Detection));
        }
        if !items.is_empty()
            && let Err(e) = settings::set_many(conn, &items)
        {
            // Not surfaced beyond the log on purpose: the flag below is still
            // set so the box is not trapped in the wizard, and every value is
            // editable on the settings page. Silent it was not meant to be.
            tracing::error!(error = %e, "the setup wizard could not save its settings");
        }
        // The microphone, as the same row the Capture tab makes. A source the
        // station already has (the same device or stream) is not added twice.
        match &mic {
            MicRequest::Alsa { device, label } => {
                add_source(conn, SourceKind::UsbAlsa, device, label.as_deref());
            }
            MicRequest::Stream { url } => {
                add_source(conn, SourceKind::Rtsp, url, None);
            }
            MicRequest::Nothing | MicRequest::Refused(_) => {}
        }
        // Set the completion flag last so a fresh box stops being redirected here
        // even if a settings write above failed.
        let _ = settings::set(
            conn,
            "onboarding_complete",
            "true",
            SettingsCategory::System,
        );
    });
    // Not the dashboard: what the answers need is a restart, and the next page
    // says what is true now and offers it.
    let mut resp = Redirect::to("/onboarding/done").into_response();
    if let Some(cookie) = set_cookie
        && let Ok(value) = axum::http::HeaderValue::from_str(&cookie)
    {
        resp.headers_mut()
            .append(axum::http::header::SET_COOKIE, value);
    }
    resp
})
.await
}

/// Add an audio source unless the station already has that device or stream.
///
/// Failures are logged and the wizard carries on: the completion flag below it
/// must still be set, and the Capture tab can add the source by hand.
///
/// Runs inside the caller's database lock, so the audit row is written on the
/// connection it was handed: `audit_user_id` takes the lock itself and would
/// wait for it forever.
fn add_source(conn: &rusqlite::Connection, kind: SourceKind, device: &str, label: Option<&str>) {
    let existing = match AudioSourceStore::list(conn) {
        Ok(list) => list,
        Err(e) => {
            tracing::error!(error = %e, "onboarding: could not read the audio sources; the microphone was not added");
            return;
        }
    };
    if existing
        .iter()
        .any(|s| s.kind.as_str() == kind.as_str() && s.device_id == device)
    {
        return;
    }
    let mut new =
        NewAudioSource::defaults(crate::routes::admin::audio::synth_id(kind), kind, device);
    new.label = label.map(str::to_owned);
    match AudioSourceStore::insert(conn, &new) {
        Ok(row) => {
            if let Err(e) = conn.record(
                None,
                "audio.source.create",
                Some(&row.device_id),
                Some(&format!("kind={}; via=setup", row.kind)),
            ) {
                tracing::warn!(error = %e, "audit log write failed");
            }
            tracing::info!(kind = %row.kind, "setup wizard added an audio source");
        }
        Err(e) => {
            tracing::error!(error = %e, "onboarding: the audio source could not be added");
        }
    }
}

/// `POST /onboarding/mic-test` — record two seconds from a listed sound card and
/// say how loud it was.
///
/// Only a card the kernel lists *now* can be tested, so the endpoint cannot be
/// pointed at an arbitrary device string. Answers `200` with `ok: false` and the
/// reason for every failure, because the page script shows the message and htmx
/// is not involved.
async fn mic_test(Form(form): Form<MicTestForm>) -> Response {
    let device = form.device.trim().to_owned();
    let outcome = tokio::task::spawn_blocking(move || {
        let listed = birdnet_core::audio::capture::alsa::list_capture_devices()
            .iter()
            .any(|d| d.spec() == device);
        if !listed {
            return Err("That sound card is not listed any more. Press Look again.".to_owned());
        }
        birdnet_core::audio::capture::probe::measure_level(&device).map_err(|e| e.to_string())
    })
    .await;
    let body = match outcome {
        Ok(Ok(level)) => serde_json::json!({
            "ok": true,
            "rms_dbfs": (level.rms_dbfs * 10.0).round() / 10.0,
            "peak_dbfs": (level.peak_dbfs * 10.0).round() / 10.0,
            "message": level_message(level.rms_dbfs),
        }),
        Ok(Err(message)) => serde_json::json!({ "ok": false, "message": message }),
        Err(e) => {
            tracing::error!(error = %e, "onboarding: the microphone test task failed");
            serde_json::json!({ "ok": false, "message": "The test could not run." })
        }
    };
    ([(header::CACHE_CONTROL, "no-store")], axum::Json(body)).into_response()
}

/// The form the microphone test posts.
#[derive(serde::Deserialize)]
struct MicTestForm {
    device: String,
}

/// What a level means, in a sentence. The cut-offs are judgement, not
/// calibration: they separate "nothing arrived" from "something did", and say
/// nothing about whether the microphone is good.
fn level_message(rms_dbfs: f64) -> &'static str {
    if rms_dbfs <= -80.0 {
        "Silent. Check the microphone is plugged in and not muted."
    } else if rms_dbfs <= -60.0 {
        "Very quiet. Normal in a quiet room; it is picking something up."
    } else if rms_dbfs <= -30.0 {
        "Picking up sound."
    } else {
        "Loud: very close to the microphone, or the gain is high."
    }
}

/// `GET /onboarding/done` — what is true now, and the button that restarts the
/// station so it starts listening.
async fn onboarding_done(State(state): State<AppState>, req: Request) -> Response {
    state
        .run_blocking(move |state| {
            if let Some(user) = req
                .extensions()
                .get::<crate::auth_middleware::RequestUser>()
                && !user.is_admin()
            {
                return not_admin_page();
            }
            let snapshot = state
                .classifier_setup()
                .snapshot(state.detection_daemon_running());
            let sources = state.with_read_db(AudioSourceStore::list);
            let body = render_done_page(
                &crate::routes::admin::classifier::render_panel(&snapshot),
                &done_microphone(&sources.as_deref().map_err(ToString::to_string)),
                pending_restart(state),
            );
            (
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                Html(body),
            )
                .into_response()
        })
        .await
}

/// Whether saved settings or sources are newer than the running station, so the
/// done page should offer the restart.
///
/// A read that fails answers `true`: offering a restart that was not needed
/// costs a few seconds, and hiding one that was leaves a station that was just
/// set up and is not listening.
fn pending_restart(state: &AppState) -> bool {
    use crate::routes::admin::settings::applies::{Applies, applies_for};
    let settings = match crate::pending_restart::changed_settings(state) {
        Ok(keys) => keys
            .iter()
            .any(|k| applies_for(k).is_some_and(Applies::needs_restart)),
        Err(e) => {
            tracing::warn!(error = %e, "onboarding: could not tell which settings await a restart");
            return true;
        }
    };
    settings
        || crate::pending_restart::sources_changed(state).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "onboarding: could not tell whether the sources changed");
            true
        })
}

/// The microphone line on the done page: what the station will listen with, or
/// that it could not be read.
fn done_microphone(sources: &Result<&[AudioSource], String>) -> String {
    match sources {
        Err(_) => r#"<p class="bnb-meta ob-password-error" role="alert">The station could not read its audio sources. <a href="/admin/doctor">Diagnostics</a> says why.</p>"#.to_owned(),
        Ok([]) => r#"<p class="bnb-meta">No microphone yet, so nothing will be detected. <a href="/station/capture">Add one in Settings → Capture</a>.</p>"#.to_owned(),
        Ok(rows) => format!(
            r#"<p class="bnb-meta">Set to listen with: {}. Whether it is hearing anything is shown on <a href="/station">Station health</a>.</p>"#,
            escape_html(&mic_summary(rows))
        ),
    }
}

/// Hash `password` onto the seed admin row and record it. Returns the
/// admin's id, for the session the wizard mints next.
fn set_first_admin_password(state: &AppState, password: &str) -> Result<i64, FirstPasswordError> {
    let hash =
        accounts::hash_password(password).map_err(|e| FirstPasswordError::Failed(e.to_string()))?;
    let admin_id = state
        .with_db(|conn| {
            let admin = conn.find_user_by_name("admin")?;
            // Conditional on the row still having no real hash, in one statement:
            // two first-run submissions that both saw "no password" would
            // otherwise both write, the last one silently replacing the first
            // person's choice while that person's session was already minted.
            if !conn.set_password_if_unset(admin.id, &hash)? {
                return Ok(None);
            }
            // Sessions minted during the open window (any `POST /login`
            // succeeds while there is no password) end here; the wizard mints
            // the owner's own session after this returns.
            accounts::SessionStore::revoke_others(conn, admin.id, "")?;
            Ok::<Option<i64>, accounts::AccountsError>(Some(admin.id))
        })
        .map_err(|e| FirstPasswordError::Failed(e.to_string()))?
        .ok_or(FirstPasswordError::AlreadySet)?;
    // No actor: there is no account yet that could be one. The target is the
    // row that just acquired a password.
    crate::audit::audit_user_id(
        state,
        None,
        "account.password.set",
        Some(&format!("user:{admin_id}")),
        None,
    );
    tracing::info!("admin password set from the setup wizard; /admin is now gated");
    Ok(admin_id)
}

/// Why [`set_first_admin_password`] did not set a password.
#[derive(Debug)]
enum FirstPasswordError {
    /// Somebody else's submission got there first: the account has a password
    /// now, and it is not this one.
    AlreadySet,
    /// The hash or the write failed.
    Failed(String),
}

impl std::fmt::Display for FirstPasswordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadySet => f.write_str("the admin account already has a password"),
            Self::Failed(e) => f.write_str(e),
        }
    }
}

static ONBOARDING_HTML: crate::seal::Sealed = crate::seal::Sealed::new(
    r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BirdNet-Behavior · Set up your station</title>
<link rel="stylesheet" href="/static/css/app.css?v={{version}}">
<script src="/static/theme-guard.js?v={{version}}"></script>
<style>
  body { margin:0; background:var(--bg); color:var(--fg); min-height:100vh; }
  .ob-root { max-width:980px; margin:0 auto; min-height:100vh; display:flex; flex-direction:column; padding:0 24px; }
  .ob-stepper { display:flex; align-items:center; gap:8px; padding:22px 0 8px; margin:0; list-style:none; position:sticky; top:0; background:color-mix(in oklab, var(--bg) 92%, transparent); backdrop-filter:saturate(1.4) blur(10px); z-index:5; }
  .ob-pip { display:flex; align-items:center; gap:8px; }
  .ob-pip .dot { width:22px; height:22px; border-radius:999px; border:0.5px solid var(--border-2); display:flex; align-items:center; justify-content:center; font-size:11px; font-family:var(--font-mono); color:var(--fg-3); background:var(--surface); }
  .ob-pip .nm { font-size:11px; letter-spacing:0.06em; text-transform:uppercase; color:var(--fg-3); }
  .ob-stepper .bar { flex:none; width:28px; height:1.5px; background:var(--hairline); }
  .ob-pip.done .dot, .ob-pip.active .dot { background:var(--moss); color:var(--bg); border-color:transparent; }
  .ob-pip.active .nm { color:var(--fg); font-weight:500; }
  .ob-stage { flex:1; display:flex; align-items:center; padding:24px 0; }
  .ob-step { display:none; width:100%; animation:ob-fade .2s ease; }
  .ob-step.active { display:block; }
  @keyframes ob-fade { from { opacity:0; transform:translateY(6px); } to { opacity:1; transform:none; } }
  .ob-two { display:grid; grid-template-columns:1fr 1fr; gap:32px; align-items:center; }
  .ob-eyebrow { font-size:10.5px; letter-spacing:0.10em; text-transform:uppercase; color:var(--fg-3); font-weight:500; }
  .ob-h { font-family:var(--font-display); font-size:46px; line-height:1.08; letter-spacing:-0.02em; margin:8px 0 12px; }
  .ob-h em { font-style:italic; color:var(--moss-ink); }
  .ob-p { color:var(--fg-2); font-size:15px; max-width:42ch; }
  .ob-bullets { list-style:none; padding:0; margin:18px 0 0; display:flex; flex-direction:column; gap:10px; }
  .ob-bullets li { display:flex; gap:10px; align-items:center; font-size:14px; }
  .ob-bullets .tick { width:18px; height:18px; border-radius:999px; background:var(--moss-soft); color:var(--moss-ink); display:inline-flex; align-items:center; justify-content:center; font-size:11px; }
  /* One axis: Back, where you are, then everything that moves you on. "Use the
     defaults and finish" used to sit centred under "Step 1 of 6", off the line
     the Continue button was on. */
  .ob-nav { display:flex; flex-wrap:wrap; align-items:center; gap:10px 12px; padding:18px 0 28px; border-top:0.5px solid var(--hairline); }
  .ob-stepcount { flex:1 1 auto; margin:0; }
  /* On step 1 there is nothing to go back to; it hid itself but kept its room, which pushed "Step 1 of 6" in from the left edge. */
  #ob-back[style*="hidden"] { display:none; }
  .ob-skip-note { flex:1 1 100%; margin:-4px 0 0; text-align:right; }
  .ob-field { display:flex; flex-direction:column; gap:6px; margin-bottom:14px; }
  .ob-field label { font-size:12.5px; font-weight:500; }
  .ob-field input { padding:9px 12px; border-radius:var(--r-sm); border:0.5px solid var(--border-2); background:var(--surface); color:var(--fg); font:inherit; }
  .ob-cards { display:grid; gap:12px; }
  .ob-card { position:relative; display:flex; gap:14px; align-items:center; padding:14px; border-radius:var(--r-md); border:0.5px solid var(--border); background:var(--surface); cursor:pointer; transition:border-color .12s, background .12s; }
  .ob-card.sel { border-color:var(--moss); background:var(--moss-soft); }
  /* The real radio behind each preference card (UX-1): kept in the tab order
     and off the screen, never display:none, which would take it out of both. */
  .ob-card .ob-pick { position:absolute; width:1px; height:1px; margin:-1px; padding:0; border:0; overflow:hidden; clip:rect(0 0 0 0); white-space:nowrap; opacity:0; }
  .ob-card:focus-within { outline:2px solid var(--moss); outline-offset:2px; }
  .ob-card .ic { width:34px; height:34px; flex-shrink:0; border-radius:8px; background:var(--surface-2); display:flex; align-items:center; justify-content:center; color:var(--fg-2); }
  .ob-card .t { font-weight:500; font-size:14px; }
  .ob-card .s { font-size:12px; color:var(--fg-3); }
  /* The chosen card is tinted, and --fg-3 on that tint is under 4.5:1 in the dark theme. */
  .ob-card.sel .s { color:var(--fg-2); }
  /* A link inside a sentence is told from its words by more than its colour (WCAG 1.4.1). */
  .bnb-meta a, .ob-p a { text-decoration:underline; text-underline-offset:2px; }
  .vu { display:flex; align-items:flex-end; gap:2px; height:26px; margin-left:auto; }
  .vu i { width:3px; background:var(--moss); border-radius:1px; animation:vu 1.1s ease-in-out infinite; }
  @keyframes vu { 0%,100% { height:20%; } 50% { height:95%; } }
  .chips { display:flex; flex-wrap:wrap; gap:8px; margin-top:12px; }
  .summary-row { display:flex; justify-content:space-between; gap:16px; padding:11px 0; border-top:0.5px solid var(--hairline); font-size:14px; }
  .summary-row:first-child { border-top:0; }
  .summary-row .k { color:var(--fg-3); }
  .calib { display:flex; align-items:flex-end; gap:2px; height:46px; }
  .calib i { flex:1; background:var(--moss); opacity:.5; border-radius:1px; animation:vu 1.6s ease-in-out infinite; }
  @media (prefers-reduced-motion: reduce) {
    .ob-step, .vu i, .calib i, .sonar * { animation:none !important; }
  }
  /* O-25: static inline style= attributes folded into this page's own <style>
     block (faithful, value-preserving). */
  .ob-center { display:flex; align-items:center; justify-content:center; }
  /* Step 1. The task here is the password, so it sits on the right beside the
     promises and the picture is its header: before, the picture floated in the
     middle of a column 270px from either edge while the card it belonged to hung
     off the left column under three bullets. */
  .ob-two--top { align-items:start; }
  .ob-aside { display:flex; flex-direction:column; gap:18px; min-width:0; }
  .ob-aside .sonar { align-self:center; width:168px; height:168px; flex:none; }
  .ob-aside .bnb-card { margin-top:0; }
  .ob-grow { flex:1; }
  .ob-mt-6 { margin-top:6px; }
  .ob-mt-16 { margin-top:16px; }
  .ob-mt-18 { margin-top:18px; }
  .ob-mb-18 { margin-bottom:18px; }
  .ob-ml-4 { margin-left:4px; }
  .ob-ml-6 { margin-left:6px; }
  .ob-latlon { display:grid; grid-template-columns:1fr 1fr; gap:12px; margin-top:16px; }
  .ob-map { background:var(--surface-2); border-radius:var(--r-lg); border:0.5px solid var(--border); }
  .ob-cards.cols2 { grid-template-columns:repeat(2,1fr); }
  .ob-summary { cursor:pointer; }
  .ob-calib-m { margin:14px 0; }
  .ob-hidden-init { visibility:hidden; }
  /* Staggered VU / calibration bar animation delays (were per-<i> inline). */
  .vu i:nth-child(1) { animation-delay:0s; }   .vu i:nth-child(2) { animation-delay:.1s; }
  .vu i:nth-child(3) { animation-delay:.2s; }  .vu i:nth-child(4) { animation-delay:.05s; }
  .vu i:nth-child(5) { animation-delay:.25s; } .vu i:nth-child(6) { animation-delay:.15s; }
  .vu i:nth-child(7) { animation-delay:.3s; }  .vu i:nth-child(8) { animation-delay:.08s; }
  .calib i:nth-child(1) { animation-delay:0s; }    .calib i:nth-child(2) { animation-delay:.1s; }
  .calib i:nth-child(3) { animation-delay:.2s; }   .calib i:nth-child(4) { animation-delay:.3s; }
  .calib i:nth-child(5) { animation-delay:.15s; }  .calib i:nth-child(6) { animation-delay:.25s; }
  .calib i:nth-child(7) { animation-delay:.05s; }  .calib i:nth-child(8) { animation-delay:.35s; }
  .calib i:nth-child(9) { animation-delay:.12s; }  .calib i:nth-child(10) { animation-delay:.22s; }
  .calib i:nth-child(11) { animation-delay:.32s; } .calib i:nth-child(12) { animation-delay:.18s; }
  /* Phones: drop the step labels (numbered dots + connectors stay), tighten
     padding, and stack the two-column step bodies so nothing overflows 390px.
     The lat/lon + notify grids were inline grids caught by the global
     `[style*="grid-template-columns"]` reset; now classes, they carry their own. */
  @media (max-width:520px) {
    .ob-root { padding:0 14px; }
    .ob-pip .nm { display:none; }
    .ob-stepper .bar { width:14px; }
    .ob-two { grid-template-columns:1fr; gap:18px; }
    .ob-latlon { grid-template-columns:1fr; }
    .ob-cards.cols2 { grid-template-columns:1fr; }
    /* A 46px Instrument Serif heading ate a third of a 390px viewport; the
       phone block adjusted five other things and never this one. */
    .ob-h { font-size:32px; }
    /* The nav is three controls in a row; below this width they need to stack
       or the primary action is squeezed to an ellipsis. */
    .ob-stepcount { order:-1; flex:1 1 100%; text-align:center; }
    .ob-skip-btn, #ob-next { flex:1 1 auto; justify-content:center; }
    .ob-skip-note { text-align:left; }
    .ob-aside .sonar { width:120px; height:120px; }
  }
  /* The only server-rendered error the wizard has was styled nowhere — it
     rendered as ordinary body text, visually identical to the paragraph above
     it, so a refused password looked like prose. */
  .ob-password-error {
    color:var(--rare); background:var(--rare-soft);
    border:0.5px solid var(--rare); border-radius:var(--r-sm);
    padding:8px 12px; margin:10px 0 0; font-size:13px;
  }
  /* The device ids and RTSP URLs `detail_for` emits have no spaces, and a flex
     item's default `min-width:auto` will not shrink below its longest word, so
     one such source pushed the page wider than the viewport. */
  .ob-card, .summary-row { min-width:0; }
  .ob-grow, .summary-row > span { min-width:0; overflow-wrap:anywhere; }
  /* The microphone cards are deliberately not selectable; the pointer cursor
     said otherwise. */
  .ob-card.ob-card--static { cursor:default; }
  /* Shown only by the <noscript> block above. */
  .ob-noscript-note, .ob-nosubmit { display:none; }
</style>
</head>
<body>
<!-- Without JavaScript the wizard used to be a dead end that the home page
     trapped you in: `.ob-step { display: none }` hides steps 2-6, the only
     controls were `href="#"` anchors, there was no submit button anywhere, so
     `onboarding_complete` could never be set — and `/` 303s a station that has
     not onboarded straight back here. The <noscript> block below un-hides every
     step so the page degrades into one long form, and the nav carries a real
     `type="submit"` that posts it. -->
<noscript>
  <style>
    .ob-step { display: block !important; border-top: 0.5px solid var(--hairline); padding-top: 22px; margin-top: 22px; }
    .ob-stage > .ob-step:first-child { border-top: 0; padding-top: 0; margin-top: 0; }
    #ob-back, #ob-next, #ob-skip, .ob-stepcount { display: none !important; }
    .ob-noscript-note { display: block !important; }
    .ob-nosubmit { display: inline-flex !important; }
  </style>
</noscript>
<main class="ob-root" id="main-content">
  <form id="ob-form" method="post" action="/onboarding/save">
  <p class="ob-noscript-note bnb-load-error" role="note">JavaScript is off, so every step is shown at once. Fill in what you want and press <strong>Save setup</strong> at the bottom.</p>
  <!-- The pips are a labelled list with a current marker. They used to be six
       bare <div>s: no role, no accessible name, no aria-current, and the
       `@media (max-width:520px)` rule hides `.nm`, so on a phone a screen
       reader and a sighted user both got six unlabelled digits. Each pip now
       carries its own name, which survives that rule. -->
  <ol class="ob-stepper" id="ob-stepper" aria-label="Setup progress">
    <li class="ob-pip" data-pip="1" aria-current="step"><span class="dot" aria-hidden="true">1</span><span class="nm">Welcome</span><span class="sr-only">Step 1 of 7: Welcome</span></li>
    <li class="bar" aria-hidden="true"></li>
    <li class="ob-pip" data-pip="2"><span class="dot" aria-hidden="true">2</span><span class="nm">Model</span><span class="sr-only">Step 2 of 7: Model</span></li>
    <li class="bar" aria-hidden="true"></li>
    <li class="ob-pip" data-pip="3"><span class="dot" aria-hidden="true">3</span><span class="nm">Location</span><span class="sr-only">Step 3 of 7: Location</span></li>
    <li class="bar" aria-hidden="true"></li>
    <li class="ob-pip" data-pip="4"><span class="dot" aria-hidden="true">4</span><span class="nm">Microphone</span><span class="sr-only">Step 4 of 7: Microphone</span></li>
    <li class="bar" aria-hidden="true"></li>
    <li class="ob-pip" data-pip="5"><span class="dot" aria-hidden="true">5</span><span class="nm">Accuracy</span><span class="sr-only">Step 5 of 7: Accuracy</span></li>
    <li class="bar" aria-hidden="true"></li>
    <li class="ob-pip" data-pip="6"><span class="dot" aria-hidden="true">6</span><span class="nm">Alerts</span><span class="sr-only">Step 6 of 7: Alerts</span></li>
    <li class="bar" aria-hidden="true"></li>
    <li class="ob-pip" data-pip="7"><span class="dot" aria-hidden="true">7</span><span class="nm">Done</span><span class="sr-only">Step 7 of 7: Done</span></li>
  </ol>
  <!-- Step changes were silent: content vanished and appeared with nothing
       announced. This says which step you are now on. -->
  <p class="sr-only" id="ob-announce" aria-live="polite" role="status"></p>

  <div class="ob-stage">
    <!-- Step 1 — Welcome -->
    <section class="ob-step active" data-step="1">
      <div class="ob-two ob-two--top">
        <div>
          <div class="ob-eyebrow">Welcome</div>
          <h1 class="ob-h">Let's teach the yard<br>to <em>listen</em>.</h1>
          <p class="ob-p">A few minutes, seven steps, nothing to type in a terminal. This station will start identifying every bird it hears — no accounts, no cloud, all yours.</p>
          <ul class="ob-bullets">
            <li><span class="tick">✓</span> No cloud accounts — everything stays on this device</li>
            <li><span class="tick">✓</span> Set once — sensible defaults the whole way</li>
            <li><span class="tick">✓</span> Always tweakable — change anything later in Settings</li>
          </ul>
        </div>
        <div class="ob-aside">
          <svg class="sonar" width="240" height="240" viewBox="0 0 240 240" aria-hidden="true">
            <g fill="none" stroke="var(--moss)" stroke-width="1">
              <circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s" repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s" repeatCount="indefinite"/></circle>
              <circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s" begin="1.6s" repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s" begin="1.6s" repeatCount="indefinite"/></circle>
              <circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s" begin="3.2s" repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s" begin="3.2s" repeatCount="indefinite"/></circle>
            </g>
            <rect x="96" y="96" width="48" height="48" rx="8" fill="var(--surface)" stroke="var(--border-2)" stroke-width="0.5"/>
            <g stroke="var(--moss)" stroke-width="2" stroke-linecap="round">
              <line x1="110" y1="120" x2="110" y2="120"/><line x1="116" y1="112" x2="116" y2="128"/>
              <line x1="122" y1="106" x2="122" y2="134"/><line x1="128" y1="113" x2="128" y2="127"/>
            </g>
          </svg>
          {{password_step}}
        </div>
      </div>
    </section>

    <!-- Step 2 — The bird-identifying model -->
    <section class="ob-step" data-step="2">
      <div class="ob-eyebrow">What it knows</div>
      <h1 class="ob-h">The bird-identifying model.</h1>
      <p class="ob-p ob-mb-18">This is the part that recognises songs and calls. It is a file the station downloads once, about 84&nbsp;MB, and checks against a fingerprint before using it. You can carry on with the other steps while it downloads.</p>
      <div class="bnb-card pad" data-cls-host>{{model_panel}}</div>
      {{setup_script}}
    </section>

    <!-- Step 3 — Location -->
    <section class="ob-step" data-step="3">
      <div class="ob-two">
        <div>
          <div class="ob-eyebrow">Where</div>
          <h1 class="ob-h">Where is the station?</h1>
          <p class="ob-p">Your coordinates let BirdNET weight species by what's actually likely in your area, and compute sunrise / sunset for the dawn-chorus window.</p>
          <div class="ob-mt-18">
            <button class="bnb-btn" type="button" id="ob-detect"><span aria-hidden="true">⌖</span> Look up my approximate location</button>
            <p class="bnb-meta ob-mt-6">This asks ip-api.com what area this station's public IP address is in. Skip it and type the coordinates yourself if you would rather not.</p>
          </div>
          <div class="ob-latlon">
            <div class="ob-field"><label for="ob-lat">Latitude</label><input id="ob-lat" name="latitude" type="text" placeholder="e.g. 42.3601" inputmode="decimal" value="{{latitude}}"></div>
            <div class="ob-field"><label for="ob-lon">Longitude</label><input id="ob-lon" name="longitude" type="text" placeholder="e.g. -71.0589" inputmode="decimal" value="{{longitude}}"></div>
          </div>
          <input type="hidden" name="timezone" id="ob-tz">
          <p class="bnb-meta ob-mt-6" id="ob-loc-pill" role="status">Look it up, or type your coordinates — you can change this any time in Settings.</p>
          <div class="ob-field ob-mt-16"><label for="ob-name">Station name <span class="bnb-meta">(optional)</span></label><input id="ob-name" name="station_name" type="text" maxlength="80" autocomplete="off" placeholder="e.g. Back garden" value="{{station_name}}"></div>
          <div class="ob-field"><label for="ob-lang">Language for bird names</label><select id="ob-lang" name="database_lang">{{lang_options}}</select></div>
        </div>
        <div class="ob-center">
          <svg width="280" height="220" viewBox="0 0 280 220" aria-hidden="true" class="ob-map">
            <g fill="none" stroke="var(--border-2)" stroke-width="0.75" opacity="0.7">
              <ellipse cx="140" cy="110" rx="40" ry="28"/><ellipse cx="140" cy="110" rx="70" ry="50"/>
              <ellipse cx="140" cy="110" rx="100" ry="72"/><ellipse cx="140" cy="110" rx="128" ry="94"/>
            </g>
            <circle cx="140" cy="110" r="86" fill="none" stroke="var(--moss)" stroke-width="1" stroke-dasharray="4 5"/>
            <circle cx="140" cy="110" r="9" fill="none" stroke="var(--moss)" stroke-width="1.5"/>
            <circle cx="140" cy="110" r="3" fill="var(--moss)"/>
            <text x="140" y="206" text-anchor="middle" font-size="11" class="mono" fill="var(--fg-3)">species likely near you</text>
          </svg>
        </div>
      </div>
    </section>

    <!-- Step 4 — Microphone -->
    <section class="ob-step" data-step="4">
      <div class="ob-eyebrow">How it hears</div>
      <h1 class="ob-h">What it's listening with.</h1>
      <p class="ob-p ob-mb-18">Pick the microphone this station should listen with. A USB microphone plugged into this device is listed by name; a network camera is added by its address.</p>
      {{mic_body}}
    </section>

    <!-- Step 5 — Detection threshold -->
    <section class="ob-step" data-step="5">
      <div class="ob-eyebrow">How sure is sure</div>
      <h1 class="ob-h" id="ob-conf-h">How picky should it be?</h1>
      <p class="ob-p ob-mb-18">Every guess comes with a confidence score. Anything below your threshold is thrown away — so this is the dial between "only the birds it's certain about" and "everything it thinks it heard".</p>
      <div class="ob-cards cols2" role="radiogroup" aria-labelledby="ob-conf-h">
        <label class="ob-card" data-radio="conf" data-value="0.9"><input class="ob-pick" type="radio" name="conf_card" value="0.9" aria-labelledby="ob-conf-strict-t" aria-describedby="ob-conf-strict-s"><div class="ob-grow"><div class="t" id="ob-conf-strict-t">Strict</div><div class="s" id="ob-conf-strict-s">0.90 — only the IDs it is near-certain about. Very few false positives; quiet and distant birds go unlogged.</div></div></label>
        <label class="ob-card" data-radio="conf" data-value="0.7"><input class="ob-pick" type="radio" name="conf_card" value="0.7" aria-labelledby="ob-conf-balanced-t" aria-describedby="ob-conf-balanced-s"><div class="ob-grow"><div class="t" id="ob-conf-balanced-t">Balanced <span class="bnb-pill moss ob-ml-6">recommended</span></div><div class="s" id="ob-conf-balanced-s">0.70 — realistic results without over-filtering. Start here.</div></div></label>
        <label class="ob-card" data-radio="conf" data-value="0.6"><input class="ob-pick" type="radio" name="conf_card" value="0.6" aria-labelledby="ob-conf-sensitive-t" aria-describedby="ob-conf-sensitive-s"><div class="ob-grow"><div class="t" id="ob-conf-sensitive-t">Sensitive</div><div class="s" id="ob-conf-sensitive-s">0.60 — catches quiet and distant birds, at the cost of more misidentifications.</div></div></label>
        <label class="ob-card" data-radio="conf" data-value="0.4"><input class="ob-pick" type="radio" name="conf_card" value="0.4" aria-labelledby="ob-conf-everything-t" aria-describedby="ob-conf-everything-s"><div class="ob-grow"><div class="t" id="ob-conf-everything-t">Everything</div><div class="s" id="ob-conf-everything-s">0.40 — for tuning and curiosity. Expect a lot of noise.</div></div></label>
      </div>
      <input type="hidden" name="confidence_threshold" id="ob-conf" value="{{confidence}}">
      <p class="bnb-meta ob-mt-16">Not permanent — change it any time in <a href="/station/capture">Settings → Capture</a>, and set per-species thresholds under Species.</p>
    </section>

    <!-- Step 6 — Notifications -->
    <section class="ob-step" data-step="6">
      <div class="ob-eyebrow">Who gets told</div>
      <h1 class="ob-h" id="ob-notify-h">When should we ping you?</h1>
      <p class="ob-p ob-mb-18">This sets <em>how often</em> alerts go out. Nothing is sent until you add somewhere to send it — you can do that whenever you like.</p>
      <div class="ob-cards" role="radiogroup" aria-labelledby="ob-notify-h">
        <label class="ob-card" data-radio="notify" data-value="new-species"><input class="ob-pick" type="radio" name="notify_card" value="new-species" aria-labelledby="ob-notify-weekly-t" aria-describedby="ob-notify-weekly-s"><div class="ob-grow"><div class="t" id="ob-notify-weekly-t">Rarely heard lately <span class="bnb-pill moss ob-ml-6">recommended</span></div><div class="s" id="ob-notify-weekly-s">Only birds you have barely heard lately — the interesting ones.</div></div></label>
        <label class="ob-card" data-radio="notify" data-value="new-species-daily"><input class="ob-pick" type="radio" name="notify_card" value="new-species-daily" aria-labelledby="ob-notify-daily-t" aria-describedby="ob-notify-daily-s"><div class="ob-grow"><div class="t" id="ob-notify-daily-t">First of each species, daily</div><div class="s" id="ob-notify-daily-s">One alert per species per day. A good middle ground.</div></div></label>
        <label class="ob-card" data-radio="notify" data-value="each"><input class="ob-pick" type="radio" name="notify_card" value="each" aria-labelledby="ob-notify-each-t" aria-describedby="ob-notify-each-s"><div class="ob-grow"><div class="t" id="ob-notify-each-t">Every detection</div><div class="s" id="ob-notify-each-s">One alert every single time. Chatty — hundreds a day at a busy feeder.</div></div></label>
      </div>
      <input type="hidden" name="notification_mode" id="ob-notify" value="{{notify_trigger}}">
      <p class="bnb-meta ob-mt-16">Add a channel — Telegram, email, MQTT, ntfy, webhooks and more — under <a href="/station/alerts">Settings → Alerts</a>. Until then this setting is simply waiting.</p>
    </section>

    <!-- Step 7 — Done -->
    <section class="ob-step" data-step="7">
      <div class="ob-two">
        <div>
          <div class="ob-eyebrow">All set</div>
          <h1 class="ob-h">You're <em>listening</em>.</h1>
          <p class="ob-p">Here is what you chose. Press <strong>Finish</strong> below to save it — nothing has been written yet. The next screen has one button that restarts the station so it starts listening with all of it.</p>
          <div class="bnb-card pad ob-mt-16">
            <div class="summary-row"><span class="k">Model</span><span id="ob-sum-model">Checking…</span></div>
            <div class="summary-row"><span class="k">Location</span><span id="ob-sum-loc">Not set</span></div>
            <div class="summary-row"><span class="k">Microphone</span><span id="ob-sum-mic" data-static="{{mic_summary}}">{{mic_summary}}</span></div>
            <div class="summary-row"><span class="k">Minimum confidence</span><span id="ob-sum-conf">0.70 · Balanced</span></div>
            <div class="summary-row"><span class="k">Alerts</span><span id="ob-sum-notify">Rare birds only</span></div>
            <div class="summary-row"><span class="k">Dashboard</span><span class="mono" id="ob-sum-url">—</span></div>
          </div>
        </div>
        <div>
          <!-- What used to be here: twelve CSS-animated bars over the words
               "Calibrating noise floor…" and a hard-coded "BirdNET+ V3.0"
               pill. Nothing was calibrating — the bars are a @keyframes — and
               a station running V2.4 was told it was running V3.0. Both
               contradicted this module's own headline claim that every value
               the page shows is real. Replaced with what the station can
               actually promise. -->
          <div class="bnb-card pad">
            <div class="ob-eyebrow">What happens next</div>
            <ul class="ob-bullets ob-mt-6">
              <li><span class="tick">1</span> Press Finish; your answers are written to the station.</li>
              <li><span class="tick">2</span> Press the restart button on the next screen.</li>
              <li><span class="tick">3</span> Detections appear on the dashboard as birds are heard.</li>
            </ul>
          </div>
        </div>
      </div>
    </section>
  </div>

  <div class="ob-nav">
    <button class="bnb-btn ghost ob-hidden-init" id="ob-back" type="button">← Back</button>
    <!-- "Skip for now" was a lie: it called the same finish() as the last step,
         which writes onboarding_complete=true, and nothing anywhere links back
         to /onboarding afterwards. It is now labelled as what it does. It is
         also a real .bnb-btn rather than 11.5px of link text beside the primary
         action, so it gets the coarse-pointer target size.
         Continue was an <a href="#" role="button">: an anchor fires click on
         Enter but not on Space, so a keyboard user pressing Space on something
         announced as a button scrolled the page instead. -->
    <p class="bnb-meta ob-stepcount">Step <span id="ob-cur">1</span> of 7</p>
    <button type="button" class="bnb-btn ghost ob-skip-btn" id="ob-skip">Use the defaults and finish</button>
    <button type="button" class="bnb-btn primary" id="ob-next">Continue →</button>
    <button type="submit" class="bnb-btn primary ob-nosubmit">Save setup</button>
    <!-- What "use the defaults" does about the password, said where the choice
         is made. Shown only on step 1 of a station that has none; the wording
         follows what is typed. It changes nothing the wizard saves. -->
    <p class="bnb-meta ob-skip-note" id="ob-skip-note" role="note" hidden></p>
  </div>
  </form>
</main>

<script>
(function () {
  var step = {{start_step}}, total = 7;
  // Same source of truth as live-detections.js: security.rs stamps
  // data-base-path onto <body> precisely so scripts can build their own URLs.
  var BASE = (document.body && document.body.dataset.basePath) || '';
  var pillEl = document.getElementById('ob-loc-pill');
  var PILL_DEFAULT = pillEl ? pillEl.textContent : '';
  var form = document.getElementById('ob-form');
  var stepsEls = document.querySelectorAll('.ob-step');
  var pips = document.querySelectorAll('.ob-pip');
  var back = document.getElementById('ob-back');
  var next = document.getElementById('ob-next');
  var skip = document.getElementById('ob-skip');
  var cur = document.getElementById('ob-cur');

  var announce = document.getElementById('ob-announce');
  var NAMES = ['Welcome', 'Model', 'Location', 'Microphone', 'Accuracy', 'Alerts', 'Done'];

  // "Use the defaults and finish" finishes with both password boxes as they are:
  // blank keeps the station open (the dashboard keeps saying so), anything typed
  // must be long enough and typed twice, and is saved. Say which, beside the button.
  var skipNote = document.getElementById('ob-skip-note');
  var pw1 = document.getElementById('ob-password-1');
  var pw2 = document.getElementById('ob-password-2');
  function refreshSkipNote() {
    if (!skipNote) { return; }
    var show = step === 1 && pw1;
    skipNote.hidden = !show;
    if (!show) { return; }
    skipNote.textContent = (pw1.value || (pw2 && pw2.value))
      ? 'Finishing now saves the password you typed.'
      : 'Finishing now with no password leaves this station open to anyone who can reach it. Set one any time under Settings \u2192 Access.';
  }
  if (pw1) { pw1.addEventListener('input', refreshSkipNote); }
  if (pw2) { pw2.addEventListener('input', refreshSkipNote); }

  function render(moveFocus) {
    stepsEls.forEach(function (s) {
      var on = +s.dataset.step === step;
      s.classList.toggle('active', on);
      // A hidden step's fields stayed in the tab order in browsers that keep
      // focusability across `display:none` ancestors only when the step is
      // shown again; `inert` also stops a stale Enter reaching them.
      if (on) { s.removeAttribute('aria-hidden'); } else { s.setAttribute('aria-hidden', 'true'); }
    });
    pips.forEach(function (p) {
      var n = +p.dataset.pip;
      p.classList.toggle('active', n === step);
      p.classList.toggle('done', n < step);
      // The current step is the one thing the stepper has to convey; it was
      // a CSS class only.
      if (n === step) { p.setAttribute('aria-current', 'step'); }
      else { p.removeAttribute('aria-current'); }
    });
    cur.textContent = step;
    refreshSkipNote();
    back.style.visibility = step === 1 ? 'hidden' : 'visible';
    next.textContent = step === total ? 'Finish & save →' : 'Continue →';
    // Step changes were entirely silent to assistive tech: the content of the
    // page swapped with nothing announced.
    if (announce) { announce.textContent = 'Step ' + step + ' of ' + total + ': ' + (NAMES[step - 1] || ''); }
    // And focus stayed where it was — on the nav, which sits *after* the stage
    // in DOM order, so reaching the first field of the new step meant
    // Shift+Tab backwards past everything. Move it to the step itself.
    if (moveFocus) {
      var active = document.querySelector('.ob-step.active');
      if (active) {
        active.setAttribute('tabindex', '-1');
        // `preventScroll` matters: a step taller than the viewport gets
        // scrolled into view by the default focus behaviour, which on a phone
        // put the step's eyebrow at y = -6 and its <h1> at y = 18, both under
        // the sticky stepper that occupies 0..52. A new step starts at the
        // top of the page, which is also where its heading is.
        active.focus({ preventScroll: true });
      }
    }
    window.scrollTo(0, 0);
    // Auto-detect fills the coordinate inputs programmatically, which fires no
    // `input` event — so recompute here too, or the summary would still read
    // "Not set" for a station that just detected its location.
    refreshSummary();
  }
  // `requestSubmit` is Safari 16+; without the fallback, Finish threw a
  // TypeError and did nothing visible on an older iPad.
  function finish() {
    if (form.requestSubmit) { form.requestSubmit(); } else { form.submit(); }
  }

  back.addEventListener('click', function () { if (step > 1) { step--; render(true); } });
  // The password block is rendered only on a station that has none. Both
  // fields blank means "not now" (the station stays open and the dashboard
  // keeps saying so); anything typed must be long enough and typed twice.
  var HINT_DEFAULT = (document.getElementById('ob-password-hint') || {}).textContent || '';
  function passwordOk() {
    var a = document.getElementById('ob-password-1');
    var b = document.getElementById('ob-password-2');
    if (!a || !b) { return true; }
    if (!a.value && !b.value) { return true; }
    var hint = document.getElementById('ob-password-hint');
    var bad = a.value.length < 10 || a.value !== b.value;
    if (hint) {
      // The useful hint used to be overwritten and never put back, and the
      // replacement was muted meta text with no role and nothing pointing at
      // it, so the refusal was neither seen nor announced.
      hint.textContent = bad
        ? (a.value.length < 10 ? 'At least 10 characters.' : 'The two passwords do not match.')
        : HINT_DEFAULT;
      hint.classList.toggle('ob-password-error', bad);
      hint.setAttribute('role', bad ? 'alert' : 'note');
    }
    a.setAttribute('aria-invalid', bad ? 'true' : 'false');
    b.setAttribute('aria-invalid', bad ? 'true' : 'false');
    if (bad) { (a.value.length < 10 ? a : b).focus(); return false; }
    return true;
  }
  next.addEventListener('click', function (e) {
    e.preventDefault();
    if (step === 1 && !passwordOk()) { return; }
    // Coordinates the server will refuse used to be accepted here, echoed back
    // in the step-6 summary, and then dropped with nothing said.
    if (step === 3 && !markCoords()) {
      var bad = document.getElementById('ob-lat');
      if (bad) { bad.focus(); }
      return;
    }
    if (step < total) { step++; render(true); } else { finish(); }
  });
  // This control completes setup — it writes onboarding_complete and the
  // wizard is not offered again — so it asks first rather than relying on its
  // label alone.
  skip.addEventListener('click', function (e) {
    e.preventDefault();
    if (!passwordOk()) { return; }
    if (window.confirm('Finish setup now with the values shown? You can change all of them later under Settings.')) { finish(); }
  });

  // Single-select radio cards; mirror the chosen value into the form's hidden
  // input for that group. Keyed by data-radio so a new group only needs an
  // entry here — the confidence group was added this way.
  var mirrors = { notify: 'ob-notify', conf: 'ob-conf' };

  // Select the card matching what the station already has. The `sel` class
  // used to be baked into the "Balanced 0.75" and "Rarely heard lately"
  // cards, which told an operator with a different setting that they had
  // chosen the default — and, because the mirrored hidden input carried that
  // same hardcoded value, completing setup then wrote it over theirs. The
  // hidden inputs are now rendered server-side from the settings table and are
  // the single source of truth; the cards follow them.
  //
  // A stored value with no matching card (a threshold hand-set to 0.55, say)
  // deliberately leaves every card unselected rather than highlighting a wrong
  // one. The hidden input still holds the real value, so finishing the wizard
  // preserves it.
  Object.keys(mirrors).forEach(function (group) {
    var input = document.getElementById(mirrors[group]);
    if (!input) { return; }
    document.querySelectorAll('[data-radio="' + group + '"]').forEach(function (c) {
      // Numbers compare as numbers (`0.7` and `0.70` are one setting); the
      // notification group's words only ever match as strings.
      if (c.dataset.value === input.value || (c.dataset.value !== '' && input.value !== '' && Number(c.dataset.value) === Number(input.value))) {
        c.classList.add('sel');
        var pick = c.querySelector('.ob-pick');
        if (pick) { pick.checked = true; }
      }
    });
  });

  // Each card is a <label> around a real radio input (UX-1), so Tab reaches
  // the group and the arrow keys move within it. A click on the card and an
  // arrow key both end in the input's `change`, so one handler serves both;
  // the delegated click handler this replaces was unreachable without a
  // mouse. The radios' own names (`conf_card`, `notify_card`) are not what
  // the server reads: the mirrored hidden input still carries the value, so
  // a station whose current setting matches no card keeps it.
  document.querySelectorAll('[data-radio] .ob-pick').forEach(function (pick) {
    pick.addEventListener('change', function () {
      var card = pick.closest('[data-radio]');
      if (!card || !pick.checked) { return; }
      document.querySelectorAll('[data-radio="' + card.dataset.radio + '"]').forEach(function (c) {
        c.classList.remove('sel');
      });
      card.classList.add('sel');
      var target = mirrors[card.dataset.radio];
      var input = target && document.getElementById(target);
      if (input && card.dataset.value) { input.value = card.dataset.value; }
      refreshSummary();
    });
  });

  // Keep the final step's summary card describing THIS station rather than a
  // mock-up. The microphone row is filled server-side from the real capture
  // sources; the rest is whatever the operator has entered so far, so it is
  // recomputed on every change and again on the way into the last step.
  function cardName(sel) {
    var c = document.querySelector(sel + '.sel .t');
    return c ? c.textContent.trim().replace(/\s*recommended$/, '') : '';
  }
  function refreshSummary() {
    var conf = document.getElementById('ob-conf');
    var sumConf = document.getElementById('ob-sum-conf');
    if (conf && sumConf) {
      var name = cardName('[data-radio="conf"]');
      var n = parseFloat(conf.value);
      var shown = isNaN(n) ? (conf.value || '').trim() : n.toFixed(2);
      sumConf.textContent = (shown || 'Unchanged') + (name ? ' · ' + name : '');
    }
    var sumNotify = document.getElementById('ob-sum-notify');
    if (sumNotify) {
      // The markup's fallback used to read "Rare birds only" — a phrase that
      // appears nowhere else in the product and describes none of the three
      // modes. It showed whenever the stored trigger matched no card, which is
      // reachable: TriggerMode::parse also accepts the underscore spellings, so
      // `APPRISE_TRIGGER=new_species` in birdnet.conf selects nothing here.
      // Fall back to the value that is actually going to be kept.
      var nm = cardName('[data-radio="notify"]');
      var notifyInput = document.getElementById('ob-notify');
      var stored = notifyInput ? (notifyInput.value || '').trim() : '';
      sumNotify.textContent = nm || (stored ? 'Unchanged (' + stored + ')' : 'Unchanged');
    }
    var sumLoc = document.getElementById('ob-sum-loc');
    if (sumLoc) {
      var la = (document.getElementById('ob-lat') || {}).value;
      var lo = (document.getElementById('ob-lon') || {}).value;
      la = (la || '').trim(); lo = (lo || '').trim();
      // Both halves are required — one alone disables the species filter just
      // as completely as neither, so half a location is not a location.
      sumLoc.textContent = (la && lo)
        ? (coordsValid(la, lo) ? la + ', ' + lo : 'Not valid — this will not be saved')
        : 'Not set — species filtering stays off';
    }
    var sumMic = document.getElementById('ob-sum-mic');
    if (sumMic) {
      // What the station will listen with after Finish: the choice made in the
      // microphone step, or — with nothing chosen there, a station that already
      // had sources — the line the server rendered.
      var text = sumMic.getAttribute('data-static') || '';
      var picked = document.querySelector('input[name="mic_choice"]:checked');
      if (picked) {
        var v = picked.value;
        var title = picked.closest('label');
        title = title && title.querySelector('.t');
        if (v === 'none') {
          text = 'None — nothing will be detected until one is added';
        } else if (v === 'rtsp') {
          var u = ((document.getElementById('mic-rtsp-url') || {}).value || '').trim();
          // A camera's password is part of its address; keep it off the summary.
          text = u ? 'Network stream · ' + u.replace(/\/\/[^@\/]*@/, '//…@') : 'Network stream — no address entered yet';
        } else if (v === 'manual') {
          var d = ((document.getElementById('mic-manual-dev') || {}).value || '').trim();
          text = d ? 'Sound card · ' + d : 'Sound card — no name entered yet';
        } else if (v.indexOf('alsa:') === 0) {
          text = (title ? title.textContent.trim() : 'Sound card') + ' · ' + v.slice(5);
        }
      }
      sumMic.textContent = text;
    }
    var sumUrl = document.getElementById('ob-sum-url');
    // The address the operator actually reached this page on. The hard-coded
    // mDNS URL this replaced does not resolve on every network.
    // Under BIRDNET_BASE_PATH this printed an address that 404s: the base-path
    // rewriter only touches quoted attribute values, so a URL built in script
    // has to read the prefix itself, the way live-detections.js and today.html
    // already do.
    if (sumUrl) { sumUrl.textContent = window.location.origin + BASE + '/'; }
  }
  // Mirrors the server's `valid_coordinate`. Without this the summary happily
  // printed "51.48, abc" under the heading that claimed the answers were saved,
  // and the server then dropped the pair with only a tracing::warn to show for
  // it — the operator was never told.
  function coordsValid(la, lo) {
    var a = parseFloat(la), o = parseFloat(lo);
    return isFinite(a) && isFinite(o) && Math.abs(a) <= 90 && Math.abs(o) <= 180
        && /^[+-]?\d*\.?\d+$/.test(la) && /^[+-]?\d*\.?\d+$/.test(lo);
  }
  function markCoords() {
    var la = document.getElementById('ob-lat'), lo = document.getElementById('ob-lon');
    if (!la || !lo) { return true; }
    var a = (la.value || '').trim(), o = (lo.value || '').trim();
    var ok = (!a && !o) || coordsValid(a, o);
    la.setAttribute('aria-invalid', ok ? 'false' : 'true');
    lo.setAttribute('aria-invalid', ok ? 'false' : 'true');
    if (pillEl) {
      pillEl.textContent = ok
        ? PILL_DEFAULT
        : 'Latitude must be between -90 and 90 and longitude between -180 and 180, or leave both blank.';
      pillEl.classList.toggle('ob-password-error', !ok);
    }
    return ok;
  }
  ['ob-lat', 'ob-lon'].forEach(function (id) {
    var el = document.getElementById(id);
    if (el) { el.addEventListener('input', function () { markCoords(); refreshSummary(); }); }
  });
  refreshSummary();

  // Auto-detect coordinates + timezone via the existing settings endpoint
  // (same-origin fetch; the endpoint itself queries ip-api.com server-side).
  var detect = document.getElementById('ob-detect');
  var pill = pillEl;
  if (detect) {
    detect.addEventListener('click', function () {
      var prev = detect.textContent;
      detect.disabled = true;
      detect.textContent = 'Detecting…';
      fetch(BASE + '/admin/settings/detect-location')
        .then(function (r) { if (!r.ok) { throw new Error('lookup failed'); } return r.json(); })
        .then(function (d) {
          if (d.lat != null) { document.getElementById('ob-lat').value = d.lat; }
          if (d.lon != null) { document.getElementById('ob-lon').value = d.lon; }
          if (d.timezone) { document.getElementById('ob-tz').value = d.timezone; }
          if (pill) {
            var where = [d.city, d.country].filter(Boolean).join(', ');
            pill.textContent = '✓ ' + (where || 'Location found') + (d.timezone ? ' · ' + d.timezone : '');
            pill.classList.add('moss');
          }
        })
        .catch(function () {
          if (pill) { pill.textContent = 'Could not auto-detect — enter your coordinates manually.'; }
        })
        .finally(function () { detect.disabled = false; detect.textContent = prev; });
    });
  }

  // ---- Microphone step ---------------------------------------------------
  // The cards are real radio buttons; choosing one shows the extra field it
  // needs (a stream address, a device name) and updates the summary.
  function micChanged() {
    var chosen = document.querySelector('input[name="mic_choice"]:checked');
    document.querySelectorAll('[data-mic-card]').forEach(function (c) {
      var r = c.querySelector('input[name="mic_choice"]');
      c.classList.toggle('sel', !!(r && r.checked));
    });
    var v = chosen ? chosen.value : '';
    var rtsp = document.getElementById('mic-rtsp-field');
    if (rtsp) { rtsp.hidden = v !== 'rtsp'; }
    var manual = document.getElementById('mic-manual-field');
    if (manual) { manual.hidden = v !== 'manual'; }
    refreshSummary();
  }
  document.querySelectorAll('input[name="mic_choice"]').forEach(function (r) {
    r.addEventListener('change', micChanged);
  });
  ['mic-rtsp-url', 'mic-manual-dev'].forEach(function (id) {
    var el = document.getElementById(id);
    if (el) { el.addEventListener('input', refreshSummary); }
  });
  // "Test": record two seconds from that sound card and say how loud it was.
  document.querySelectorAll('[data-mic-test]').forEach(function (btn) {
    btn.addEventListener('click', function (e) {
      e.preventDefault();
      e.stopPropagation();
      var out = btn.parentElement.querySelector('[data-mic-level]');
      btn.disabled = true;
      if (out) { out.textContent = 'Listening for two seconds…'; }
      fetch(BASE + '/onboarding/mic-test', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: 'device=' + encodeURIComponent(btn.getAttribute('data-mic-test'))
      })
        .then(function (r) { return r.json(); })
        .then(function (d) {
          if (out) { out.textContent = d.ok ? (d.rms_dbfs + ' dB — ' + d.message) : d.message; }
        })
        .catch(function () { if (out) { out.textContent = 'The test did not reach the station.'; } })
        .finally(function () { btn.disabled = false; });
    });
  });
  micChanged();

  // ---- Model step --------------------------------------------------------
  // The summary says what the panel says, in the panel's own words for the
  // state, and keeps saying it as the download proceeds.
  var MODEL_WORDS = {
    'ready': 'Installed and running',
    'needs-restart': 'Installed — starts after the restart',
    'installing': 'Downloading…',
    'missing': 'Not installed — nothing will be detected',
    'failed': 'Install failed — nothing will be detected',
    'stopped': 'Stopped',
    'web-only': 'Not used (this station shows pages only)',
    'configured': 'Set in the configuration file',
    'unavailable': 'Not available'
  };
  function modelState(st) {
    var el = document.getElementById('ob-sum-model');
    if (el) { el.textContent = MODEL_WORDS[st] || st; }
  }
  document.addEventListener('cls:state', function (e) { modelState(e.detail && e.detail.state); });
  var firstPanel = document.querySelector('[data-cls-panel]');
  if (firstPanel) { modelState(firstPanel.getAttribute('data-cls-state')); }

  // Reduced motion. The stylesheet's `@media (prefers-reduced-motion: reduce)`
  // block sets `animation: none` on `.sonar *`, which has no effect on the
  // Welcome sonar: those rings are SMIL <animate> elements, not CSS
  // animations. And the block never saw the app's own Display preference,
  // because theme-guard.js — the only script this page loads — mirrors theme
  // and density but not `bnb-motion`. Honour both here.
  try {
    var reduce = (window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches)
      || localStorage.getItem('bnb-motion') === 'reduced';
    if (reduce) {
      document.documentElement.setAttribute('data-motion', 'reduced');
      document.querySelectorAll('animate, animateTransform, animateMotion').forEach(function (a) {
        if (a.endElement) { try { a.endElement(); } catch (_) {} }
        a.setAttribute('dur', 'indefinite');
      });
    }
  } catch (_) { /* storage blocked or no matchMedia — motion stays as declared */ }

  render(false);
})();
</script>
</body>
</html>"##,
);

/// The page after Finish. Same chrome as the wizard, one screen.
static ONBOARDING_DONE_HTML: crate::seal::Sealed = crate::seal::Sealed::new(
    r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BirdNet-Behavior · Setup saved</title>
<link rel="stylesheet" href="/static/css/app.css?v={{version}}">
<script src="/static/theme-guard.js?v={{version}}"></script>
<style>
  body { margin:0; background:var(--bg); color:var(--fg); min-height:100vh; }
  .ob-root { max-width:980px; margin:0 auto; min-height:100vh; display:flex; flex-direction:column; padding:0 24px; }
  .ob-stage { flex:1; display:flex; align-items:center; padding:24px 0; }
  .ob-step { width:100%; }
  .ob-two { display:grid; grid-template-columns:1fr 1fr; gap:32px; align-items:start; }
  .ob-eyebrow { font-size:10.5px; letter-spacing:0.10em; text-transform:uppercase; color:var(--fg-3); font-weight:500; }
  .ob-h { font-family:var(--font-display); font-size:46px; line-height:1.08; letter-spacing:-0.02em; margin:8px 0 12px; }
  .ob-h em { font-style:italic; color:var(--moss-ink); }
  .ob-p { color:var(--fg-2); font-size:15px; max-width:42ch; }
  .bnb-meta a, .ob-p a { text-decoration:underline; text-underline-offset:2px; }
  .ob-mt-16 { margin-top:16px; }
  .ob-password-error { color:var(--rare); background:var(--rare-soft); border:0.5px solid var(--rare); border-radius:var(--r-sm); padding:8px 12px; margin:10px 0 0; font-size:13px; }
  @media (max-width:520px) {
    .ob-root { padding:0 14px; }
    .ob-two { grid-template-columns:1fr; gap:18px; }
    .ob-h { font-size:32px; }
  }
</style>
</head>
<body>
<main class="ob-root" id="main-content">
  <div class="ob-stage">
    <section class="ob-step">
      <div class="ob-two">
        <div>
          <div class="ob-eyebrow">{{eyebrow}}</div>
          <h1 class="ob-h">{{heading}}</h1>
          {{restart_block}}
          <p class="ob-mt-16"><a class="bnb-btn" href="/">Go to the dashboard →</a></p>
        </div>
        <div>
          <div class="bnb-card pad">
            <div class="ob-eyebrow">Bird-identifying model</div>
            <div class="ob-mt-16" data-cls-host>{{model_panel}}</div>
          </div>
          <div class="bnb-card pad ob-mt-16">
            <div class="ob-eyebrow">Microphone</div>
            <div class="ob-mt-16">{{microphone}}</div>
          </div>
        </div>
      </div>
    </section>
  </div>
</main>
{{setup_script}}
</body>
</html>"#,
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Both of the wizard's style blocks (the steps, and the done page) carry
    /// their own rules, because neither loads the app stylesheet. A link inside
    /// a sentence must be underlined in each, or axe's `link-in-text-block`
    /// fails on whichever page lacks the rule: it did, on the steps, then on the
    /// done page once that gained a link.
    #[test]
    fn every_wizard_style_block_underlines_links_inside_sentences() {
        let src = include_str!("onboarding.rs");
        let src = src.split("#[cfg(test)]").next().unwrap_or(src);
        let blocks = src.matches("  .ob-p { color:var(--fg-2);").count();
        let underlined = src
            .matches(".bnb-meta a, .ob-p a { text-decoration:underline;")
            .count();
        assert_eq!(
            blocks, 2,
            "the wizard has two style blocks; update this test if that changed"
        );
        assert_eq!(
            underlined, blocks,
            "each style block needs the in-text link rule"
        );
    }

    /// The page script shows a field by clearing its `hidden` attribute. A class
    /// rule that sets `display` outranks the browser's own `[hidden]` rule, so
    /// such an element needs its own `.class[hidden]` rule or it is on screen
    /// all along — the stream address and the device name were, before the
    /// matching card was chosen, and no test of the markup could see it.
    #[test]
    fn an_element_the_page_hides_is_hidden_by_the_stylesheet() {
        let css = include_str!("../../../static/css/app.css");
        let markup = include_str!("onboarding.rs");
        let markup = markup.split("#[cfg(test)]").next().unwrap_or(markup);
        let mut checked = 0;
        for (at, _) in markup.match_indices("class=\"") {
            let rest = &markup[at + 7..];
            let Some(quote) = rest.find('"') else {
                continue;
            };
            let tag_end = rest.find('>').unwrap_or(rest.len());
            if !rest[quote..tag_end].contains(" hidden") {
                continue;
            }
            for class in rest[..quote].split_whitespace() {
                let rule = format!(".{class} {{");
                let sets_display = css.match_indices(&rule).any(|(i, _)| {
                    css[i..]
                        .split('}')
                        .next()
                        .is_some_and(|body| body.contains("display:"))
                });
                if sets_display {
                    checked += 1;
                    assert!(
                        css.contains(&format!(".{class}[hidden]")),
                        ".{class} sets display and is rendered hidden, but app.css has no .{class}[hidden] rule"
                    );
                }
            }
        }
        assert!(
            checked > 0,
            "the scan found no hidden element whose class sets display: it is not looking at the markup"
        );
    }

    /// The wizard is the third place a person can type a confidence or a
    /// coordinate, and it keeps its own copy of the bounds. It may be stricter
    /// than `birdnet_core::config::validate` (it does not read a comma decimal
    /// for the confidence) but never looser: a value it stores is a value the
    /// validator would have accepted in the config file.
    #[test]
    fn the_wizard_never_accepts_what_the_validator_refuses() {
        use birdnet_core::config::validate::{Severity, validate};
        let refused = |key: &str, v: &str| {
            let config = birdnet_core::config::Config::parse(&format!("{key}={v}"))
                .expect("a key=value line parses");
            validate(&config)
                .iter()
                .any(|f| f.key == key && f.severity == Severity::Error)
        };
        let probes = [
            "0",
            "1",
            "0.7",
            "0.0",
            "1.0",
            "1.0000001",
            "-0.0000001",
            "75",
            "100",
            "-1",
            "abc",
            "NaN",
            "inf",
            "-inf",
            "1e0",
            "1e1",
            "1e-1",
            "0,7",
            "1,5",
            "1,2,3",
            ".5",
            "5.",
            "+0.5",
            "90",
            "90.0000001",
            "-90",
            "-90.5",
            "180",
            "-180",
            "180.0000001",
            "-181",
            "42,36",
            "0x10",
            "",
        ];
        for v in probes {
            if valid_confidence(v) {
                assert!(
                    !refused("CONFIDENCE", v),
                    "the wizard stores a confidence of {v:?} that the validator refuses"
                );
            }
            if valid_coordinate(v, LATITUDE_LIMIT).is_some() {
                assert!(
                    !refused("LATITUDE", v),
                    "the wizard stores a latitude of {v:?} that the validator refuses"
                );
            }
            if valid_coordinate(v, LONGITUDE_LIMIT).is_some() {
                assert!(
                    !refused("LONGITUDE", v),
                    "the wizard stores a longitude of {v:?} that the validator refuses"
                );
            }
        }
        // The counterparts, so "never looser" is not satisfied by a wizard that
        // refuses everything: the ends of each range are real answers...
        for v in ["0", "1", "0.7"] {
            assert!(valid_confidence(v), "{v}");
        }
        assert!(
            valid_coordinate("90", LATITUDE_LIMIT).is_some()
                && valid_coordinate("-90", LATITUDE_LIMIT).is_some()
        );
        assert!(
            valid_coordinate("180", LONGITUDE_LIMIT).is_some()
                && valid_coordinate("-180", LONGITUDE_LIMIT).is_some()
        );
        assert_eq!(
            valid_coordinate("42,36", LATITUDE_LIMIT).as_deref(),
            Some("42.36")
        );
        // ...and one past the end is not.
        assert!(!valid_confidence("1.0000001") && !valid_confidence("75"));
        assert!(valid_coordinate("90.0000001", LATITUDE_LIMIT).is_none());
        assert!(valid_coordinate("180.0000001", LONGITUDE_LIMIT).is_none());
        // Stricter, on purpose and in one place: the confidence has no comma form.
        assert!(!valid_confidence("0,7") && !refused("CONFIDENCE", "0,7"));
    }

    fn prefill() -> Prefill {
        Prefill {
            latitude: "51.48".into(),
            longitude: "-0.13".into(),
            station_name: "Back garden".into(),
            database_lang: String::new(),
            confidence: "0.75".into(),
            notify_trigger: "new-species".into(),
        }
    }

    fn parts(mic_body: &str, mic_summary: &str) -> PageParts {
        PageParts {
            mic_body: mic_body.to_owned(),
            mic_summary: mic_summary.to_owned(),
            model_panel: "MODEL".to_owned(),
            lang_options: "<option value=\"en\">English</option>".to_owned(),
            start_step: 1,
        }
    }

    /// A value substituted into the template must never be re-read as template.
    ///
    /// Observed failing against the pre-fix `.replace()` chain, which
    /// substituted `{{mic_body}}` and `{{mic_summary}}` first and therefore
    /// re-scanned them: a source labelled `{{password_step}}` had the entire
    /// password `<form>` spliced into the microphone card by a later link in
    /// the chain, and a source labelled `{{latitude}}` became the station's
    /// coordinates. Audio-source labels are operator-controlled text, so this
    /// was reachable from the admin UI. The failure was
    /// `assert!(body.contains("{{password_step}}"))` finding the rendered form
    /// instead of the literal.
    #[test]
    fn a_placeholder_inside_a_substituted_value_is_not_re_substituted() {
        // Every placeholder name, used as a microphone label — the one field
        // whose text an operator controls.
        for name in [
            "mic_summary",
            "password_step",
            "latitude",
            "longitude",
            "confidence",
            "notify_trigger",
            "version",
            "model_panel",
            "lang_options",
            "station_name",
            "start_step",
            "setup_script",
        ] {
            let label = format!("{{{{{name}}}}}");
            let html = render_page(
                &parts(&label, &label),
                &prefill(),
                PasswordStep {
                    needed: true,
                    error: false,
                },
            );
            assert!(
                html.matches(&label).count() >= 2,
                "a label of {label} was substituted away instead of being emitted twice \
                 (once in the body, once in the summary)"
            );
        }
    }

    /// The counterpart: real placeholders in the *template* must still be
    /// filled, so the gate above cannot be satisfied by substituting nothing.
    #[test]
    fn every_template_placeholder_is_filled() {
        let html = render_page(
            &parts("MIC", "SUMMARY"),
            &prefill(),
            PasswordStep {
                needed: true,
                error: false,
            },
        );
        for leftover in [
            "{{mic_body}}",
            "{{mic_summary}}",
            "{{latitude}}",
            "{{longitude}}",
            "{{confidence}}",
            "{{notify_trigger}}",
            "{{password_step}}",
            "{{version}}",
            "{{model_panel}}",
            "{{lang_options}}",
            "{{station_name}}",
            "{{start_step}}",
            "{{setup_script}}",
        ] {
            assert!(
                !html.contains(leftover),
                "{leftover} was left unsubstituted in the rendered wizard"
            );
        }
        assert!(html.contains("MIC"), "the microphone body reached the page");
        assert!(
            html.contains("51.48"),
            "the stored latitude reached the page"
        );
        assert!(
            html.contains(env!("CARGO_PKG_VERSION")),
            "the asset version reached the page"
        );
    }

    /// An unknown placeholder survives rather than eating the text around it.
    #[test]
    fn an_unknown_placeholder_is_passed_through() {
        let html = render_page(
            &parts("{{not_a_real_key}}", "S"),
            &prefill(),
            PasswordStep {
                needed: true,
                error: false,
            },
        );
        assert!(
            html.contains("{{not_a_real_key}}"),
            "unknown key was dropped"
        );
    }
    /// A camera address is `rtsp://user:password@host/…`, and the wizard adds a
    /// stream without a label, so the address is the card's heading and the
    /// summary's name. Neither may print the password; both keep the host.
    #[test]
    fn the_wizard_does_not_print_a_cameras_password() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let secret = format!("camsecret{}", std::process::id());
        let url = format!("rtsp://viewer:{secret}@camera.local:554/stream1");
        conn.insert(&NewAudioSource::defaults(
            "unlabelled",
            SourceKind::Rtsp,
            url.clone(),
        ))
        .unwrap();
        let mut labelled = NewAudioSource::defaults("labelled", SourceKind::Rtsp, url);
        labelled.label = Some("Garden camera".to_owned());
        conn.insert(&labelled).unwrap();

        let all = AudioSourceStore::list(&conn).unwrap();
        let body = render_mic_body(&all, &[], None, None);
        assert!(
            !body.contains(&secret),
            "the card printed the password: {body}"
        );
        assert!(
            body.contains("camera.local"),
            "the host that makes the camera recognisable is kept: {body}"
        );
        for one in &all {
            let summary = mic_summary(std::slice::from_ref(one));
            assert!(!summary.contains(&secret), "{}: {summary}", one.id);
        }
        assert!(
            mic_summary(&all[..1]).contains("camera.local"),
            "an unlabelled stream is still named by its host"
        );
    }

    /// Two first-run submissions that both saw "no password": exactly one
    /// writes. Before, both did, the last silently replacing the first — whose
    /// session had already been minted and then pointed at a password nobody
    /// would ever type.
    #[test]
    fn two_first_run_submissions_do_not_both_set_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(dir.path().join("birds.db")).unwrap();
        let pid = std::process::id();
        let passwords = [format!("first-owner-{pid}"), format!("second-owner-{pid}")];
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        // Collected on purpose: both threads must be running before either is joined.
        #[allow(clippy::needless_collect)]
        let handles: Vec<_> = passwords
            .iter()
            .cloned()
            .map(|pw| {
                let (state, barrier) = (state.clone(), std::sync::Arc::clone(&barrier));
                std::thread::spawn(move || {
                    barrier.wait();
                    set_first_admin_password(&state, &pw).map(|_| pw)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let winners: Vec<&String> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        assert_eq!(
            winners.len(),
            1,
            "exactly one submission sets it: {results:?}"
        );
        assert!(
            results
                .iter()
                .any(|r| matches!(r, Err(FirstPasswordError::AlreadySet))),
            "the other is told it lost: {results:?}"
        );
        let stored = state
            .with_read_db(|c| c.find_user_by_name("admin"))
            .unwrap()
            .pwd_argon2;
        assert!(
            accounts::verify_password(&stored, winners[0]).unwrap(),
            "the stored password is the winner's"
        );
        // The counterpart to the race: with a password set, a later submission
        // does not replace it either.
        let later = format!("third-owner-{pid}");
        assert!(matches!(
            set_first_admin_password(&state, &later),
            Err(FirstPasswordError::AlreadySet)
        ));
    }
}
