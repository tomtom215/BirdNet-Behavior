//! The classifier panel: is the station able to identify birds, and if not, one
//! button to fix it.
//!
//! Shown in two places that must agree, so they share one renderer: the setup
//! wizard's "Species model" step and the Settings → Capture tab. Both poll
//! `GET /admin/classifier/panel` (see `static/classifier-panel.js`), which
//! returns the panel's own markup, so the words live here once and the script
//! only swaps them in.
//!
//! The panel reports the truth about the *running* station: "Listening" only
//! when the detection engine is up with a classifier. Files on disk that this
//! start did not load are "Installed — restart to start listening", not green.

use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::{Html, IntoResponse as _, Response};
use axum::routing::{get, post};

use crate::classifier_setup::{Phase, Snapshot, StartError};
use crate::routes::pages::escape_html;
use crate::state::AppState;

/// The panel and its install action, behind the admin gate.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/classifier/panel", get(panel))
        .route("/admin/classifier/install", post(install))
        .route("/admin/system/boot-id", get(boot))
}

/// `GET /admin/system/boot-id` — which process is answering.
///
/// A restart is "done" when a different process answers, and the old process
/// goes on answering for a moment after the button is pressed, so the page
/// compares this with what it saw before.
async fn boot() -> Response {
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
        ],
        boot_id(),
    )
        .into_response()
}

/// The script that drives the panel and the restart buttons.
pub(crate) static CONTROLS_SCRIPT: crate::seal::Sealed = crate::seal::Sealed::new(
    r#"<script src="/static/setup-controls.js" defer></script>"#,
);

/// The panel inside the element the script swaps it in and out of, with the
/// script. For the Capture tab; the setup wizard embeds the same two parts.
#[must_use]
pub fn host_html(state: &AppState) -> String {
    format!(
        r#"<div class="bnb-card pad" data-cls-host>{panel}</div>{script}"#,
        panel = render_panel(
            &state
                .classifier_setup()
                .snapshot(state.detection_daemon_running())
        ),
        script = CONTROLS_SCRIPT.as_str(),
    )
}

/// `GET /admin/classifier/panel` — the current panel, as markup.
async fn panel(State(state): State<AppState>) -> Response {
    no_store(Html(render_panel(
        &state
            .classifier_setup()
            .snapshot(state.detection_daemon_running()),
    )))
}

/// `POST /admin/classifier/install` — start installing, and return the panel.
///
/// Answers `200` with the panel in every case, including a refusal, because
/// htmx never swaps the body of a `4xx` and the script that calls this would
/// otherwise show nothing at all when the click was refused.
async fn install(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
) -> Response {
    state
        .run_blocking(move |state| {
            let setup = state.classifier_setup();
            let started = setup.begin_install(crate::classifier_setup::install_from_catalogue);
            let note = match started {
                Ok(()) => {
                    crate::audit::audit(
                        state,
                        Some(&request_user),
                        "classifier.install",
                        Some(birdnet_integrations::model_catalog::DEFAULT_CLASSIFIER_ID),
                        None,
                    );
                    None
                }
                Err(StartError::AlreadyRunning) => None,
                Err(e) => Some(refusal(e)),
            };
            let mut html = render_panel(&setup.snapshot(state.detection_daemon_running()));
            if let Some(note) = note {
                html.push_str(&format!(
                    r#"<p class="cls-note" role="alert">{}</p>"#,
                    escape_html(note)
                ));
            }
            no_store(Html(html))
        })
        .await
}

/// What a refused install says, in words for the person who clicked.
const fn refusal(e: StartError) -> &'static str {
    match e {
        StartError::WebOnly => {
            "This station was started without its detection engine, so installing a classifier would not change anything."
        }
        StartError::NotNeeded => {
            "A classifier is already running or named in the configuration, so nothing was installed."
        }
        StartError::NoLayout | StartError::NoCatalogue => {
            "This station cannot say where to put a classifier, so nothing was installed."
        }
        StartError::Thread => "The download could not be started. Try again.",
        StartError::AlreadyRunning => "An install is already running.",
    }
}

fn no_store(html: Html<String>) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], html).into_response()
}

/// The panel's markup for one snapshot.
///
/// The root carries `data-cls-state`, so the script knows when to poll.
#[must_use]
pub fn render_panel(snap: &Snapshot) -> String {
    let (tone, pill, headline, detail) = words(snap);
    let mut actions = String::new();
    match snap.phase {
        Phase::Missing | Phase::Failed => {
            let label = if snap.phase == Phase::Failed {
                "Try again".to_owned()
            } else {
                format!("Install ({})", megabytes(snap.download_bytes))
            };
            actions.push_str(&format!(
                r#"<button type="button" class="bnb-btn primary" data-cls-install>{label}</button>"#
            ));
        }
        Phase::NeedsRestart => {
            actions.push_str(
                r#"<button type="button" class="bnb-btn primary" data-cls-restart>Restart the station now</button>"#,
            );
        }
        _ => {}
    }
    let progress = snap.progress.as_ref().map_or_else(String::new, |p| {
        let line = if p.file.is_empty() {
            "Starting the download…".to_owned()
        } else {
            format!(
                "{file} — file {step} of {steps}, {done} of {total}",
                file = escape_html(&p.file),
                step = p.step,
                steps = p.steps,
                done = megabytes(p.done),
                total = megabytes(p.total),
            )
        };
        format!(
            r#"<progress class="cls-bar" max="100" value="{pct}" aria-label="Download progress">{pct}%</progress><p class="bnb-meta" role="status">{line}</p>"#,
            pct = p.percent(),
        )
    });
    format!(
        r#"<div class="cls-panel" data-cls-panel data-cls-state="{state}" data-tone="{tone}">
<div class="cls-head"><span class="bnb-pill {pill_class}">{pill}</span><strong class="cls-name">{headline}</strong></div>
<p class="cls-detail">{detail}</p>{progress}
<div class="cls-actions">{actions}</div>
</div>"#,
        state = snap.phase.as_str(),
        pill_class = match tone {
            "ok" => "moss",
            "warn" => "dawn",
            "bad" => "rare",
            "info" => "info",
            _ => "",
        },
    )
}

/// `(tone, pill, headline, detail)` for a snapshot. All text here is authored
/// or escaped: nothing a visitor sent reaches it, but a download error can
/// contain a URL.
fn words(snap: &Snapshot) -> (&'static str, &'static str, String, String) {
    let name = escape_html(&snap.name);
    let size = megabytes(snap.download_bytes);
    match snap.phase {
        Phase::Unavailable => (
            "muted",
            "Not available",
            "Classifier status is not available in this process".to_owned(),
            String::new(),
        ),
        Phase::Missing => (
            "warn",
            "Not installed",
            "The bird-identifying model is not installed yet".to_owned(),
            format!(
                "Without it the station records nothing. {name} and its local species filter are \
                 {size} together; the station needs an internet connection this once, and every \
                 file is checked against a fingerprint built into the program before it is used."
            ),
        ),
        Phase::Installing => (
            "info",
            "Installing…",
            "Downloading the bird-identifying model".to_owned(),
            "You can leave this page open or come back; the download carries on in the station."
                .to_owned(),
        ),
        Phase::Failed => (
            "bad",
            "Install failed",
            "The model could not be installed".to_owned(),
            format!(
                "{} Nothing half-downloaded is used. Check the station's internet connection and \
                 free disk space, then try again.",
                escape_html(snap.failure.as_deref().unwrap_or("The download did not finish."))
            ),
        ),
        Phase::NeedsRestart => (
            "warn",
            "Installed — restart to start listening",
            format!("{name} is installed"),
            "The station loads its model when it starts, so it is not listening yet. Restart it \
             once and it will begin."
                .to_owned(),
        ),
        Phase::Ready => (
            "ok",
            "Listening",
            format!(
                "Identifying birds with {}",
                escape_html(snap.in_use.as_deref().unwrap_or("the installed model"))
            ),
            if snap.range_filter {
                "The local species filter is on: species that do not occur near the station's \
                 location are held back (it needs the station's coordinates to take effect)."
                    .to_owned()
            } else {
                "The local species filter is off, so every species the model knows stays possible \
                 wherever the station is."
                    .to_owned()
            },
        ),
        Phase::Stopped => (
            "bad",
            "Stopped",
            "The detection engine started and has stopped".to_owned(),
            r#"Nothing is being identified. <a href="/station">Station health</a> says why."#
                .to_owned(),
        ),
        Phase::WebOnly => (
            "muted",
            "Pages only",
            "This station is running without its detection engine".to_owned(),
            "It was started with <code>--web-only</code>, so it shows pages and history but does \
             not listen. Start it without that option to identify birds."
                .to_owned(),
        ),
        Phase::Configured => (
            "warn",
            "Set in the configuration file",
            "This station is set to use its own model file".to_owned(),
            snap.missing_path.as_deref().map_or_else(
                || {
                    r#"The detection engine did not start with it. <a href="/admin/doctor">Diagnostics</a> says why."#
                        .to_owned()
                },
                |p| {
                    format!(
                        "The file <code>{}</code> named in the configuration is not there, so \
                         nothing is being identified. Put it there, or remove the setting.",
                        escape_html(p)
                    )
                },
            ),
        ),
    }
}

/// `"84 MB"`, rounded up so a figure never understates a download.
fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1_000_000))
}

/// Which process rendered a panel: constant for the life of the process and
/// different after a restart.
fn boot_id() -> &'static str {
    use std::sync::OnceLock;
    static BOOT: OnceLock<String> = OnceLock::new();
    BOOT.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        format!("{nanos:x}-{}", std::process::id())
    })
}
